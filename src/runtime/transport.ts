import {
  PI_EVENT_NAMES,
  type PermissionDecision,
  type PiRunEvent,
  type PiRunHandle,
  type PiRunInput,
  type PiRunResult,
  type PiRuntimeTransport,
  type PiToolCall,
  type PiPermissionRequest,
} from "./types";

type UnknownRecord = Record<string, unknown>;

function asRecord(value: unknown): UnknownRecord | null {
  return value !== null && typeof value === "object"
    ? (value as UnknownRecord)
    : null;
}

function stringValue(...values: unknown[]): string | undefined {
  for (const value of values) {
    if (typeof value === "string" && value.trim()) return value;
    if (typeof value === "number" && Number.isFinite(value))
      return String(value);
  }
  return undefined;
}

function parsePayload(value: unknown): unknown {
  if (typeof value !== "string") return value;
  try {
    return JSON.parse(value) as unknown;
  } catch {
    return value;
  }
}

function nestedEvent(value: unknown): UnknownRecord | null {
  const parsed = parsePayload(value);
  const record = asRecord(parsed);
  if (!record) return null;
  const payload = parsePayload(record.payload ?? record.data ?? record.event);
  const nested = asRecord(payload);
  return nested ? { ...record, ...nested } : record;
}

function normaliseType(
  value: unknown,
  record: UnknownRecord,
): PiRunEvent["type"] | null {
  const raw = stringValue(value)?.toLowerCase().replace(/[-\s]/g, "_");
  switch (raw) {
    case "run_started":
    case "agent_start":
    case "agent_started":
    case "turn_start":
    case "start":
      return "run_started";
    case "text_delta":
    case "text_chunk":
    case "assistant_text_delta":
      return "text_delta";
    case "thinking_delta":
    case "thinking_chunk":
    case "reasoning_delta":
      return "thinking_delta";
    case "tool_call":
    case "toolcall_start":
    case "tool_call_start":
    case "tool_use":
    case "tool_execution_start":
      return "tool_call";
    case "tool_update":
    case "tool_execution_update":
    case "toolcall_delta":
    case "tool_call_delta":
      return "tool_update";
    case "tool_result":
    case "tool_execution_end":
    case "toolcall_end":
    case "tool_call_end":
      return "tool_result";
    case "permission_requested":
    case "permission_request":
    case "approval_requested":
    case "before_tool_call":
      return "permission_requested";
    case "done":
    case "complete":
    case "completed":
    case "agent_end":
    case "run_finished":
      return "done";
    case "turn_end":
      // A Pi run can contain several turns around tool calls. The enclosing
      // agent_end/done event is the terminal event consumed by the shell.
      return null;
    case "message_end": {
      // Pi emits message_end for every message, including the assistant
      // message that requested a tool.  Only a tool-result message is a tool
      // update; the assistant/message end is not the end of the agent run.
      const message = asRecord(
        record.message ?? record.toolResult ?? record.tool_result,
      );
      const role = stringValue(message?.role, message?.type)?.toLowerCase();
      if (role?.includes("tool")) return "tool_result";
      return null;
    }
    case "cancelled":
    case "canceled":
    case "aborted":
    case "abort":
      return "cancelled";
    case "error":
    case "failed":
    case "run_error":
      return "error";
    case "message_update": {
      const message = asRecord(
        record.message ??
          record.assistantMessage ??
          record.assistantMessageEvent ??
          record.update,
      );
      const messageType = stringValue(message?.type)?.toLowerCase();
      if (messageType?.includes("thinking")) return "thinking_delta";
      if (
        messageType?.includes("toolcall_start") ||
        messageType?.includes("tool_call_start")
      ) {
        return "tool_call";
      }
      if (
        messageType?.includes("toolcall_end") ||
        messageType?.includes("tool_call_end")
      ) {
        return "tool_result";
      }
      if (messageType?.includes("tool")) return "tool_update";
      return "text_delta";
    }
    default:
      break;
  }

  if (record.error !== undefined || record.errorMessage !== undefined)
    return "error";
  if (record.permission !== undefined || record.approval !== undefined) {
    return "permission_requested";
  }
  if (
    record.delta !== undefined ||
    record.textDelta !== undefined ||
    record.text_delta !== undefined
  ) {
    return "text_delta";
  }
  return null;
}

function readTool(
  record: UnknownRecord,
  type: PiRunEvent["type"],
): PiToolCall | undefined {
  const raw =
    asRecord(record.toolCall) ??
    asRecord(record.tool_call) ??
    asRecord(record.tool) ??
    asRecord(record.call) ??
    asRecord(record.partial);
  const id = stringValue(
    raw?.id,
    raw?.toolCallId,
    raw?.tool_call_id,
    record.toolCallId,
    record.tool_call_id,
  );
  const name = stringValue(
    raw?.name,
    raw?.toolName,
    raw?.tool_name,
    record.toolName,
    record.tool_name,
  );
  if (!id && !name) return undefined;
  const argumentsValue =
    raw?.arguments ??
    raw?.args ??
    raw?.input ??
    record.arguments ??
    record.args;
  const result = raw?.result ?? raw?.output ?? record.result ?? record.output;
  const status =
    stringValue(raw?.status, record.status) ??
    (type === "tool_result" ? "completed" : undefined);
  return {
    id: id ?? name ?? `tool-${Date.now().toString(36)}`,
    name: name ?? "tool",
    ...(argumentsValue !== undefined ? { arguments: argumentsValue } : {}),
    ...(result !== undefined ? { result } : {}),
    ...(status ? { status } : {}),
    ...(stringValue(raw?.error, record.toolError)
      ? { error: stringValue(raw?.error, record.toolError) }
      : {}),
  };
}

function readPermission(
  record: UnknownRecord,
): PiPermissionRequest | undefined {
  const raw =
    asRecord(record.permission) ??
    asRecord(record.approval) ??
    asRecord(record.request);
  const id = stringValue(
    raw?.id,
    raw?.requestId,
    raw?.request_id,
    record.permissionRequestId,
    record.requestId,
  );
  if (!id) return undefined;
  return {
    id,
    toolName: stringValue(
      raw?.toolName,
      raw?.tool_name,
      raw?.name,
      record.toolName,
    ),
    description: stringValue(raw?.description, raw?.reason, record.description),
    arguments: raw?.arguments ?? raw?.args ?? record.arguments,
    risk: stringValue(raw?.risk, record.risk),
  };
}

/** Convert Pi, Tauri, and historical gateway event shapes to the shell contract. */
export function normalisePiEvent(value: unknown): PiRunEvent | null {
  const record = nestedEvent(value);
  if (!record) return null;
  const type = normaliseType(
    record.type ?? record.eventType ?? record.kind,
    record,
  );
  if (!type) return null;

  const assistantMessageEvent = asRecord(
    record.assistantMessageEvent ?? record.assistant_message_event,
  );
  const message = asRecord(
    record.message ?? record.assistantMessage ?? record.assistant_message,
  );
  const nestedContent = assistantMessageEvent ?? message;

  const event: PiRunEvent = {
    type,
    eventId: stringValue(record.eventId, record.event_id),
    sequence:
      typeof record.sequence === "number"
        ? record.sequence
        : typeof record.seq === "number"
          ? record.seq
          : undefined,
    sessionId: stringValue(record.sessionId, record.session_id, record.session),
    conversationId: stringValue(
      record.conversationId,
      record.conversation_id,
      record.conversation,
    ),
    turnId: stringValue(record.turnId, record.turn_id, record.turn),
    requestId: stringValue(
      record.requestId,
      record.request_id,
      record.correlationId,
      record.correlation_id,
    ),
    delta: stringValue(
      record.delta,
      record.textDelta,
      record.text_delta,
      record.contentDelta,
      nestedContent?.delta,
      nestedContent?.textDelta,
      nestedContent?.text_delta,
    ),
    text: stringValue(
      record.text,
      record.content,
      record.assistantText,
      record.assistant_text,
      nestedContent?.text,
      nestedContent?.content,
    ),
    thinking: stringValue(
      record.thinking,
      record.thinkingText,
      record.thinking_text,
      record.reasoningContent,
      record.reasoning_content,
      nestedContent?.thinking,
      nestedContent?.thinkingText,
      nestedContent?.thinking_text,
      nestedContent?.reasoningContent,
      nestedContent?.reasoning_content,
    ),
    error: stringValue(
      record.error,
      record.errorMessage,
      record.error_message,
      record.message,
    ),
    usage: asRecord(record.usage) ?? undefined,
    toolCall: readTool({ ...record, ...(nestedContent ?? {}) }, type),
    permission: readPermission(record),
    metadata: asRecord(record.metadata) ?? undefined,
  };

  // Pi's final event often carries a complete message object rather than
  // top-level text fields. Pull both visible text and reasoning out without
  // imposing a renderer-specific message format on the visual shell.
  if (
    type === "done" &&
    (event.text === undefined || event.thinking === undefined)
  ) {
    const finalMessage =
      message ??
      asRecord(record.assistantMessageEvent) ??
      asRecord(
        record.messages && Array.isArray(record.messages)
          ? record.messages.at(-1)
          : undefined,
      );
    const content = finalMessage?.content;
    if (typeof content === "string" && event.text === undefined)
      event.text = content;
    else if (Array.isArray(content)) {
      if (event.text === undefined) {
        const text = content
          .map((block) => {
            const item = asRecord(block);
            return stringValue(item?.text, item?.content) ?? "";
          })
          .join("");
        if (text) event.text = text;
      }
      if (event.thinking === undefined) {
        const thinking = content
          .map((block) => stringValue(asRecord(block)?.thinking) ?? "")
          .join("");
        if (thinking) event.thinking = thinking;
      }
    }
  }
  if (type === "permission_requested" && !event.permission) {
    event.permission = {
      id: event.requestId ?? `permission-${Date.now().toString(36)}`,
      description: event.text ?? event.error,
    };
  }
  return event;
}

// Keep the US spelling available to host integrations; the implementation
// uses the project’s existing British spelling internally.
export const normalizePiEvent = normalisePiEvent;

function normaliseResult(value: unknown, input: PiRunInput): PiRunResult {
  const record = asRecord(value) ?? {};
  return {
    sessionId: stringValue(
      record.sessionId,
      record.session_id,
      input.sessionId,
    ),
    conversationId: stringValue(
      record.conversationId,
      record.conversation_id,
      input.conversationId,
    ),
    turnId: stringValue(record.turnId, record.turn_id),
    requestId:
      stringValue(record.requestId, record.request_id, input.requestId) ??
      input.requestId,
    assistantText: stringValue(
      record.assistantText,
      record.assistant_text,
      record.text,
    ),
    mode: stringValue(record.mode),
  };
}

function invokeArgs(input: PiRunInput): Record<string, unknown> {
  // Include both spellings while the Rust command contract settles. Tauri
  // ignores unknown fields, and this keeps the adapter compatible with the
  // existing snake_case command implementations.
  // Plan approval is a renderer-local coordination marker, never a native
  // capability or a host enforcement claim.
  const nativeInput = { ...input };
  delete nativeInput.planApproval;
  return {
    ...nativeInput,
    session_id: input.sessionId,
    conversation_id: input.conversationId,
    provider_id: input.providerId,
    request_id: input.requestId,
    cwd: input.cwd,
  };
}

function isUnknownCommand(error: unknown, command: string) {
  const message = String(error).toLowerCase();
  return (
    message.includes(command.toLowerCase()) &&
    (message.includes("not found") ||
      message.includes("unknown") ||
      message.includes("not allowed") ||
      message.includes("command"))
  );
}

export function createEmbeddedPiTransport(): PiRuntimeTransport | null {
  const transport = window.__novaveiPiEmbedded;
  return transport &&
    typeof transport.run === "function" &&
    typeof transport.subscribe === "function"
    ? transport
    : null;
}

export function createTauriPiTransport(): PiRuntimeTransport | null {
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) return null;
  const listen = window.__TAURI__?.event?.listen;
  let agentCommandSupported: boolean | undefined;

  return {
    subscribe(listener) {
      if (!listen) return () => undefined;
      let disposed = false;
      const unlistenPromises = PI_EVENT_NAMES.map((name) =>
        listen(name, (event) => {
          if (disposed) return;
          const normalized = normalisePiEvent(event.payload);
          if (normalized) listener(normalized);
        }).catch(() => () => undefined),
      );
      return Promise.all(unlistenPromises).then((unlisteners) => () => {
        disposed = true;
        for (const unlisten of unlisteners) unlisten();
      });
    },
    async run(input) {
      const args = invokeArgs(input);
      if (agentCommandSupported !== false) {
        try {
          const response = await invoke("agent_run", args);
          agentCommandSupported = true;
          return {
            ...normaliseResult(response, input),
            mode: normaliseResult(response, input).mode ?? "pi",
          };
        } catch (error) {
          if (!isUnknownCommand(error, "agent_run")) throw error;
          agentCommandSupported = false;
        }
      }
      throw new Error(
        "Pi runtime unavailable: the Tauri agent_run command is not registered. " +
          "Connect an embedded Pi runtime or enable the native agent bridge.",
      );
    },
    async cancel(handle: PiRunHandle) {
      if (agentCommandSupported === false) {
        throw new Error(
          "Pi cancellation is unavailable because the native agent command is not registered.",
        );
      }
      try {
        await invoke("agent_cancel", {
          ...handle,
          session_id: handle.sessionId,
          conversation_id: handle.conversationId,
          turn_id: handle.turnId,
          request_id: handle.requestId,
        });
      } catch (error) {
        if (!isUnknownCommand(error, "agent_cancel")) throw error;
        agentCommandSupported = false;
        throw new Error(
          "Pi cancellation is unavailable because the native agent command is not registered.",
        );
      }
    },
    async answerPermission(requestId: string, decision: PermissionDecision) {
      try {
        await invoke("agent_permission", {
          requestId,
          request_id: requestId,
          decision,
        });
      } catch (error) {
        if (!isUnknownCommand(error, "agent_permission")) throw error;
      }
    },
  };
}

export function createDefaultPiTransport(): PiRuntimeTransport | null {
  return createEmbeddedPiTransport() ?? createTauriPiTransport();
}
