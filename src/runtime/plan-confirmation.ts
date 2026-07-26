/**
 * Structured execution-plan protocol and renderer-side tool gate.
 *
 * The gate deliberately lives beside (not instead of) the native capability
 * boundary. It only coordinates a user-visible plan with the embedded Agent;
 * Rust still owns filesystem, process, and capability validation.
 */

import type {
  PiPlanApproval,
  PiPlanConfirmation,
  PiPlanConfirmationDecision,
  PiPlanStep,
  PiPlanToolScope,
} from "./types";

const MAX_PLAN_BUFFER_CHARS = 24_000;
const MAX_PLAN_SUMMARY_CHARS = 600;
const MAX_PLAN_STEP_CHARS = 500;
const MAX_PLAN_STEPS = 12;
const MAX_PLAN_IMPACT_CHARS = 900;
const MAX_PLAN_NOTICE_CHARS = 360;
const MAX_PLAN_NOTICES = 8;
const MAX_PLAN_TOOL_SCOPES = 24;
const MAX_PLAN_TOOL_NAME_CHARS = 160;
const MAX_PLAN_SCOPE_ARGUMENT_CHARS = 12_000;
const MAX_PLAN_SCOPE_DEPTH = 8;
const MAX_PLAN_SCOPE_COLLECTION_ITEMS = 128;

const READ_ONLY_TOOL_NAMES = new Set([
  "read",
  "projectread",
  "globalread",
  "list",
  "grep",
  "memorysearch",
  "skillslist",
  "skillread",
  "knowledgesearch",
  "knowledgebaseread",
  "delegatereadonly",
  // This native-bounded progress record is deliberately not a workspace or
  // command execution capability. It retains its existing low-risk policy.
  "goalprogressupdate",
]);

type PlanGateBlock = { block: true; reason: string };

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function boundedText(value: unknown, maximum: number): string | undefined {
  if (typeof value !== "string") return undefined;
  const normalized = value.trim().replace(/\s+/g, " ");
  if (!normalized) return undefined;
  return normalized.slice(0, maximum);
}

function makePlanId() {
  try {
    if (typeof crypto?.randomUUID === "function")
      return `plan-${crypto.randomUUID()}`;
  } catch {
    // Older WebViews can expose crypto without randomUUID.
  }
  return `plan-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

type PlanPayload = { index: number; end: number; payload: string };

function planPayloadsFromText(value: string): PlanPayload[] {
  const payloads: PlanPayload[] = [];
  const tagged = /<novavei-plan\s*>([\s\S]*?)<\/novavei-plan>/gi;
  const fenced = /```novavei-plan\s*\r?\n([\s\S]*?)```/gi;
  for (const match of value.matchAll(tagged)) {
    const payload = match[1]?.trim();
    if (payload)
      payloads.push({
        index: match.index ?? 0,
        end: (match.index ?? 0) + match[0].length,
        payload,
      });
  }
  for (const match of value.matchAll(fenced)) {
    const payload = match[1]?.trim();
    if (payload)
      payloads.push({
        index: match.index ?? 0,
        end: (match.index ?? 0) + match[0].length,
        payload,
      });
  }
  return payloads.sort((left, right) => left.index - right.index);
}

function boundedStringList(
  value: unknown,
  maximumItems: number,
  maximumChars: number,
): string[] | undefined {
  if (!Array.isArray(value) || !value.length || value.length > maximumItems)
    return undefined;
  const values: string[] = [];
  for (const item of value) {
    const text = boundedText(item, maximumChars);
    if (!text) return undefined;
    if (!values.includes(text)) values.push(text);
  }
  return values.length ? values : undefined;
}

function canonicalJson(value: unknown, depth = 0): string | undefined {
  if (depth > MAX_PLAN_SCOPE_DEPTH) return undefined;
  if (value === null) return "null";
  if (typeof value === "string") return JSON.stringify(value);
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "number")
    return Number.isFinite(value) ? JSON.stringify(value) : undefined;
  if (Array.isArray(value)) {
    if (value.length > MAX_PLAN_SCOPE_COLLECTION_ITEMS) return undefined;
    const items = value.map((item) => canonicalJson(item, depth + 1));
    return items.some((item) => item === undefined)
      ? undefined
      : `[${items.join(",")}]`;
  }
  const record = asRecord(value);
  if (!record) return undefined;
  const keys = Object.keys(record).sort();
  if (keys.length > MAX_PLAN_SCOPE_COLLECTION_ITEMS) return undefined;
  const entries: string[] = [];
  for (const key of keys) {
    const item = canonicalJson(record[key], depth + 1);
    if (item === undefined) return undefined;
    entries.push(`${JSON.stringify(key)}:${item}`);
  }
  return `{${entries.join(",")}}`;
}

function parseExecutionScope(value: unknown): PiPlanToolScope[] | undefined {
  if (
    !Array.isArray(value) ||
    !value.length ||
    value.length > MAX_PLAN_TOOL_SCOPES
  ) {
    return undefined;
  }
  const scopes: PiPlanToolScope[] = [];
  let totalArgumentChars = 0;
  for (const candidate of value) {
    const scope = asRecord(candidate);
    if (
      !scope ||
      Object.keys(scope).some(
        (key) => key !== "toolName" && key !== "arguments",
      ) ||
      !Object.hasOwn(scope, "arguments")
    ) {
      return undefined;
    }
    const toolName = boundedText(scope.toolName, MAX_PLAN_TOOL_NAME_CHARS);
    const canonicalArguments = canonicalJson(scope.arguments);
    if (
      !toolName ||
      !isPlanGatedTool(toolName) ||
      canonicalArguments === undefined ||
      canonicalArguments.length > MAX_PLAN_SCOPE_ARGUMENT_CHARS
    ) {
      return undefined;
    }
    totalArgumentChars += canonicalArguments.length;
    if (totalArgumentChars > MAX_PLAN_BUFFER_CHARS) return undefined;
    scopes.push({ toolName, arguments: scope.arguments });
  }
  return scopes;
}

function planFingerprint(value: {
  version: 1;
  summary: string;
  steps: PiPlanStep[];
  expectedImpact: string;
  risks: string[];
  permissions: string[];
  executionScope: PiPlanToolScope[];
}) {
  // A fixed local fingerprint lets us detect a material change without
  // retaining the provider's raw protocol block. It is a coordination value,
  // not a host capability or a cryptographic authorization primitive.
  const canonical = canonicalJson(value) ?? JSON.stringify(value);
  let hash = 0x811c9dc5;
  for (let index = 0; index < canonical.length; index += 1) {
    hash ^= canonical.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return `v1-${(hash >>> 0).toString(16).padStart(8, "0")}-${canonical.length}`;
}

function samePlan(
  left: PiPlanConfirmation,
  right: Omit<PiPlanConfirmation, "id" | "status" | "source">,
) {
  return (
    left.version === right.version &&
    left.fingerprint === right.fingerprint &&
    left.summary === right.summary &&
    left.expectedImpact === right.expectedImpact &&
    left.risks.length === right.risks.length &&
    left.risks.every((value, index) => value === right.risks[index]) &&
    left.permissions.length === right.permissions.length &&
    left.permissions.every(
      (value, index) => value === right.permissions[index],
    ) &&
    left.executionScope.length === right.executionScope.length &&
    left.executionScope.every(
      (scope, index) =>
        normalizedToolName(scope.toolName) ===
          normalizedToolName(right.executionScope[index]?.toolName ?? "") &&
        canonicalJson(scope.arguments) ===
          canonicalJson(right.executionScope[index]?.arguments),
    ) &&
    left.steps.length === right.steps.length &&
    left.steps.every(
      (step, index) =>
        step.title === right.steps[index]?.title &&
        step.detail === right.steps[index]?.detail,
    )
  );
}

function parsePlanPayload(
  payload: string,
): Omit<PiPlanConfirmation, "id" | "status"> | undefined {
  let parsed: unknown;
  try {
    parsed = JSON.parse(payload);
  } catch {
    return undefined;
  }
  const record = asRecord(parsed);
  if (!record) return undefined;
  const fields = new Set([
    "version",
    "summary",
    "expectedImpact",
    "risks",
    "permissions",
    "executionScope",
    "steps",
  ]);
  if (Object.keys(record).some((key) => !fields.has(key))) return undefined;
  if (record.version !== 1) return undefined;
  const summary = boundedText(record.summary, MAX_PLAN_SUMMARY_CHARS);
  const expectedImpact = boundedText(
    record.expectedImpact,
    MAX_PLAN_IMPACT_CHARS,
  );
  const rawSteps = Array.isArray(record.steps) ? record.steps : undefined;
  if (!rawSteps?.length || rawSteps.length > MAX_PLAN_STEPS) return undefined;
  const steps: PiPlanStep[] = [];
  for (const candidate of rawSteps) {
    const step = asRecord(candidate);
    if (
      !step ||
      Object.keys(step).some((key) => key !== "title" && key !== "detail")
    )
      return undefined;
    const title = boundedText(step?.title, MAX_PLAN_STEP_CHARS);
    if (!title) continue;
    const detail = boundedText(step?.detail, MAX_PLAN_STEP_CHARS);
    steps.push({ title, ...(detail ? { detail } : {}) });
  }
  const risks = boundedStringList(
    record.risks,
    MAX_PLAN_NOTICES,
    MAX_PLAN_NOTICE_CHARS,
  );
  const permissions = boundedStringList(
    record.permissions,
    MAX_PLAN_NOTICES,
    MAX_PLAN_NOTICE_CHARS,
  );
  const executionScope = parseExecutionScope(record.executionScope);
  if (
    !summary ||
    !expectedImpact ||
    steps.length !== rawSteps.length ||
    !risks ||
    !permissions ||
    !executionScope
  ) {
    return undefined;
  }
  const protocol = {
    version: 1 as const,
    summary,
    steps,
    expectedImpact,
    risks,
    permissions,
    executionScope,
  };
  return {
    ...protocol,
    fingerprint: planFingerprint(protocol),
    source: payload,
  };
}

function latestPlanFromText(value: string) {
  let latest: Omit<PiPlanConfirmation, "id" | "status"> | undefined;
  for (const { payload } of planPayloadsFromText(value)) {
    const parsed = parsePlanPayload(payload);
    if (parsed) latest = parsed;
  }
  return latest;
}

/**
 * Hide only complete, valid protocol blocks from ordinary reply Markdown.
 * Invalid or incomplete model output remains visible, so the card cannot
 * silently conceal a malformed plan while it is streaming.
 */
export function stripPlanProtocolBlocks(value: string) {
  const ranges = planPayloadsFromText(value)
    .filter(({ payload }) => Boolean(parsePlanPayload(payload)))
    .map(({ index, end }) => ({ index, end }))
    .sort((left, right) => left.index - right.index);
  if (!ranges.length) return value;
  let cursor = 0;
  let visible = "";
  for (const range of ranges) {
    if (range.index < cursor) continue;
    visible += value.slice(cursor, range.index);
    cursor = range.end;
  }
  return `${visible}${value.slice(cursor)}`.replace(/\n{3,}/g, "\n\n").trim();
}

/**
 * Reconstruct presentation-only cards from durable assistant text. Historical
 * plans never regain an Execute action after restart; this keeps the record
 * readable without treating stored model text as a permission capability.
 */
export function planConfirmationsFromText(
  value: string,
  idPrefix = "history-plan",
): PiPlanConfirmation[] {
  const plans: PiPlanConfirmation[] = [];
  for (const { index, payload } of planPayloadsFromText(value)) {
    const parsed = parsePlanPayload(payload);
    if (!parsed) continue;
    plans.push({
      ...parsed,
      id: `${idPrefix}-${index}-${parsed.fingerprint}`,
      source: "",
      status: "deferred",
    });
  }
  return plans;
}

function normalizedToolName(name: string) {
  return name
    .trim()
    .toLowerCase()
    .replace(/[-_\s]/g, "");
}

const UNBOUND_CONTENT_ARGUMENTS = new Map<string, ReadonlySet<string>>([
  ["write", new Set(["content"])],
  ["edit", new Set(["old_string", "new_string"])],
]);

function scopedToolArguments(value: unknown) {
  return value === undefined ? {} : value;
}

function cloneExecutionScope(
  executionScope: readonly PiPlanToolScope[],
): PiPlanToolScope[] {
  return executionScope.map((scope) => {
    const canonicalArguments = canonicalJson(scope.arguments);
    return {
      toolName: scope.toolName,
      arguments:
        canonicalArguments === undefined
          ? null
          : JSON.parse(canonicalArguments),
    };
  });
}

function clonePlan(plan: PiPlanConfirmation): PiPlanConfirmation {
  return {
    ...plan,
    steps: plan.steps.map((step) => ({ ...step })),
    risks: [...plan.risks],
    permissions: [...plan.permissions],
    executionScope: cloneExecutionScope(plan.executionScope),
  };
}

function cloneApproval(approval: PiPlanApproval): PiPlanApproval {
  return {
    ...approval,
    executionScope: cloneExecutionScope(approval.executionScope),
  };
}

function planToolScopeMatches(
  scope: PiPlanToolScope | undefined,
  toolName: string,
  toolArguments: unknown,
) {
  if (!scope) return false;
  const normalizedName = normalizedToolName(toolName);
  if (normalizedToolName(scope.toolName) !== normalizedName) return false;
  const expected = asRecord(scope.arguments);
  const actualValue = scopedToolArguments(toolArguments);
  const actual = asRecord(actualValue);
  if (!expected || !actual) {
    return canonicalJson(scope.arguments) === canonicalJson(actualValue);
  }
  for (const [key, value] of Object.entries(expected)) {
    if (
      !Object.hasOwn(actual, key) ||
      canonicalJson(value) !== canonicalJson(actual[key])
    ) {
      return false;
    }
  }
  const mayRemainUnbound = UNBOUND_CONTENT_ARGUMENTS.get(normalizedName);
  for (const key of Object.keys(actual)) {
    if (Object.hasOwn(expected, key) || mayRemainUnbound?.has(key)) continue;
    return false;
  }
  return true;
}

function planScopeKey(toolName: string, toolArguments: unknown) {
  return `${normalizedToolName(toolName)}:${canonicalJson(
    scopedToolArguments(toolArguments),
  )}`;
}

function planScopeLabel(scope: PiPlanToolScope) {
  const argumentsRecord = asRecord(scope.arguments);
  if (!argumentsRecord) {
    return `${scope.toolName}(${canonicalJson(scope.arguments) ?? "invalid arguments"})`;
  }
  const normalizedName = normalizedToolName(scope.toolName);
  const bodyArguments = UNBOUND_CONTENT_ARGUMENTS.get(normalizedName);
  const labels = Object.keys(argumentsRecord)
    .sort()
    .map((key) => {
      const value = argumentsRecord[key];
      if (bodyArguments?.has(key)) {
        const length = canonicalJson(value)?.length ?? 0;
        return `${key}=<bound body: ${length} JSON chars>`;
      }
      return `${key}=${canonicalJson(value) ?? "invalid"}`;
    });
  for (const key of bodyArguments ?? []) {
    if (!Object.hasOwn(argumentsRecord, key))
      labels.push(`${key}=<not bound by plan>`);
  }
  return `${scope.toolName}(${labels.join(", ")})`;
}

function planScopeArgumentText(
  scope: PiPlanToolScope,
  key: string,
  value: unknown,
) {
  const bodyArguments = UNBOUND_CONTENT_ARGUMENTS.get(
    normalizedToolName(scope.toolName),
  );
  if (bodyArguments?.has(key)) return "内容在执行时提供，未由计划绑定";
  return canonicalJson(value) ?? "无效参数";
}

/**
 * Unknown tools, including MCP tools, are conservative: unless they are in
 * the small explicit read-only allowlist, a structured plan gates them.
 */
export function isPlanGatedTool(name: string) {
  return !READ_ONLY_TOOL_NAMES.has(normalizedToolName(name));
}

/**
 * Stable protocol text added to the provider system prompt for normal runs.
 */
export const PLAN_CONFIRMATION_SYSTEM_PROMPT = [
  "NovaVei execution-plan protocol:",
  "Before invoking a tool that can write, delete, persist, delegate implementation, call MCP, or run a shell command, first emit exactly one structured plan block and do not mutate before that block.",
  'Use either <novavei-plan>{"version":1,"summary":"...","expectedImpact":"...","risks":["..."],"permissions":["..."],"executionScope":[{"toolName":"Write","arguments":{"path":"src/example.ts","mode":"overwrite"}},{"toolName":"Bash","arguments":{"command":"npm test","cwd":"."}}],"steps":[{"title":"...","detail":"..."}]}</novavei-plan> or an equivalent ```novavei-plan JSON fence. Every field is required: use explicit no-risk/no-new-permission text where appropriate.',
  "executionScope is an ordered, one-use list of the mutable tool calls this plan may perform. Bind every actual argument exactly. For Write only content may be omitted; for Edit only old_string and new_string may be omitted. Paths, commands, cwd, modes, flags, expected counts/hashes, and all arguments of every other tool must be present exactly. Never put secrets in this scope.",
  "After the complete plan block, request the needed tool call. NovaVei will pause that write or command until the user chooses Execute. Read-only inspection tools may continue without a plan.",
  "If the user asks to modify or defer a plan, do not retry a write or command automatically; wait for the next user instruction. If scope, impact, risk, permissions, or steps materially change, emit a replacement plan block and wait for another confirmation.",
].join("\n");

/**
 * Renderer-local approval token for a follow-up turn after a plan-only reply.
 * This is not a native capability and must never be described as host policy.
 */
export function planExecutionFollowUpText(plan: PiPlanConfirmation) {
  const steps = plan.steps
    .map(
      (step, index) =>
        `${index + 1}. ${step.title}${step.detail ? ` — ${step.detail}` : ""}`,
    )
    .join("\n");
  return [
    "The user explicitly approved this NovaVei structured plan. Execute only this version; do not broaden its scope or substitute a different plan.",
    `Goal: ${plan.summary}`,
    `Expected impact: ${plan.expectedImpact}`,
    `Steps:\n${steps}`,
    `Risks: ${plan.risks.join("; ")}`,
    `Permission notes: ${plan.permissions.join("; ")}`,
    `Approved tool scope: ${plan.executionScope.map(planScopeLabel).join("; ")}`,
    "Re-check the current workspace before changing it and preserve normal per-tool permission checks. If this plan materially changes, emit a replacement structured plan and wait for confirmation.",
  ].join(" ");
}

/**
 * Accumulates only assistant text for one run. Once a complete protocol block
 * appears, medium/high-risk tools wait here before PermissionBroker evaluates
 * their existing native-backed permission policy.
 */
export class PlanConfirmationGate {
  private buffer = "";
  private plan: PiPlanConfirmation | undefined;
  private decision: PiPlanConfirmationDecision | undefined;
  private invalidated = false;
  private nextScopeIndex = 0;
  private pendingScope: { index: number; invocationKey: string } | undefined;
  private readonly waiters = new Set<
    (decision: PiPlanConfirmationDecision | "invalidated") => void
  >();
  private readonly approvedFollowUp?: PiPlanApproval;
  private readonly enabled: boolean;

  constructor(approval?: PiPlanApproval, enabled = true) {
    this.approvedFollowUp = approval ? cloneApproval(approval) : undefined;
    this.enabled = enabled;
  }

  observeAssistantText(delta: string): PiPlanConfirmation | undefined {
    if (!this.enabled || this.invalidated || !delta) return undefined;
    this.buffer = `${this.buffer}${delta}`.slice(-MAX_PLAN_BUFFER_CHARS);
    const parsed = latestPlanFromText(this.buffer);
    if (!parsed) return undefined;
    if (this.plan && samePlan(this.plan, parsed)) return undefined;
    // A later valid protocol block is a material plan replacement.  Revoke
    // any Execute decision, unblock waiters as invalidated, and force the new
    // card to be reviewed before another gated tool can begin.
    if (this.plan) {
      for (const resolve of this.waiters) resolve("invalidated");
      this.waiters.clear();
    }
    this.nextScopeIndex = 0;
    this.pendingScope = undefined;
    this.decision = undefined;
    this.plan = { id: makePlanId(), status: "pending", ...parsed };
    return clonePlan(this.plan);
  }

  currentPlan() {
    return this.plan ? clonePlan(this.plan) : undefined;
  }

  answer(planId: string, decision: PiPlanConfirmationDecision) {
    if (
      this.invalidated ||
      !this.plan ||
      this.plan.id !== planId ||
      this.decision
    ) {
      return false;
    }
    this.decision = decision;
    this.plan = {
      ...this.plan,
      status:
        decision === "execute"
          ? "approved"
          : decision === "modify"
            ? "modify_requested"
            : "deferred",
    };
    for (const resolve of this.waiters) resolve(decision);
    this.waiters.clear();
    return true;
  }

  invalidate(planId?: string) {
    const currentPlanId = this.plan?.id ?? this.approvedFollowUp?.planId;
    if (
      this.invalidated ||
      !currentPlanId ||
      (planId && currentPlanId !== planId)
    ) {
      return false;
    }
    this.invalidated = true;
    if (!this.decision) this.decision = "not_now";
    for (const resolve of this.waiters) resolve("invalidated");
    this.waiters.clear();
    return true;
  }

  private waitForDecision(signal?: AbortSignal) {
    if (this.decision) return Promise.resolve(this.decision);
    return new Promise<PiPlanConfirmationDecision | "invalidated">(
      (resolve) => {
        let settled = false;
        const finish = (
          decision: PiPlanConfirmationDecision | "invalidated",
        ) => {
          if (settled) return;
          settled = true;
          this.waiters.delete(finish);
          signal?.removeEventListener("abort", onAbort);
          resolve(decision);
        };
        const onAbort = () => finish("invalidated");
        this.waiters.add(finish);
        signal?.addEventListener("abort", onAbort, { once: true });
        if (signal?.aborted) finish("invalidated");
      },
    );
  }

  private executionScope() {
    return (
      this.plan?.executionScope ?? this.approvedFollowUp?.executionScope ?? []
    );
  }

  private matchingScopeIndex(toolName: string, toolArguments: unknown) {
    const invocationKey = planScopeKey(toolName, toolArguments);
    const scopeIndex = this.pendingScope?.index ?? this.nextScopeIndex;
    if (
      scopeIndex !== this.nextScopeIndex ||
      (this.pendingScope && this.pendingScope.invocationKey !== invocationKey)
    ) {
      return -1;
    }
    return planToolScopeMatches(
      this.executionScope()[scopeIndex],
      toolName,
      toolArguments,
    )
      ? scopeIndex
      : -1;
  }

  async checkTool(
    toolName: string,
    toolArguments?: unknown,
    signal?: AbortSignal,
  ): Promise<PlanGateBlock | undefined> {
    if (!this.enabled || !isPlanGatedTool(toolName)) return undefined;
    if (this.invalidated) {
      return {
        block: true,
        reason: "执行计划确认已失效；本轮写入或命令未执行。",
      };
    }
    // Never rely solely on the provider instruction. A gated tool without a
    // valid structured plan is denied before PermissionBroker can expose a
    // native approval path.
    if (!this.plan && !this.approvedFollowUp) {
      return {
        block: true,
        reason:
          "请先生成包含目标、影响、风险、权限提示、机器执行范围和步骤的结构化执行计划；本轮写入或命令未执行。",
      };
    }
    const scopeIndex = this.matchingScopeIndex(toolName, toolArguments);
    if (scopeIndex < 0) {
      return {
        block: true,
        reason:
          "工具名称或参数超出已展示执行计划的机器范围；本轮写入或命令未执行。",
      };
    }
    const decision = this.plan
      ? await this.waitForDecision(signal)
      : this.approvedFollowUp
        ? "execute"
        : "invalidated";
    if (decision === "execute" && !this.invalidated) {
      this.pendingScope = {
        index: scopeIndex,
        invocationKey: planScopeKey(toolName, toolArguments),
      };
      return undefined;
    }
    return {
      block: true,
      reason:
        decision === "modify"
          ? "用户要求先修改执行计划；本轮写入或命令未执行。"
          : decision === "not_now"
            ? "用户选择暂不执行该计划；本轮写入或命令未执行。"
            : "执行计划确认已失效；本轮写入或命令未执行。",
    };
  }

  /**
   * Re-check immediately after PermissionBroker resolves. This closes the
   * focus-switch race where Execute was clicked, then the plan was revoked
   * while a native per-tool confirmation was still pending.
   */
  canProceedAfterPermission(toolName: string, toolArguments?: unknown) {
    if (!this.enabled || !isPlanGatedTool(toolName)) return true;
    if (this.invalidated) return false;
    if (this.plan ? this.decision !== "execute" : !this.approvedFollowUp)
      return false;
    const invocationKey = planScopeKey(toolName, toolArguments);
    if (
      !this.pendingScope ||
      this.pendingScope.index !== this.nextScopeIndex ||
      this.pendingScope.invocationKey !== invocationKey ||
      !planToolScopeMatches(
        this.executionScope()[this.pendingScope.index],
        toolName,
        toolArguments,
      )
    ) {
      return false;
    }
    this.nextScopeIndex += 1;
    this.pendingScope = undefined;
    return true;
  }
}

export type PlanConfirmationCardResult =
  | "approved"
  | "executing"
  | "modify_requested"
  | "deferred"
  | "recorded"
  | "retry"
  | "invalidated";

export type PlanConfirmationCardAction = (
  plan: PiPlanConfirmation,
  decision: PiPlanConfirmationDecision,
) => Promise<PlanConfirmationCardResult> | PlanConfirmationCardResult;

type PlanCardRecord = {
  root: HTMLElement;
  requestId?: string;
  plan: PiPlanConfirmation;
  action: PlanConfirmationCardAction;
  busy: boolean;
  manualStatus?: PlanConfirmationCardResult;
};

function planCardStatus(
  plan: PiPlanConfirmation,
  manualStatus?: PlanConfirmationCardResult,
) {
  switch (manualStatus ?? plan.status) {
    case "approved":
      return "已确认，正在进入执行与权限审批。";
    case "executing":
      return "正在启动已确认的计划。";
    case "modify_requested":
      return "已请求修改；当前写入或命令已阻止。";
    case "deferred":
      return "已暂不执行；当前写入或命令已阻止。";
    case "recorded":
      return "历史计划记录；如需执行，请基于当前状态重新确认。";
    case "retry":
      return "未能启动执行；可再次点击“执行”重试。";
    case "invalidated":
      return "该计划确认已失效；当前写入或命令未执行。";
    default:
      return "请确认后再进入写入或命令执行。";
  }
}

function cardAllowsDecision(record: PlanCardRecord) {
  return (
    !record.busy &&
    (record.manualStatus === "retry" ||
      (!record.manualStatus && record.plan.status === "pending"))
  );
}

/**
 * A compact, accessible card rendered inside the assistant message. It uses
 * existing Luminous Quiet button/card classes, avoiding a new visual shell.
 */
export class PlanConfirmationCards {
  private readonly records = new Map<string, PlanCardRecord>();

  private key(requestId: string | undefined, planId: string) {
    return `${requestId ?? "unbound"}:${planId}`;
  }

  private update(record: PlanCardRecord) {
    const root = record.root;
    const summary = root.querySelector<HTMLElement>("[data-plan-summary]");
    const steps = root.querySelector<HTMLOListElement>("[data-plan-steps]");
    const impact = root.querySelector<HTMLElement>("[data-plan-impact]");
    const risks = root.querySelector<HTMLUListElement>("[data-plan-risks]");
    const permissions = root.querySelector<HTMLUListElement>(
      "[data-plan-permissions]",
    );
    const executionScope = root.querySelector<HTMLUListElement>(
      "[data-plan-execution-scope]",
    );
    const state = root.querySelector<HTMLElement>("[data-plan-status]");
    const stateChip = root.querySelector<HTMLElement>("[data-plan-state-chip]");
    const stepCount = root.querySelector<HTMLElement>("[data-plan-step-count]");
    const scopeCount = root.querySelector<HTMLElement>(
      "[data-plan-scope-count]",
    );
    if (summary) summary.textContent = record.plan.summary;
    if (impact) impact.textContent = `预期影响：${record.plan.expectedImpact}`;
    if (steps) {
      steps.replaceChildren(
        ...record.plan.steps.map((step) => {
          const item = document.createElement("li");
          const title = document.createElement("strong");
          title.textContent = step.title;
          item.appendChild(title);
          if (step.detail) {
            const detail = document.createElement("span");
            detail.textContent = ` — ${step.detail}`;
            item.appendChild(detail);
          }
          return item;
        }),
      );
    }
    if (risks) {
      risks.replaceChildren(
        ...record.plan.risks.map((risk) => {
          const item = document.createElement("li");
          item.textContent = risk;
          return item;
        }),
      );
    }
    if (permissions) {
      permissions.replaceChildren(
        ...record.plan.permissions.map((permission) => {
          const item = document.createElement("li");
          item.textContent = permission;
          return item;
        }),
      );
    }
    if (executionScope) {
      executionScope.replaceChildren(
        ...record.plan.executionScope.map((scope) => {
          const item = document.createElement("li");
          item.className = "plan-scope-entry";
          const command = document.createElement("div");
          command.className = "plan-scope-command";
          const tool = document.createElement("code");
          tool.textContent = scope.toolName;
          const note = document.createElement("span");
          note.textContent = "仅限以下参数";
          command.append(tool, note);

          const argumentsRecord = asRecord(scope.arguments);
          if (argumentsRecord) {
            const argumentsList = document.createElement("dl");
            argumentsList.className = "plan-scope-arguments";
            for (const key of Object.keys(argumentsRecord).sort()) {
              const term = document.createElement("dt");
              term.textContent = key;
              const description = document.createElement("dd");
              const value = document.createElement("code");
              value.textContent = planScopeArgumentText(
                scope,
                key,
                argumentsRecord[key],
              );
              description.append(value);
              argumentsList.append(term, description);
            }
            item.append(command, argumentsList);
          } else {
            const preview = document.createElement("code");
            preview.className = "plan-scope-preview";
            preview.textContent = planScopeLabel(scope);
            item.append(command, preview);
          }
          return item;
        }),
      );
    }
    if (state)
      state.textContent = record.busy
        ? "正在提交确认…"
        : planCardStatus(record.plan, record.manualStatus);
    if (stateChip) {
      const status = record.manualStatus ?? record.plan.status;
      stateChip.textContent = record.busy
        ? "确认中"
        : status === "approved" || status === "executing"
          ? "已确认"
          : status === "modify_requested"
            ? "待修改"
            : status === "deferred"
              ? "已暂缓"
              : status === "recorded"
                ? "历史记录"
                : status === "invalidated"
                  ? "已失效"
                  : status === "retry"
                    ? "可重试"
                    : "等待确认";
    }
    if (stepCount) stepCount.textContent = `${record.plan.steps.length} 步`;
    if (scopeCount)
      scopeCount.textContent = `${record.plan.executionScope.length} 项`;
    const enabled = cardAllowsDecision(record);
    for (const button of root.querySelectorAll<HTMLButtonElement>(
      "[data-plan-decision]",
    )) {
      button.disabled = !enabled;
    }
    root.dataset.planStatus = record.manualStatus ?? record.plan.status;
    root.setAttribute("aria-busy", record.busy ? "true" : "false");
  }

  private create(
    requestId: string | undefined,
    plan: PiPlanConfirmation,
    action: PlanConfirmationCardAction,
  ) {
    const root = document.createElement("section");
    root.className = "composer-permission-prompt novavei-plan-confirmation";
    root.dataset.novaveiPlanConfirmation = plan.id;
    root.setAttribute("role", "region");
    const headingId = `novavei-plan-${plan.id}`;
    root.setAttribute("aria-labelledby", headingId);

    const header = document.createElement("div");
    header.className = "plan-confirmation-header";
    const headerCopy = document.createElement("div");
    headerCopy.className = "plan-confirmation-header-copy";
    const eyebrow = document.createElement("span");
    eyebrow.className = "plan-confirmation-eyebrow";
    eyebrow.textContent = "执行确认";
    const heading = document.createElement("h3");
    heading.className = "composer-permission-message";
    heading.id = headingId;
    heading.textContent = "执行计划";
    const stateChip = document.createElement("span");
    stateChip.className = "plan-confirmation-state";
    stateChip.dataset.planStateChip = "true";
    headerCopy.append(eyebrow, heading);
    header.append(headerCopy, stateChip);
    const summary = document.createElement("p");
    summary.className = "composer-permission-detail";
    summary.dataset.planSummary = "true";
    const impact = document.createElement("p");
    impact.className = "composer-permission-detail";
    impact.dataset.planImpact = "true";
    const reviewGrid = document.createElement("div");
    reviewGrid.className = "plan-review-grid";
    const stepsSection = document.createElement("section");
    stepsSection.className = "plan-review-section plan-review-section-steps";
    const stepsHeader = document.createElement("div");
    stepsHeader.className = "plan-review-section-head";
    const stepsHeading = document.createElement("h4");
    stepsHeading.textContent = "执行步骤";
    const stepCount = document.createElement("span");
    stepCount.dataset.planStepCount = "true";
    stepsHeader.append(stepsHeading, stepCount);
    const steps = document.createElement("ol");
    steps.className = "composer-run-steps";
    steps.dataset.planSteps = "true";
    steps.setAttribute("aria-label", "执行步骤");
    stepsSection.append(stepsHeader, steps);
    const scopeSection = document.createElement("section");
    scopeSection.className = "plan-review-section plan-review-section-scope";
    const scopeHeader = document.createElement("div");
    scopeHeader.className = "plan-review-section-head";
    const scopeHeading = document.createElement("h4");
    scopeHeading.textContent = "允许的工具范围";
    const scopeCount = document.createElement("span");
    scopeCount.dataset.planScopeCount = "true";
    scopeHeader.append(scopeHeading, scopeCount);
    const scopeHint = document.createElement("p");
    scopeHint.className = "plan-review-hint";
    scopeHint.textContent = "执行时将逐项校验工具和参数；范围外调用会被阻止。";
    const executionScope = document.createElement("ul");
    executionScope.className = "composer-run-steps";
    executionScope.dataset.planExecutionScope = "true";
    executionScope.setAttribute("aria-label", "允许的工具范围");
    scopeSection.append(scopeHeader, scopeHint, executionScope);
    const safetySection = document.createElement("section");
    safetySection.className = "plan-review-section plan-review-section-safety";
    const riskHeading = document.createElement("h4");
    riskHeading.textContent = "风险提示";
    const risks = document.createElement("ul");
    risks.className = "composer-run-steps plan-safety-list";
    risks.dataset.planRisks = "true";
    risks.setAttribute("aria-label", "风险提示");
    const permissionHeading = document.createElement("h4");
    permissionHeading.textContent = "权限提示";
    const permissions = document.createElement("ul");
    permissions.className = "composer-run-steps plan-safety-list";
    permissions.dataset.planPermissions = "true";
    permissions.setAttribute("aria-label", "权限提示");
    safetySection.append(riskHeading, risks, permissionHeading, permissions);
    reviewGrid.append(stepsSection, safetySection, scopeSection);
    const status = document.createElement("p");
    status.className = "composer-permission-detail plan-confirmation-status";
    status.dataset.planStatus = "true";
    status.setAttribute("role", "status");
    status.setAttribute("aria-live", "polite");
    const actions = document.createElement("div");
    actions.className =
      "row-actions composer-permission-actions plan-confirmation-actions";
    actions.setAttribute("aria-label", "执行计划操作");
    const footer = document.createElement("div");
    footer.className = "plan-confirmation-footer";
    const record: PlanCardRecord = {
      root,
      requestId,
      plan,
      action,
      busy: false,
    };
    const choices: Array<[PiPlanConfirmationDecision, string, string]> = [
      ["execute", "执行", "primary"],
      ["modify", "修改计划", ""],
      ["not_now", "暂不执行", "ghost"],
    ];
    for (const [decision, label, appearance] of choices) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = `btn${appearance ? ` ${appearance}` : ""}`;
      button.dataset.planDecision = decision;
      button.textContent = label;
      button.addEventListener("click", () => {
        if (!cardAllowsDecision(record)) return;
        record.busy = true;
        this.update(record);
        void Promise.resolve(record.action(record.plan, decision)).then(
          (result) => {
            record.busy = false;
            record.manualStatus = result;
            this.update(record);
          },
          () => {
            record.busy = false;
            record.manualStatus = "retry";
            this.update(record);
          },
        );
      });
      actions.appendChild(button);
    }
    root.append(header, summary, impact, reviewGrid, footer);
    footer.append(status, actions);
    return record;
  }

  render(
    article: HTMLElement,
    requestId: string | undefined,
    plan: PiPlanConfirmation,
    action: PlanConfirmationCardAction,
  ) {
    const key = this.key(requestId, plan.id);
    const record =
      this.records.get(key) ?? this.create(requestId, plan, action);
    if (!this.records.has(key)) this.records.set(key, record);
    record.requestId = requestId;
    record.plan = plan;
    record.action = action;
    const actions = article.querySelector<HTMLElement>(".msg-actions");
    if (record.root.parentElement !== article) {
      if (actions) actions.before(record.root);
      else article.appendChild(record.root);
    }
    this.update(record);
  }

  setStatus(
    requestId: string | undefined,
    planId: string,
    status: PlanConfirmationCardResult,
  ) {
    const record = this.records.get(this.key(requestId, planId));
    if (!record) return;
    record.busy = false;
    record.manualStatus = status;
    this.update(record);
  }

  invalidateRequest(requestId: string | undefined) {
    for (const record of this.records.values()) {
      if (record.requestId !== requestId) continue;
      record.busy = false;
      record.manualStatus = "invalidated";
      this.update(record);
    }
  }

  invalidateAll() {
    for (const record of this.records.values()) {
      record.busy = false;
      record.manualStatus = "invalidated";
      this.update(record);
    }
  }

  dispose() {
    this.records.clear();
  }
}
