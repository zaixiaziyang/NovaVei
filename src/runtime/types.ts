/**
 * Frontend-only contract between NovaVei's visual shell and the Pi runtime.
 *
 * Pi emits a fairly large event vocabulary.  The shell deliberately consumes
 * this small, stable set so the runtime can be hosted by the embedded Pi
 * runner, a Tauri command, or a test transport without changing the DOM code.
 */

export const PI_EVENT_NAMES = [
  "agent:event",
  "pi:agent-event",
  "agent_event",
] as const;
export const SESSION_GOAL_UPDATED_EVENT =
  "novavei:session-goal-updated" as const;

export type SessionGoalUpdatedDetail = {
  sessionId: string;
};

export type PiEventType =
  | "run_started"
  | "text_delta"
  | "thinking_delta"
  | "tool_call"
  | "tool_update"
  | "tool_result"
  | "permission_requested"
  | "plan_confirmation_requested"
  | "done"
  | "error"
  | "cancelled";

export type PermissionDecision = "allow" | "deny" | "cancel";
export type PiPlanConfirmationDecision = "execute" | "modify" | "not_now";

export type PiReasoningLevel =
  | "off"
  | "minimal"
  | "low"
  | "medium"
  | "high"
  | "xhigh"
  | "max";

export type PiToolCall = {
  id: string;
  name: string;
  arguments?: unknown;
  status?: "queued" | "running" | "completed" | "failed" | "cancelled" | string;
  result?: unknown;
  error?: string;
};

export type PiPermissionRequest = {
  id: string;
  toolName?: string;
  description?: string;
  arguments?: unknown;
  risk?: "low" | "medium" | "high" | string;
};

export type PiPlanStep = {
  title: string;
  detail?: string;
};

/** One planned mutable tool invocation with machine-checkable argument scope. */
export type PiPlanToolScope = {
  toolName: string;
  /**
   * Exact JSON constraints for the call. Write/Edit content bodies may be
   * omitted, but every resource, command, mode, and other argument is bound.
   */
  arguments: unknown;
};

/**
 * A renderer-local structured plan. `source` is intentionally transient: it
 * is never sent to the native event log and exists only to retain the exact
 * parsed plan while its confirmation card is active.
 */
export type PiPlanConfirmation = {
  id: string;
  /** Versioned protocol field.  Version 1 is the only accepted wire shape. */
  version: 1;
  /**
   * A stable, renderer-local projection of the reviewable fields.  It binds a
   * terminal-plan continuation to the exact plan the user saw, without
   * putting the model's raw JSON back into the prompt or durable history.
   */
  fingerprint: string;
  summary: string;
  steps: PiPlanStep[];
  expectedImpact: string;
  risks: string[];
  permissions: string[];
  executionScope: PiPlanToolScope[];
  source: string;
  status: "pending" | "approved" | "modify_requested" | "deferred";
};

/**
 * A short-lived continuation grant issued by the embedded transport after a
 * user approves a terminal plan.  `token` is opaque and one-use; renderer
 * supplied ids/version/fingerprint alone are never enough to unlock a new
 * turn.
 */
export type PiPlanApproval = {
  planId: string;
  version: 1;
  fingerprint: string;
  executionScope: PiPlanToolScope[];
  token: string;
  sessionId?: string;
  conversationId?: string;
  approvedAt: number;
};

export type PiRunEvent = {
  type: PiEventType;
  eventId?: string;
  sequence?: number;
  sessionId?: string;
  conversationId?: string;
  turnId?: string;
  requestId?: string;
  delta?: string;
  text?: string;
  /** Complete provider thinking retained with a durable terminal response. */
  thinking?: string;
  toolCall?: PiToolCall;
  permission?: PiPermissionRequest;
  plan?: PiPlanConfirmation;
  decision?: PermissionDecision;
  error?: string;
  usage?: Record<string, unknown>;
  /** Keep provider-specific metadata available to diagnostics and the dock. */
  metadata?: Record<string, unknown>;
};

export type PiRunInput = {
  text: string;
  /**
   * Text shown in the local transcript when a composer command expands into a
   * richer runtime instruction. The provider only receives `text`.
   */
  displayText?: string;
  /**
   * Typed image blocks for the active turn. These are deliberately separate
   * from `text`, so binary media is never serialized into the chat transcript.
   */
  images?: ReadonlyArray<{
    data: string;
    mimeType: string;
  }>;
  sessionId?: string;
  conversationId?: string;
  providerId?: string;
  model?: string;
  reasoning?: PiReasoningLevel;
  permission?: string;
  /** Renderer-local marker created only after the user chooses Execute. */
  planApproval?: PiPlanApproval;
  cwd?: string;
  requestId: string;
};

/**
 * Identity material for the native-memory-only, one-use Full-access grant. The
 * renderer never owns a reusable authorization: this describes only the one
 * exact run for which the native host may mint an opaque token.
 */
export type FullPermissionGrantRequest = {
  requestId: string;
  sessionId: string;
  conversationId: string;
  workdir: string;
  text: string;
  providerId: string;
  model: string;
  reasoning?: PiReasoningLevel;
};

export type PiRunHandle = {
  sessionId?: string;
  conversationId?: string;
  turnId?: string;
  requestId: string;
};

export type PiRunResult = PiRunHandle & {
  assistantText?: string;
  mode?: string;
};

export type PiRuntimeTransport = {
  /** Subscribe before starting a turn; this avoids losing the first token. */
  subscribe(
    listener: (event: PiRunEvent) => void,
  ): Promise<() => void> | (() => void);
  run(input: PiRunInput): Promise<PiRunResult>;
  cancel(handle: PiRunHandle): Promise<void>;
  answerPermission?(
    requestId: string,
    decision: PermissionDecision,
  ): Promise<void>;
  answerPlanConfirmation?(
    requestId: string,
    planId: string,
    decision: PiPlanConfirmationDecision,
  ): Promise<boolean>;
  /** Issue a one-use continuation only for the exact terminal plan shown. */
  issuePlanContinuationApproval?(
    requestId: string,
    planId: string,
    sessionId?: string,
    conversationId?: string,
  ): PiPlanApproval | undefined;
  /** Invalidate a live renderer plan when its session/request loses focus. */
  invalidatePlanConfirmation?(requestId: string, planId?: string): void;
};

export type PiRuntimeStatus =
  | "idle"
  | "starting"
  | "running"
  | "waiting_permission"
  /** A cancellation signal is in flight; the underlying turn may still end normally. */
  | "cancelling"
  /** The cancellation signal was rejected, so the live turn remains retryable. */
  | "cancel_failed"
  | "completed"
  | "cancelled"
  | "error";

/** Terminal states are immutable with respect to late transport events. */
export function isPiRuntimeTerminal(status: PiRuntimeStatus) {
  return status === "completed" || status === "cancelled" || status === "error";
}

export type PiToolState = PiToolCall & {
  startedAt?: number;
  finishedAt?: number;
};

export type PiRuntimeState = {
  status: PiRuntimeStatus;
  sessionId?: string;
  conversationId?: string;
  turnId?: string;
  requestId?: string;
  prompt?: string;
  assistantText: string;
  thinkingText: string;
  tools: Record<string, PiToolState>;
  pendingPermission?: PiPermissionRequest;
  pendingPlan?: PiPlanConfirmation;
  error?: string;
  /** A failed cancellation is distinct from an error emitted by the Pi run. */
  cancellationError?: string;
  usage?: Record<string, unknown>;
  /** Stable run-start accounting retained after subsequent stream events. */
  contextTrim?: Record<string, unknown>;
  lastEvent?: PiRunEvent;
};

export const INITIAL_PI_RUNTIME_STATE: PiRuntimeState = {
  status: "idle",
  assistantText: "",
  thinkingText: "",
  tools: {},
};

export type PiRuntimeSnapshot = PiRuntimeState;

/**
 * A renderer-local transcript projection for a turn that has not necessarily
 * reached durable history yet. The id is stable for the lifetime of the
 * optimistic card; request and turn identities are filled in as the native
 * run acknowledges them.
 */
export type LiveTranscriptMessage = {
  id: string;
  sessionId: string;
  role: "user" | "assistant";
  content: string;
  /**
   * Provider reasoning is kept alongside the optimistic assistant projection
   * so a virtual-transcript rebuild cannot make an already-visible thinking
   * disclosure disappear before durable history catches up.
   */
  thinking?: string;
  /** Renderer-visible tool summary for this assistant turn. */
  tools?: readonly PiToolState[];
  createdAt: number;
  /** Terminal display time; unlike createdAt this is stable once assigned. */
  finishedAt?: number;
  requestId?: string;
  turnId?: string;
  model?: string;
  reasoning?: PiReasoningLevel;
  status?: PiRuntimeStatus;
  prompt?: string;
};

export type TranscriptWindowRenderedDetail = {
  sessionId: string;
  pageGeneration: number;
};

export type SessionViewInvalidatedDetail = {
  epoch: number;
  targetSessionId?: string;
};

/** A run-state change for one durable chat session, including background runs. */
export type PiSessionRunStateListener = (
  sessionId: string,
  state: PiRuntimeSnapshot,
) => void;

export type PiRuntimePublicApi = {
  submit(
    input: Omit<PiRunInput, "requestId">,
  ): Promise<PiRunResult | undefined>;
  /** A direct stop pauses queued prompts; the Composer stop button opts into
   * continuing the next prompt after the current cancellation settles. */
  cancel(options?: { resumeQueuedPrompt?: boolean }): Promise<void>;
  answerPermission(decision: PermissionDecision): Promise<void>;
  answerPlanConfirmation(
    decision: PiPlanConfirmationDecision,
  ): Promise<boolean>;
  issuePlanContinuationApproval(planId: string): PiPlanApproval | undefined;
  /** Revoke a visible plan card when the user defers or abandons it. */
  invalidatePlanConfirmation(planId: string, requestId?: string): void;
  getState(): PiRuntimeSnapshot;
  subscribe(listener: (state: PiRuntimeSnapshot) => void): () => void;
  /** Observe status transitions for every session, not only the visible one. */
  subscribeSessionState(listener: PiSessionRunStateListener): () => void;
  ready: Promise<void>;
};

declare global {
  interface Window {
    /** Optional direct Pi host supplied by the desktop bootstrap. */
    __novaveiPiEmbedded?: PiRuntimeTransport;
    /** Public API used by the existing static shell bridge. */
    __novaveiPiRuntime?: PiRuntimePublicApi;
    __novaveiFloorNav?: {
      refresh?: () => void;
    };
    /** Composer permission picker surface (permission-picker.ts). */
    __novaveiPermission?: {
      get: () => string;
      set: (value: string) => void;
      /**
       * Issues the grant for one exact Full-access run and returns its
       * short-lived opaque token. It is intentionally not cached.
       */
      requestFullPermissionGrant?: (
        request: FullPermissionGrantRequest,
      ) => Promise<string | undefined>;
    };
    /** Tauri's `withGlobalTauri` surface (kept intentionally minimal). */
    __TAURI__?: {
      core?: {
        invoke?: <T = unknown>(
          command: string,
          args?: Record<string, unknown>,
        ) => Promise<T>;
      };
      event?: {
        listen?: <T = unknown>(
          event: string,
          handler: (event: { payload: T }) => void,
        ) => Promise<() => void>;
      };
    };
  }
}
