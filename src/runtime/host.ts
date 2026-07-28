/**
 * Native shell bindings for the existing HTML surface.
 *
 * This module deliberately creates no new layout. It replaces the prototype
 * session and project actions with the already-registered Tauri commands once
 * the app is running inside a desktop WebView. Plain browser preview mode keeps
 * the original static behavior.
 */

import { renderComposerMessageMedia } from "./attachments";
import { requestAppChoice } from "./app-dialogs";
import {
  createAssistantThinkingPanel,
  renderAssistantThinkingTools,
} from "./dom";
import { renderMarkdown } from "./markdown";
import {
  applyFullMessageTimestampPreference,
  formatMessageTimestamp,
  normalizeFullMessageTimestampPreference,
} from "./message-time";
import { displayPath, pathKey, pathName } from "./path-display";
import { stripPlanProtocolBlocks } from "./plan-confirmation";
import type {
  LiveTranscriptMessage,
  PiReasoningLevel,
  PiRuntimeSnapshot,
  PiToolState,
} from "./types";

type UnknownRecord = Record<string, unknown>;
type Invoke = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

export type SessionSummary = {
  id: string;
  title: string;
  cwd: string;
  /**
   * Native, non-authoritative accessibility hint for the historical workspace.
   * The shell also refreshes a batched status map for project-only paths.
   */
  workspaceStatus?: WorkspaceLocationStatus;
  updatedAt?: number;
  isPinned?: boolean;
  isArchived?: boolean;
  lastRunStatus?: PersistedSessionRunStatus;
  lastRunFinishedAt?: number;
};

type WorkspaceLocationStatus =
  | "available"
  | "missing"
  | "not_directory"
  | "unavailable";

type PersistedSessionRunStatus =
  | "completed"
  | "cancelled"
  | "error"
  | "interrupted";

type SessionModelSelection = {
  providerId: string;
  modelId: string;
};

export type ProjectModelPreference = {
  providerId: string;
  modelId: string;
};

/**
 * Durable project-level interaction policy. Full access is intentionally
 * absent: it always uses a fresh, one-use grant for one exact run.
 */
export type ProjectPermissionPreference = "readonly" | "ask" | "auto-approve";

/**
 * Per-project defaults are deliberately separate from a conversation's saved
 * model selection. Conversation overrides and provider defaults retain their
 * existing precedence when this object is absent.
 */
export type ProjectPreferences = {
  model?: ProjectModelPreference;
  reasoning?: PiReasoningLevel;
  permission?: ProjectPermissionPreference;
};

type ProjectEntry = {
  id: string;
  name: string;
  path: string;
  lastSessionId?: string;
  pinned?: boolean;
  preferences?: ProjectPreferences;
};

type ProjectSettings = {
  initialized: boolean;
  entries: ProjectEntry[];
};

type WorkspaceRelocationConflictResolution =
  | "keep_source"
  | "merge_into_target";

type WorkspaceRelocationProject = {
  id: string;
  name: string;
  path: string;
};

type WorkspaceRelocationConflict = {
  sourceProject?: WorkspaceRelocationProject;
  targetProject: WorkspaceRelocationProject;
};

type WorkspaceRelocationResponse =
  | {
      status: "conflict";
      fromWorkdir: string;
      toWorkdir: string;
      updatedSessionIds: string[];
      updatedProjectIds: string[];
      conflict: WorkspaceRelocationConflict;
      conflictToken: string;
    }
  | {
      status: "relocated";
      fromWorkdir: string;
      toWorkdir: string;
      updatedSessionIds: string[];
      updatedProjectIds: string[];
    };

type NativeShellState =
  | "loading"
  | "ready"
  | "needs_workspace"
  | "needs_session"
  | "needs_relocation"
  | "storage_recovery"
  | "error";

class StorageRecoveryRequiredError extends Error {
  constructor() {
    super("NovaVei 本地存储需要恢复后才能继续使用。");
    this.name = "StorageRecoveryRequiredError";
  }
}

type MessageRecord = {
  id?: string;
  /** Stable renderer identity retained while a provisional row reconciles. */
  liveId?: string;
  role?: string;
  content?: string;
  createdAt?: number;
  turnId?: string;
  requestId?: string;
  status?: string;
  prompt?: string;
  model?: string;
  modelId?: string;
  modelLabel?: string;
  reasoning?: string;
  reasoningLevel?: string;
  finishedAt?: number;
  endedAt?: number;
  thinking?: string;
  tools?: PiToolState[];
};

type SessionsGetPage = {
  messages: MessageRecord[];
  totalCount: number;
  hasMoreBefore: boolean;
};

type SessionMessagePageState = {
  messages: MessageRecord[];
  totalCount: number;
  hasMoreBefore: boolean;
  /** Inclusive start index into `messages` for the currently rendered DOM window. */
  domStart: number;
  /** Object-local generation invalidates stale pagination completions. */
  generation: number;
  /** Session presentation metadata shared by all renders of this page. */
  history?: unknown;
};

type SessionPageRequest = {
  sessionId: string;
  pageState: SessionMessagePageState;
  viewEpoch: number;
};

type SessionViewNavigation = {
  epoch: number;
  serial: number;
  targetSessionId?: string;
};

/** Cap on transcript DOM nodes; full history stays in pageState.messages. */
const MAX_DOM_MESSAGES = 160;
/** Bound inactive transcript pages while retaining the selected and live runs. */
export const MAX_CACHED_SESSION_MESSAGE_PAGES = 8;
const DEFAULT_HISTORY_MESSAGE_PAGE_SIZE = 80;
const MIN_HISTORY_MESSAGE_PAGE_SIZE = 40;
const MAX_HISTORY_MESSAGE_PAGE_SIZE = 200;
const MAX_WORKSPACE_PATH_STATUS_BATCH_SIZE = 512;
const HISTORY_MESSAGE_PAGE_SIZE_STORAGE_KEY = "novavei.historyMessagePageSize";
/** Pixels from the bottom that count as "near the end" for window expansion. */
const VIRTUAL_WINDOW_BOTTOM_THRESHOLD_PX = 96;
let nextSessionMessagePageGeneration = 0;

export function pruneSessionMessagePageCache<T>(
  cache: Map<string, T>,
  protectedSessionIds: ReadonlySet<string>,
  maxEntries = MAX_CACHED_SESSION_MESSAGE_PAGES,
) {
  const boundedMax = Math.max(0, Math.trunc(maxEntries));
  for (const cachedSessionId of cache.keys()) {
    if (cache.size <= boundedMax) break;
    if (protectedSessionIds.has(cachedSessionId)) continue;
    cache.delete(cachedSessionId);
  }
}

/** Prefer camelCase (serde) but accept snake_case for defensive IPC parsing. */
function optionalString(...candidates: unknown[]): string | undefined {
  for (const candidate of candidates) {
    if (typeof candidate === "string" && candidate.trim()) return candidate;
  }
  return undefined;
}

function optionalNumber(...candidates: unknown[]): number | undefined {
  for (const candidate of candidates) {
    if (typeof candidate === "number" && Number.isFinite(candidate))
      return candidate;
    if (typeof candidate === "string" && candidate.trim()) {
      const parsed = Number(candidate);
      if (Number.isFinite(parsed)) return parsed;
    }
  }
  return undefined;
}

function normalizeMessageTool(
  value: unknown,
  index: number,
): PiToolState | undefined {
  if (!value || typeof value !== "object") return undefined;
  const raw = value as Record<string, unknown>;
  const id =
    optionalString(raw.id, raw.toolCallId, raw.tool_call_id) ??
    `message-tool-${index}`;
  const name = optionalString(raw.name, raw.toolName, raw.tool_name) ?? "工具";
  return {
    id,
    name,
    arguments: raw.arguments ?? raw.args ?? raw.input,
    result: raw.result ?? raw.output,
    error: optionalString(raw.error, raw.toolError, raw.tool_error),
    status: optionalString(raw.status),
    startedAt: optionalNumber(raw.startedAt, raw.started_at),
    finishedAt: optionalNumber(raw.finishedAt, raw.finished_at),
  };
}

function normalizeMessageTools(value: unknown): PiToolState[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const tools = value
    .slice(0, 128)
    .map(normalizeMessageTool)
    .filter((tool): tool is PiToolState => Boolean(tool));
  return tools.length ? tools : undefined;
}

function normalizeMessageRecord(value: unknown): MessageRecord {
  if (!value || typeof value !== "object") return {};
  const raw = value as Record<string, unknown>;
  return {
    id: optionalString(raw.id),
    role: optionalString(raw.role),
    content:
      typeof raw.content === "string"
        ? raw.content
        : optionalString(raw.content),
    createdAt: optionalNumber(raw.createdAt, raw.created_at),
    turnId: optionalString(raw.turnId, raw.turn_id),
    requestId: optionalString(raw.requestId, raw.request_id),
    status: optionalString(raw.status),
    prompt: optionalString(raw.prompt),
    model: optionalString(raw.model),
    modelId: optionalString(raw.modelId, raw.model_id),
    modelLabel: optionalString(raw.modelLabel, raw.model_label),
    reasoning: optionalString(raw.reasoning),
    reasoningLevel: optionalString(raw.reasoningLevel, raw.reasoning_level),
    finishedAt: optionalNumber(raw.finishedAt, raw.finished_at),
    endedAt: optionalNumber(raw.endedAt, raw.ended_at),
    thinking: optionalString(raw.thinking),
    tools: normalizeMessageTools(raw.tools),
  };
}

/** Accept legacy MessageRecord[] or the paged {messages,totalCount,hasMoreBefore} shape. */
function parseSessionsGetResponse(value: unknown): SessionsGetPage {
  if (Array.isArray(value)) {
    const messages = sortedTranscriptMessages(
      value.map(normalizeMessageRecord),
    );
    return {
      messages,
      totalCount: messages.length,
      hasMoreBefore: false,
    };
  }
  if (value && typeof value === "object") {
    const raw = value as Record<string, unknown>;
    const messages = Array.isArray(raw.messages)
      ? sortedTranscriptMessages(raw.messages.map(normalizeMessageRecord))
      : [];
    const totalCount =
      optionalNumber(raw.totalCount, raw.total_count) ?? messages.length;
    const hasMoreBefore = Boolean(
      raw.hasMoreBefore ?? raw.has_more_before ?? false,
    );
    return { messages, totalCount, hasMoreBefore };
  }
  return { messages: [], totalCount: 0, hasMoreBefore: false };
}

function messageCursor(message: MessageRecord | undefined): {
  createdAt: number;
  id: string;
} | null {
  if (!message) return null;
  const createdAt = optionalNumber(message.createdAt);
  const id = optionalString(message.id);
  if (createdAt === undefined || !id) return null;
  return { createdAt, id };
}

function stableMessageId(message: MessageRecord): string {
  const direct = optionalString(message.liveId, message.id);
  if (direct) return direct;
  const role = optionalString(message.role) ?? "message";
  const createdAt = optionalNumber(message.createdAt) ?? 0;
  const content = typeof message.content === "string" ? message.content : "";
  // Legacy records should still anchor deterministically even if an older host
  // omitted ids. This is a renderer key only; durable records always use id.
  let hash = 5381;
  for (let index = 0; index < content.length; index += 1) {
    hash = (hash * 33) ^ content.charCodeAt(index);
  }
  return ["legacy", role, String(createdAt), String(hash >>> 0)].join(":");
}

function transcriptRole(message: MessageRecord) {
  return optionalString(message.role)?.toLowerCase() ?? "";
}

function transcriptTurnKey(message: MessageRecord) {
  const turnId = optionalString(message.turnId);
  if (turnId) return `turn:${turnId}`;
  const requestId = optionalString(message.requestId);
  if (requestId) return `request:${requestId}`;
  const id = optionalString(message.liveId, message.id);
  if (!id) return undefined;
  const live = id.match(/^live:(?:user|assistant):(.+)$/);
  if (live?.[1]) return `live:${live[1]}`;
  const assistant = id.match(/^assistant:(.+)$/);
  if (assistant?.[1]) return `request:${assistant[1]}`;
  return undefined;
}

function compareTranscriptMessages(left: MessageRecord, right: MessageRecord) {
  const leftTurn = transcriptTurnKey(left);
  const rightTurn = transcriptTurnKey(right);
  const leftRole = transcriptRole(left);
  const rightRole = transcriptRole(right);
  if (leftTurn && leftTurn === rightTurn && leftRole !== rightRole) {
    if (leftRole === "user") return -1;
    if (rightRole === "user") return 1;
  }
  const time = (left.createdAt ?? 0) - (right.createdAt ?? 0);
  if (time) return time;
  if (leftRole !== rightRole) {
    if (leftRole === "user") return -1;
    if (rightRole === "user") return 1;
  }
  return stableMessageId(left).localeCompare(stableMessageId(right));
}

function sortedTranscriptMessages(messages: readonly MessageRecord[]) {
  return [...messages].sort(compareTranscriptMessages);
}

function transcriptOrderChanged(
  previous: MessageRecord | undefined,
  next: MessageRecord,
) {
  if (!previous) return true;
  return (
    transcriptTurnKey(previous) !== transcriptTurnKey(next) ||
    transcriptRole(previous) !== transcriptRole(next) ||
    previous.createdAt !== next.createdAt
  );
}

/** Pixels from the top of the transcript that count as "near the start". */
const LOAD_EARLIER_SCROLL_THRESHOLD_PX = 72;

function clampHistoryMessagePageSize(value: unknown): number {
  const parsed =
    typeof value === "number"
      ? value
      : typeof value === "string"
        ? Number(value)
        : Number.NaN;
  if (!Number.isFinite(parsed)) return DEFAULT_HISTORY_MESSAGE_PAGE_SIZE;
  return Math.min(
    MAX_HISTORY_MESSAGE_PAGE_SIZE,
    Math.max(MIN_HISTORY_MESSAGE_PAGE_SIZE, Math.trunc(parsed)),
  );
}

function readStoredHistoryMessagePageSize(): number {
  try {
    const stored = window.localStorage?.getItem(
      HISTORY_MESSAGE_PAGE_SIZE_STORAGE_KEY,
    );
    if (stored != null && stored !== "") {
      return clampHistoryMessagePageSize(stored);
    }
  } catch {
    // localStorage may be unavailable in restricted WebViews.
  }
  return DEFAULT_HISTORY_MESSAGE_PAGE_SIZE;
}

function defaultDomStart(messageCount: number): number {
  return Math.max(0, messageCount - MAX_DOM_MESSAGES);
}

function createSessionMessagePageState(
  messages: MessageRecord[],
  totalCount: number,
  hasMoreBefore: boolean,
  history?: unknown,
): SessionMessagePageState {
  return {
    messages: sortedTranscriptMessages(messages),
    totalCount,
    hasMoreBefore,
    domStart: defaultDomStart(messages.length),
    generation: ++nextSessionMessagePageGeneration,
    history,
  };
}

function liveMessageMatchesPersisted(
  live: MessageRecord,
  persisted: MessageRecord,
) {
  if (!live.liveId || live.role !== persisted.role) return false;
  if (live.turnId && live.turnId === persisted.turnId) return true;
  if (
    live.role === "assistant" &&
    live.requestId &&
    persisted.id === `assistant:${live.requestId}`
  )
    return true;
  // Native writes the user projection before the handle containing turnId is
  // returned. This narrow fallback prevents a rapid leave-and-return from
  // showing the same optimistic prompt twice during that acknowledgement gap.
  return (
    live.role === "user" &&
    live.content === persisted.content &&
    Math.abs((live.createdAt ?? 0) - (persisted.createdAt ?? 0)) < 30_000
  );
}

function mergeServerPageWithLive(
  persistedMessages: MessageRecord[],
  previousMessages: readonly MessageRecord[],
) {
  const provisional = previousMessages.filter((message) => message.liveId);
  const consumed = new Set<MessageRecord>();
  const merged = persistedMessages.map((persisted) => {
    const live = provisional.find(
      (candidate) =>
        !consumed.has(candidate) &&
        liveMessageMatchesPersisted(candidate, persisted),
    );
    if (!live) return persisted;
    consumed.add(live);
    return {
      ...persisted,
      liveId: live.liveId,
      requestId: live.requestId,
      status: live.status,
      prompt: live.prompt,
    };
  });
  for (const live of provisional) {
    if (!consumed.has(live)) merged.push(live);
  }
  return sortedTranscriptMessages(merged);
}

type HistoryMessageDefaults = {
  model?: string;
  reasoning?: string;
};

type WorkspaceCapability = {
  capabilityToken: string;
  workdir: string;
  sessionId?: string;
};

type SystemInfo = {
  product: string;
  skin: string;
  version: string;
  backend: string;
  piRuntime: string;
};

export type ProjectConversationSelection = {
  workdir: string;
  sessionIds: string[];
};

type ProjectConversationSelectionRenderState = {
  workdir: string;
  selectedIds: ReadonlySet<string>;
  isBusy: boolean;
};

type DiagnosticsExportResponse = {
  bytes: number;
};

type AppHealth = {
  sessionStore: "ready" | "recovery_required";
  settings: "ready" | "locked";
  writes: "enabled" | "blocked";
  proxy: "ready" | "unavailable";
  recoveryGuidance:
    | "none"
    | "restart_and_check_local_storage"
    | "unlock_protected_settings";
};

function isLiveSessionRun(state: PiRuntimeSnapshot) {
  return [
    "starting",
    "running",
    "waiting_permission",
    "cancelling",
    "cancel_failed",
  ].includes(state.status);
}

function sessionRunLabel(state: PiRuntimeSnapshot) {
  switch (state.status) {
    case "starting":
      return "启动中";
    case "waiting_permission":
      return "等待授权";
    case "cancelling":
      return "停止中";
    case "cancel_failed":
      return "停止失败";
    default:
      return "执行中";
  }
}

function persistedSessionRunStatus(
  value: unknown,
): PersistedSessionRunStatus | undefined {
  return value === "completed" ||
    value === "cancelled" ||
    value === "error" ||
    value === "interrupted"
    ? value
    : undefined;
}

function persistedSessionRunLabel(
  status: PersistedSessionRunStatus,
  finishedAt?: number,
) {
  const label =
    status === "completed"
      ? "已完成"
      : status === "cancelled"
        ? "已取消"
        : status === "interrupted"
          ? "已中断"
          : "执行失败";
  if (!finishedAt || !Number.isFinite(finishedAt)) return label;
  const finished = new Date(finishedAt);
  if (Number.isNaN(finished.getTime())) return label;
  const now = new Date();
  const sameDay =
    now.getFullYear() === finished.getFullYear() &&
    now.getMonth() === finished.getMonth() &&
    now.getDate() === finished.getDate();
  const time = finished.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
  return sameDay
    ? `${label} · ${time}`
    : `${label} · ${finished.toLocaleDateString([], {
        month: "numeric",
        day: "numeric",
      })} ${time}`;
}

function terminalSessionRunStatus(
  state: PiRuntimeSnapshot,
): PersistedSessionRunStatus | undefined {
  return persistedSessionRunStatus(state.status);
}

export type NativeShellApi = {
  getSessionId: () => string | undefined;
  getWorkdir: () => string | undefined;
  getCurrentProjectPreferences: () => ProjectPreferences | undefined;
  saveCurrentProjectPreferences: (
    patch: Partial<ProjectPreferences>,
  ) => Promise<ProjectPreferences | undefined>;
  /** Keep optimistic and streamed turn projections in the virtual-list state. */
  upsertLiveTranscriptMessage: (message: LiveTranscriptMessage) => void;
  selectSession: (id: string) => Promise<void>;
  createSession: (cwd?: string) => Promise<void>;
  refreshSessions: (options?: { loadActive?: boolean }) => Promise<void>;
  getSessions: () => SessionSummary[];
  onSessionsChanged: (
    listener: (sessions: readonly SessionSummary[]) => void,
  ) => () => void;
  exportDiagnostics: () => Promise<DiagnosticsExportResponse | undefined>;
  issueWorkspaceCapability: () => Promise<WorkspaceCapability>;
  branchSession: (id: string, title?: string) => Promise<SessionSummary>;
  removeProject: (
    workdir: string,
  ) => Promise<{ removed: boolean; wasCurrent: boolean }>;
  getProjectConversationSelection: () =>
    | ProjectConversationSelection
    | undefined;
  clearProjectConversationSelection: () => void;
  setProjectConversationSelectionBusy: (isBusy: boolean) => void;
  restoreProjectConversationSelectionFocus: () => void;
};

declare global {
  interface Window {
    __novaveiHost?: NativeShellApi;
  }
}

function getInvoke(): Invoke | undefined {
  return window.__TAURI__?.core?.invoke as Invoke | undefined;
}

function node<T extends HTMLElement>(id: string): T | null {
  return document.getElementById(id) as T | null;
}

function toast(message: string) {
  const target = node<HTMLElement>("toast");
  if (!target) return;
  target.textContent = message;
  target.classList.add("show");
  window.setTimeout(() => target.classList.remove("show"), 2200);
}

function record(value: unknown): UnknownRecord | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as UnknownRecord)
    : undefined;
}

function stringArray(value: unknown): string[] | null {
  if (!Array.isArray(value) || value.some((item) => typeof item !== "string"))
    return null;
  return value as string[];
}

function workspaceRelocationProject(
  value: unknown,
): WorkspaceRelocationProject | null {
  const raw = record(value);
  const id = typeof raw?.id === "string" ? raw.id.trim() : "";
  const name = typeof raw?.name === "string" ? raw.name.trim() : "";
  const path = typeof raw?.path === "string" ? displayPath(raw.path) : "";
  if (!id || !name || !path) return null;
  return { id, name, path };
}

function workspaceRelocationResponse(
  value: unknown,
): WorkspaceRelocationResponse | null {
  const raw = record(value);
  const fromWorkdir =
    typeof raw?.fromWorkdir === "string" ? displayPath(raw.fromWorkdir) : "";
  const toWorkdir =
    typeof raw?.toWorkdir === "string" ? displayPath(raw.toWorkdir) : "";
  const updatedSessionIds = stringArray(raw?.updatedSessionIds);
  const updatedProjectIds = stringArray(raw?.updatedProjectIds);
  if (!fromWorkdir || !toWorkdir || !updatedSessionIds || !updatedProjectIds)
    return null;

  if (raw?.status === "relocated") {
    if (raw.conflict != null || raw.conflictToken != null) return null;
    return {
      status: "relocated",
      fromWorkdir,
      toWorkdir,
      updatedSessionIds,
      updatedProjectIds,
    };
  }
  if (raw?.status !== "conflict") return null;
  const rawConflict = record(raw.conflict);
  const targetProject = workspaceRelocationProject(rawConflict?.targetProject);
  const sourceProject =
    rawConflict?.sourceProject == null
      ? undefined
      : workspaceRelocationProject(rawConflict.sourceProject);
  const sourceProjectInvalid =
    rawConflict?.sourceProject != null && !sourceProject;
  const conflictToken =
    typeof raw.conflictToken === "string" ? raw.conflictToken.trim() : "";
  const updatedSessions = Array.isArray(raw.updatedSessions)
    ? raw.updatedSessions
    : null;
  if (
    !targetProject ||
    sourceProjectInvalid ||
    !conflictToken ||
    updatedSessionIds.length > 0 ||
    updatedProjectIds.length > 0 ||
    !updatedSessions ||
    updatedSessions.length > 0
  ) {
    return null;
  }
  return {
    status: "conflict",
    fromWorkdir,
    toWorkdir,
    updatedSessionIds,
    updatedProjectIds,
    conflict: { sourceProject: sourceProject ?? undefined, targetProject },
    conflictToken,
  };
}

function requestWorkspaceRelocationResolution(
  conflict: WorkspaceRelocationConflict,
): Promise<WorkspaceRelocationConflictResolution | null> {
  const source = conflict.sourceProject;
  const target = conflict.targetProject;
  return requestAppChoice<WorkspaceRelocationConflictResolution>({
    title: "目标目录已登记为项目",
    message: source
      ? `“${target.name}”已经使用所选目录。请选择迁移后保留哪一份项目资料。历史会话不会被删除。`
      : `“${target.name}”已经使用所选目录。当前历史路径没有项目资料，请确认是否将历史会话并入该项目。`,
    choices: source
      ? [
          {
            value: "keep_source",
            label: `保留原项目“${source.name}”`,
            description: `保留 ${source.path} 的名称和设置，并替换 ${target.path} 的现有项目资料。`,
            tone: "danger",
          },
          {
            value: "merge_into_target",
            label: `并入目标项目“${target.name}”`,
            description: `保留 ${target.path} 的现有项目资料，并将 ${source.path} 的历史会话移入。`,
            tone: "primary",
          },
        ]
      : [
          {
            value: "merge_into_target",
            label: `并入目标项目“${target.name}”`,
            description: `保留 ${target.path} 的现有项目资料，并将原路径的历史会话移入。`,
            tone: "primary",
          },
        ],
    cancelLabel: "取消迁移",
  });
}

function safeDisplayText(value: unknown, maxLength: number) {
  const text = typeof value === "string" ? value.trim() : "";
  if (!text || text.length > maxLength || /[\u0000-\u001F\u007F]/.test(text)) {
    return undefined;
  }
  return text;
}

function normaliseSystemInfo(value: unknown): SystemInfo | undefined {
  const raw = record(value);
  const product = safeDisplayText(raw?.product, 80);
  const skin = safeDisplayText(raw?.skin, 80);
  const version = safeDisplayText(raw?.version, 64);
  const backend = safeDisplayText(raw?.backend, 64);
  const piRuntime = safeDisplayText(raw?.piRuntime, 64);
  return product && skin && version && backend && piRuntime
    ? { product, skin, version, backend, piRuntime }
    : undefined;
}

/**
 * The browser preview intentionally keeps the static About copy. In the
 * desktop WebView, replace its generic runtime claim with the native version
 * and runtime descriptor without accepting filesystem or storage metadata.
 */
function installAboutSystemInfo(invoke: Invoke) {
  const panel = document.querySelector<HTMLElement>(
    '.settings-panel[data-settings="about"]',
  );
  const field = panel?.querySelector<HTMLElement>(".field");
  if (!panel || !field || panel.dataset.novaveiSystemInfoBound === "true")
    return;
  panel.dataset.novaveiSystemInfoBound = "true";

  const detail = document.createElement("p");
  detail.id = "aboutRuntimeInfo";
  detail.setAttribute("role", "status");
  detail.setAttribute("aria-live", "polite");
  field.appendChild(detail);

  let info: SystemInfo | undefined;
  let failed = false;
  const english = () =>
    document.documentElement.lang.toLowerCase().startsWith("en");
  const render = () => {
    if (info) {
      detail.textContent = `${info.product} v${info.version} · ${info.skin} · ${info.backend} · ${info.piRuntime}`;
      detail.removeAttribute("aria-busy");
      return;
    }
    detail.textContent = failed
      ? english()
        ? "Native runtime details are unavailable."
        : "原生运行时详情暂不可用。"
      : english()
        ? "Loading native runtime details…"
        : "正在读取原生运行时详情…";
    detail.setAttribute("aria-busy", String(!failed));
  };
  render();
  window.addEventListener("novavei:language-changed", render);
  void invoke<unknown>("system_info")
    .then((response) => {
      info = normaliseSystemInfo(response);
      failed = !info;
      render();
    })
    .catch(() => {
      failed = true;
      render();
    });
}

/**
 * Treat the native health payload as a closed contract before broadcasting it
 * to other renderer modules. An incomplete/unknown payload is deliberately a
 * blocked recovery state, so an older or malformed host cannot make a
 * transient session/settings projection look durable.
 */
function normaliseAppHealth(value: unknown): AppHealth {
  const raw = record(value);
  const sessionStore =
    raw?.sessionStore === "ready" ? "ready" : "recovery_required";
  const settings = raw?.settings === "ready" ? "ready" : "locked";
  const writes =
    raw?.writes === "enabled" &&
    sessionStore === "ready" &&
    settings === "ready"
      ? "enabled"
      : "blocked";

  return {
    sessionStore,
    settings,
    writes,
    proxy: raw?.proxy === "ready" ? "ready" : "unavailable",
    recoveryGuidance:
      sessionStore !== "ready"
        ? "restart_and_check_local_storage"
        : settings !== "ready"
          ? "unlock_protected_settings"
          : "none",
  };
}

function appHealthAllowsWrites(value: AppHealth) {
  return value.writes === "enabled";
}

function boundedProviderId(value: unknown): string | undefined {
  const providerId = typeof value === "string" ? value.trim() : "";
  return /^[A-Za-z0-9._-]{1,128}$/.test(providerId) ? providerId : undefined;
}

function boundedModelId(value: unknown): string | undefined {
  const modelId = typeof value === "string" ? value.trim() : "";
  if (
    !modelId ||
    new TextEncoder().encode(modelId).byteLength > 256 ||
    /[\u0000-\u001F\u007F]/.test(modelId)
  ) {
    return undefined;
  }
  return modelId;
}

function storedSessionModelSelection(
  value: unknown,
): SessionModelSelection | undefined {
  const history = record(value);
  const raw = history?.selectedModelJson ?? history?.selected_model_json;
  if (typeof raw !== "string" || raw.length > 512) return undefined;
  try {
    const parsed = record(JSON.parse(raw));
    if (
      !parsed ||
      Object.keys(parsed).some(
        (key) => key !== "providerId" && key !== "modelId",
      )
    )
      return undefined;
    const providerId = boundedProviderId(parsed.providerId);
    const modelId = boundedModelId(parsed.modelId);
    return providerId && modelId ? { providerId, modelId } : undefined;
  } catch {
    return undefined;
  }
}

function estimatedMessageTokens(content: string) {
  let cjkCharacters = 0;
  let otherCharacters = 0;
  for (const character of content) {
    if (
      /^[\u3400-\u4DBF\u4E00-\u9FFF\uF900-\uFAFF\u3040-\u30FF\uAC00-\uD7AF]$/u.test(
        character,
      )
    ) {
      cjkCharacters += 1;
    } else if (!/\s/u.test(character)) {
      otherCharacters += 1;
    }
  }
  // This is a local estimate for the selected session. Provider accounting
  // replaces it once the active run reports precise input usage.
  return cjkCharacters + Math.ceil(otherCharacters / 4);
}

function estimatedSessionContextTokens(messages: readonly MessageRecord[]) {
  return messages.reduce((total, message) => {
    const content = typeof message.content === "string" ? message.content : "";
    return total + (content ? estimatedMessageTokens(content) + 4 : 0);
  }, 0);
}

function notifySessionChanged(
  sessionId: string,
  history?: unknown,
  messages: readonly MessageRecord[] = [],
) {
  window.dispatchEvent(
    new CustomEvent("novavei:session-changed", {
      detail: {
        sessionId,
        modelSelection: storedSessionModelSelection(history) ?? null,
        // The dock only receives aggregate accounting, never a second copy of
        // the selected conversation text.
        contextTokenEstimate: estimatedSessionContextTokens(messages),
      },
    }),
  );
}

/** Publish only completed workspace changes so dependent panes never query an old capability. */
function notifyWorkdirChanged(workdir: string) {
  const displayedWorkdir = displayPath(workdir);
  window.dispatchEvent(
    new CustomEvent("novavei:workdir-changed", {
      detail: { workdir: displayedWorkdir },
    }),
  );
}

function workspaceLocationStatus(
  value: unknown,
): WorkspaceLocationStatus | undefined {
  switch (value) {
    case "available":
    case "missing":
    case "not_directory":
    case "unavailable":
      return value;
    default:
      return undefined;
  }
}

function workspaceLocationStatusFromRecord(
  value: unknown,
): WorkspaceLocationStatus | undefined {
  const raw = record(value);
  const explicit = workspaceLocationStatus(
    raw?.workspaceStatus ?? raw?.workspace_status ?? raw?.status,
  );
  return (
    explicit ??
    workspaceLocationStatus(raw?.reason) ??
    (raw?.accessible === true
      ? "available"
      : raw?.accessible === false
        ? "unavailable"
        : undefined)
  );
}

function isRelocationRequired(status: WorkspaceLocationStatus | undefined) {
  return status === "missing" || status === "not_directory";
}

function workspaceStatusLabel(status: WorkspaceLocationStatus | undefined) {
  switch (status) {
    case "missing":
      return "路径已失效";
    case "not_directory":
      return "路径不是目录";
    case "unavailable":
      return "路径暂无法检查";
    default:
      return "";
  }
}

function workspaceStatusDescription(
  status: WorkspaceLocationStatus | undefined,
) {
  switch (status) {
    case "missing":
      return "原项目目录不存在或对应磁盘未连接。历史会话仍会保留。";
    case "not_directory":
      return "原项目路径不再是目录。历史会话仍会保留。";
    case "unavailable":
      return "暂时无法确认原项目路径是否可用。历史会话仍会保留。";
    default:
      return "";
  }
}

function parseWorkspacePathStatuses(value: unknown) {
  const raw = record(value);
  const values = Array.isArray(raw?.paths)
    ? raw.paths
    : Array.isArray(value)
      ? value
      : [];
  const statuses = new Map<string, WorkspaceLocationStatus>();
  for (const item of values) {
    const itemRecord = record(item);
    const path = displayPath(
      typeof itemRecord?.path === "string" ? itemRecord.path : "",
    );
    if (!path) continue;
    const explicit = workspaceLocationStatusFromRecord(itemRecord);
    const reason = workspaceLocationStatus(itemRecord?.reason);
    const status =
      explicit ??
      reason ??
      (itemRecord?.accessible === true
        ? "available"
        : itemRecord?.accessible === false
          ? "unavailable"
          : undefined);
    if (status) statuses.set(pathKey(path), status);
  }
  return statuses;
}

function sessionRecord(value: unknown): SessionSummary | null {
  if (!value || typeof value !== "object") return null;
  const raw = value as Record<string, unknown>;
  const record =
    raw.session && typeof raw.session === "object"
      ? (raw.session as Record<string, unknown>)
      : raw.summary && typeof raw.summary === "object"
        ? (raw.summary as Record<string, unknown>)
        : raw;
  const id = typeof record.id === "string" ? record.id.trim() : "";
  const cwd = displayPath(typeof record.cwd === "string" ? record.cwd : "");
  if (!id || !cwd) return null;
  const lastRunStatus = persistedSessionRunStatus(
    record.lastRunStatus ?? record.last_run_status,
  );
  const rawLastRunFinishedAt =
    record.lastRunFinishedAt ?? record.last_run_finished_at;
  const lastRunFinishedAt =
    typeof rawLastRunFinishedAt === "number" &&
    Number.isFinite(rawLastRunFinishedAt)
      ? rawLastRunFinishedAt
      : undefined;
  const workspaceStatus = workspaceLocationStatusFromRecord(record);
  return {
    id,
    title:
      typeof record.title === "string" && record.title.trim()
        ? record.title.trim()
        : "新建对话",
    cwd,
    ...(typeof record.updatedAt === "number"
      ? { updatedAt: record.updatedAt }
      : {}),
    ...(typeof record.updated_at === "number"
      ? { updatedAt: record.updated_at }
      : {}),
    ...(record.isPinned === true || record.is_pinned === true
      ? { isPinned: true }
      : {}),
    ...(record.isArchived === true || record.is_archived === true
      ? { isArchived: true }
      : {}),
    ...(lastRunStatus ? { lastRunStatus } : {}),
    ...(lastRunFinishedAt !== undefined ? { lastRunFinishedAt } : {}),
    ...(workspaceStatus ? { workspaceStatus } : {}),
  };
}

const PROJECT_UUID_ID =
  /^project-[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const PROJECT_PREFERENCE_REASONING = new Set<PiReasoningLevel>([
  "off",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
]);

function hasStableProjectId(value: string) {
  return PROJECT_UUID_ID.test(value.trim());
}

function newProjectId() {
  const uuid = globalThis.crypto?.randomUUID?.();
  if (uuid && /^[0-9a-f-]{36}$/i.test(uuid))
    return `project-${uuid.toLowerCase()}`;
  // The native projects-settings normalizer replaces this non-canonical,
  // renderer-local placeholder with `Uuid::new_v4()` before it is persisted.
  // Never fall back to a path hash for identity.
  return `project-pending-${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 10)}`;
}

function readProjectPreferences(
  value: unknown,
): ProjectPreferences | undefined {
  const preferences = record(value);
  if (
    !preferences ||
    Object.keys(preferences).some(
      (key) => key !== "model" && key !== "reasoning" && key !== "permission",
    )
  )
    return undefined;
  const model = record(preferences.model);
  const providerId =
    typeof model?.providerId === "string" ? model.providerId.trim() : "";
  const modelId =
    typeof model?.modelId === "string" ? model.modelId.trim() : "";
  const hasModel = Boolean(
    model &&
      Object.keys(model).every(
        (key) => key === "providerId" || key === "modelId",
      ) &&
      /^[A-Za-z0-9._-]{1,128}$/.test(providerId) &&
      modelId &&
      new TextEncoder().encode(modelId).byteLength <= 256 &&
      !/[\u0000-\u001F\u007F]/.test(modelId),
  );
  if (preferences.model !== undefined && !hasModel) return undefined;
  const reasoning =
    typeof preferences.reasoning === "string"
      ? preferences.reasoning.trim().toLowerCase()
      : undefined;
  if (
    preferences.reasoning !== undefined &&
    !PROJECT_PREFERENCE_REASONING.has(reasoning as PiReasoningLevel)
  )
    return undefined;
  const permission =
    typeof preferences.permission === "string"
      ? preferences.permission.trim().toLowerCase()
      : undefined;
  if (
    preferences.permission !== undefined &&
    permission !== "readonly" &&
    permission !== "ask" &&
    permission !== "auto-approve"
  )
    return undefined;
  if (!hasModel && !reasoning && !permission) return undefined;
  return {
    ...(hasModel ? { model: { providerId, modelId } } : {}),
    ...(reasoning ? { reasoning: reasoning as PiReasoningLevel } : {}),
    ...(permission
      ? { permission: permission as ProjectPermissionPreference }
      : {}),
  };
}

function projectEntry(value: unknown): ProjectEntry | null {
  const raw = record(value);
  const id = typeof raw?.id === "string" ? raw.id.trim() : "";
  const name = typeof raw?.name === "string" ? raw.name.trim() : "";
  const path = displayPath(typeof raw?.path === "string" ? raw.path : "");
  if (!/^[A-Za-z0-9._-]{1,128}$/.test(id) || !name || !path) return null;
  const preferences = readProjectPreferences(raw?.preferences);
  return {
    id,
    name,
    path,
    ...(typeof raw?.lastSessionId === "string" && raw.lastSessionId.trim()
      ? { lastSessionId: raw.lastSessionId.trim() }
      : typeof raw?.last_session_id === "string" && raw.last_session_id.trim()
        ? { lastSessionId: raw.last_session_id.trim() }
        : {}),
    ...(raw?.pinned === true ? { pinned: true } : {}),
    ...(preferences ? { preferences } : {}),
  };
}

function readProjectSettings(value: unknown): ProjectSettings {
  const raw = record(value);
  const entries = Array.isArray(raw?.entries)
    ? raw.entries
        .map(projectEntry)
        .filter((entry): entry is ProjectEntry => Boolean(entry))
    : [];
  return { initialized: raw?.initialized === true, entries };
}

function projectFromWorkdir(
  workdir: string,
  sessions: readonly SessionSummary[],
): ProjectEntry {
  const cleaned = displayPath(workdir);
  const lastSession = sessions.find(
    (session) => pathKey(session.cwd) === pathKey(cleaned),
  );
  return {
    id: newProjectId(),
    name: pathName(cleaned),
    path: cleaned,
    ...(lastSession ? { lastSessionId: lastSession.id } : {}),
  };
}

function projectFolderForWorkdir(
  workdir: string,
  create = false,
  preferredName?: string,
): HTMLElement | null {
  const cleanedWorkdir = displayPath(workdir);
  const key = pathKey(cleanedWorkdir);
  const rows = [
    ...document.querySelectorAll<HTMLElement>(".project-row[data-workdir]"),
  ];
  const existing = rows.find((row) => pathKey(row.dataset.workdir) === key);
  let preservedExpanded: boolean | undefined;
  if (existing) {
    const existingFolder = existing.closest<HTMLElement>(".project-folder");
    const isUnregistered =
      existing.dataset.novaveiWorkspaceKind === "unregistered" ||
      existingFolder?.dataset.novaveiWorkspaceKind === "unregistered";
    // Registering a historical workspace promotes it out of "其他工作空间".
    // Recreate the compact folder under the canonical project list rather than
    // mutating the old node in place, which would otherwise leave it hidden in
    // the now-empty secondary group.
    if (isUnregistered) {
      if (!create) return null;
      preservedExpanded =
        existing.dataset.expanded === "true"
          ? true
          : existing.dataset.expanded === "false"
            ? false
            : undefined;
      existingFolder?.remove();
    } else {
      existing.dataset.novaveiProject = "true";
      existing.dataset.novaveiWorkspaceKind = "project";
      existingFolder?.setAttribute("data-novavei-workspace-kind", "project");
      if (preferredName?.trim()) {
        existing.dataset.project = preferredName.trim();
        const label = existing.querySelector<HTMLElement>(
          ".project-copy strong",
        );
        if (label) label.textContent = preferredName.trim();
      }
      const subtitle = existing.querySelector<HTMLElement>(
        ".project-copy small",
      );
      if (subtitle) subtitle.textContent = cleanedWorkdir;
      existing.dataset.workdir = cleanedWorkdir;
      existing.title = cleanedWorkdir;
      return existingFolder;
    }
  }
  if (!create) return null;
  const list =
    node<HTMLElement>("projectSection")?.querySelector<HTMLElement>(
      ".project-list",
    );
  if (!list) return null;
  const id = `novavei-project-${Math.random().toString(36).slice(2, 9)}`;
  const folder = document.createElement("article");
  folder.className = "project-folder";
  folder.dataset.novaveiProject = "true";
  folder.dataset.novaveiWorkspaceKind = "project";
  const row = document.createElement("button");
  row.type = "button";
  row.className = "project-row";
  row.dataset.project = preferredName?.trim() || pathName(cleanedWorkdir);
  row.dataset.workdir = cleanedWorkdir;
  row.dataset.novaveiWorkspaceKind = "project";
  row.dataset.state = "0 对话";
  // Preserve the user's expansion choice when promoting an unregistered
  // history group. Otherwise leave the dataset unset until the user toggles or
  // this project becomes current.
  if (preservedExpanded !== undefined)
    row.dataset.expanded = String(preservedExpanded);
  row.setAttribute("aria-expanded", String(preservedExpanded === true));
  row.setAttribute("aria-controls", id);
  row.title = cleanedWorkdir;
  const icon = workspaceFolderIcon();
  const copy = document.createElement("span");
  copy.className = "project-copy";
  const strong = document.createElement("strong");
  strong.textContent = preferredName?.trim() || pathName(cleanedWorkdir);
  const small = document.createElement("small");
  small.textContent = cleanedWorkdir;
  copy.append(strong, small);
  const state = document.createElement("span");
  state.className = "project-state";
  state.textContent = "0 对话";
  const chevron = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  chevron.setAttribute("class", "ico project-chevron");
  chevron.setAttribute("viewBox", "0 0 24 24");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", "m6 9 6 6 6-6");
  chevron.appendChild(path);
  row.append(icon, copy, state, chevron);
  const conversations = document.createElement("div");
  conversations.className = "project-conversations";
  conversations.id = id;
  conversations.hidden = preservedExpanded !== true;
  conversations.setAttribute(
    "aria-label",
    `${preferredName?.trim() || pathName(cleanedWorkdir)} 对话`,
  );
  folder.append(row, conversations);
  list.appendChild(folder);
  return folder;
}

const OTHER_WORKSPACES_SECTION_ID = "novaveiOtherWorkspacesSection";
const OTHER_WORKSPACES_LIST_ID = "novaveiOtherWorkspacesList";

function workspaceFolderIcon() {
  const icon = document.createElement("span");
  icon.className = "project-icon";
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", "ico");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  const folderPath = document.createElementNS(
    "http://www.w3.org/2000/svg",
    "path",
  );
  folderPath.setAttribute("d", "M3 7h6l2 2h10v10H3z");
  svg.appendChild(folderPath);
  icon.appendChild(svg);
  return icon;
}

function workspaceStatusIcon() {
  const icon = document.createElement("span");
  icon.className = "workspace-path-notice-icon";
  icon.setAttribute("aria-hidden", "true");
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", "ico");
  svg.setAttribute("viewBox", "0 0 24 24");
  const triangle = document.createElementNS(
    "http://www.w3.org/2000/svg",
    "path",
  );
  triangle.setAttribute("d", "M12 3 2.7 20h18.6L12 3Z");
  const mark = document.createElementNS("http://www.w3.org/2000/svg", "path");
  mark.setAttribute("d", "M12 9v4m0 3h.01");
  svg.append(triangle, mark);
  icon.appendChild(svg);
  return icon;
}

function workspaceFolderForWorkdir(workdir: string) {
  const key = pathKey(workdir);
  if (!key) return null;
  const rows = [
    ...document.querySelectorAll<HTMLElement>(".project-row[data-workdir]"),
  ].filter((candidate) => pathKey(candidate.dataset.workdir) === key);
  const row =
    rows.find(
      (candidate) => !candidate.closest<HTMLElement>(".project-folder")?.hidden,
    ) ?? rows[0];
  return row?.closest<HTMLElement>(".project-folder") ?? null;
}

function otherWorkspacesSection(create = false) {
  let section = node<HTMLElement>(OTHER_WORKSPACES_SECTION_ID);
  if (section || !create) return section;
  const projectSection = node<HTMLElement>("projectSection");
  const parent = projectSection?.parentElement;
  if (!projectSection || !parent) return null;

  section = document.createElement("section");
  section.id = OTHER_WORKSPACES_SECTION_ID;
  section.className = "project-section other-workspaces-section";
  section.hidden = true;
  const label = document.createElement("h3");
  label.id = "novaveiOtherWorkspacesLabel";
  label.className = "group-label";
  label.textContent = "其他工作空间";
  const hint = document.createElement("p");
  hint.className = "other-workspaces-hint";
  hint.textContent = "历史会话的目录尚未登记为项目。";
  const list = document.createElement("div");
  list.id = OTHER_WORKSPACES_LIST_ID;
  list.className = "project-list";
  list.setAttribute("aria-label", "未登记工作空间");
  section.setAttribute("aria-labelledby", label.id);
  section.append(label, hint, list);
  parent.insertBefore(section, projectSection.nextSibling);
  return section;
}

function otherWorkspaceFolderForWorkdir(workdir: string, create = false) {
  const cleanedWorkdir = displayPath(workdir);
  const key = pathKey(cleanedWorkdir);
  if (!key) return null;
  const section = otherWorkspacesSection(create);
  const list = section?.querySelector<HTMLElement>(
    `#${OTHER_WORKSPACES_LIST_ID}`,
  );
  if (!list) return null;
  const matchingRows = [
    ...document.querySelectorAll<HTMLElement>(".project-row[data-workdir]"),
  ].filter((row) => pathKey(row.dataset.workdir) === key);
  let preservedExpanded: boolean | undefined;
  if (create) {
    // Removing a registered project must move its remaining history into the
    // secondary group, not leave the old project-list node beside a newly
    // created unregistered copy. Recreate in the canonical container so the
    // two lists can never render the same cwd at once.
    for (const candidate of matchingRows) {
      if (!list.contains(candidate)) {
        if (preservedExpanded === undefined) {
          preservedExpanded =
            candidate.dataset.expanded === "true"
              ? true
              : candidate.dataset.expanded === "false"
                ? false
                : undefined;
        }
        candidate.closest<HTMLElement>(".project-folder")?.remove();
      }
    }
  }
  const existing = matchingRows.find((row) => list.contains(row));
  if (existing) {
    const existingFolder = existing.closest<HTMLElement>(".project-folder");
    existing.dataset.novaveiWorkspaceKind = "unregistered";
    existing.dataset.workdir = cleanedWorkdir;
    existing.title = cleanedWorkdir;
    if (existingFolder) {
      existingFolder.dataset.novaveiWorkspaceKind = "unregistered";
      existingFolder.classList.add("is-unregistered-workspace");
    }
    const pathLabel = existing.querySelector<HTMLElement>(
      ".project-copy small",
    );
    if (pathLabel) pathLabel.textContent = cleanedWorkdir;
    return existingFolder;
  }
  if (!create) return null;

  const conversationId = `novavei-other-workspace-${Math.random()
    .toString(36)
    .slice(2, 10)}`;
  const folder = document.createElement("article");
  folder.className =
    "project-folder workspace-folder is-unregistered-workspace";
  folder.dataset.novaveiWorkspaceKind = "unregistered";
  const row = document.createElement("button");
  row.type = "button";
  row.className = "project-row";
  row.dataset.project = pathName(cleanedWorkdir);
  row.dataset.workdir = cleanedWorkdir;
  row.dataset.novaveiWorkspaceKind = "unregistered";
  row.dataset.state = "未登记";
  if (preservedExpanded !== undefined)
    row.dataset.expanded = String(preservedExpanded);
  row.setAttribute("aria-expanded", String(preservedExpanded === true));
  row.setAttribute("aria-controls", conversationId);
  row.title = cleanedWorkdir;
  const copy = document.createElement("span");
  copy.className = "project-copy";
  const strong = document.createElement("strong");
  strong.textContent = pathName(cleanedWorkdir);
  const small = document.createElement("small");
  small.textContent = cleanedWorkdir;
  copy.append(strong, small);
  const state = document.createElement("span");
  state.className = "project-state";
  state.textContent = "未登记";
  const chevron = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  chevron.setAttribute("class", "ico project-chevron");
  chevron.setAttribute("viewBox", "0 0 24 24");
  chevron.setAttribute("aria-hidden", "true");
  const chevronPath = document.createElementNS(
    "http://www.w3.org/2000/svg",
    "path",
  );
  chevronPath.setAttribute("d", "m6 9 6 6 6-6");
  chevron.appendChild(chevronPath);
  row.append(workspaceFolderIcon(), copy, state, chevron);

  const actions = document.createElement("div");
  actions.className = "workspace-folder-actions";
  const register = document.createElement("button");
  register.type = "button";
  register.className = "folder-add workspace-register-action";
  register.dataset.novaveiRegisterWorkspace = cleanedWorkdir;
  register.setAttribute(
    "aria-label",
    `将 ${pathName(cleanedWorkdir)} 添加为项目`,
  );
  register.textContent = "添加为项目";
  actions.appendChild(register);

  const conversations = document.createElement("div");
  conversations.className = "project-conversations";
  conversations.id = conversationId;
  conversations.hidden = preservedExpanded !== true;
  conversations.setAttribute(
    "aria-label",
    `${pathName(cleanedWorkdir)} 的未登记历史会话`,
  );
  folder.append(row, actions, conversations);
  list.appendChild(folder);
  return folder;
}

function setWorkspaceRowStatus(
  row: HTMLElement,
  sessionCount: number,
  status: WorkspaceLocationStatus | undefined,
  isCurrent: boolean,
) {
  row.dataset.novaveiSessionCount = String(sessionCount);
  if (status) row.dataset.workspaceStatus = status;
  else delete row.dataset.workspaceStatus;
  const relocationRequired = isRelocationRequired(status);
  row.classList.toggle("is-workspace-unavailable", relocationRequired);
  row.classList.toggle("is-workspace-status-unknown", status === "unavailable");
  const unregistered = row.dataset.novaveiWorkspaceKind === "unregistered";
  const state = row.querySelector<HTMLElement>(".project-state");
  if (state) {
    state.textContent = relocationRequired
      ? workspaceStatusLabel(status)
      : status === "unavailable"
        ? workspaceStatusLabel(status)
        : unregistered
          ? "未登记"
          : isCurrent
            ? "当前"
            : `${sessionCount} 对话`;
  }
  const path = displayPath(row.dataset.workdir);
  const statusDescription = workspaceStatusDescription(status);
  row.title = statusDescription ? `${path}\n${statusDescription}` : path;
  if (statusDescription) {
    row.setAttribute(
      "aria-label",
      `${row.dataset.project || pathName(path)}，${unregistered ? "未登记工作空间，" : ""}${workspaceStatusLabel(status)}。${statusDescription}`,
    );
  } else if (unregistered) {
    row.setAttribute(
      "aria-label",
      `${row.dataset.project || pathName(path)}，未登记工作空间，${sessionCount} 个对话。历史会话仍可只读打开。`,
    );
  } else {
    row.removeAttribute("aria-label");
  }
}

function workspacePathNotice(workdir: string, status: WorkspaceLocationStatus) {
  const notice = document.createElement("div");
  notice.className = "workspace-path-notice";
  notice.dataset.workspaceStatus = status;
  const copy = document.createElement("div");
  copy.className = "workspace-path-notice-copy";
  const title = document.createElement("strong");
  title.textContent = workspaceStatusLabel(status);
  const detail = document.createElement("span");
  detail.textContent =
    status === "unavailable"
      ? `${workspaceStatusDescription(status)} 可重新检查或选择新的目录；历史会话仍可只读打开。`
      : `${workspaceStatusDescription(status)} 可在磁盘恢复后重新检查，或选择新的目录重新绑定这些会话。`;
  copy.append(title, detail);
  const actions = document.createElement("div");
  actions.className = "workspace-path-notice-actions";
  const refresh = document.createElement("button");
  refresh.type = "button";
  refresh.className = "folder-add workspace-relocate-action";
  refresh.dataset.novaveiRefreshWorkspaceStatus = displayPath(workdir);
  refresh.setAttribute(
    "aria-label",
    `重新检查 ${pathName(workdir)} 的路径状态`,
  );
  refresh.textContent = "重新检查";
  actions.appendChild(refresh);
  const relocate = document.createElement("button");
  relocate.type = "button";
  relocate.className = "folder-add workspace-relocate-action";
  relocate.dataset.novaveiRelocateWorkspace = displayPath(workdir);
  relocate.setAttribute("aria-label", `为 ${pathName(workdir)} 重新选择目录`);
  relocate.textContent = "重新选择目录";
  actions.appendChild(relocate);
  notice.append(workspaceStatusIcon(), copy, actions);
  return notice;
}

function projectConversationHost(folder: HTMLElement | null) {
  return folder?.querySelector<HTMLElement>(".project-conversations") ?? null;
}

function setProjectFolderExpanded(row: HTMLElement | null, expanded: boolean) {
  if (!row) return;
  row.dataset.expanded = String(expanded);
  row.setAttribute("aria-expanded", String(expanded));
  const host = projectConversationHost(
    row.closest<HTMLElement>(".project-folder"),
  );
  if (host) host.hidden = !expanded;
}

function publishCurrentProjectChanged(
  row: HTMLElement | null,
  workdir: string | undefined,
) {
  window.dispatchEvent(
    new CustomEvent("novavei:current-project-changed", {
      detail: {
        workdir: workdir ? displayPath(workdir) : null,
        registered: row?.dataset.novaveiWorkspaceKind === "project",
      },
    }),
  );
}

function setProjectCurrent(row: HTMLElement | null, workdir: string) {
  if (!row) return;
  const cleanedWorkdir = displayPath(workdir);
  document
    .querySelectorAll<HTMLElement>(".project-row[data-workdir]")
    .forEach((candidate) => {
      const current = candidate === row;
      candidate.toggleAttribute("aria-current", current);
      if (candidate.dataset.novaveiWorkspaceKind === "project")
        candidate.dataset.novaveiProject = "true";
      const state = candidate.querySelector<HTMLElement>(".project-state");
      const status = workspaceLocationStatus(candidate.dataset.workspaceStatus);
      if (state) {
        const unregistered =
          candidate.dataset.novaveiWorkspaceKind === "unregistered";
        state.textContent = isRelocationRequired(status)
          ? workspaceStatusLabel(status)
          : status === "unavailable"
            ? workspaceStatusLabel(status)
            : unregistered
              ? "未登记"
              : current
                ? "当前"
                : `${candidate.dataset.novaveiSessionCount || "0"} 对话`;
      }
    });
  row.dataset.workdir = cleanedWorkdir;
  row.dataset.project = row.dataset.project || pathName(cleanedWorkdir);
  if (!workspaceLocationStatus(row.dataset.workspaceStatus))
    row.title = cleanedWorkdir;
  const subtitle = row.querySelector<HTMLElement>(".project-copy small");
  if (subtitle) subtitle.textContent = cleanedWorkdir;
  publishCurrentProjectChanged(row, cleanedWorkdir);
}

function transcriptAxis(): HTMLElement | null {
  return (
    node<HTMLElement>("transcriptAxis") ??
    document.querySelector<HTMLElement>(".axis")
  );
}

function clearTranscript() {
  transcriptAxis()?.replaceChildren();
  window.__novaveiFloorNav?.refresh?.();
}

function removeLoadEarlierControl() {
  document
    .querySelectorAll("[data-novavei-load-earlier]")
    .forEach((node) => node.remove());
}

function isTranscriptNearTop(transcript: HTMLElement) {
  return transcript.scrollTop <= LOAD_EARLIER_SCROLL_THRESHOLD_PX;
}

function isTranscriptNearBottom(transcript: HTMLElement) {
  const remaining =
    transcript.scrollHeight - transcript.scrollTop - transcript.clientHeight;
  return remaining <= VIRTUAL_WINDOW_BOTTOM_THRESHOLD_PX;
}

function ensureLoadEarlierControl(
  show: boolean,
  onLoad: () => void | Promise<void>,
  label?: string,
) {
  removeLoadEarlierControl();
  if (!show) return;
  const axis = transcriptAxis();
  if (!axis) return;
  const button = document.createElement("button");
  button.type = "button";
  button.dataset.novaveiLoadEarlier = "true";
  button.className = "history-load-earlier";
  button.textContent = label || "加载更早消息";
  button.setAttribute("aria-label", label || "加载更早的对话消息");
  button.addEventListener("click", () => {
    void onLoad();
  });
  axis.insertBefore(button, axis.firstChild);
}

type TranscriptScrollAnchor = {
  messageId: string;
  offset: number;
};

function transcriptMessageNodes(axis: HTMLElement | null) {
  if (!axis) return [] as HTMLElement[];
  return [...axis.querySelectorAll<HTMLElement>("[data-message-id]")];
}

function transcriptMessageNodeById(
  axis: HTMLElement | null,
  messageId: string,
) {
  return transcriptMessageNodes(axis).find(
    (item) => item.dataset.messageId === messageId,
  );
}

function captureTranscriptScrollAnchor(
  transcript: HTMLElement | null,
  retainedMessageIds?: ReadonlySet<string>,
): TranscriptScrollAnchor | undefined {
  if (!transcript) return undefined;
  const transcriptRect = transcript.getBoundingClientRect();
  const candidates = transcriptMessageNodes(transcriptAxis()).filter((item) => {
    const id = item.dataset.messageId;
    return !retainedMessageIds || Boolean(id && retainedMessageIds.has(id));
  });
  const target =
    candidates.find(
      (item) => item.getBoundingClientRect().bottom > transcriptRect.top,
    ) ?? candidates[0];
  const messageId = target?.dataset.messageId;
  if (!target || !messageId) return undefined;
  return {
    messageId,
    offset: target.getBoundingClientRect().top - transcriptRect.top,
  };
}

function restoreTranscriptScrollAnchor(
  transcript: HTMLElement,
  anchor: TranscriptScrollAnchor,
) {
  const target = transcriptMessageNodeById(transcriptAxis(), anchor.messageId);
  if (!target) return false;
  const offset =
    target.getBoundingClientRect().top - transcript.getBoundingClientRect().top;
  transcript.scrollTop += offset - anchor.offset;
  return true;
}

function notifyTranscriptWindowRendered(
  sessionId: string,
  pageState: SessionMessagePageState,
) {
  window.dispatchEvent(
    new CustomEvent("novavei:transcript-window-rendered", {
      detail: { sessionId, pageGeneration: pageState.generation },
    }),
  );
}

/**
 * Render only a DOM window of pageState.messages to keep long transcripts light.
 * Full history remains in memory; scroll handlers shift `domStart`.
 */
function renderMessageWindow(
  pageState: SessionMessagePageState,
  sessionId: string,
  history?: unknown,
  options: {
    onLoadEarlier?: () => void | Promise<void>;
    scroll?: "bottom" | "preserve" | "none";
    preserveScrollTop?: number;
    preserveScrollHeight?: number;
    scrollAnchor?: TranscriptScrollAnchor;
  } = {},
) {
  if (history !== undefined) pageState.history = history;
  const renderHistory = history ?? pageState.history;
  const messageCount = pageState.messages.length;
  if (messageCount === 0) {
    pageState.domStart = 0;
    clearTranscript();
    notifyTranscriptWindowRendered(sessionId, pageState);
    return;
  }
  const maxStart = Math.max(0, messageCount - MAX_DOM_MESSAGES);
  pageState.domStart = Math.min(Math.max(0, pageState.domStart), maxStart);
  const windowEnd = Math.min(
    messageCount,
    pageState.domStart + MAX_DOM_MESSAGES,
  );
  const visible = pageState.messages.slice(pageState.domStart, windowEnd);
  const transcript = node<HTMLElement>("transcript");
  const previousHeight =
    options.preserveScrollHeight ?? transcript?.scrollHeight ?? 0;
  const previousTop = options.preserveScrollTop ?? transcript?.scrollTop ?? 0;

  clearTranscript();
  const defaults = historyMessageDefaults(renderHistory);
  for (const message of visible) {
    appendHistoryMessage(message, sessionId, defaults);
  }

  const hiddenAbove = pageState.domStart;
  const needsEarlierChrome = pageState.hasMoreBefore || pageState.domStart > 0;
  if (needsEarlierChrome && options.onLoadEarlier) {
    const english = document.documentElement.lang
      .toLowerCase()
      .startsWith("en");
    let label = english ? "Load earlier messages" : "加载更早消息";
    if (hiddenAbove > 0 && !pageState.hasMoreBefore) {
      label = english
        ? `${hiddenAbove} earlier loaded messages above — scroll up to view`
        : `上方还有 ${hiddenAbove} 条已加载消息，向上滚动查看`;
    } else if (hiddenAbove > 0) {
      label = english
        ? `Load earlier · ${hiddenAbove} loaded messages above`
        : `加载更早消息 · 上方还有 ${hiddenAbove} 条已加载`;
    }
    ensureLoadEarlierControl(true, options.onLoadEarlier, label);
  }

  if (transcript) {
    if (options.scroll === "bottom") {
      transcript.scrollTop = transcript.scrollHeight;
    } else if (options.scroll === "preserve") {
      if (
        !options.scrollAnchor ||
        !restoreTranscriptScrollAnchor(transcript, options.scrollAnchor)
      ) {
        const delta = transcript.scrollHeight - previousHeight;
        transcript.scrollTop = previousTop + Math.max(0, delta);
      }
    }
  }
  window.__novaveiFloorNav?.refresh?.();
  notifyTranscriptWindowRendered(sessionId, pageState);
}

function renderHistory(
  messages: MessageRecord[],
  sessionId: string,
  history?: unknown,
  options: {
    hasMoreBefore?: boolean;
    onLoadEarlier?: () => void | Promise<void>;
    pageState?: SessionMessagePageState;
  } = {},
) {
  const pageState =
    options.pageState ??
    createSessionMessagePageState(
      messages,
      messages.length,
      Boolean(options.hasMoreBefore),
      history,
    );
  if (!options.pageState) {
    pageState.messages = sortedTranscriptMessages(messages);
    pageState.hasMoreBefore = Boolean(options.hasMoreBefore);
    pageState.domStart = defaultDomStart(messages.length);
    pageState.history = history;
  }
  renderMessageWindow(pageState, sessionId, history, {
    onLoadEarlier: options.onLoadEarlier,
    scroll: "bottom",
  });
}

function historyActionButton(action: string, label: string) {
  const button = document.createElement("button");
  button.type = "button";
  button.dataset.historyAction = action;
  button.textContent = label;
  return button;
}

function displayModelLabel(value: unknown) {
  const model = typeof value === "string" ? value.trim() : "";
  if (!model) return undefined;
  const option = [
    ...document.querySelectorAll<HTMLElement>(".model-option"),
  ].find(
    (candidate) =>
      candidate.dataset.modelId === model ||
      candidate.dataset.piModelLabel === model,
  );
  return (
    option?.dataset.piModelLabel ||
    option?.textContent?.trim().replace(/\s+/g, " ") ||
    model
  );
}

function displayReasoningLabel(value: unknown) {
  const reasoning = typeof value === "string" ? value.trim().toLowerCase() : "";
  switch (reasoning) {
    case "off":
      return "关闭";
    case "minimal":
      return "最少";
    case "low":
      return "轻度";
    case "medium":
      return "中";
    case "high":
      return "高";
    case "xhigh":
      return "极高";
    case "max":
      return "最高";
    default:
      return reasoning || undefined;
  }
}

function historyMessageDefaults(value: unknown): HistoryMessageDefaults {
  const history = record(value);
  const selection = storedSessionModelSelection(value);
  return {
    model: displayModelLabel(history?.model ?? selection?.modelId),
    reasoning: displayReasoningLabel(
      history?.reasoning ?? history?.reasoningLevel,
    ),
  };
}

function historyMessageTime(value: unknown) {
  const numeric = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(numeric) || numeric <= 0) return undefined;
  const date = new Date(numeric < 1_000_000_000_000 ? numeric * 1000 : numeric);
  return Number.isNaN(date.getTime()) ? undefined : date;
}

/**
 * Only rebuild the virtual transcript when the renderer-facing live
 * projection actually changed. In particular, a terminal state can be
 * observed again while the DOM is rebinding after a window render; rebuilding
 * that identical row would synchronously emit the same render event again.
 */
function liveProjectionChanged(
  previous: MessageRecord | undefined,
  next: MessageRecord,
) {
  if (!previous) return true;
  return (
    previous.liveId !== next.liveId ||
    previous.role !== next.role ||
    previous.content !== next.content ||
    previous.createdAt !== next.createdAt ||
    previous.requestId !== next.requestId ||
    previous.turnId !== next.turnId ||
    previous.status !== next.status ||
    previous.prompt !== next.prompt ||
    previous.model !== next.model ||
    previous.reasoning !== next.reasoning ||
    previous.thinking !== next.thinking ||
    messageToolsSignature(previous.tools) !==
      messageToolsSignature(next.tools) ||
    previous.finishedAt !== next.finishedAt
  );
}

function messageToolsSignature(tools: readonly PiToolState[] | undefined) {
  if (!tools?.length) return "";
  return tools
    .map((tool) =>
      [
        tool.id,
        tool.name,
        tool.status,
        tool.startedAt ?? "",
        tool.finishedAt ?? "",
        tool.error ?? "",
        messageToolValueSignature(tool.arguments),
        messageToolValueSignature(tool.result),
      ].join("\u001f"),
    )
    .join("\u001e");
}

function messageToolValueSignature(value: unknown) {
  if (value === undefined) return "";
  try {
    const json = JSON.stringify(value);
    return json ?? String(value);
  } catch {
    return String(value);
  }
}

function appendHistoryMessage(
  message: MessageRecord,
  sessionId: string,
  defaults: HistoryMessageDefaults,
) {
  const axis = transcriptAxis();
  if (!axis) return;
  const content = typeof message.content === "string" ? message.content : "";
  const role = (message.role ?? "").toLowerCase();
  const id = stableMessageId(message);
  if (role === "user") {
    const item = document.createElement("div");
    item.className = "msg-user";
    item.dataset.floorId = id;
    item.dataset.messageId = id;
    if (message.liveId) item.dataset.liveMessageId = message.liveId;
    item.dataset.novaveiHistory = "true";
    renderComposerMessageMedia(item, content, sessionId);
    axis.appendChild(item);
    return;
  }
  if (role === "tool" || role === "toolresult" || role === "tool_result") {
    const item = document.createElement("div");
    item.className = "tool-row";
    item.dataset.messageId = id;
    if (message.liveId) item.dataset.liveMessageId = message.liveId;
    item.dataset.novaveiHistory = "true";
    item.setAttribute("role", "status");
    const label = document.createElement("span");
    label.textContent = "工具";
    const body = document.createElement("span");
    body.textContent = content;
    item.append(label, body);
    axis.appendChild(item);
    return;
  }
  const article = document.createElement("article");
  article.className = "msg-assistant";
  article.dataset.messageId = id;
  if (message.liveId) article.dataset.liveMessageId = message.liveId;
  article.dataset.novaveiHistory = "true";
  article.dataset.historySessionId = sessionId;
  const persistedMessageId =
    typeof message.id === "string" ? message.id.trim() : "";
  const turnId =
    typeof message.turnId === "string" ? message.turnId.trim() : "";
  if (persistedMessageId) article.dataset.historyMessageId = persistedMessageId;
  if (turnId) article.dataset.historyTurnId = turnId;
  const who = document.createElement("div");
  who.className = "who";
  const name = document.createElement("b");
  name.textContent = "NovaVei";
  const exactModel = displayModelLabel(
    message.modelLabel ?? message.model ?? message.modelId,
  );
  const exactReasoning = displayReasoningLabel(
    message.reasoning ?? message.reasoningLevel,
  );
  const displayedModel = exactModel ?? defaults.model ?? "未记录模型";
  const displayedReasoning = exactReasoning ?? defaults.reasoning ?? "未记录";
  const badge = document.createElement("span");
  badge.className = "badge-soft";
  badge.textContent = `${displayedModel} · Agent`;
  who.append(name, badge);
  const text = document.createElement("div");
  text.className = "markdown-body";
  text.dataset.historyContent = "true";
  renderMarkdown(text, stripPlanProtocolBlocks(content));
  const actions = document.createElement("div");
  actions.className = "msg-actions";
  const trace = historyActionButton("trace", "查看轨迹");
  trace.setAttribute("aria-expanded", "false");
  if (!persistedMessageId || !turnId) {
    trace.disabled = true;
    trace.textContent = "轨迹不可用";
    trace.title = "该历史回复未保存可验证的运行标识，无法安全显示轨迹。";
  }
  const meta = document.createElement("span");
  meta.className = "msg-meta";
  meta.title =
    exactModel && exactReasoning
      ? "模型与思考等级"
      : "旧记录未保存完整的逐轮模型或思考等级；缺失值使用会话级信息。";
  const metaModel = document.createElement("b");
  metaModel.textContent = displayedModel;
  const separator = document.createElement("span");
  separator.className = "sep";
  separator.setAttribute("aria-hidden", "true");
  const metaReasoning = document.createElement("span");
  metaReasoning.textContent = displayedReasoning;
  meta.append(metaModel, separator, metaReasoning);
  const ended = document.createElement("time");
  ended.className = "msg-ended";
  const finished = historyMessageTime(
    message.finishedAt ?? message.endedAt ?? message.createdAt,
  );
  if (finished) {
    ended.dateTime = finished.toISOString();
    ended.textContent = formatMessageTimestamp(finished);
  } else {
    ended.textContent = "—";
    ended.title = "该历史回复未保存结束时间";
  }
  actions.append(
    historyActionButton("copy", "复制"),
    trace,
    historyActionButton("retry", "重试"),
    historyActionButton("branch", "分叉新对话"),
    meta,
    ended,
  );
  const thinking = typeof message.thinking === "string" ? message.thinking : "";
  const tools = message.tools ?? [];
  const thinkingPanel = createAssistantThinkingPanel(thinking);
  if (thinking.trim() || tools.length) {
    if (tools.length) thinkingPanel.hidden = false;
    article.append(who, thinkingPanel, text, actions);
    if (tools.length) {
      renderAssistantThinkingTools(article, tools);
    } else if (persistedMessageId && turnId) {
      const thinkingTrace = historyActionButton("trace", "查看详情");
      thinkingTrace.setAttribute("aria-expanded", "false");
      renderAssistantThinkingTools(article, [], {
        forceVisible: true,
        emptyText: "点击查看本回复调用轨迹",
        detailButton: thinkingTrace,
      });
    }
  } else {
    article.append(who, text, actions);
  }
  axis.appendChild(article);
}

function sessionButton(
  session: SessionSummary,
  projectName?: string,
  selection?: ProjectConversationSelectionRenderState,
  runState?: PiRuntimeSnapshot,
  workspaceStatus?: WorkspaceLocationStatus,
  workspaceRegistered = true,
): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "session";
  button.dataset.novaveiSession = "true";
  button.dataset.sessionId = session.id;
  button.dataset.title = session.title;
  button.dataset.workdir = displayPath(session.cwd);
  if (projectName) button.dataset.project = projectName;
  if (workspaceStatus) button.dataset.workspaceStatus = workspaceStatus;
  button.dataset.novaveiWorkspaceKind = workspaceRegistered
    ? "project"
    : "unregistered";
  button.classList.toggle("is-unregistered-workspace", !workspaceRegistered);
  const relocationRequired = isRelocationRequired(workspaceStatus);
  button.classList.toggle("is-workspace-unavailable", relocationRequired);
  button.classList.toggle(
    "is-workspace-status-unknown",
    workspaceStatus === "unavailable",
  );
  if (session.isPinned) button.classList.add("is-pinned-entry");
  if (session.isArchived) button.classList.add("is-archived-entry");
  if (selection) {
    const belongsToSelectedProject =
      pathKey(session.cwd) === pathKey(selection.workdir);
    button.classList.add("is-selection-mode");
    button.setAttribute(
      "aria-pressed",
      String(belongsToSelectedProject && selection.selectedIds.has(session.id)),
    );
    if (!belongsToSelectedProject || selection.isBusy) {
      button.disabled = true;
      button.setAttribute("aria-disabled", "true");
    }
  }
  const strong = document.createElement("strong");
  strong.textContent = session.title || "新建对话";
  const small = document.createElement("small");
  const displayedWorkdir = displayPath(session.cwd) || "本地工作区";
  const workspaceStateLabel = workspaceRegistered
    ? workspaceStatusLabel(workspaceStatus)
    : "未登记";
  small.textContent = workspaceStateLabel
    ? `${workspaceStateLabel} · ${displayedWorkdir}`
    : displayedWorkdir;
  const workspaceDescription =
    workspaceStatusDescription(workspaceStatus) ||
    (!workspaceRegistered
      ? "该工作空间尚未登记为项目；历史会话仍可只读打开。"
      : "");
  if (workspaceDescription) {
    small.title = workspaceDescription;
    button.setAttribute(
      "aria-label",
      `${session.title || "新建对话"}，${workspaceStateLabel || "路径不可用"}。${workspaceDescription}`,
    );
  }
  const liveRun = runState && isLiveSessionRun(runState) ? runState : undefined;
  const storedStatus = liveRun
    ? undefined
    : persistedSessionRunStatus(session.lastRunStatus);
  if (liveRun || storedStatus) {
    const status = document.createElement("span");
    status.className = "session-run-status";
    const statusKey = liveRun?.status ?? storedStatus;
    status.dataset.piRunStatus = statusKey;
    button.dataset.piRunStatus = statusKey;
    if (liveRun) {
      status.setAttribute("role", "status");
      status.textContent = sessionRunLabel(liveRun);
    } else if (storedStatus) {
      const label = persistedSessionRunLabel(
        storedStatus,
        session.lastRunFinishedAt,
      );
      status.classList.add("is-terminal");
      status.textContent = label;
      status.title = `最近一次执行：${label}`;
      status.setAttribute("aria-label", status.title);
    }
    button.append(strong, status, small);
  } else {
    button.append(strong, small);
  }
  return button;
}

function workspaceStatusForWorkdir(
  workdir: string,
  sessions: readonly SessionSummary[],
  workspaceStatuses: ReadonlyMap<string, WorkspaceLocationStatus>,
) {
  const fromBatch = workspaceStatuses.get(pathKey(workdir));
  if (fromBatch) return fromBatch;
  const statuses = sessions
    .map((session) => session.workspaceStatus)
    .filter(
      (status): status is WorkspaceLocationStatus => status !== undefined,
    );
  return statuses.find((status) => isRelocationRequired(status)) ?? statuses[0];
}

function renderSessions(
  sessions: SessionSummary[],
  activeId: string | undefined,
  workdir: string | undefined,
  projects: readonly ProjectEntry[],
  selection?: ProjectConversationSelectionRenderState,
  sessionRuns: ReadonlyMap<string, PiRuntimeSnapshot> = new Map(),
  workspaceStatuses: ReadonlyMap<string, WorkspaceLocationStatus> = new Map(),
) {
  const currentKey = pathKey(workdir);
  const projectKeys = new Set(projects.map((project) => pathKey(project.path)));
  const byWorkdir = new Map<string, SessionSummary[]>();
  for (const session of sessions) {
    const key = pathKey(session.cwd);
    if (!key) continue;
    const group = byWorkdir.get(key) ?? [];
    group.push(session);
    byWorkdir.set(key, group);
  }

  for (const project of projects)
    projectFolderForWorkdir(project.path, true, project.name);

  // A portable copy can carry durable transcripts whose folders belong to a
  // previous computer. Keep those transcripts in local history/search, but
  // show only deliberately registered projects in the sidebar.
  const otherSection = otherWorkspacesSection();
  if (otherSection) otherSection.hidden = true;
  document
    .querySelectorAll<HTMLElement>(
      '.project-folder[data-novavei-workspace-kind="unregistered"]',
    )
    .forEach((folder) => folder.remove());

  // Native workspace rows are rebuilt from durable project state on every
  // refresh. Historical session roots never create sidebar folders by itself.
  document
    .querySelectorAll<HTMLElement>(".project-folder")
    .forEach((folder) => {
      const host = projectConversationHost(folder);
      const row = folder.querySelector<HTMLElement>(
        ".project-row[data-workdir]",
      );
      if (!host || !row) return;
      host
        .querySelectorAll<HTMLElement>(".session, .workspace-path-notice")
        .forEach((item) => item.remove());
      const key = pathKey(row.dataset.workdir);
      const isRegistered = projectKeys.has(key);
      if (!isRegistered) {
        folder.remove();
        return;
      }
      folder.hidden = false;
      folder.dataset.novaveiWorkspaceKind = "project";
      folder.classList.remove("is-unregistered-workspace");
      row.dataset.novaveiWorkspaceKind = "project";
      row.dataset.novaveiProject = "true";

      const group = byWorkdir.get(key) ?? [];
      const visible = group.filter((session) => !session.isArchived);
      const status = workspaceStatusForWorkdir(
        row.dataset.workdir || "",
        group,
        workspaceStatuses,
      );
      setWorkspaceRowStatus(row, visible.length, status, key === currentKey);

      const register = folder.querySelector<HTMLButtonElement>(
        "[data-novavei-register-workspace]",
      );
      if (register) register.hidden = true;
      if (status && status !== "available" && row.dataset.workdir) {
        host.appendChild(workspacePathNotice(row.dataset.workdir, status));
      }

      const projectName =
        row.dataset.project || pathName(row.dataset.workdir || "");
      for (const session of visible) {
        const button = sessionButton(
          session,
          projectName,
          selection,
          sessionRuns.get(session.id),
          status,
          isRegistered,
        );
        button.classList.toggle("active", session.id === activeId);
        if (session.id === activeId)
          button.setAttribute("aria-current", "page");
        host.appendChild(button);
      }
    });

  const currentRow = workdir
    ? (workspaceFolderForWorkdir(workdir)?.querySelector<HTMLElement>(
        ".project-row[data-workdir]",
      ) ?? null)
    : null;
  if (currentRow && workdir) {
    setProjectCurrent(currentRow, workdir);
    // Keep the user's collapse choice across list refreshes. Only default the
    // active workspace to expanded when it has never been toggled.
    if (
      currentRow.dataset.expanded !== "true" &&
      currentRow.dataset.expanded !== "false"
    ) {
      currentRow.dataset.expanded = "true";
    }
    setProjectFolderExpanded(
      currentRow,
      currentRow.dataset.expanded === "true",
    );
  } else publishCurrentProjectChanged(null, undefined);
  document
    .querySelectorAll<HTMLElement>(".project-row[data-workdir]")
    .forEach((row) => {
      if (row === currentRow) return;
      setProjectFolderExpanded(row, row.dataset.expanded === "true");
    });

  document
    .querySelectorAll<HTMLElement>("#pinnedSessionsGroup .session")
    .forEach((item) => item.remove());
  const projectNameFor = (session: SessionSummary) =>
    projects.find((project) => pathKey(project.path) === pathKey(session.cwd))
      ?.name || pathName(session.cwd);
  const pinned = document.getElementById("pinnedSessionsGroup");
  if (pinned) {
    for (const session of sessions.filter(
      (item) =>
        item.isPinned && !item.isArchived && projectKeys.has(pathKey(item.cwd)),
    )) {
      const button = sessionButton(
        session,
        projectNameFor(session),
        selection,
        sessionRuns.get(session.id),
        workspaceStatusForWorkdir(
          session.cwd,
          byWorkdir.get(pathKey(session.cwd)) ?? [],
          workspaceStatuses,
        ),
        projectKeys.has(pathKey(session.cwd)),
      );
      button.classList.add("is-pinned-entry");
      button.classList.toggle("active", session.id === activeId);
      if (session.id === activeId) button.setAttribute("aria-current", "page");
      pinned.appendChild(button);
    }
    pinned.hidden = !pinned.querySelector(".session");
  }

  const empty = node<HTMLElement>("sidebarEmpty");
  if (empty) {
    const hasWorkspaces = projects.length > 0;
    empty.hidden = hasWorkspaces;
    if (!hasWorkspaces)
      empty.textContent = "还没有项目。点击「打开」添加一个。";
  }
}

function sessionTitle(sessionId: string | undefined): string {
  if (!sessionId) return "新建对话";
  return (
    document
      .querySelector<HTMLElement>(
        `.session[data-session-id="${CSS.escape(sessionId)}"] strong`,
      )
      ?.textContent?.trim() || "新建对话"
  );
}

export function installNativeShell(): NativeShellApi | undefined {
  const invoke = getInvoke();
  if (!invoke) return undefined;

  installAboutSystemInfo(invoke);

  let sessionId: string | undefined;
  // The HTML shell contains example project rows for browser preview. Never
  // adopt one of those paths as a desktop working directory before native
  // projects and sessions have completed hydration.
  let workdir: string | undefined;
  let workdirTrusted = false;
  let sessions: SessionSummary[] = [];
  let projects: ProjectEntry[] = [];
  let projectMutationQueue: Promise<void> = Promise.resolve();
  let workspaceStatuses = new Map<string, WorkspaceLocationStatus>();
  let nativeShellState: NativeShellState = "loading";
  let workspaceCapability: WorkspaceCapability | undefined;
  let sessionLoadSerial = 0;
  let sessionViewEpoch = 0;
  let activeSessionView: { sessionId: string; epoch: number } | undefined;
  let selectedProjectConversationWorkdir: string | undefined;
  let newChatOperation: Promise<void> | undefined;
  const sessionMessagePages = new Map<string, SessionMessagePageState>();
  let loadEarlierRequest: SessionPageRequest | undefined;
  let windowShiftRequest: SessionPageRequest | undefined;
  let transcriptScrollListener: ((event: Event) => void) | undefined;
  let transcriptScrollTimer: number | undefined;
  let cachedHistoryMessagePageSize = readStoredHistoryMessagePageSize();
  function historyMessagePageSize(): number {
    return clampHistoryMessagePageSize(cachedHistoryMessagePageSize);
  }
  window.addEventListener("novavei:history-page-size-changed", (event) => {
    const detail =
      event instanceof CustomEvent ? record(event.detail) : undefined;
    if (detail && "pageSize" in detail) {
      cachedHistoryMessagePageSize = clampHistoryMessagePageSize(
        detail.pageSize,
      );
      try {
        window.localStorage?.setItem(
          HISTORY_MESSAGE_PAGE_SIZE_STORAGE_KEY,
          String(cachedHistoryMessagePageSize),
        );
      } catch {
        // ignore storage failures
      }
    }
  });
  const sessionsGetArgs = (id: string, extra: Record<string, unknown> = {}) => {
    const pageSize = historyMessagePageSize();
    return {
      sessionId: id,
      session_id: id,
      limit: pageSize,
      ...extra,
    };
  };
  const sessionRuns = new Map<string, PiRuntimeSnapshot>();
  const protectedSessionMessagePageIds = () =>
    new Set([
      ...sessionRuns.keys(),
      ...(sessionId ? [sessionId] : []),
      ...(activeSessionView?.sessionId ? [activeSessionView.sessionId] : []),
    ]);
  const pruneCachedSessionMessagePages = () =>
    pruneSessionMessagePageCache(
      sessionMessagePages,
      protectedSessionMessagePageIds(),
    );
  const cacheSessionMessagePage = (
    cachedSessionId: string,
    pageState: SessionMessagePageState,
  ) => {
    // Refresh insertion order so eviction follows least-recently-loaded pages.
    sessionMessagePages.delete(cachedSessionId);
    sessionMessagePages.set(cachedSessionId, pageState);
    pruneCachedSessionMessagePages();
  };
  const selectedProjectConversationIds = new Set<string>();
  let isProjectConversationSelectionBusy = false;
  const sessionListeners = new Set<
    (sessions: readonly SessionSummary[]) => void
  >();

  const publishAppHealth = (health: AppHealth) => {
    // Other runtime surfaces use this compact native projection to gate only
    // the capabilities that need it. No filesystem, database, DPAPI, proxy,
    // or provider diagnostics are forwarded into the WebView.
    window.dispatchEvent(
      new CustomEvent("novavei:app-health-changed", { detail: health }),
    );
  };

  const publishSessionsChanged = () => {
    const snapshot = sessions.slice();
    window.dispatchEvent(
      new CustomEvent("novavei:sessions-changed", {
        detail: { sessions: snapshot },
      }),
    );
    for (const listener of sessionListeners) listener(snapshot);
  };

  const currentProjectForWorkdir = () =>
    workdir
      ? projects.find((project) => pathKey(project.path) === pathKey(workdir))
      : undefined;

  const publishCurrentProjectPreferences = () => {
    const project = currentProjectForWorkdir();
    // This DTO intentionally contains only model identity, reasoning level,
    // and a safe permission tier. It is an event for defaults, never a
    // provider-config transport or a Full-access grant.
    window.dispatchEvent(
      new CustomEvent("novavei:project-preferences-changed", {
        detail: {
          projectId: project?.id ?? null,
          workdir: workdir ? displayPath(workdir) : null,
          preferences: project?.preferences ?? null,
        },
      }),
    );
  };

  const defaultComposerPlaceholder =
    node<HTMLTextAreaElement>("composerInput")?.placeholder ||
    "描述目标，或粘贴上下文…";

  const setNativeShellState = (next: NativeShellState) => {
    nativeShellState = next;
    document.body.dataset.novaveiShellState = next;
    const ready = next === "ready";
    const input = node<HTMLTextAreaElement>("composerInput");
    const send = node<HTMLButtonElement>("btnSend");
    const attachment = node<HTMLButtonElement>("btnComposerAdd");
    const newChat = node<HTMLButtonElement>("btnNewChat");
    const selectProjectConversations = node<HTMLButtonElement>(
      "btnSelectProjectConversations",
    );
    const openProject = document.querySelector<HTMLButtonElement>(
      ".folder-add:not(.folder-select)",
    );

    if (!ready) {
      // Do not retain a token that was minted while the durable store was
      // healthy. A recovery transition must make every capability request go
      // through the native persistence gate again after the shell is ready.
      workspaceCapability = undefined;
      // Selection mode itself is not durable, but leaving a bulk-action surface
      // visible during recovery suggests that the underlying archive/delete
      // writes are available. Clear it before its controls can be re-rendered.
      selectedProjectConversationWorkdir = undefined;
      selectedProjectConversationIds.clear();
      isProjectConversationSelectionBusy = false;
      node<HTMLElement>("projectConversationSelectionToolbar")?.setAttribute(
        "hidden",
        "",
      );
    }

    if (input) {
      input.disabled = !ready;
      input.placeholder = ready
        ? defaultComposerPlaceholder
        : next === "loading"
          ? "正在读取本地项目与会话…"
          : next === "needs_workspace"
            ? currentWorkspaceStatus() === "unavailable"
              ? "当前项目路径暂不可用，恢复后再试…"
              : workdir && !isRegisteredWorkspace(workdir)
                ? "历史工作空间尚未登记为项目…"
                : "请先打开一个项目文件夹…"
            : next === "needs_session"
              ? "请先选择项目并创建新对话…"
              : next === "needs_relocation"
                ? "当前项目目录已失效，请先重新选择目录…"
                : next === "storage_recovery"
                  ? "本地存储需要恢复后才能发送…"
                  : "本地项目暂不可用，请重试…";
    }
    if (send) {
      // Composer chrome is shared with the active Pi run and provider
      // onboarding. Host readiness may disable the control, but must not
      // overwrite a live Stop / Retry-stop button while a turn is active.
      if (!ready) {
        send.disabled = true;
        send.title = input?.placeholder || "当前不可发送";
      } else if (send.dataset.piRunning !== "true") {
        send.disabled = false;
        // Provider readiness will refine labels/disabled state via the
        // host-state-changed listener in the DOM runtime.
      }
    }
    if (attachment) attachment.disabled = !ready;
    if (newChat)
      newChat.disabled =
        next === "loading" ||
        next === "needs_relocation" ||
        next === "storage_recovery" ||
        next === "error";
    if (selectProjectConversations)
      selectProjectConversations.disabled = !ready;
    if (openProject)
      openProject.disabled =
        next === "loading" || next === "storage_recovery" || next === "error";
    document
      .querySelectorAll<HTMLButtonElement>(
        '[data-history-action="retry"], [data-history-action="branch"]',
      )
      .forEach((button) => {
        button.disabled = !ready;
        button.setAttribute("aria-disabled", String(!ready));
        button.title = ready
          ? ""
          : "只读历史不能重试或分叉；请先登记并恢复项目路径";
      });
    // Other desktop surfaces (notably provider onboarding) share Composer
    // controls. Publish the authoritative host readiness so they cannot
    // re-enable a control while no native project/session exists.
    window.dispatchEvent(
      new CustomEvent("novavei:host-state-changed", {
        detail: { state: next },
      }),
    );
  };

  const updateWorkdirChrome = (nextWorkdir: string | undefined) => {
    const displayedWorkdir = nextWorkdir
      ? displayPath(nextWorkdir)
      : "未打开项目";
    const status = node<HTMLElement>("workdirStatus");
    if (status) status.textContent = displayedWorkdir;
  };

  const currentWorkspaceStatus = () => {
    if (!workdir) return undefined;
    return workspaceStatusForWorkdir(
      workdir,
      sessions.filter((session) => pathKey(session.cwd) === pathKey(workdir)),
      workspaceStatuses,
    );
  };

  /**
   * A historical cwd is intentionally not a project-root grant.  Keep this
   * check in the renderer as a UX boundary as well as in native code: a user
   * may still read its transcript, but composing or opening File Dock must
   * first promote it through the explicit project-registration action.
   */
  const isRegisteredWorkspace = (candidate: string | undefined) =>
    Boolean(
      candidate &&
        projects.some(
          (project) => pathKey(project.path) === pathKey(candidate),
        ),
    );

  const workspaceIsReadyForTools = (candidate: string | undefined) => {
    if (!candidate || !isRegisteredWorkspace(candidate)) return false;
    return workspaceStatusFor(candidate) === "available";
  };

  const setLoadedSessionWorkspaceState = () => {
    const status = currentWorkspaceStatus();
    if (isRelocationRequired(status)) {
      workdirTrusted = false;
      setNativeShellState("needs_relocation");
      return;
    }
    // A status probe failure and an unregistered historical group both fail
    // closed. The transcript remains visible; only new runs/capabilities are
    // paused until the user registers a verified root or retries later.
    workdirTrusted = workspaceIsReadyForTools(workdir);
    setNativeShellState(workdirTrusted ? "ready" : "needs_workspace");
  };

  const activeProjectConversationSelection = () => {
    const selectedWorkdir = selectedProjectConversationWorkdir;
    if (!selectedWorkdir || !workdir) return undefined;
    const currentProjectExists =
      pathKey(selectedWorkdir) === pathKey(workdir) &&
      projects.some(
        (project) => pathKey(project.path) === pathKey(selectedWorkdir),
      );
    if (!currentProjectExists) return undefined;
    return {
      workdir: selectedWorkdir,
      selectedIds: selectedProjectConversationIds,
      isBusy: isProjectConversationSelectionBusy,
    } satisfies ProjectConversationSelectionRenderState;
  };

  const pruneProjectConversationSelection = () => {
    const selection = activeProjectConversationSelection();
    if (!selection) {
      selectedProjectConversationWorkdir = undefined;
      selectedProjectConversationIds.clear();
      isProjectConversationSelectionBusy = false;
      return;
    }
    const validIds = new Set(
      sessions
        .filter(
          (session) =>
            pathKey(session.cwd) === pathKey(selection.workdir) &&
            !session.isArchived,
        )
        .map((session) => session.id),
    );
    for (const selectedId of selectedProjectConversationIds) {
      if (!validIds.has(selectedId))
        selectedProjectConversationIds.delete(selectedId);
    }
  };

  const selectableProjectConversationIds = (targetWorkdir: string) =>
    sessions
      .filter(
        (session) =>
          pathKey(session.cwd) === pathKey(targetWorkdir) &&
          !session.isArchived,
      )
      .map((session) => session.id);

  const renderProjectConversationSelectionToolbar = () => {
    const selection = activeProjectConversationSelection();
    const toggle = node<HTMLButtonElement>("btnSelectProjectConversations");
    const toolbar = node<HTMLElement>("projectConversationSelectionToolbar");
    const count = node<HTMLElement>("projectConversationSelectionCount");
    const selectAll = node<HTMLButtonElement>(
      "btnSelectAllProjectConversations",
    );
    const archive = node<HTMLButtonElement>("btnArchiveSelectedConversations");
    const remove = node<HTMLButtonElement>("btnDeleteSelectedConversations");
    const cancel = node<HTMLButtonElement>(
      "btnCancelProjectConversationSelection",
    );
    const hasCurrentProject = Boolean(
      workdir &&
        projects.some((project) => pathKey(project.path) === pathKey(workdir)),
    );
    const interactionsAllowed = nativeShellState === "ready";
    if (toggle) {
      toggle.disabled =
        !interactionsAllowed ||
        !hasCurrentProject ||
        isProjectConversationSelectionBusy;
      toggle.setAttribute("aria-pressed", String(Boolean(selection)));
    }
    if (!toolbar) return;
    toolbar.hidden = !selection || !interactionsAllowed;
    toolbar.setAttribute("aria-busy", String(Boolean(selection?.isBusy)));
    if (!selection || !interactionsAllowed) return;
    const selectableIds = selectableProjectConversationIds(selection.workdir);
    const selectedCount = selectedProjectConversationIds.size;
    const allSelected =
      selectableIds.length > 0 &&
      selectableIds.every((id) => selectedProjectConversationIds.has(id));
    if (count) count.textContent = `已选 ${selectedCount} 项`;
    if (selectAll) {
      selectAll.disabled = selection.isBusy || selectableIds.length === 0;
      selectAll.setAttribute("aria-pressed", String(allSelected));
      selectAll.textContent = allSelected ? "取消全选" : "全选";
      selectAll.setAttribute(
        "aria-label",
        allSelected ? "取消全选当前项目会话" : "全选当前项目会话",
      );
    }
    if (archive) archive.disabled = selection.isBusy || selectedCount === 0;
    if (remove) remove.disabled = selection.isBusy || selectedCount === 0;
    if (cancel) cancel.disabled = selection.isBusy;
  };

  const renderSessionNavigation = () => {
    pruneProjectConversationSelection();
    renderSessions(
      sessions,
      sessionId,
      workdir,
      projects,
      activeProjectConversationSelection(),
      sessionRuns,
      workspaceStatuses,
    );
    renderProjectConversationSelectionToolbar();
    publishSessionsChanged();
  };

  window.__novaveiPiRuntime?.subscribeSessionState(
    (updatedSessionId, state) => {
      if (isLiveSessionRun(state)) sessionRuns.set(updatedSessionId, state);
      else {
        sessionRuns.delete(updatedSessionId);
        const terminalStatus = terminalSessionRunStatus(state);
        if (terminalStatus) {
          const finishedAt = Date.now();
          sessions = sessions.map((session) =>
            session.id === updatedSessionId
              ? {
                  ...session,
                  lastRunStatus: terminalStatus,
                  lastRunFinishedAt: finishedAt,
                }
              : session,
          );
        }
      }
      pruneCachedSessionMessagePages();
      renderSessionNavigation();
    },
  );

  const clearProjectConversationSelection = () => {
    selectedProjectConversationWorkdir = undefined;
    selectedProjectConversationIds.clear();
    isProjectConversationSelectionBusy = false;
    renderSessionNavigation();
  };

  const toggleProjectConversationSelection = () => {
    if (nativeShellState !== "ready") return;
    const currentProject = workdir
      ? projects.find((project) => pathKey(project.path) === pathKey(workdir))
      : undefined;
    if (!currentProject) {
      toast("请先打开当前项目的会话");
      return;
    }
    if (activeProjectConversationSelection()) {
      clearProjectConversationSelection();
      return;
    }
    selectedProjectConversationWorkdir = currentProject.path;
    selectedProjectConversationIds.clear();
    isProjectConversationSelectionBusy = false;
    renderSessionNavigation();
  };

  const toggleSelectedProjectConversation = (
    targetSessionId: string,
    targetWorkdir: string,
  ) => {
    if (nativeShellState !== "ready") return;
    const selection = activeProjectConversationSelection();
    if (
      !selection ||
      selection.isBusy ||
      pathKey(targetWorkdir) !== pathKey(selection.workdir)
    )
      return;
    if (selectedProjectConversationIds.has(targetSessionId))
      selectedProjectConversationIds.delete(targetSessionId);
    else selectedProjectConversationIds.add(targetSessionId);
    renderSessionNavigation();
  };

  const toggleSelectAllProjectConversations = () => {
    if (nativeShellState !== "ready") return;
    const selection = activeProjectConversationSelection();
    if (!selection || selection.isBusy) return;
    const selectableIds = selectableProjectConversationIds(selection.workdir);
    if (!selectableIds.length) return;
    const allSelected = selectableIds.every((id) =>
      selectedProjectConversationIds.has(id),
    );
    selectedProjectConversationIds.clear();
    if (!allSelected) {
      for (const id of selectableIds) selectedProjectConversationIds.add(id);
    }
    renderSessionNavigation();
  };

  const clearPrototypeSessionSurface = () => {
    // The desktop shell always begins from its native source of truth. Keep
    // the static surface empty while local projects and sessions hydrate.
    document
      .querySelectorAll<HTMLElement>(".session:not([data-novavei-session])")
      .forEach((item) => item.remove());
    document
      .querySelectorAll<HTMLElement>(".project-folder")
      .forEach((folder) => {
        // Remove, rather than merely hide, any non-native folders. Several
        // runtime consumers deliberately look for `[data-workdir]`; leaving
        // stale markup behind could make it an accidental cwd fallback.
        if (folder.dataset.novaveiProject !== "true") folder.remove();
      });
    node<HTMLElement>("sidebarEmpty")?.setAttribute("hidden", "");
    node<HTMLElement>("chatTitle")?.replaceChildren(
      document.createTextNode("正在打开工作区…"),
    );
    node<HTMLElement>("workdirStatus")?.replaceChildren(
      document.createTextNode("正在读取本地项目与会话…"),
    );
    clearTranscript();
  };

  const clearSessionLoadNotice = () =>
    node<HTMLElement>("novaveiNativeSessionNotice")?.remove();

  const openingAction = (
    actions: HTMLElement,
    action: "open-workspace" | "create-session" | "retry-hydration",
    label: string,
    primary = false,
    workdir?: string,
  ) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = primary ? "btn primary" : "btn";
    button.dataset.novaveiOpeningAction = action;
    if (workdir) button.dataset.novaveiWorkdir = workdir;
    button.textContent = label;
    actions.appendChild(button);
  };

  const renderOpeningState = (state: Exclude<NativeShellState, "ready">) => {
    clearTranscript();
    const axis = transcriptAxis();
    if (!axis) return;

    const panel = document.createElement("section");
    panel.className = "principle";
    panel.dataset.novaveiOpeningState = state;
    panel.style.maxWidth = "560px";
    panel.style.margin = "auto";
    panel.style.alignSelf = "center";
    const alert = state === "error" || state === "storage_recovery";
    panel.setAttribute("aria-live", alert ? "assertive" : "polite");
    if (alert) panel.setAttribute("role", "alert");

    const eyebrow = document.createElement("span");
    eyebrow.className = "kicker";
    eyebrow.textContent = "NovaVei";
    const title = document.createElement("h3");
    const detail = document.createElement("p");
    detail.style.margin = "0";
    detail.style.color = "var(--muted)";
    detail.style.lineHeight = "1.55";
    const actions = document.createElement("div");
    actions.className = "row-actions";
    actions.style.marginTop = "16px";

    if (state === "loading") {
      title.textContent = "正在读取本地工作区";
      detail.textContent = "正在恢复你已保存的项目文件夹和会话。";
    } else if (state === "needs_workspace") {
      title.textContent = "先打开一个项目文件夹";
      detail.textContent =
        "NovaVei 只会在你明确选择的项目根中创建会话、读取文件和运行工具。";
      openingAction(actions, "open-workspace", "打开项目文件夹", true);
    } else if (state === "needs_session") {
      const currentProject = currentProjectForWorkdir();
      const project =
        currentProject && workspaceIsReadyForTools(currentProject.path)
          ? currentProject
          : projects.find((candidate) =>
              workspaceIsReadyForTools(candidate.path),
            );
      title.textContent = "创建第一段对话";
      detail.textContent = project
        ? `已保存项目「${project.name}」。创建对话后即可开始使用 Agent；也可以从左侧选择其他项目。`
        : "请选择一个项目文件夹，然后创建新的对话。";
      if (project)
        openingAction(
          actions,
          "create-session",
          `在「${project.name}」中新建对话`,
          true,
          project.path,
        );
      openingAction(
        actions,
        "open-workspace",
        project ? "打开其他项目文件夹" : "打开项目文件夹",
        !project,
      );
    } else if (state === "storage_recovery") {
      title.textContent = "本地存储需要恢复";
      detail.textContent =
        "NovaVei 无法确认已保存的会话和设置是否完整，已停止新的写入以避免造成更多不一致。请检查本地文件访问权限后重启；可导出诊断包协助恢复。";
    } else {
      title.textContent = "无法读取本地项目与会话";
      detail.textContent =
        "当前不会用临时内容替代本地数据。请重试；恢复后再继续创建或打开对话。";
      openingAction(actions, "retry-hydration", "重试", true);
    }

    panel.append(eyebrow, title, detail);
    if (actions.childElementCount) panel.appendChild(actions);
    axis.appendChild(panel);
  };

  const presentOpeningState = (state: Exclude<NativeShellState, "ready">) => {
    setNativeShellState(state);
    const title = node<HTMLElement>("chatTitle");
    const workdirStatus = node<HTMLElement>("workdirStatus");
    if (state === "loading") {
      title?.replaceChildren(document.createTextNode("正在打开工作区…"));
      workdirStatus?.replaceChildren(
        document.createTextNode("正在读取本地项目与会话…"),
      );
    } else if (state === "needs_workspace") {
      title?.replaceChildren(document.createTextNode("欢迎使用 NovaVei"));
      workdirStatus?.replaceChildren(document.createTextNode("未打开项目"));
    } else if (state === "needs_session") {
      title?.replaceChildren(document.createTextNode("选择或创建对话"));
      workdirStatus?.replaceChildren(
        document.createTextNode("已保存项目文件夹 · 尚未打开会话"),
      );
    } else if (state === "storage_recovery") {
      title?.replaceChildren(document.createTextNode("本地存储需要恢复"));
      workdirStatus?.replaceChildren(document.createTextNode("已暂停本地写入"));
    } else {
      title?.replaceChildren(document.createTextNode("无法读取本地数据"));
      workdirStatus?.replaceChildren(
        document.createTextNode("本地项目暂不可用"),
      );
    }
    renderOpeningState(state);
  };

  const presentNoSessionState = () => {
    const hasReadyProject = projects.some((project) =>
      workspaceIsReadyForTools(project.path),
    );
    // A fresh installation should still look like a new conversation, not a
    // welcome/demo screen. Composer controls remain safely gated until the
    // user selects a project, because every durable session is workspace-bound.
    setNativeShellState(hasReadyProject ? "needs_session" : "needs_workspace");
    clearTranscript();
    node<HTMLElement>("chatTitle")?.replaceChildren(
      document.createTextNode("新建对话"),
    );
    node<HTMLElement>("workdirStatus")?.replaceChildren(
      document.createTextNode(
        hasReadyProject
          ? "已打开项目 · 正在准备新对话"
          : "选择项目文件夹后开始",
      ),
    );
  };

  const persistProjectEntries = async (entries: readonly ProjectEntry[]) => {
    const response = await invoke<unknown>("settings_save_projects", {
      payload: { version: 2, initialized: true, entries },
    });
    const saved = readProjectSettings(response);
    if (!saved.initialized) throw new Error("项目设置保存响应无效");
    projects = saved.entries;
    publishCurrentProjectPreferences();
    return projects;
  };

  const mutateProjects = (
    mutation: (current: readonly ProjectEntry[]) => ProjectEntry[],
  ) => {
    const task = projectMutationQueue
      .catch(() => undefined)
      .then(() => persistProjectEntries(mutation(projects)));
    projectMutationQueue = task.then(
      () => undefined,
      () => undefined,
    );
    return task;
  };

  const saveProjects = (entries: ProjectEntry[]) =>
    mutateProjects(() => entries);

  const mergeProjectPreferences = (
    current: ProjectPreferences | undefined,
    patch: Partial<ProjectPreferences>,
  ): ProjectPreferences | undefined => {
    const model = patch.model ?? current?.model;
    const reasoning = patch.reasoning ?? current?.reasoning;
    const permission = patch.permission ?? current?.permission;
    return model || reasoning || permission
      ? {
          ...(model ? { model } : {}),
          ...(reasoning ? { reasoning } : {}),
          ...(permission ? { permission } : {}),
        }
      : undefined;
  };

  const saveCurrentProjectPreferences = async (
    patch: Partial<ProjectPreferences>,
  ) => {
    const target = currentProjectForWorkdir();
    if (!target) return undefined;
    const projectId = target.id;
    const saved = await mutateProjects((current) => {
      const matching = current.find((project) => project.id === projectId);
      if (!matching) throw new Error("当前项目已不在已保存列表中");
      const preferences = mergeProjectPreferences(matching.preferences, patch);
      return current.map((project) =>
        project.id === projectId
          ? {
              ...project,
              ...(preferences ? { preferences } : {}),
            }
          : project,
      );
    });
    return saved.find((project) => project.id === projectId)?.preferences;
  };

  const hydrateProjects = async (settings: unknown) => {
    const stored = readProjectSettings(record(settings)?.projects);
    if (stored.initialized) {
      projects = stored.entries;
      // Old renderer builds persisted a path hash in `id`.  Keep it readable
      // through this hydration, then let the native normalizer replace it with
      // a UUID on the next successful protected-settings write.
      if (projects.some((project) => !hasStableProjectId(project.id))) {
        try {
          await saveProjects(projects);
        } catch (error) {
          console.warn(
            "[NovaVei projects] stable id migration could not be persisted",
            error,
          );
        }
      }
      return;
    }
    // Missing project settings is not consent to turn every historical cwd
    // into a live project. The sidebar keeps history without a registered
    // root out of view, avoiding stale folders and implicit file authority.
    projects = [];
  };

  const refreshWorkspacePathStatuses = async () => {
    const pathsByKey = new Map<string, string>();
    for (const candidate of projects.map((project) => project.path)) {
      const cleaned = displayPath(candidate);
      const key = pathKey(cleaned);
      if (key && !pathsByKey.has(key)) pathsByKey.set(key, cleaned);
    }
    const paths = [...pathsByKey.values()];
    if (!paths.length) {
      workspaceStatuses = new Map();
      return true;
    }
    const nextStatuses = new Map<string, WorkspaceLocationStatus>();
    try {
      for (
        let start = 0;
        start < paths.length;
        start += MAX_WORKSPACE_PATH_STATUS_BATCH_SIZE
      ) {
        const response = await invoke<unknown>("workspace_paths_status", {
          paths: paths.slice(
            start,
            start + MAX_WORKSPACE_PATH_STATUS_BATCH_SIZE,
          ),
        });
        for (const [key, status] of parseWorkspacePathStatuses(response))
          nextStatuses.set(key, status);
      }
      const complete = paths.every((path) => nextStatuses.has(pathKey(path)));
      for (const path of paths) {
        const key = pathKey(path);
        if (!nextStatuses.has(key)) nextStatuses.set(key, "unavailable");
      }
      workspaceStatuses = nextStatuses;
      if (!complete) {
        console.warn(
          "[NovaVei workspaces] native status response omitted known paths",
        );
      }
      return complete;
    } catch (error) {
      // A failed status probe must not turn a transient native problem into a
      // false "path missing" marker or erase the recovery controls entirely.
      // Mark every requested known root as temporarily unavailable so tools
      // fail closed while the user still has a visible “重新检查” action.
      for (const path of paths) {
        const key = pathKey(path);
        if (!nextStatuses.has(key)) nextStatuses.set(key, "unavailable");
      }
      workspaceStatuses = nextStatuses;
      console.warn("[NovaVei workspaces] could not refresh path status", error);
      return false;
    }
  };

  const upsertProject = async (nextWorkdir: string, lastSessionId?: string) => {
    const key = pathKey(nextWorkdir);
    await mutateProjects((current) => {
      const existing = current.find((project) => pathKey(project.path) === key);
      return existing
        ? current.map((project) =>
            project === existing
              ? { ...project, ...(lastSessionId ? { lastSessionId } : {}) }
              : project,
          )
        : [
            ...current,
            {
              ...projectFromWorkdir(nextWorkdir, sessions),
              ...(lastSessionId ? { lastSessionId } : {}),
            },
          ];
    });
  };

  const rememberProjectSession = (
    nextWorkdir: string,
    lastSessionId: string,
  ) => {
    const project = projects.find(
      (entry) => pathKey(entry.path) === pathKey(nextWorkdir),
    );
    if (!project || project.lastSessionId === lastSessionId) return;
    void mutateProjects((current) =>
      current.map((entry) =>
        entry.id === project.id ? { ...entry, lastSessionId } : entry,
      ),
    ).catch((error) =>
      console.warn("[NovaVei projects] could not save last session", error),
    );
  };

  const showSessionLoadNotice = (
    state: "loading" | "storage_recovery" | "error",
  ) => {
    const section = node<HTMLElement>("projectSection");
    if (!section) return;
    let notice = node<HTMLElement>("novaveiNativeSessionNotice");
    if (!notice) {
      notice = document.createElement("div");
      notice.id = "novaveiNativeSessionNotice";
      notice.className = "sidebar-empty";
      section.appendChild(notice);
    }
    notice.replaceChildren();
    notice.setAttribute(
      "aria-live",
      state === "error" || state === "storage_recovery"
        ? "assertive"
        : "polite",
    );
    if (state === "loading") {
      notice.removeAttribute("role");
      notice.textContent = "正在读取本地会话…";
      return;
    }
    notice.setAttribute("role", "alert");
    const message = document.createElement("span");
    message.textContent =
      state === "storage_recovery"
        ? "本地存储需要恢复。已停止新的写入。"
        : "无法读取本地会话。请重试。";
    if (state === "storage_recovery") {
      notice.appendChild(message);
      return;
    }
    const actions = document.createElement("div");
    actions.className = "row-actions";
    actions.style.marginTop = "8px";
    const retry = document.createElement("button");
    retry.type = "button";
    retry.className = "btn";
    retry.textContent = "重试";
    retry.setAttribute("aria-label", "重新读取本地会话");
    retry.addEventListener("click", () => {
      startNativeHydration();
    });
    actions.appendChild(retry);
    notice.append(message, actions);
  };

  /**
   * Health can turn blocked after the initial desktop hydration has already
   * made the shell usable. Keep that later transition just as fail-closed as
   * the startup path: invalidate renderer capabilities, publish the durable
   * health projection, and replace the currently visible session with the
   * recovery UI.
   */
  const enterStorageRecovery = (health: AppHealth) => {
    publishAppHealth(health);
    showSessionLoadNotice("storage_recovery");
    presentOpeningState("storage_recovery");
  };

  const requireReadyForDesktopCapability = () => {
    if (nativeShellState === "storage_recovery")
      throw new StorageRecoveryRequiredError();
    if (nativeShellState === "needs_relocation")
      throw new Error("当前项目路径已失效，请先重新选择目录");
    if (nativeShellState !== "ready")
      throw new Error("本地项目与会话尚未准备完成");
  };

  const requireReadyForWorkspaceRelocation = () => {
    if (nativeShellState === "storage_recovery")
      throw new StorageRecoveryRequiredError();
    if (
      nativeShellState !== "ready" &&
      nativeShellState !== "needs_relocation" &&
      // Relocation is a recovery action for an inactive registered project
      // (`needs_session`) and for a readable-but-unregistered historical
      // session (`needs_workspace`), not only for the current failed session.
      // Do not make a visible “重新选择目录” action fail merely because no
      // other workspace has been opened yet.
      nativeShellState !== "needs_workspace" &&
      nativeShellState !== "needs_session"
    )
      throw new Error("本地项目与会话尚未准备完成");
  };

  const isPageRequestCurrent = (request: SessionPageRequest) =>
    sessionViewEpoch === request.viewEpoch &&
    activeSessionView?.epoch === request.viewEpoch &&
    activeSessionView?.sessionId === request.sessionId &&
    sessionId === request.sessionId &&
    sessionMessagePages.get(request.sessionId) === request.pageState;

  const hasCurrentPageRequest = (request: SessionPageRequest | undefined) =>
    Boolean(request && isPageRequestCurrent(request));

  const loadEarlierMessages = async (id: string, history?: unknown) => {
    const pageState = sessionMessagePages.get(id);
    if (!pageState) return;
    const request: SessionPageRequest = {
      sessionId: id,
      pageState,
      viewEpoch: sessionViewEpoch,
    };
    if (
      !isPageRequestCurrent(request) ||
      hasCurrentPageRequest(loadEarlierRequest) ||
      hasCurrentPageRequest(windowShiftRequest)
    )
      return;

    // Prefer shifting the in-memory window before hitting the network.
    if (pageState.domStart > 0) {
      windowShiftRequest = request;
      try {
        if (!isPageRequestCurrent(request)) return;
        const transcript = node<HTMLElement>("transcript");
        const scrollAnchor = captureTranscriptScrollAnchor(transcript);
        const anchorIndex = scrollAnchor
          ? pageState.messages.findIndex(
              (message) => stableMessageId(message) === scrollAnchor.messageId,
            )
          : -1;
        const shift = Math.min(
          pageState.domStart,
          Math.max(historyMessagePageSize(), 40),
          MAX_DOM_MESSAGES - 1,
        );
        const desiredStart = Math.max(0, pageState.domStart - shift);
        // Keep the exact visible anchor inside the next window even when the
        // configured page size is close to the DOM-window capacity.
        pageState.domStart =
          anchorIndex >= 0
            ? Math.max(desiredStart, anchorIndex - MAX_DOM_MESSAGES + 1)
            : desiredStart;
        if (!isPageRequestCurrent(request)) return;
        renderMessageWindow(pageState, id, history, {
          onLoadEarlier: () => loadEarlierMessages(id, history),
          scroll: "preserve",
          scrollAnchor,
        });
      } finally {
        if (windowShiftRequest === request) windowShiftRequest = undefined;
      }
      return;
    }

    if (!pageState.hasMoreBefore || pageState.messages.length === 0) return;
    const cursor = messageCursor(pageState.messages[0]);
    if (!cursor) return;
    loadEarlierRequest = request;
    const control = document.querySelector<HTMLButtonElement>(
      "[data-novavei-load-earlier]",
    );
    if (control && isPageRequestCurrent(request)) {
      control.disabled = true;
      control.textContent = "加载中…";
    }
    try {
      const raw = await invoke<unknown>(
        "sessions_get",
        sessionsGetArgs(id, {
          beforeCreatedAt: cursor.createdAt,
          before_created_at: cursor.createdAt,
          beforeId: cursor.id,
          before_id: cursor.id,
        }),
      );
      if (!isPageRequestCurrent(request)) return;
      const older = parseSessionsGetResponse(raw);
      const existingIds = new Set(pageState.messages.map(stableMessageId));
      const fresh = older.messages.filter(
        (message) => !existingIds.has(stableMessageId(message)),
      );
      const transcript = node<HTMLElement>("transcript");
      const scrollAnchor = captureTranscriptScrollAnchor(transcript);
      const anchorIndex = scrollAnchor
        ? pageState.messages.findIndex(
            (message) => stableMessageId(message) === scrollAnchor.messageId,
          )
        : -1;
      if (fresh.length > 0) {
        pageState.messages = sortedTranscriptMessages([
          ...fresh,
          ...pageState.messages,
        ]);
        // A configured page may be larger than the DOM window. Keep the
        // visible anchor inside that window rather than rendering only the
        // freshly prepended rows and losing the reader's place.
        const sortedAnchorIndex = scrollAnchor
          ? pageState.messages.findIndex(
              (message) => stableMessageId(message) === scrollAnchor.messageId,
            )
          : -1;
        const newAnchorIndex =
          sortedAnchorIndex >= 0
            ? sortedAnchorIndex
            : anchorIndex >= 0
              ? fresh.length + anchorIndex
              : fresh.length;
        const maxStart = Math.max(
          0,
          pageState.messages.length - MAX_DOM_MESSAGES,
        );
        pageState.domStart = Math.min(
          maxStart,
          Math.max(0, newAnchorIndex - Math.floor(MAX_DOM_MESSAGES / 2)),
        );
      }
      pageState.totalCount = Math.max(
        older.totalCount,
        pageState.messages.length,
      );
      pageState.hasMoreBefore = older.hasMoreBefore;
      if (!isPageRequestCurrent(request)) return;
      renderMessageWindow(pageState, id, history, {
        onLoadEarlier: () => loadEarlierMessages(id, history),
        scroll: "preserve",
        scrollAnchor,
      });
    } catch (error) {
      if (!isPageRequestCurrent(request)) return;
      toast(
        error instanceof Error ? error.message : "加载更早消息失败，请重试",
      );
      // The control is recreated only for the exact still-active page object.
      ensureLoadEarlierControl(true, () => loadEarlierMessages(id, history));
    } finally {
      if (loadEarlierRequest === request) loadEarlierRequest = undefined;
    }
  };

  const shiftMessageWindowDown = (id: string, history?: unknown) => {
    const pageState = sessionMessagePages.get(id);
    if (!pageState) return;
    const request: SessionPageRequest = {
      sessionId: id,
      pageState,
      viewEpoch: sessionViewEpoch,
    };
    if (
      !isPageRequestCurrent(request) ||
      hasCurrentPageRequest(windowShiftRequest) ||
      hasCurrentPageRequest(loadEarlierRequest)
    )
      return;
    const maxStart = Math.max(0, pageState.messages.length - MAX_DOM_MESSAGES);
    if (pageState.domStart >= maxStart) return;
    windowShiftRequest = request;
    try {
      if (!isPageRequestCurrent(request)) return;
      const shift = Math.min(
        maxStart - pageState.domStart,
        Math.max(historyMessagePageSize(), 40),
        MAX_DOM_MESSAGES - 1,
      );
      const nextStart = Math.min(maxStart, pageState.domStart + shift);
      const retainedMessageIds = new Set(
        pageState.messages
          .slice(nextStart, nextStart + MAX_DOM_MESSAGES)
          .map(stableMessageId),
      );
      const scrollAnchor = captureTranscriptScrollAnchor(
        node<HTMLElement>("transcript"),
        retainedMessageIds,
      );
      pageState.domStart = nextStart;
      if (!isPageRequestCurrent(request)) return;
      renderMessageWindow(pageState, id, history, {
        onLoadEarlier: () => loadEarlierMessages(id, history),
        scroll: "preserve",
        scrollAnchor,
      });
    } finally {
      if (windowShiftRequest === request) windowShiftRequest = undefined;
    }
  };

  const detachTranscriptScrollPaging = () => {
    const transcript = node<HTMLElement>("transcript");
    if (transcript && transcriptScrollListener) {
      transcript.removeEventListener("scroll", transcriptScrollListener);
    }
    transcriptScrollListener = undefined;
    if (transcriptScrollTimer !== undefined) {
      window.clearTimeout(transcriptScrollTimer);
      transcriptScrollTimer = undefined;
    }
  };

  const attachTranscriptScrollPaging = (
    id: string,
    history: unknown | undefined,
  ) => {
    detachTranscriptScrollPaging();
    const transcript = node<HTMLElement>("transcript");
    const pageState = sessionMessagePages.get(id);
    if (!transcript || !pageState) return;
    const attachedRequest: SessionPageRequest = {
      sessionId: id,
      pageState,
      viewEpoch: sessionViewEpoch,
    };
    const onScroll = () => {
      if (
        !isPageRequestCurrent(attachedRequest) ||
        hasCurrentPageRequest(loadEarlierRequest) ||
        hasCurrentPageRequest(windowShiftRequest)
      )
        return;
      if (transcriptScrollTimer !== undefined)
        window.clearTimeout(transcriptScrollTimer);
      // Debounce so rapid wheel/trackpad events do not stampede the host.
      transcriptScrollTimer = window.setTimeout(() => {
        transcriptScrollTimer = undefined;
        if (
          !isPageRequestCurrent(attachedRequest) ||
          hasCurrentPageRequest(loadEarlierRequest) ||
          hasCurrentPageRequest(windowShiftRequest)
        )
          return;
        const current = sessionMessagePages.get(id);
        if (current !== pageState) return;
        if (isTranscriptNearTop(transcript)) {
          if (current.domStart > 0 || current.hasMoreBefore) {
            void loadEarlierMessages(id, history);
          }
          return;
        }
        if (isTranscriptNearBottom(transcript)) {
          const maxStart = Math.max(
            0,
            current.messages.length - MAX_DOM_MESSAGES,
          );
          if (current.domStart < maxStart) {
            shiftMessageWindowDown(id, history);
          }
        }
      }, 120);
    };
    transcriptScrollListener = onScroll;
    transcript.addEventListener("scroll", onScroll, { passive: true });
  };

  const beginSessionNavigation = (
    targetSessionId?: string,
  ): SessionViewNavigation => {
    const navigation: SessionViewNavigation = {
      epoch: ++sessionViewEpoch,
      serial: ++sessionLoadSerial,
      targetSessionId,
    };
    // A page request captures both this epoch and its page object. Clearing the
    // active view before any await closes A -> B -> A write-back windows.
    activeSessionView = undefined;
    loadEarlierRequest = undefined;
    windowShiftRequest = undefined;
    detachTranscriptScrollPaging();
    removeLoadEarlierControl();
    window.dispatchEvent(
      new CustomEvent("novavei:session-view-invalidated", {
        detail: { epoch: navigation.epoch, targetSessionId },
      }),
    );
    return navigation;
  };

  const isNavigationCurrent = (navigation: SessionViewNavigation) =>
    sessionViewEpoch === navigation.epoch &&
    sessionLoadSerial === navigation.serial;

  const loadSession = async (id: string) => {
    // Search results and archived/settings rows can outlive a concurrent list
    // refresh. Never let an unknown session inherit the currently selected
    // cwd: that would make a stale history result appear writable under an
    // unrelated project. Keep the current view intact and ask the caller to
    // refresh instead.
    if (!sessions.some((item) => item.id === id)) {
      throw new Error("找不到要打开的本地会话；请刷新会话列表后重试");
    }
    const navigation = beginSessionNavigation(id);
    const previousWorkdirKey = pathKey(workdir);
    const previousPage = sessionMessagePages.get(id);
    const historyRequest =
      previousPage?.history !== undefined
        ? Promise.resolve(previousPage.history)
        : invoke<unknown>("chat_history_get", { id }).catch(() => undefined);
    try {
      const [messagesRaw, history] = await Promise.all([
        invoke<unknown>("sessions_get", sessionsGetArgs(id)),
        historyRequest,
      ]);
      if (!isNavigationCurrent(navigation)) return;
      // Re-check after native reads complete. A delete/archive refresh may have
      // removed the row while the transcript request was in flight.
      const selected = sessions.find((item) => item.id === id);
      if (!selected) {
        throw new Error("该本地会话已不存在；请刷新会话列表后重试");
      }
      const page = parseSessionsGetResponse(messagesRaw);
      const messages = mergeServerPageWithLive(
        page.messages,
        previousPage?.messages ?? [],
      );
      const pageState = createSessionMessagePageState(
        messages,
        Math.max(page.totalCount, messages.length),
        page.hasMoreBefore,
        history,
      );
      sessionId = id;
      activeSessionView = { sessionId: id, epoch: navigation.epoch };
      cacheSessionMessagePage(id, pageState);
      const nextWorkdir = selected.cwd;
      if (nextWorkdir !== workdir) {
        workspaceCapability = undefined;
      }
      workdir = nextWorkdir;
      workdirTrusted = workspaceIsReadyForTools(nextWorkdir);
      if (workdir) {
        const registeredProject = projects.find(
          (project) => pathKey(project.path) === pathKey(workdir),
        );
        const folder = registeredProject
          ? projectFolderForWorkdir(
              registeredProject.path,
              true,
              registeredProject.name,
            )
          : otherWorkspaceFolderForWorkdir(workdir, true);
        setProjectCurrent(
          folder?.querySelector<HTMLElement>(".project-row[data-workdir]") ??
            null,
          workdir,
        );
      }
      if (!isNavigationCurrent(navigation)) return;
      renderHistory(messages, id, history, {
        hasMoreBefore: pageState.hasMoreBefore,
        onLoadEarlier: () => loadEarlierMessages(id, history),
        pageState,
      });
      attachTranscriptScrollPaging(id, history);
      const title = node<HTMLElement>("chatTitle");
      if (title) title.textContent = selected.title || sessionTitle(id);
      updateWorkdirChrome(workdir);
      renderSessionNavigation();
      setLoadedSessionWorkspaceState();
      if (workdir) rememberProjectSession(workdir, id);
      if (workdir && pathKey(workdir) !== previousWorkdirKey)
        notifyWorkdirChanged(workdir);
      notifySessionChanged(id, history, pageState.messages);
      publishCurrentProjectPreferences();
    } catch (error) {
      // A superseded navigation must not surface an old load error as a toast
      // through the caller that initiated it.
      if (!isNavigationCurrent(navigation)) return;
      throw error;
    }
  };

  const upsertLiveTranscriptMessage = (message: LiveTranscriptMessage) => {
    const targetSessionId = message.sessionId.trim();
    const liveId = message.id.trim();
    if (!targetSessionId || !liveId) return;
    let pageState = sessionMessagePages.get(targetSessionId);
    if (!pageState) {
      pageState = createSessionMessagePageState([], 0, false);
      cacheSessionMessagePage(targetSessionId, pageState);
    }
    const next: MessageRecord = {
      id: liveId,
      liveId,
      role: message.role,
      content: message.content,
      createdAt: message.createdAt,
      ...(message.requestId !== undefined
        ? { requestId: message.requestId }
        : {}),
      ...(message.turnId !== undefined ? { turnId: message.turnId } : {}),
      ...(message.status !== undefined ? { status: message.status } : {}),
      ...(message.prompt !== undefined ? { prompt: message.prompt } : {}),
      ...(message.model !== undefined ? { model: message.model } : {}),
      ...(message.reasoning !== undefined
        ? { reasoning: message.reasoning }
        : {}),
      ...(message.thinking !== undefined ? { thinking: message.thinking } : {}),
      ...(message.tools !== undefined
        ? { tools: normalizeMessageTools(message.tools) ?? [] }
        : {}),
      ...(message.finishedAt !== undefined
        ? { finishedAt: message.finishedAt }
        : {}),
    };
    const index = pageState.messages.findIndex(
      (candidate) => candidate.liveId === liveId,
    );
    const inserted = index < 0;
    const previous = inserted ? undefined : pageState.messages[index];
    const projectionChanged = liveProjectionChanged(previous, next);
    const orderChanged = transcriptOrderChanged(previous, next);
    if (inserted) {
      pageState.messages.push(next);
      pageState.domStart = defaultDomStart(pageState.messages.length);
    } else {
      const previous = pageState.messages[index];
      pageState.messages[index] = {
        ...previous,
        ...next,
        // A subsequent sessions_get may already have reconciled this live row
        // to its durable id. Keep that durable key while retaining liveId.
        id: previous.id ?? next.id,
        liveId,
        createdAt: previous.createdAt ?? next.createdAt,
      };
    }
    if (inserted || orderChanged)
      pageState.messages = sortedTranscriptMessages(pageState.messages);
    pageState.totalCount = Math.max(
      pageState.totalCount,
      pageState.messages.length,
    );

    // Streaming updates paint directly into the active assistant node, so
    // rebuilding the virtual transcript for every token would be wasteful.
    // A terminal event is already durably persisted, however. Repaint that
    // one final assistant projection so the visible page reconciles to the
    // completed/cancelled/error state without requiring the user to leave and
    // reopen the conversation.
    const terminalAssistantUpdate =
      message.role === "assistant" &&
      persistedSessionRunStatus(message.status) !== undefined;

    if (
      (inserted ||
        orderChanged ||
        (terminalAssistantUpdate && projectionChanged)) &&
      activeSessionView?.sessionId === targetSessionId &&
      activeSessionView?.epoch === sessionViewEpoch &&
      sessionId === targetSessionId
    ) {
      renderMessageWindow(pageState, targetSessionId, pageState.history, {
        onLoadEarlier: () =>
          loadEarlierMessages(targetSessionId, pageState?.history),
        scroll: "bottom",
      });
    }
  };

  const refreshSessions = async (options: { loadActive?: boolean } = {}) => {
    // Health is intentionally the first host read. Do not load fallback
    // sessions/settings when the native store cannot make durable promises;
    // that would make a transient in-memory default look like real data.
    let health: AppHealth;
    try {
      health = normaliseAppHealth(await invoke<unknown>("app_health"));
    } catch {
      // A missing/failed health read is indistinguishable from an unreadable
      // durable store to the renderer. Do not leave a previously ready shell
      // interactive while callers receive only a rejected refresh promise.
      const recoveryHealth = normaliseAppHealth(undefined);
      enterStorageRecovery(recoveryHealth);
      throw new StorageRecoveryRequiredError();
    }
    if (!appHealthAllowsWrites(health)) {
      enterStorageRecovery(health);
      throw new StorageRecoveryRequiredError();
    }
    publishAppHealth(health);
    const [result, settings] = await Promise.all([
      invoke<unknown>("sessions_list"),
      invoke<unknown>("settings_load_all"),
    ]);
    const system = record(record(settings)?.system);
    if (system) {
      const fromSettings =
        system.historyMessagePageSize ?? system.history_message_page_size;
      if (fromSettings !== undefined) {
        cachedHistoryMessagePageSize =
          clampHistoryMessagePageSize(fromSettings);
        try {
          window.localStorage?.setItem(
            HISTORY_MESSAGE_PAGE_SIZE_STORAGE_KEY,
            String(cachedHistoryMessagePageSize),
          );
        } catch {
          // ignore storage failures
        }
      }
      const fullMessageTimestamp =
        system.showFullMessageTimestamp ??
        system.show_full_message_timestamp ??
        system.messageTimestampFormat ??
        system.message_timestamp_format;
      if (fullMessageTimestamp !== undefined) {
        applyFullMessageTimestampPreference(
          normalizeFullMessageTimestampPreference(fullMessageTimestamp),
        );
      }
    }
    sessions = Array.isArray(result)
      ? result
          .map(sessionRecord)
          .filter((item): item is SessionSummary => Boolean(item))
      : [];
    await hydrateProjects(settings);
    await refreshWorkspacePathStatuses();
    const registered = new Set(
      projects.map((project) => pathKey(project.path)),
    );
    let targetSessionId = sessionId;
    if (
      !targetSessionId ||
      !sessions.some((item) => item.id === targetSessionId)
    ) {
      const preferred = projects
        .map((project) => project.lastSessionId)
        .find((id): id is string =>
          Boolean(id && sessions.some((session) => session.id === id)),
        );
      targetSessionId =
        preferred ??
        sessions.find((session) => registered.has(pathKey(session.cwd)))?.id;
    }
    if (!targetSessionId) {
      const firstReadyProject = projects.find((project) =>
        workspaceIsReadyForTools(project.path),
      );
      if (firstReadyProject) {
        // A saved project with no history opens straight into a durable blank
        // session, instead of showing a separate empty-state landing page.
        await createSession(firstReadyProject.path);
        clearSessionLoadNotice();
        return;
      }
      beginSessionNavigation();
      workdir = undefined;
      sessionId = undefined;
      workdirTrusted = false;
      workspaceCapability = undefined;
      renderSessionNavigation();
      presentNoSessionState();
      clearSessionLoadNotice();
      return;
    }
    const selected = sessions.find((item) => item.id === targetSessionId);
    if (selected?.cwd) {
      if (pathKey(selected.cwd) !== pathKey(workdir)) {
        workspaceCapability = undefined;
      }
      workdir = selected.cwd;
      workdirTrusted = workspaceIsReadyForTools(selected.cwd);
      if (isRelocationRequired(currentWorkspaceStatus()))
        setNativeShellState("needs_relocation");
    }
    renderSessionNavigation();
    if (options.loadActive !== false) await loadSession(targetSessionId);
    else {
      sessionId = targetSessionId;
      setLoadedSessionWorkspaceState();
      publishCurrentProjectPreferences();
    }
    clearSessionLoadNotice();
  };

  const findReusableDraftSession = async (cwd: string) => {
    const candidates = sessions.filter(
      (session) => pathKey(session.cwd) === pathKey(cwd),
    );
    for (const candidate of candidates) {
      try {
        const isBlank = await invoke<boolean>("sessions_is_blank", {
          sessionId: candidate.id,
          session_id: candidate.id,
        });
        if (isBlank) return candidate;
      } catch {
        // Fall back to a paged sessions_get only when the blank probe is
        // unavailable (older host / mock).
        const page = parseSessionsGetResponse(
          await invoke<unknown>("sessions_get", sessionsGetArgs(candidate.id)),
        );
        if (
          !page.messages.some(
            (message) => message.role?.trim().toLowerCase() === "user",
          )
        ) {
          return candidate;
        }
      }
    }
    return undefined;
  };

  const createSession = async (cwd?: string) => {
    const previousShellState = nativeShellState;
    const previousWorkdirKey = pathKey(workdir);
    const requestedCwd = cwd?.trim() || (workdirTrusted ? workdir : undefined);
    if (!requestedCwd) throw new Error("请先打开项目文件夹");
    if (!isRegisteredWorkspace(requestedCwd)) {
      throw new Error("该历史工作空间尚未登记为项目，不能创建新对话或运行工具");
    }
    if (!workspaceIsReadyForTools(requestedCwd)) {
      throw new Error(
        "当前项目路径暂不可用，请恢复路径或重新选择目录后再创建对话",
      );
    }
    const navigation = beginSessionNavigation();
    if (previousShellState !== "ready") setNativeShellState("loading");
    try {
      const createdRaw = await invoke<unknown>("sessions_create", {
        title: "新建对话",
        cwd: requestedCwd,
      });
      const created = sessionRecord(createdRaw);
      if (!created?.id) throw new Error("创建会话失败");
      if (!isNavigationCurrent(navigation)) {
        // The durable create succeeded, but a newer navigation owns the view.
        // Keep sidebar data coherent without stealing focus back to this row.
        sessions = [
          created,
          ...sessions.filter((item) => item.id !== created.id),
        ];
        return;
      }
      sessionId = created.id;
      activeSessionView = { sessionId: created.id, epoch: navigation.epoch };
      workspaceCapability = undefined;
      workdir = created.cwd || requestedCwd;
      workdirTrusted = workspaceIsReadyForTools(workdir);
      sessions = [
        created,
        ...sessions.filter((item) => item.id !== created.id),
      ];
      cacheSessionMessagePage(
        created.id,
        createSessionMessagePageState([], 0, false),
      );
      try {
        await upsertProject(created.cwd, created.id);
      } catch (error) {
        console.warn(
          "[NovaVei projects] could not update the last session",
          error,
        );
      }
      if (!isNavigationCurrent(navigation)) return;
      renderSessionNavigation();
      clearTranscript();
      const title = node<HTMLElement>("chatTitle");
      if (title) title.textContent = created.title || "新建对话";
      updateWorkdirChrome(workdir);
      setLoadedSessionWorkspaceState();
      if (workdir && pathKey(workdir) !== previousWorkdirKey)
        notifyWorkdirChanged(workdir);
      notifySessionChanged(created.id);
      publishCurrentProjectPreferences();
      node<HTMLTextAreaElement>("composerInput")?.focus();
    } catch (error) {
      if (!isNavigationCurrent(navigation)) return;
      if (previousShellState !== "ready" && !sessionId) presentNoSessionState();
      throw error;
    }
  };

  const createNewChat = async () => {
    const requestedCwd =
      workdir && workdirTrusted
        ? workdir
        : projects.find((project) => workspaceIsReadyForTools(project.path))
            ?.path;
    // There may already be an empty draft while another conversation is
    // selected. Reopen that draft instead of creating a second blank session.
    if (requestedCwd) {
      const draft = await findReusableDraftSession(requestedCwd);
      if (draft) {
        if (draft.id !== sessionId) await loadSession(draft.id);
        node<HTMLTextAreaElement>("composerInput")?.focus();
        return;
      }
    }
    if (requestedCwd) {
      await createSession(requestedCwd);
    } else {
      await pickWorkspace();
    }
  };

  const openNewChat = () => {
    if (newChatOperation) return newChatOperation;
    const operation = createNewChat();
    newChatOperation = operation;
    void operation.then(
      () => {
        if (newChatOperation === operation) newChatOperation = undefined;
      },
      () => {
        if (newChatOperation === operation) newChatOperation = undefined;
      },
    );
    return operation;
  };

  const pickWorkspace = async () => {
    const selected = await invoke<string | null>("workspace_pick", {
      startDir: workdirTrusted ? workdir : undefined,
    });
    if (!selected?.trim()) return;
    const nextWorkdir = displayPath(selected.trim());
    const previousWorkdir = workdir;
    updateWorkdirChrome(nextWorkdir);
    try {
      await upsertProject(nextWorkdir);
      // The picker proves the directory existed at selection time, but the
      // status map is still authoritative for the visible project state. Do
      // not send/create against a root whose follow-up probe is unavailable.
      await refreshWorkspacePathStatuses();
      if (!workspaceIsReadyForTools(nextWorkdir)) {
        throw new Error("无法确认所选项目路径，请稍后重试");
      }
      const existing = sessions.find(
        (item) => pathKey(item.cwd) === pathKey(nextWorkdir),
      );
      await (existing ? loadSession(existing.id) : createSession(nextWorkdir));
    } catch (error) {
      updateWorkdirChrome(previousWorkdir);
      throw error;
    }
  };

  const workspaceStatusFor = (targetWorkdir: string) =>
    workspaceStatusForWorkdir(
      targetWorkdir,
      sessions.filter(
        (session) => pathKey(session.cwd) === pathKey(targetWorkdir),
      ),
      workspaceStatuses,
    );

  const setWorkspaceActionBusy = (
    attribute:
      | "data-novavei-register-workspace"
      | "data-novavei-relocate-workspace"
      | "data-novavei-refresh-workspace-status",
    targetWorkdir: string,
    isBusy: boolean,
  ) => {
    const targetKey = pathKey(targetWorkdir);
    document
      .querySelectorAll<HTMLButtonElement>(`[${attribute}]`)
      .forEach((button) => {
        if (pathKey(button.getAttribute(attribute)) !== targetKey) return;
        button.disabled = isBusy;
        button.setAttribute("aria-busy", String(isBusy));
      });
  };

  const registerWorkspaceAsProject = async (targetWorkdir: string) => {
    // Registration is the recovery action for an otherwise read-only
    // historical workspace, so it is deliberately allowed from
    // `needs_workspace`. Native code still verifies that this is an existing
    // history path (or a picker-approved path) before persisting anything.
    if (nativeShellState === "storage_recovery")
      throw new StorageRecoveryRequiredError();
    if (nativeShellState === "loading" || nativeShellState === "error")
      throw new Error("本地项目与会话尚未准备完成");
    const cleanedWorkdir = displayPath(targetWorkdir);
    if (!cleanedWorkdir) return;
    const status = workspaceStatusFor(cleanedWorkdir);
    if (isRelocationRequired(status)) {
      toast("原路径不可用，请先重新选择目录后再登记项目");
      return;
    }
    if (status === "unavailable") {
      toast("暂时无法确认该路径，请稍后刷新或重新选择目录");
      return;
    }
    setWorkspaceActionBusy(
      "data-novavei-register-workspace",
      cleanedWorkdir,
      true,
    );
    try {
      const response = await invoke<unknown>("workspace_register_project", {
        workdir: cleanedWorkdir,
        name: pathName(cleanedWorkdir),
      });
      const created = record(response)?.created === true;
      await refreshSessions();
      workspaceFolderForWorkdir(cleanedWorkdir)
        ?.querySelector<HTMLElement>(".project-row[data-workdir]")
        ?.focus({ preventScroll: true });
      toast(created ? "已添加为项目" : "该工作空间已登记为项目");
    } finally {
      setWorkspaceActionBusy(
        "data-novavei-register-workspace",
        cleanedWorkdir,
        false,
      );
    }
  };

  const relocateWorkspace = async (targetWorkdir: string) => {
    requireReadyForWorkspaceRelocation();
    const fromWorkdir = displayPath(targetWorkdir);
    if (!fromWorkdir) return;
    // The native picker binds its result to this exact historical source for
    // one relocation. Passing only the selected target back later is not a
    // sufficient authority to rewrite durable workspace metadata.
    const selected = await invoke<string | null>("workspace_pick", {
      relocationFrom: fromWorkdir,
    });
    if (!selected?.trim()) return;
    const toWorkdir = displayPath(selected.trim());
    if (pathKey(fromWorkdir) === pathKey(toWorkdir)) {
      const refreshed = await refreshWorkspacePathStatuses();
      renderSessionNavigation();
      setLoadedSessionWorkspaceState();
      const recovered = workspaceStatusFor(fromWorkdir) === "available";
      toast(
        recovered
          ? "原项目路径已恢复可用"
          : refreshed
            ? "选择的目录与原路径相同，路径仍不可用"
            : "暂时无法重新检查原项目路径，请稍后重试",
      );
      return;
    }
    setWorkspaceActionBusy(
      "data-novavei-relocate-workspace",
      fromWorkdir,
      true,
    );
    try {
      const invokeRelocation = async (
        conflictResolution?: WorkspaceRelocationConflictResolution,
        conflictToken?: string,
      ) => {
        const response = await invoke<unknown>("sessions_relocate_workspace", {
          fromWorkdir,
          toWorkdir,
          ...(conflictResolution && conflictToken
            ? { conflictResolution, conflictToken }
            : {}),
        });
        const result = workspaceRelocationResponse(response);
        if (
          !result ||
          pathKey(result.fromWorkdir) !== pathKey(fromWorkdir) ||
          pathKey(result.toWorkdir) !== pathKey(toWorkdir)
        ) {
          throw new Error("工作空间迁移响应无效，未应用任何界面更新");
        }
        return result;
      };

      let result = await invokeRelocation();
      if (result.status === "conflict") {
        const conflictToken = result.conflictToken;
        let conflictResolved = false;
        try {
          const resolution = await requestWorkspaceRelocationResolution(
            result.conflict,
          );
          if (!resolution) return;
          result = await invokeRelocation(resolution, conflictToken);
          if (result.status !== "relocated") {
            throw new Error("项目资料在确认期间发生变化，请重新选择目录");
          }
          conflictResolved = true;
        } finally {
          // A choice-dialog failure should have the same authority outcome as
          // an explicit cancellation. The native command is idempotent, so it
          // is also safe after a failed or already-consumed confirmation.
          if (!conflictResolved) {
            await invoke<void>("sessions_relocate_workspace_cancel", {
              conflictToken,
            }).catch((error) =>
              console.warn(
                "[NovaVei workspaces] could not revoke unresolved relocation confirmation",
                error,
              ),
            );
          }
        }
      }

      const updatedCount = result.updatedSessionIds.length;
      await refreshSessions();
      toast(
        updatedCount > 0
          ? `已重新绑定 ${updatedCount} 个历史会话`
          : "已重新绑定工作空间路径",
      );
    } finally {
      setWorkspaceActionBusy(
        "data-novavei-relocate-workspace",
        fromWorkdir,
        false,
      );
    }
  };

  function startNativeHydration() {
    presentOpeningState("loading");
    showSessionLoadNotice("loading");
    void refreshSessions().catch((error) => {
      console.warn("[NovaVei native shell] session hydration failed", error);
      const state =
        error instanceof StorageRecoveryRequiredError
          ? "storage_recovery"
          : "error";
      // `refreshSessions` has already published and presented recovery for a
      // blocked or unreadable health result. Avoid replacing that exact
      // projection with a second generic event while startup unwinds.
      if (state === "storage_recovery") {
        if (nativeShellState !== "storage_recovery")
          enterStorageRecovery(normaliseAppHealth(undefined));
        return;
      }
      showSessionLoadNotice(state);
      presentOpeningState(state);
    });
  }

  const removeProject = async (targetWorkdir: string) => {
    const targetKey = pathKey(targetWorkdir);
    const target = projects.find(
      (project) => pathKey(project.path) === targetKey,
    );
    if (!target) return { removed: false, wasCurrent: false };
    const wasCurrent = pathKey(workdir) === targetKey;
    await mutateProjects((current) =>
      current.filter((project) => project.id !== target.id),
    );
    if (!wasCurrent) {
      renderSessionNavigation();
      return { removed: true, wasCurrent: false };
    }

    // Persisting the removal means the old cwd is now historical metadata,
    // not a usable project root. Revoke renderer-side readiness before an
    // asynchronous fallback session is selected so File Dock, composer, and
    // programmatic capability requests cannot use the just-unregistered root
    // during that small transition window.
    workdirTrusted = false;
    workspaceCapability = undefined;
    setNativeShellState("needs_workspace");
    const nextProject = projects.find((project) =>
      workspaceIsReadyForTools(project.path),
    );
    if (nextProject) {
      const nextSession =
        nextProject.lastSessionId &&
        sessions.some((session) => session.id === nextProject.lastSessionId)
          ? nextProject.lastSessionId
          : sessions.find(
              (session) => pathKey(session.cwd) === pathKey(nextProject.path),
            )?.id;
      if (nextSession) {
        try {
          await loadSession(nextSession);
          return { removed: true, wasCurrent: true };
        } catch (error) {
          // The project removal is already durable. A failed fallback read must
          // not turn that successful mutation into a false “list unchanged”
          // error; leave the user in a coherent no-session state instead.
          console.warn(
            "[NovaVei projects] removed project but could not open fallback session",
            error,
          );
        }
      } else {
        try {
          await createSession(nextProject.path);
          return { removed: true, wasCurrent: true };
        } catch (error) {
          console.warn(
            "[NovaVei projects] removed project but could not create fallback session",
            error,
          );
        }
      }
    }
    beginSessionNavigation();
    sessionId = undefined;
    workspaceCapability = undefined;
    workdir = undefined;
    workdirTrusted = false;
    updateWorkdirChrome(workdir);
    renderSessionNavigation();
    presentNoSessionState();
    publishCurrentProjectPreferences();
    return { removed: true, wasCurrent: true };
  };

  const exportDiagnostics = async () =>
    invoke<DiagnosticsExportResponse | null>("diagnostics_export").then(
      (result) => result ?? undefined,
    );

  const api: NativeShellApi = {
    getSessionId: () => sessionId,
    getWorkdir: () => workdir,
    getCurrentProjectPreferences: () => currentProjectForWorkdir()?.preferences,
    saveCurrentProjectPreferences,
    upsertLiveTranscriptMessage,
    selectSession: loadSession,
    createSession,
    refreshSessions,
    getSessions: () => sessions.slice(),
    onSessionsChanged: (listener) => {
      sessionListeners.add(listener);
      listener(sessions.slice());
      return () => {
        sessionListeners.delete(listener);
      };
    },
    exportDiagnostics,
    issueWorkspaceCapability: async () => {
      requireReadyForDesktopCapability();
      const requestedWorkdir = workdir?.trim();
      if (!requestedWorkdir) throw new Error("工作目录不可用");
      // `ready` alone is not a durable permission grant: a project can be
      // removed while an async navigation is choosing its fallback. Keep the
      // renderer fail-closed unless this exact cwd is still registered and
      // its native status probe says it is usable.
      if (!workspaceIsReadyForTools(requestedWorkdir))
        throw new Error("当前工作区尚未登记为可用项目");
      if (!sessionId) throw new Error("工作区权限需要先创建会话");
      if (
        workspaceCapability?.workdir === requestedWorkdir &&
        workspaceCapability.sessionId === sessionId
      ) {
        // Keep cached capabilities behind the same current-state gate as a
        // newly issued token. A caller must never receive one while recovery
        // has stopped durable operations.
        requireReadyForDesktopCapability();
        return workspaceCapability;
      }
      const issued = await invoke<{
        capabilityToken?: string;
        capability_token?: string;
        workdir?: string;
        sessionId?: string;
        session_id?: string;
      }>("workspace_capability_issue", {
        workdir: requestedWorkdir,
        sessionId,
        session_id: sessionId,
      });
      // Do not expose a capability that completed after the shell left its
      // explicitly-ready state.
      requireReadyForDesktopCapability();
      const capabilityToken =
        issued?.capabilityToken || issued?.capability_token;
      if (!capabilityToken) throw new Error("无法取得工作区权限");
      workspaceCapability = {
        capabilityToken,
        workdir: issued.workdir?.trim() || requestedWorkdir,
        sessionId: issued.sessionId || issued.session_id || sessionId,
      };
      return workspaceCapability;
    },
    branchSession: async (id: string, title?: string) => {
      const source = sessions.find((session) => session.id === id);
      if (!source) throw new Error("找不到要分叉的本地会话");
      if (!workspaceIsReadyForTools(source.cwd))
        throw new Error("只读历史不能创建分支；请先登记并恢复项目路径");
      const response = await invoke<unknown>("chat_history_branch", {
        id,
        title: title?.trim() || undefined,
      });
      const branch = sessionRecord(response);
      if (!branch) throw new Error("分叉会话响应无效");
      sessions = [branch, ...sessions.filter((item) => item.id !== branch.id)];
      return branch;
    },
    removeProject,
    getProjectConversationSelection: () => {
      const selection = activeProjectConversationSelection();
      if (!selection) return undefined;
      return {
        workdir: selection.workdir,
        sessionIds: sessions
          .filter(
            (session) =>
              pathKey(session.cwd) === pathKey(selection.workdir) &&
              selection.selectedIds.has(session.id),
          )
          .map((session) => session.id),
      };
    },
    clearProjectConversationSelection,
    setProjectConversationSelectionBusy: (isBusy: boolean) => {
      if (!activeProjectConversationSelection()) return;
      isProjectConversationSelectionBusy = isBusy;
      renderSessionNavigation();
    },
    restoreProjectConversationSelectionFocus: () => {
      const activeRow = sessionId
        ? document.querySelector<HTMLButtonElement>(
            `.session[data-session-id="${CSS.escape(sessionId)}"]`,
          )
        : null;
      (
        activeRow || node<HTMLButtonElement>("btnSelectProjectConversations")
      )?.focus();
    },
  };
  window.__novaveiHost = api;

  const diagnosticsExportButton = node<HTMLButtonElement>(
    "btnExportDiagnostics",
  );
  diagnosticsExportButton?.addEventListener("click", () => {
    diagnosticsExportButton.disabled = true;
    diagnosticsExportButton.textContent = "正在导出…";
    void exportDiagnostics()
      .then((result) => {
        toast(result ? "诊断包已导出" : "已取消诊断包导出");
      })
      .catch(() => {
        toast("诊断包导出失败，请检查本地文件访问权限");
      })
      .finally(() => {
        if (!diagnosticsExportButton.isConnected) return;
        diagnosticsExportButton.disabled = false;
        diagnosticsExportButton.textContent = "导出诊断包";
      });
  });

  document.addEventListener(
    "click",
    (event) => {
      const target = event.target instanceof Element ? event.target : null;
      const opening = target?.closest<HTMLButtonElement>(
        "[data-novavei-opening-action]",
      );
      if (opening) {
        event.preventDefault();
        event.stopImmediatePropagation();
        switch (opening.dataset.novaveiOpeningAction) {
          case "open-workspace":
            void pickWorkspace().catch((error) => toast(String(error)));
            return;
          case "create-session": {
            const requestedWorkdir = opening.dataset.novaveiWorkdir;
            const project = requestedWorkdir
              ? projects.find(
                  (entry) => pathKey(entry.path) === pathKey(requestedWorkdir),
                )
              : undefined;
            if (!project) {
              presentNoSessionState();
              return;
            }
            void createSession(project.path).catch((error) =>
              toast(String(error)),
            );
            return;
          }
          case "retry-hydration":
            startNativeHydration();
            return;
          default:
            return;
        }
      }
      if (target?.closest("#btnSelectProjectConversations")) {
        event.preventDefault();
        event.stopImmediatePropagation();
        toggleProjectConversationSelection();
        return;
      }
      if (target?.closest("#btnSelectAllProjectConversations")) {
        event.preventDefault();
        event.stopImmediatePropagation();
        toggleSelectAllProjectConversations();
        return;
      }
      if (target?.closest("#btnCancelProjectConversationSelection")) {
        event.preventDefault();
        event.stopImmediatePropagation();
        clearProjectConversationSelection();
        node<HTMLButtonElement>("btnSelectProjectConversations")?.focus();
        return;
      }
      const registerWorkspace = target?.closest<HTMLButtonElement>(
        "[data-novavei-register-workspace]",
      );
      if (registerWorkspace?.dataset.novaveiRegisterWorkspace) {
        event.preventDefault();
        event.stopImmediatePropagation();
        void registerWorkspaceAsProject(
          registerWorkspace.dataset.novaveiRegisterWorkspace,
        ).catch((error) => toast(String(error)));
        return;
      }
      const refreshWorkspaceStatusAction = target?.closest<HTMLButtonElement>(
        "[data-novavei-refresh-workspace-status]",
      );
      if (refreshWorkspaceStatusAction?.dataset.novaveiRefreshWorkspaceStatus) {
        event.preventDefault();
        event.stopImmediatePropagation();
        const targetWorkdir =
          refreshWorkspaceStatusAction.dataset.novaveiRefreshWorkspaceStatus;
        setWorkspaceActionBusy(
          "data-novavei-refresh-workspace-status",
          targetWorkdir,
          true,
        );
        void (async () => {
          try {
            const refreshed = await refreshWorkspacePathStatuses();
            renderSessionNavigation();
            setLoadedSessionWorkspaceState();
            const recovered = workspaceStatusFor(targetWorkdir) === "available";
            if (!refreshed && !recovered) {
              throw new Error("暂时无法重新检查项目路径，请稍后重试");
            }
            toast(recovered ? "项目路径已恢复可用" : "已重新检查项目路径状态");
          } finally {
            setWorkspaceActionBusy(
              "data-novavei-refresh-workspace-status",
              targetWorkdir,
              false,
            );
          }
        })().catch((error) => toast(String(error)));
        return;
      }
      const relocateWorkspaceAction = target?.closest<HTMLButtonElement>(
        "[data-novavei-relocate-workspace]",
      );
      if (relocateWorkspaceAction?.dataset.novaveiRelocateWorkspace) {
        event.preventDefault();
        event.stopImmediatePropagation();
        void relocateWorkspace(
          relocateWorkspaceAction.dataset.novaveiRelocateWorkspace,
        ).catch((error) => toast(String(error)));
        return;
      }
      if (target?.closest(".folder-add")) {
        event.preventDefault();
        event.stopImmediatePropagation();
        void pickWorkspace().catch((error) => toast(String(error)));
        return;
      }
      const project = target?.closest<HTMLElement>(
        ".project-row[data-workdir]",
      );
      if (project?.dataset.workdir?.trim()) {
        event.preventDefault();
        event.stopImmediatePropagation();
        const nextWorkdir = project.dataset.workdir.trim();
        const previousWorkdir = workdir;
        const changedProject = pathKey(nextWorkdir) !== pathKey(workdir);
        if (!changedProject) {
          // Clicking the active project toggles its conversation list.
          const nextExpanded = project.getAttribute("aria-expanded") !== "true";
          setProjectFolderExpanded(project, nextExpanded);
          return;
        }
        const workspaceStatus = workspaceLocationStatus(
          project.dataset.workspaceStatus,
        );
        const existing = sessions.find(
          (item) => pathKey(item.cwd) === pathKey(nextWorkdir),
        );
        setProjectCurrent(project, nextWorkdir);
        updateWorkdirChrome(nextWorkdir);
        setProjectFolderExpanded(project, true);
        // Selection is deliberately scoped to one current project; do not
        // carry selected ids into a project change while its session loads.
        if (activeProjectConversationSelection())
          clearProjectConversationSelection();

        // Historical sessions remain readable even after their folder moves,
        // disappears, or is temporarily unavailable. Only the no-session
        // branch below could create a new durable/workspace-bound session.
        if (existing && workspaceStatus !== "available") {
          void loadSession(existing.id).catch((error) => {
            updateWorkdirChrome(previousWorkdir);
            renderSessionNavigation();
            toast(String(error));
          });
          return;
        }
        if (!existing && isRelocationRequired(workspaceStatus)) {
          updateWorkdirChrome(previousWorkdir);
          renderSessionNavigation();
          toast("该项目原路径不可用，请先重新选择目录");
          return;
        }
        if (!existing && workspaceStatus === "unavailable") {
          updateWorkdirChrome(previousWorkdir);
          renderSessionNavigation();
          toast("当前无法确认项目路径；历史会话仍可只读打开");
          return;
        }
        if (!existing && !isRegisteredWorkspace(nextWorkdir)) {
          updateWorkdirChrome(previousWorkdir);
          renderSessionNavigation();
          toast("该工作空间尚未登记为项目，请先添加为项目");
          return;
        }
        void (
          existing ? loadSession(existing.id) : createSession(nextWorkdir)
        ).catch((error) => {
          updateWorkdirChrome(previousWorkdir);
          renderSessionNavigation();
          toast(String(error));
        });
        return;
      }
      const session = target?.closest<HTMLElement>(
        ".session[data-novavei-session]",
      );
      if (session?.dataset.sessionId) {
        event.preventDefault();
        event.stopImmediatePropagation();
        if (activeProjectConversationSelection()) {
          toggleSelectedProjectConversation(
            session.dataset.sessionId,
            session.dataset.workdir || "",
          );
          return;
        }
        void loadSession(session.dataset.sessionId).catch((error) =>
          toast(String(error)),
        );
        return;
      }
      if (target?.closest("#btnNewChat")) {
        event.preventDefault();
        event.stopImmediatePropagation();
        void openNewChat().catch((error) => toast(String(error)));
      }
    },
    true,
  );

  document.addEventListener(
    "keydown",
    (event) => {
      if (event.key !== "Escape" || !activeProjectConversationSelection())
        return;
      if (document.querySelector("dialog[open]")) return;
      event.preventDefault();
      clearProjectConversationSelection();
      node<HTMLButtonElement>("btnSelectProjectConversations")?.focus();
    },
    true,
  );

  clearPrototypeSessionSurface();
  startNativeHydration();
  return api;
}
