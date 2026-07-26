import { createEmbeddedPiTransport } from "./embedded";
import type {
  PiPermissionRequest,
  PiReasoningLevel,
  PiRunEvent,
  PiRunHandle,
  PiRuntimeTransport,
} from "./types";

type Invoke = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

export type WorkflowModel = {
  providerId?: string;
  model?: string;
  label: string;
};

export type WorkflowRunInput = {
  title: string;
  prompt: string;
  systemPrompt: string;
  cwd: string;
  model: WorkflowModel;
  reasoning?: PiReasoningLevel;
  archiveSession?: boolean;
  onEvent?: (event: PiRunEvent) => void;
  onPermission?: (permission: PiPermissionRequest) => void;
};

export type WorkflowRunResult = {
  sessionId: string;
  requestId: string;
  text: string;
  usage?: Record<string, unknown>;
  metadata?: Record<string, unknown>;
  model: WorkflowModel;
};

type SessionSummary = {
  id?: string;
  cwd?: string;
};

type ActiveWorkflow = {
  transport: PiRuntimeTransport;
  handle: PiRunHandle;
};

type PendingWorkflow = {
  /**
   * A cancellation latch spans setup and execution.  It is deliberately kept
   * after the request reaches `active`, so a cancellation that races a late
   * subscription or provider completion still wins the workflow result.
   */
  cancelled: boolean;
  /**
   * `cancel()` reports newly accepted cancellation requests.  A failed active
   * transport signal returns to `ready` so the caller can retry instead of
   * being told that a request was cancelled when it was not.
   */
  cancellationState: "ready" | "signalling" | "signalled";
};

type CancellationTarget = {
  requestId: string;
  pending: PendingWorkflow;
  active?: ActiveWorkflow;
};

function nativeInvoke(): Invoke {
  const invoke = window.__TAURI__?.core?.invoke as Invoke | undefined;
  if (!invoke) throw new Error("工作流需要 NovaVei 桌面运行时");
  return invoke;
}

function workflowId(prefix: string) {
  const uuid = globalThis.crypto?.randomUUID?.();
  return uuid
    ? `${prefix}-${uuid}`
    : `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

function bounded(value: string, max: number, field: string) {
  const normalized = value.trim();
  if (!normalized) throw new Error(`${field}不能为空`);
  if (normalized.length > max) throw new Error(`${field}超过 ${max} 个字符`);
  return normalized;
}

function workflowCancelledError(): DOMException {
  return new DOMException("工作流已取消", "AbortError");
}

/**
 * Runs detached Compare/Council turns through the same embedded Pi adapter as
 * the main chat. Each run gets an ordinary native history session so provider
 * selection, cancellation, event ordering, and recovery retain one contract.
 */
export class PiWorkflowRunner {
  private readonly active = new Map<string, ActiveWorkflow>();
  // A workflow needs a durable native session before it can enter `active`.
  // Keep that setup interval cancelable too: a user may press Cancel while
  // `sessions_create` is in flight, and that must prevent a later provider
  // request rather than merely resolving an empty active-map cancellation.
  private readonly pending = new Map<string, PendingWorkflow>();

  get activeRequestIds() {
    // `pending` is the lifecycle registry: active entries are always a
    // subset of it.  Keeping one authority avoids a transient duplicate or
    // an active request disappearing from cancellation accounting early.
    return [...this.pending.keys()];
  }

  async run(input: WorkflowRunInput): Promise<WorkflowRunResult> {
    const invoke = nativeInvoke();
    const cwd = bounded(input.cwd, 32_768, "工作目录");
    const prompt = bounded(input.prompt, 200_000, "工作流提示词");
    const systemPrompt = bounded(
      input.systemPrompt,
      32_000,
      "工作流系统提示词",
    );
    const title = bounded(input.title, 160, "工作流标题");
    const requestId = workflowId("workflow");
    const pending: PendingWorkflow = {
      cancelled: false,
      cancellationState: "ready",
    };
    this.pending.set(requestId, pending);
    const archiveSession = input.archiveSession !== false;
    let sessionId: string | undefined;
    let text = "";
    let usage: Record<string, unknown> | undefined;
    let metadata: Record<string, unknown> | undefined;
    let terminalError: string | undefined;
    let cancelled = false;
    let unsubscribe: (() => void) | undefined;

    try {
      const created = await invoke<SessionSummary>("sessions_create", {
        title,
        cwd,
      });
      const createdSessionId = created?.id?.trim();
      if (!createdSessionId) throw new Error("无法创建工作流会话");
      sessionId = createdSessionId;
      if (pending.cancelled) throw workflowCancelledError();

      const handle: PiRunHandle = {
        requestId,
        sessionId: createdSessionId,
        conversationId: createdSessionId,
      };
      const transport = createEmbeddedPiTransport({
        systemPrompt,
        tools: [],
        runKind: "tool-free-workflow",
      });
      this.active.set(requestId, { transport, handle });
      if (pending.cancelled) throw workflowCancelledError();
      unsubscribe = await transport.subscribe((event) => {
        if (event.requestId && event.requestId !== requestId) return;
        if (event.type === "text_delta")
          text += event.delta ?? event.text ?? "";
        if (event.type === "done") {
          text = event.text ?? text;
          usage = event.usage;
          metadata = event.metadata;
        }
        if (event.type === "error")
          terminalError = event.error || "Pi 工作流失败";
        if (event.type === "cancelled") cancelled = true;
        if (event.type === "permission_requested" && event.permission) {
          try {
            input.onPermission?.(event.permission);
          } catch (error) {
            console.warn(
              "[NovaVei Pi] workflow permission handler failed",
              error,
            );
          } finally {
            // Detached workflows never gain tools, so an unexpected permission
            // request must remain denied even if a renderer callback fails.
            void Promise.resolve(
              transport.answerPermission?.(event.permission.id, "deny"),
            ).catch(() => undefined);
          }
        }
        try {
          input.onEvent?.(event);
        } catch (error) {
          // A rendering callback must not be able to strand an otherwise
          // recoverable provider run or its native history session.
          console.warn("[NovaVei Pi] workflow event handler failed", error);
        }
      });
      if (pending.cancelled) throw workflowCancelledError();
      const result = await transport
        .run({
          text: prompt,
          sessionId: createdSessionId,
          conversationId: createdSessionId,
          providerId: input.model.providerId,
          model: input.model.model,
          reasoning: input.reasoning,
          permission: "readonly",
          cwd,
          requestId,
        })
        .catch((error) => {
          // Initialization cancellation can reject the transport with its
          // ordinary Error before it emits a workflow-level cancellation event.
          if (pending.cancelled || cancelled) throw workflowCancelledError();
          throw error;
        });
      text = result.assistantText ?? text;
      // A transport can finish at the same time as a cancel click.  The local
      // latch is authoritative for this detached workflow, even when a
      // terminal `cancelled` event loses that race to `done`.
      if (pending.cancelled || cancelled) throw workflowCancelledError();
      if (terminalError) throw new Error(terminalError);
      return {
        sessionId: createdSessionId,
        requestId,
        text,
        usage,
        metadata,
        model: input.model,
      };
    } finally {
      try {
        unsubscribe?.();
      } catch (error) {
        console.warn("[NovaVei Pi] workflow unsubscribe failed", error);
      }
      this.active.delete(requestId);
      this.pending.delete(requestId);
      if (sessionId && archiveSession) {
        try {
          await invoke("chat_history_set_archived", {
            id: sessionId,
            isArchived: true,
          });
        } catch {
          // Archival is a presentation concern; do not replace the real Pi
          // result or provider error with a best-effort history failure.
        }
      }
    }
  }

  async cancel(requestId?: string): Promise<number> {
    const ids = requestId ? [requestId] : this.activeRequestIds;
    const targets: CancellationTarget[] = ids.flatMap((id) => {
      const pending = this.pending.get(id);
      // `signalling` suppresses duplicate clicks while an abort is in flight;
      // `signalled` makes repeated cancel calls accurately report zero.
      if (pending?.cancellationState !== "ready") return [];
      const active = this.active.get(id);
      if (!active) {
        // The setup latch itself is enough to guarantee that this request
        // cannot later enter `transport.run`.
        pending.cancelled = true;
        pending.cancellationState = "signalled";
      } else {
        pending.cancellationState = "signalling";
      }
      return [{ requestId: id, pending, active }];
    });

    const outcomes = await Promise.all(
      targets.map(async (target) => {
        if (!target.active) return true;
        try {
          await target.active.transport.cancel(target.active.handle);
          if (this.pending.get(target.requestId) === target.pending) {
            // Do not latch until the transport acknowledges cancellation: a
            // provider may complete normally while the signal is still pending.
            target.pending.cancelled = true;
            target.pending.cancellationState = "signalled";
            return true;
          }
          return false;
        } catch (error) {
          // Keep the lifecycle entry alive until `run()` cleans it up.  Deleting
          // it here would make a failed cancellation impossible to retry and
          // would make the caller's count disagree with the real live request.
          if (this.pending.get(target.requestId) === target.pending) {
            target.pending.cancellationState = "ready";
          }
          console.warn(
            "[NovaVei Pi] workflow cancellation signal failed",
            error,
          );
          return false;
        }
      }),
    );
    return outcomes.filter(Boolean).length;
  }
}
