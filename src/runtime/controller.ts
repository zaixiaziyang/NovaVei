import { reducePiRuntime, type PiRuntimeAction } from "./reducer";
import { createDefaultPiTransport } from "./transport";
import {
  INITIAL_PI_RUNTIME_STATE,
  isPiRuntimeTerminal,
  type PermissionDecision,
  type PiPlanConfirmationDecision,
  type PiRunEvent,
  type PiRunHandle,
  type PiRunInput,
  type PiRunResult,
  type PiRuntimePublicApi,
  type PiSessionRunStateListener,
  type PiRuntimeSnapshot,
  type PiRuntimeState,
  type PiRuntimeTransport,
} from "./types";

type StateListener = (state: PiRuntimeSnapshot) => void;

const FALLBACK_SESSION_KEY = "__novavei_unbound_session__";

function createRequestId() {
  try {
    if (typeof crypto?.randomUUID === "function") return crypto.randomUUID();
  } catch {
    // Older WebViews may expose crypto without randomUUID.
  }
  return `novavei-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

function errorMessage(error: unknown) {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string") return error;
  try {
    return JSON.stringify(error);
  } catch {
    return "Pi runtime failed";
  }
}

function initialState(): PiRuntimeState {
  return {
    ...INITIAL_PI_RUNTIME_STATE,
    tools: {},
  };
}

function sessionKey(sessionId?: string) {
  const normalized = sessionId?.trim();
  return normalized ? `session:${normalized}` : FALLBACK_SESSION_KEY;
}

/**
 * Owns the active Pi state for every session while presenting the selected
 * session's state to the single Composer surface. A run in another session
 * must never make the current Composer a Stop button or receive its events.
 */
export class PiRuntimeController implements PiRuntimePublicApi {
  readonly ready: Promise<void>;

  private readonly transport: PiRuntimeTransport | null;
  private readonly states = new Map<string, PiRuntimeState>();
  private readonly listeners = new Set<StateListener>();
  private readonly sessionStateListeners = new Set<PiSessionRunStateListener>();
  private readonly publishedSessionStates = new Map<string, string>();
  private readonly seenEventKeys = new Set<string>();
  private readonly activeHandles = new Map<string, PiRunHandle>();
  private readonly requestSessions = new Map<string, string>();
  private readonly cancellations = new Map<string, Promise<void>>();
  private selectedSessionKey = FALLBACK_SESSION_KEY;
  private unsubscribeTransport: (() => void) | null = null;

  constructor(
    transport: PiRuntimeTransport | null = createDefaultPiTransport(),
  ) {
    this.transport = transport;
    this.ready = this.connect();
  }

  private async connect() {
    if (!this.transport) return;
    const unsubscribe = await this.transport.subscribe((event) =>
      this.acceptEvent(event),
    );
    this.unsubscribeTransport = unsubscribe;
  }

  private stateFor(key: string) {
    return this.states.get(key) ?? initialState();
  }

  private visibleState(key = this.selectedSessionKey) {
    const state = this.states.get(key);
    // Terminal output is durably stored by the native host. Once a run is no
    // longer active, switching back reloads that history instead of creating a
    // duplicate transient assistant message in the Composer.
    return state && this.activeHandles.has(key) ? state : initialState();
  }

  private notify(key: string) {
    if (key !== this.selectedSessionKey) return;
    const state = this.visibleState(key);
    for (const listener of this.listeners) listener(state);
  }

  /** Broadcast only lifecycle transitions, never each streamed token. */
  private notifySessionState(key: string, state: PiRuntimeState) {
    const sessionId = state.sessionId?.trim();
    if (!sessionId) return;
    const signature = `${state.requestId ?? ""}:${state.status}`;
    if (this.publishedSessionStates.get(key) === signature) return;
    this.publishedSessionStates.set(key, signature);
    for (const listener of this.sessionStateListeners)
      listener(sessionId, state);
  }

  private dispatch(key: string, action: PiRuntimeAction) {
    const state = this.stateFor(key);
    const next = reducePiRuntime(state, action);
    if (next === state) return;
    this.states.set(key, next);
    this.notify(key);
    this.notifySessionState(key, next);
  }

  private rememberEvent(event: PiRunEvent) {
    const eventKey = event.eventId
      ? `id:${event.eventId}`
      : event.sequence !== undefined && (event.requestId || event.sessionId)
        ? `seq:${event.requestId ?? event.sessionId}:${event.sequence}`
        : null;
    if (!eventKey) return true;
    if (this.seenEventKeys.has(eventKey)) return false;
    this.seenEventKeys.add(eventKey);
    if (this.seenEventKeys.size > 1024) {
      const oldest = this.seenEventKeys.values().next().value;
      if (oldest) this.seenEventKeys.delete(oldest);
    }
    return true;
  }

  private eventSessionKey(event: PiRunEvent) {
    if (event.requestId) {
      const mapped = this.requestSessions.get(event.requestId);
      if (mapped) return mapped;
    }
    if (event.sessionId?.trim()) return sessionKey(event.sessionId);
    // Legacy hosts without a request/session identity can only be routed when
    // one live run exists. Dropping ambiguous events is safer than appending
    // output to another conversation.
    if (this.activeHandles.size === 1)
      return this.activeHandles.keys().next().value;
    return undefined;
  }

  private acceptEvent(event: PiRunEvent) {
    if (!this.rememberEvent(event)) return;
    const key = this.eventSessionKey(event);
    if (!key) return;
    const activeRequestId = this.activeHandles.get(key)?.requestId;
    this.dispatch(key, { type: "event", event });
    if (activeRequestId) this.releaseTerminalHandle(key, activeRequestId);
  }

  private isCurrentRequest(key: string, requestId: string) {
    return (
      this.activeHandles.get(key)?.requestId === requestId &&
      this.stateFor(key).requestId === requestId
    );
  }

  private releaseTerminalHandle(key: string, requestId: string) {
    const state = this.states.get(key);
    if (
      this.activeHandles.get(key)?.requestId === requestId &&
      state?.requestId === requestId &&
      isPiRuntimeTerminal(state.status)
    ) {
      this.activeHandles.delete(key);
      this.requestSessions.delete(requestId);
      this.cancellations.delete(requestId);
      this.publishedSessionStates.delete(key);
    }
  }

  /** Select the session whose state is reflected by the shared Composer. */
  selectSession(sessionId?: string) {
    const nextKey = sessionKey(sessionId);
    const previousKey = this.selectedSessionKey;
    if (nextKey !== previousKey) {
      const previous = this.states.get(previousKey);
      const pendingPlan = previous?.pendingPlan;
      // Plan approval is intentionally tied to the focused run/session. A
      // background run may retain read-only work, but it cannot keep a stale
      // write confirmation alive after the user navigates away.
      if (pendingPlan && previous?.requestId) {
        this.transport?.invalidatePlanConfirmation?.(
          previous.requestId,
          pendingPlan.id,
        );
        this.dispatch(previousKey, {
          type: "plan_confirmation_invalidated",
          requestId: previous.requestId,
          planId: pendingPlan.id,
        });
      }
    }
    this.selectedSessionKey = nextKey;
    this.notify(this.selectedSessionKey);
  }

  getState() {
    return this.visibleState();
  }

  subscribe(listener: StateListener) {
    this.listeners.add(listener);
    listener(this.getState());
    return () => this.listeners.delete(listener);
  }

  subscribeSessionState(listener: PiSessionRunStateListener) {
    this.sessionStateListeners.add(listener);
    for (const [key, handle] of this.activeHandles) {
      const state = this.states.get(key);
      const sessionId = state?.sessionId?.trim() ?? handle.sessionId?.trim();
      if (state && sessionId) listener(sessionId, state);
    }
    return () => this.sessionStateListeners.delete(listener);
  }

  async submit(
    input: Omit<PiRunInput, "requestId">,
  ): Promise<PiRunResult | undefined> {
    await this.ready;
    if (!this.transport) return undefined;

    const key = sessionKey(input.sessionId);
    const active = this.activeHandles.get(key);
    if (active) {
      await this.cancelSession(key);
      // A rejected cancellation leaves the existing native turn alive. Do not
      // overwrite that session's handle; other sessions remain independent.
      if (this.activeHandles.has(key)) {
        throw new Error(
          this.stateFor(key).cancellationError ||
            "当前对话仍在运行，无法发送新消息",
        );
      }
    }

    const requestId = createRequestId();
    const request: PiRunInput = { ...input, requestId };
    this.activeHandles.set(key, {
      requestId,
      sessionId: input.sessionId,
      conversationId: input.conversationId,
    });
    this.requestSessions.set(requestId, key);
    this.dispatch(key, {
      type: "run_started",
      prompt: input.text,
      requestId,
      sessionId: input.sessionId,
      conversationId: input.conversationId,
    });

    try {
      const result = await this.transport.run(request);
      if (
        result.requestId === requestId &&
        this.isCurrentRequest(key, requestId)
      ) {
        this.activeHandles.set(key, {
          requestId,
          sessionId: result.sessionId ?? input.sessionId,
          conversationId: result.conversationId ?? input.conversationId,
          turnId: result.turnId,
        });
      }

      // Command-backed Pi runs normally return after scheduling a turn and
      // emit `done` later. A synchronous embedded/test transport may return
      // the final answer instead; normalize it into the event path.
      if (
        this.isCurrentRequest(key, requestId) &&
        result.assistantText !== undefined &&
        !isPiRuntimeTerminal(this.stateFor(key).status)
      ) {
        this.acceptEvent({
          type: "done",
          ...this.activeHandles.get(key),
          text: result.assistantText,
        });
      }
      return result;
    } catch (error) {
      // An abort commonly rejects the original `run()` promise after the
      // transport has already emitted `cancelled`. Wait for that request's
      // cancellation acknowledgement before treating it as a provider error.
      const cancellation = this.cancellations.get(requestId);
      if (cancellation) await cancellation;
      const state = this.stateFor(key);
      if (
        state.requestId === requestId &&
        (state.status === "cancelled" || state.status === "completed")
      ) {
        return {
          requestId,
          sessionId: state.sessionId ?? input.sessionId,
          conversationId: state.conversationId ?? input.conversationId,
          turnId: state.turnId,
        };
      }
      if (this.isCurrentRequest(key, requestId)) {
        this.acceptEvent({
          type: "error",
          ...this.activeHandles.get(key),
          error: errorMessage(error),
        });
      }
      throw error;
    } finally {
      this.releaseTerminalHandle(key, requestId);
    }
  }

  private async cancelSession(key: string) {
    const handle = this.activeHandles.get(key);
    const transport = this.transport;
    if (!handle || !transport) return;
    const state = this.stateFor(key);
    if (
      state.requestId !== handle.requestId ||
      isPiRuntimeTerminal(state.status)
    ) {
      this.releaseTerminalHandle(key, handle.requestId);
      return;
    }

    const inFlight = this.cancellations.get(handle.requestId);
    if (inFlight) return inFlight;

    if (state.pendingPlan) {
      transport.invalidatePlanConfirmation?.(
        handle.requestId,
        state.pendingPlan.id,
      );
    }

    this.dispatch(key, {
      type: "cancel_requested",
      requestId: handle.requestId,
    });
    // Defer the first transport call by one microtask so the in-flight record
    // is installed even when a test/embedded transport throws synchronously.
    const promise = Promise.resolve().then(async () => {
      try {
        await transport.cancel(handle);
        if (
          this.isCurrentRequest(key, handle.requestId) &&
          !isPiRuntimeTerminal(this.stateFor(key).status)
        ) {
          // Embedded Pi normally emits its own terminal event. This fallback
          // is only reached for transports that acknowledge without emitting.
          this.acceptEvent({ type: "cancelled", ...handle });
        }
      } catch (error) {
        if (
          this.isCurrentRequest(key, handle.requestId) &&
          !isPiRuntimeTerminal(this.stateFor(key).status)
        ) {
          // Keep only this session's request active and retryable.
          this.dispatch(key, {
            type: "cancel_failed",
            requestId: handle.requestId,
            error: errorMessage(error),
          });
        }
      } finally {
        this.releaseTerminalHandle(key, handle.requestId);
        this.cancellations.delete(handle.requestId);
      }
    });
    this.cancellations.set(handle.requestId, promise);
    return promise;
  }

  async cancel() {
    return this.cancelSession(this.selectedSessionKey);
  }

  async answerPermission(decision: PermissionDecision) {
    const key = this.selectedSessionKey;
    const state = this.stateFor(key);
    const pending = state.pendingPermission;
    const requestId = state.requestId;
    if (!pending || !requestId) return;
    if (this.transport?.answerPermission) {
      await this.transport.answerPermission(pending.id, decision);
    }
    const answeredPermissionIsCurrent =
      this.isCurrentRequest(key, requestId) &&
      this.stateFor(key).pendingPermission?.id === pending.id;
    this.dispatch(key, {
      type: "permission_resolved",
      requestId,
      permissionId: pending.id,
    });
    if (decision === "cancel" && answeredPermissionIsCurrent) {
      // The UI label means cancel the whole selected turn, not merely deny
      // this one tool call. Resolve the broker before aborting that exact run;
      // a late answer must never cancel a replacement run in the same session.
      await this.cancelSession(key);
    }
  }

  async answerPlanConfirmation(decision: PiPlanConfirmationDecision) {
    const key = this.selectedSessionKey;
    const state = this.stateFor(key);
    const pending = state.pendingPlan;
    const requestId = state.requestId;
    if (pending?.status !== "pending" || !requestId) return false;
    const accepted = await this.transport?.answerPlanConfirmation?.(
      requestId,
      pending.id,
      decision,
    );
    if (!accepted) return false;
    this.dispatch(key, {
      type: "plan_confirmation_resolved",
      requestId,
      planId: pending.id,
      decision,
    });
    return true;
  }

  issuePlanContinuationApproval(planId: string) {
    const state = this.stateFor(this.selectedSessionKey);
    const plan = state.pendingPlan;
    const requestId = state.requestId;
    if (
      !requestId ||
      !plan ||
      plan.id !== planId ||
      plan.status !== "pending" ||
      state.status !== "completed"
    ) {
      return undefined;
    }
    return this.transport?.issuePlanContinuationApproval?.(
      requestId,
      planId,
      state.sessionId,
      state.conversationId,
    );
  }

  invalidatePlanConfirmation(planId: string, requestId?: string) {
    const key = this.selectedSessionKey;
    const state = this.stateFor(key);
    const targetRequestId = requestId ?? state.requestId;
    if (!targetRequestId) return;
    this.transport?.invalidatePlanConfirmation?.(targetRequestId, planId);
    if (
      state.pendingPlan?.id === planId &&
      state.requestId === targetRequestId
    ) {
      this.dispatch(key, {
        type: "plan_confirmation_invalidated",
        requestId: targetRequestId,
        planId,
      });
    }
  }

  /** Called by teardown code and useful for tests that own a controller. */
  dispose() {
    this.unsubscribeTransport?.();
    this.unsubscribeTransport = null;
    this.listeners.clear();
    this.sessionStateListeners.clear();
    this.activeHandles.clear();
    this.cancellations.clear();
    this.requestSessions.clear();
    this.publishedSessionStates.clear();
  }
}
