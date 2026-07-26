import {
  INITIAL_PI_RUNTIME_STATE,
  isPiRuntimeTerminal,
  type PiRunEvent,
  type PiRuntimeState,
  type PiToolCall,
} from "./types";

export type PiRuntimeAction =
  | {
      type: "run_started";
      prompt: string;
      requestId: string;
      sessionId?: string;
      conversationId?: string;
      turnId?: string;
    }
  | { type: "event"; event: PiRunEvent }
  | {
      type: "permission_resolved";
      requestId: string;
      permissionId: string;
    }
  | {
      type: "plan_confirmation_resolved";
      requestId: string;
      planId: string;
      decision: "execute" | "modify" | "not_now";
    }
  | {
      type: "plan_confirmation_invalidated";
      requestId: string;
      planId?: string;
    }
  | { type: "cancel_requested"; requestId: string }
  | { type: "cancel_failed"; requestId: string; error: string }
  | { type: "reset" };

function hasRunIdentity(state: PiRuntimeState, event: PiRunEvent) {
  // Events from an older run must never append into the current transcript.
  // Once an active run has a request/turn identity, an identity-poor legacy
  // event is no longer safe: it could belong to an earlier run in this same
  // session. Current transports attach both fields to every run event.
  if (state.requestId && state.requestId !== event.requestId) return false;
  if (state.turnId && state.turnId !== event.turnId) return false;
  if (
    state.conversationId &&
    event.conversationId &&
    state.conversationId !== event.conversationId
  ) {
    return false;
  }
  return true;
}

function mergeTool(
  previous: PiToolCall | undefined,
  next: PiToolCall,
): PiToolCall {
  return {
    ...(previous ?? { id: next.id, name: next.name }),
    ...next,
  };
}

function toolStatusForEvent(type: PiRunEvent["type"]): string | undefined {
  if (type === "tool_call" || type === "tool_update") return "running";
  if (type === "tool_result") return "completed";
  return undefined;
}

function isTerminalEvent(event: PiRunEvent) {
  return (
    event.type === "done" ||
    event.type === "cancelled" ||
    event.type === "error"
  );
}

export function reducePiRuntime(
  state: PiRuntimeState = INITIAL_PI_RUNTIME_STATE,
  action: PiRuntimeAction,
): PiRuntimeState {
  if (action.type === "reset") return { ...INITIAL_PI_RUNTIME_STATE };

  if (action.type === "run_started") {
    return {
      status: "starting",
      prompt: action.prompt,
      requestId: action.requestId,
      sessionId: action.sessionId,
      conversationId: action.conversationId,
      turnId: action.turnId,
      assistantText: "",
      thinkingText: "",
      tools: {},
    };
  }

  if (action.type === "permission_resolved") {
    // A permission answer can race the transport's terminal event or a
    // cancellation request. It can also finish after the same run has already
    // presented its next permission. Only the exact card that was answered may
    // be cleared, and no answer may revive an already-finished turn.
    if (
      state.requestId !== action.requestId ||
      state.pendingPermission?.id !== action.permissionId ||
      isPiRuntimeTerminal(state.status) ||
      state.status === "cancelling" ||
      state.status === "cancel_failed"
    ) {
      return state;
    }
    return {
      ...state,
      status: "running",
      pendingPermission: undefined,
      cancellationError: undefined,
    };
  }

  if (action.type === "plan_confirmation_resolved") {
    if (
      state.requestId !== action.requestId ||
      state.pendingPlan?.id !== action.planId ||
      state.pendingPlan.status !== "pending"
    ) {
      return state;
    }
    const status =
      action.decision === "execute"
        ? "approved"
        : action.decision === "modify"
          ? "modify_requested"
          : "deferred";
    return {
      ...state,
      pendingPlan: { ...state.pendingPlan, status },
    };
  }

  if (action.type === "plan_confirmation_invalidated") {
    if (
      state.requestId !== action.requestId ||
      !state.pendingPlan ||
      (action.planId && state.pendingPlan.id !== action.planId)
    ) {
      return state;
    }
    return { ...state, pendingPlan: undefined };
  }

  if (action.type === "cancel_requested") {
    if (
      state.requestId !== action.requestId ||
      isPiRuntimeTerminal(state.status) ||
      state.status === "cancelling"
    ) {
      return state;
    }
    return {
      ...state,
      status: "cancelling",
      cancellationError: undefined,
      pendingPlan: undefined,
    };
  }

  if (action.type === "cancel_failed") {
    // Do not replace an observed terminal outcome with a cancellation error.
    // Keep the run handle/live state so the user can retry cancellation.
    if (
      state.requestId !== action.requestId ||
      isPiRuntimeTerminal(state.status) ||
      state.status !== "cancelling"
    ) {
      return state;
    }
    return {
      ...state,
      status: "cancel_failed",
      cancellationError: action.error,
    };
  }

  const event = action.event;
  if (!hasRunIdentity(state, event)) return state;

  // A terminal event is authoritative for its request.  Providers can still
  // flush text/tool events after abort or completion; never let those reopen
  // the finished transcript.  While cancellation is being signalled, only a
  // real terminal event may change the visible state.
  if (isPiRuntimeTerminal(state.status)) return state;
  // After a rejected cancellation the underlying provider may still flush
  // deltas. Keep the visible failure/retry state stable until a real terminal
  // outcome arrives; otherwise the next delta would erase the cancellation
  // error and turn “重试停止” back into an ambiguous running state.
  if (
    (state.status === "cancelling" || state.status === "cancel_failed") &&
    !isTerminalEvent(event)
  )
    return state;

  const identity = {
    sessionId: event.sessionId ?? state.sessionId,
    conversationId: event.conversationId ?? state.conversationId,
    turnId: event.turnId ?? state.turnId,
    requestId: event.requestId ?? state.requestId,
  };

  switch (event.type) {
    case "run_started":
      return {
        ...state,
        ...identity,
        status: "running",
        cancellationError: undefined,
        contextTrim:
          event.metadata?.contextTrim &&
          typeof event.metadata.contextTrim === "object"
            ? (event.metadata.contextTrim as Record<string, unknown>)
            : undefined,
        lastEvent: event,
      };
    case "text_delta":
      return {
        ...state,
        ...identity,
        status: "running",
        cancellationError: undefined,
        assistantText: state.assistantText + (event.delta ?? event.text ?? ""),
        lastEvent: event,
      };
    case "thinking_delta":
      return {
        ...state,
        ...identity,
        status: "running",
        cancellationError: undefined,
        thinkingText: state.thinkingText + (event.delta ?? event.text ?? ""),
        lastEvent: event,
      };
    case "tool_call":
    case "tool_update":
    case "tool_result": {
      const tool = event.toolCall;
      if (!tool?.id) {
        return {
          ...state,
          ...identity,
          status: "running",
          cancellationError: undefined,
          lastEvent: event,
        };
      }
      const existing = state.tools[tool.id];
      const inferredStatus = tool.status ?? toolStatusForEvent(event.type);
      const nextTool = {
        ...(existing ?? {
          id: tool.id,
          name: tool.name,
          startedAt: Date.now(),
        }),
        ...mergeTool(existing, tool),
        ...(inferredStatus ? { status: inferredStatus } : {}),
        ...(event.type === "tool_result" ? { finishedAt: Date.now() } : {}),
      };
      return {
        ...state,
        ...identity,
        status: "running",
        cancellationError: undefined,
        tools: { ...state.tools, [tool.id]: nextTool },
        lastEvent: event,
      };
    }
    case "permission_requested":
      return {
        ...state,
        ...identity,
        status: "waiting_permission",
        cancellationError: undefined,
        pendingPermission: event.permission,
        lastEvent: event,
      };
    case "plan_confirmation_requested":
      if (!event.plan) return state;
      return {
        ...state,
        ...identity,
        pendingPlan: event.plan,
        lastEvent: event,
      };
    case "done":
      return {
        ...state,
        ...identity,
        status: "completed",
        // A host may send a final complete text in addition to deltas.  Use it
        // as the authoritative value; otherwise retain accumulated deltas.
        assistantText:
          event.text !== undefined ? event.text : state.assistantText,
        // A number of providers expose reasoning only in their completed
        // message, without token-level thinking deltas.
        thinkingText:
          event.thinking !== undefined ? event.thinking : state.thinkingText,
        usage: event.usage ?? state.usage,
        pendingPermission: undefined,
        cancellationError: undefined,
        lastEvent: event,
      };
    case "cancelled":
      return {
        ...state,
        ...identity,
        status: "cancelled",
        pendingPermission: undefined,
        pendingPlan: undefined,
        cancellationError: undefined,
        lastEvent: event,
      };
    case "error":
      return {
        ...state,
        ...identity,
        status: "error",
        error: event.error ?? event.text ?? "Pi runtime failed",
        pendingPermission: undefined,
        pendingPlan: undefined,
        cancellationError: undefined,
        lastEvent: event,
      };
    default:
      return state;
  }
}
