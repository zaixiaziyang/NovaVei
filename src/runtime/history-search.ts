import { installConversationFind } from "./conversation-find";

import { pathKey } from "./path-display";

type Invoke = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

type HistorySearchMatch = {
  conversationId: string;
  conversationTitle: string;
  messageId: string;
  role: string;
  text: string;
};

type HistorySearchResponse = {
  matches?: unknown;
};

/**
 * Native contract for session_metadata_search.
 *
 * The response must be an object with a matches array. A session match carries
 * a conversation id/title and may also carry its project name, path, and
 * model. A project match carries a project name/path and may optionally point
 * at one of that project's conversations. The renderer treats this as
 * untrusted IPC data and deliberately degrades to the currently loaded local
 * list if the command is unavailable or returns an invalid payload.
 */
type SessionMetadataMatch = {
  kind: "session" | "project";
  conversationId?: string;
  conversationTitle?: string;
  projectName?: string;
  workdir?: string;
  model?: string;
  registered: boolean;
  workspaceStatus?: WorkspacePathAccessibility;
};

/**
 * A small, renderer-safe projection of the native workspace probe.  It is
 * deliberately separate from the shell's richer state: search results only
 * need to explain whether reopening the historical record is read-only.
 */
type WorkspacePathStatus =
  | "available"
  | "missing"
  | "not_directory"
  | "unavailable";

type WorkspacePathAccessibility = {
  status: WorkspacePathStatus;
  accessible: boolean;
};

type SessionMetadataSearchResponse = {
  matches?: unknown;
};

type MetadataSearchState = {
  pending: boolean;
  matches: SessionMetadataMatch[];
  failure?: string;
  fallback: boolean;
};

type ContentSearchState = {
  pending: boolean;
  matches: HistorySearchMatch[];
  failure?: string;
};

const SEARCH_DEBOUNCE_MS = 180;
const MAX_QUERY_LENGTH = 256;
const MAX_RESULTS = 12;
const MAX_PREVIEW_LENGTH = 180;
const MAX_METADATA_TITLE_LENGTH = 256;
const MAX_METADATA_PATH_LENGTH = 1024;
const HISTORY_REVEAL_WAIT_MS = 4_500;

function invokeApi(): Invoke | undefined {
  return window.__TAURI__?.core?.invoke as Invoke | undefined;
}

function element<T extends HTMLElement>(id: string): T | null {
  return document.getElementById(id) as T | null;
}

function nativeSessionId(value: unknown): value is string {
  return typeof value === "string" && /^[A-Za-z0-9_-]{1,128}$/.test(value);
}

function nativeMessageId(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 256 &&
    !/[\u0000-\u001F\u007F]/.test(value)
  );
}

function text(value: unknown, fallback = ""): string {
  return typeof value === "string"
    ? value.replace(/\s+/g, " ").trim() || fallback
    : fallback;
}

function boundedText(value: unknown, maxLength: number, fallback = ""): string {
  const normalized = text(value);
  if (!normalized) return fallback;
  const characters = Array.from(normalized);
  return characters.length > maxLength
    ? characters.slice(0, maxLength).join("")
    : normalized;
}

function errorText(value: unknown): string {
  try {
    return boundedText(
      value instanceof Error ? value.message : String(value),
      MAX_PREVIEW_LENGTH,
      "未说明原因",
    );
  } catch {
    return "未说明原因";
  }
}

function preview(value: string): string {
  const characters = Array.from(value);
  return characters.length > MAX_PREVIEW_LENGTH
    ? `${characters
        .slice(0, MAX_PREVIEW_LENGTH - 1)
        .join("")
        .trimEnd()}…`
    : value;
}

function roleLabel(role: string): string {
  if (role === "user") return "用户消息";
  if (role === "assistant") return "Agent 回复";
  return "会话记录";
}

function workspaceKey(value: string | undefined): string {
  return pathKey(value);
}

function workspaceName(value: string | undefined): string {
  const normalized = text(value).replace(/[\\/]+$/, "");
  if (!normalized) return "未命名项目";
  const segments = normalized.split(/[\\/]/);
  return segments[segments.length - 1] || normalized;
}

/**
 * The sidebar's legacy text filter does not normalize path separators.  The
 * renderer fallback must, otherwise `E:/work/app` cannot find the same local
 * record stored as `E:\\work\\app` when native metadata search is unavailable.
 */
function localMetadataPathSearchText(value: string | undefined): string {
  return text(value)
    .replace(/[\\/]+/g, "\\")
    .toLocaleLowerCase();
}

function workspacePathStatus(value: unknown): WorkspacePathStatus | undefined {
  const normalized = text(value).replace(/[\s-]/g, "_").toLocaleLowerCase();
  switch (normalized) {
    case "available":
    case "missing":
    case "not_directory":
    case "unavailable":
      return normalized;
    case "notdirectory":
      // Rust's `rename_all = "camelCase"` serializes this reason as
      // `notDirectory`; keep accepting the shell-style snake case as well.
      return "not_directory";
    default:
      return undefined;
  }
}

function workspaceAccessibilityFromValue(
  value: unknown,
): WorkspacePathAccessibility | undefined {
  if (typeof value === "string") {
    const status = workspacePathStatus(value);
    return status ? { status, accessible: status === "available" } : undefined;
  }
  if (!value || typeof value !== "object" || Array.isArray(value))
    return undefined;
  const record = value as Record<string, unknown>;
  const status = workspacePathStatus(
    record.workspaceStatus ?? record.workspace_status ?? record.status,
  );
  const reason = workspacePathStatus(record.reason);

  if (record.accessible === true)
    return { status: "available", accessible: true };
  if (record.accessible === false)
    return {
      status:
        status && status !== "available"
          ? status
          : reason && reason !== "available"
            ? reason
            : "unavailable",
      accessible: false,
    };
  if (status) return { status, accessible: status === "available" };
  if (reason) return { status: reason, accessible: reason === "available" };
  return undefined;
}

/**
 * Native `session_metadata_search` returns the status as a nested DTO:
 * `{ workspaceStatus: { path, accessible, reason? } }`.  The extra aliases
 * make a stale desktop binary degrade safely, but the nested object always
 * takes precedence over a top-level field.
 */
function metadataWorkspaceAccessibility(
  record: Record<string, unknown>,
): WorkspacePathAccessibility | undefined {
  const nested = workspaceAccessibilityFromValue(
    record.workspaceStatus ?? record.workspace_status ?? record.workspace,
  );
  if (nested) return nested;
  return workspaceAccessibilityFromValue({
    status: record.status,
    accessible:
      record.workspaceAccessible ??
      record.workspace_accessible ??
      record.accessible,
    reason: record.workspaceReason ?? record.workspace_reason ?? record.reason,
  });
}

function searchMatches(value: unknown): HistorySearchMatch[] {
  const response =
    value !== null && typeof value === "object"
      ? (value as HistorySearchResponse)
      : undefined;
  const records: unknown[] = Array.isArray(response?.matches)
    ? response.matches
    : [];
  const seen = new Set<string>();
  const matches: HistorySearchMatch[] = [];
  for (const entry of records) {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) continue;
    const record = entry as Record<string, unknown>;
    const conversationId = record.conversationId;
    const messageId = record.messageId;
    const body = text(record.text);
    const role = text(record.role).toLowerCase();
    if (
      !nativeSessionId(conversationId) ||
      !nativeMessageId(messageId) ||
      !body ||
      (role !== "user" && role !== "assistant")
    )
      continue;
    const title = text(record.conversationTitle, "未命名对话");
    const key = `${conversationId}\u0000${messageId}`;
    if (seen.has(key)) continue;
    seen.add(key);
    matches.push({
      conversationId,
      conversationTitle: title,
      messageId,
      role,
      text: body,
    });
    if (matches.length >= MAX_RESULTS) break;
  }
  return matches;
}

function metadataMatches(value: unknown): {
  valid: boolean;
  matches: SessionMetadataMatch[];
} {
  const response =
    value !== null && typeof value === "object"
      ? (value as SessionMetadataSearchResponse)
      : undefined;
  if (!Array.isArray(response?.matches)) return { valid: false, matches: [] };

  const seen = new Set<string>();
  const matches: SessionMetadataMatch[] = [];
  for (const entry of response.matches) {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) continue;
    const record = entry as Record<string, unknown>;
    const rawConversationId =
      record.conversationId ??
      record.conversation_id ??
      record.sessionId ??
      record.session_id ??
      record.id;
    const conversationId = boundedText(rawConversationId, 128);
    if (conversationId && !nativeSessionId(conversationId)) continue;
    const conversationTitle = boundedText(
      record.conversationTitle ?? record.conversation_title ?? record.title,
      MAX_METADATA_TITLE_LENGTH,
    );
    const projectName = boundedText(
      record.projectName ?? record.project_name ?? record.name,
      MAX_METADATA_TITLE_LENGTH,
    );
    const workdir = boundedText(
      record.workdir ?? record.cwd ?? record.path,
      MAX_METADATA_PATH_LENGTH,
    );
    const model = boundedText(
      record.modelLabel ??
        record.model_label ??
        record.model ??
        record.modelId ??
        record.model_id,
      MAX_METADATA_TITLE_LENGTH,
    );
    const workspaceStatus = metadataWorkspaceAccessibility(record);
    const rawKind = text(record.kind ?? record.type).toLowerCase();
    const kind: SessionMetadataMatch["kind"] =
      rawKind === "project"
        ? "project"
        : rawKind === "session"
          ? "session"
          : conversationId
            ? "session"
            : "project";
    const registered =
      typeof record.registered === "boolean"
        ? record.registered
        : kind === "project" || Boolean(projectName);

    if (kind === "session" && !conversationId) continue;
    if (kind === "project" && !workdir && !conversationId) continue;
    if (!conversationTitle && !projectName && !workdir && !model) continue;

    const key =
      kind === "session"
        ? `session\u0000${conversationId}`
        : `project\u0000${workspaceKey(workdir)}\u0000${projectName.toLocaleLowerCase()}`;
    if (seen.has(key)) continue;
    seen.add(key);
    matches.push({
      kind,
      ...(conversationId ? { conversationId } : {}),
      ...(conversationTitle ? { conversationTitle } : {}),
      ...(projectName ? { projectName } : {}),
      ...(workdir ? { workdir } : {}),
      ...(model ? { model } : {}),
      registered,
      ...(workspaceStatus ? { workspaceStatus } : {}),
    });
    if (matches.length >= MAX_RESULTS) break;
  }
  return { valid: true, matches };
}

function metadataMatchesQuery(
  match: SessionMetadataMatch,
  query: string,
): boolean {
  const normalizedQuery = text(query).toLocaleLowerCase();
  if (!normalizedQuery) return false;
  return (
    [match.projectName, match.conversationTitle, match.model].some(
      (candidate) =>
        text(candidate).toLocaleLowerCase().includes(normalizedQuery),
    ) ||
    localMetadataPathSearchText(match.workdir).includes(
      localMetadataPathSearchText(query),
    )
  );
}

function projectRowName(row: HTMLElement): string {
  return (
    boundedText(row.dataset.project, MAX_METADATA_TITLE_LENGTH) ||
    boundedText(
      row.querySelector<HTMLElement>("strong, .project-name")?.textContent,
      MAX_METADATA_TITLE_LENGTH,
    ) ||
    workspaceName(row.dataset.workdir)
  );
}

function projectRows(): HTMLElement[] {
  return [
    ...document.querySelectorAll<HTMLElement>(".project-row[data-workdir]"),
  ].filter((row) => row.dataset.novaveiProject === "true");
}

/**
 * The renderer-only fallback is intentionally limited to the current Native
 * Shell snapshot and rendered project rows. It cannot infer durable model or
 * unloaded project data, so callers must pair its results with an explicit
 * degradation notice. Rows hidden by the legacy text filter remain eligible:
 * its raw text comparison cannot recognize slash/backslash path aliases.
 */
function localMetadataMatches(
  query: string,
  host: NonNullable<Window["__novaveiHost"]>,
): SessionMetadataMatch[] {
  const rows = projectRows();
  const projectNames = new Map<string, string>();
  const projectWorkspaceStatuses = new Map<
    string,
    WorkspacePathAccessibility
  >();
  for (const row of rows) {
    const workdir = boundedText(row.dataset.workdir, MAX_METADATA_PATH_LENGTH);
    const key = workspaceKey(workdir);
    if (key) projectNames.set(key, projectRowName(row));
    const workspaceStatus = workspaceAccessibilityFromValue(
      row.dataset.workspaceStatus,
    );
    if (key && workspaceStatus)
      projectWorkspaceStatuses.set(key, workspaceStatus);
  }

  const seen = new Set<string>();
  const matches: SessionMetadataMatch[] = [];
  const add = (match: SessionMetadataMatch) => {
    if (!metadataMatchesQuery(match, query) || matches.length >= MAX_RESULTS)
      return;
    const key =
      match.kind === "session"
        ? `session\u0000${match.conversationId || ""}`
        : `project\u0000${workspaceKey(match.workdir)}`;
    if (seen.has(key)) return;
    seen.add(key);
    matches.push(match);
  };

  try {
    for (const session of host.getSessions()) {
      if (!nativeSessionId(session.id)) continue;
      const workdir = boundedText(session.cwd, MAX_METADATA_PATH_LENGTH);
      const title = boundedText(
        session.title,
        MAX_METADATA_TITLE_LENGTH,
        "未命名对话",
      );
      const key = workspaceKey(workdir);
      const projectName = projectNames.get(key);
      const workspaceStatus =
        workspaceAccessibilityFromValue(session.workspaceStatus) ??
        projectWorkspaceStatuses.get(key);
      add({
        kind: "session",
        conversationId: session.id,
        conversationTitle: title,
        ...(workdir ? { workdir } : {}),
        ...(projectName ? { projectName } : {}),
        registered: Boolean(projectName),
        ...(workspaceStatus ? { workspaceStatus } : {}),
      });
    }
  } catch {
    // A partially initialized host must not make the palette unusable.
  }

  for (const row of rows) {
    const workdir = boundedText(row.dataset.workdir, MAX_METADATA_PATH_LENGTH);
    if (!workdir) continue;
    const workspaceStatus = workspaceAccessibilityFromValue(
      row.dataset.workspaceStatus,
    );
    add({
      kind: "project",
      projectName: projectRowName(row),
      workdir,
      registered: true,
      ...(workspaceStatus ? { workspaceStatus } : {}),
    });
  }
  return matches;
}

function closePalette(
  palette: HTMLElement,
  input: HTMLInputElement,
  focusTarget?: HTMLElement,
) {
  palette.classList.remove("show");
  palette.hidden = true;
  input.value = "";
  input.dispatchEvent(new Event("input", { bubbles: true }));
  (focusTarget ?? element<HTMLTextAreaElement>("composerInput"))?.focus({
    preventScroll: true,
  });
}

function renderedMessage(messageId: string): HTMLElement | undefined {
  return [...document.querySelectorAll<HTMLElement>("[data-message-id]")].find(
    (candidate) => candidate.dataset.messageId === messageId,
  );
}

function focusMessage(message: HTMLElement) {
  message.tabIndex = -1;
  message.scrollIntoView({
    behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches
      ? "auto"
      : "smooth",
    block: "center",
  });
  message.focus({ preventScroll: true });
}

function waitForTranscriptMutation(axis: HTMLElement) {
  return new Promise<boolean>((resolve) => {
    let complete = false;
    const finish = (changed: boolean) => {
      if (complete) return;
      complete = true;
      observer.disconnect();
      window.clearTimeout(timeout);
      resolve(changed);
    };
    const observer = new MutationObserver(() => finish(true));
    const timeout = window.setTimeout(
      () => finish(false),
      HISTORY_REVEAL_WAIT_MS,
    );
    observer.observe(axis, { childList: true, subtree: true });
  });
}

/**
 * Search results can point at a message outside the current virtual transcript
 * window. Walk the existing “load earlier” UI one page at a time rather than
 * silently treating a successful session open as a successful location.
 */
async function revealMessageInPagedTranscript(
  host: NonNullable<Window["__novaveiHost"]>,
  sessionId: string,
  messageId: string,
  onProgress: (page: number) => void,
  isActive: () => boolean,
) {
  const attemptedWindows = new Set<string>();
  while (isActive() && host.getSessionId?.() === sessionId) {
    const direct = renderedMessage(messageId);
    if (direct) return direct;
    const axis = element<HTMLElement>("transcriptAxis");
    if (!axis) return undefined;
    const windowKey = [
      ...axis.querySelectorAll<HTMLElement>("[data-message-id]"),
    ]
      .map((node) => node.dataset.messageId)
      .filter((id): id is string => Boolean(id))
      .join("\u001F");
    if (!windowKey || attemptedWindows.has(windowKey)) return undefined;
    attemptedWindows.add(windowKey);
    const loadEarlier = document.querySelector<HTMLButtonElement>(
      "[data-novavei-load-earlier]",
    );
    if (!loadEarlier || loadEarlier.disabled) return undefined;
    onProgress(attemptedWindows.size);
    const changed = waitForTranscriptMutation(axis);
    loadEarlier.click();
    if (!(await changed) || !isActive() || host.getSessionId?.() !== sessionId)
      return undefined;
  }
  return undefined;
}

/**
 * Enhances the existing Cmd/Ctrl+K title/project filter with local history
 * content search and native metadata search. Neither command sends data to a
 * provider or network service; opening a result always delegates to the
 * existing NativeShell session loader.
 */
export function installHistorySearch() {
  // Ctrl/Cmd+K remains global history/project search. Install the separate
  // Ctrl/Cmd+F surface first so it also works in a browser preview where the
  // native global-search API is intentionally unavailable.
  installConversationFind();
  const invoke = invokeApi();
  const host = window.__novaveiHost;
  const palette = element<HTMLElement>("searchPalette");
  const input = element<HTMLInputElement>("sessionSearch");
  const panel = palette?.querySelector<HTMLElement>(".search-palette-panel");
  const hint = panel?.querySelector<HTMLElement>(".search-palette-hint");
  if (!host || !palette || !input || !panel || !hint) return;

  const section = document.createElement("section");
  section.id = "novaveiHistorySearch";
  section.className = "novavei-history-search";
  section.hidden = true;
  section.setAttribute("aria-label", "项目、会话与本地内容搜索结果");
  section.setAttribute("aria-busy", "false");
  const status = document.createElement("p");
  status.className = "novavei-history-search-status";
  status.setAttribute("role", "status");
  status.setAttribute("aria-live", "polite");
  status.setAttribute("aria-atomic", "true");
  const results = document.createElement("div");
  results.className = "novavei-history-search-results";
  results.setAttribute("role", "list");
  section.append(status, results);
  panel.insertBefore(section, hint);

  let timer: number | undefined;
  let request = 0;
  let opening = false;
  let openingSerial = 0;
  let actionNotice:
    | {
        message: string;
        failure: boolean;
      }
    | undefined;
  let metadataState: MetadataSearchState = {
    pending: false,
    matches: [],
    fallback: false,
  };
  let contentState: ContentSearchState = {
    pending: false,
    matches: [],
  };

  const emptyMetadataState = (): MetadataSearchState => ({
    pending: false,
    matches: [],
    fallback: false,
  });
  const emptyContentState = (): ContentSearchState => ({
    pending: false,
    matches: [],
  });

  const setStatus = (message: string, failure = false) => {
    status.setAttribute("role", failure ? "alert" : "status");
    status.setAttribute("aria-live", failure ? "assertive" : "polite");
    status.textContent = message;
  };

  const metadataTitle = (match: SessionMetadataMatch) =>
    match.kind === "project"
      ? match.projectName || workspaceName(match.workdir)
      : match.conversationTitle || "未命名对话";

  const workspaceStatusLabel = (
    workspaceStatus: WorkspacePathAccessibility | undefined,
  ) => {
    if (!workspaceStatus) return "";
    if (workspaceStatus.accessible) return "路径可用";
    switch (workspaceStatus.status) {
      case "missing":
        return "路径已失效";
      case "not_directory":
        return "路径不是目录";
      case "unavailable":
        return "路径暂无法检查";
      default:
        return "路径不可用";
    }
  };

  const isReadOnlyHistory = (match: SessionMetadataMatch) =>
    match.registered === false ||
    Boolean(match.workspaceStatus && !match.workspaceStatus.accessible);

  const readOnlyHistoryDescription = (match: SessionMetadataMatch) => {
    if (!isReadOnlyHistory(match)) return "";
    switch (match.workspaceStatus?.status) {
      case "missing":
        return "原项目目录不存在或磁盘未连接；历史记录以只读方式打开。";
      case "not_directory":
        return "原项目路径不再是目录；历史记录以只读方式打开。";
      default:
        return match.registered === false
          ? "该工作空间尚未登记为项目；历史记录以只读方式打开。"
          : "原项目路径暂不可访问；历史记录以只读方式打开。";
    }
  };

  const metadataDetails = (match: SessionMetadataMatch) =>
    [
      match.workdir ? `路径：${match.workdir}` : "",
      match.kind === "session"
        ? `模型：${match.model || "未记录"}`
        : match.model
          ? `模型：${match.model}`
          : "",
      readOnlyHistoryDescription(match),
    ]
      .filter(Boolean)
      .join(" · ");

  const appendMetadataResult = (
    match: SessionMetadataMatch,
    fallback: boolean,
  ) => {
    const item = document.createElement("div");
    item.setAttribute("role", "listitem");
    const result = document.createElement("button");
    result.type = "button";
    result.className = "novavei-history-search-result";
    result.disabled = opening;
    const title = metadataTitle(match);
    const readOnly = isReadOnlyHistory(match);
    const pathStatus = workspaceStatusLabel(match.workspaceStatus);
    const readOnlyDescription = readOnlyHistoryDescription(match);
    const action =
      match.kind === "project"
        ? readOnly
          ? "定位只读历史项目"
          : "定位项目"
        : readOnly
          ? "打开只读历史会话"
          : "打开会话";
    if (match.workspaceStatus) {
      result.dataset.workspaceStatus = match.workspaceStatus.status;
    }
    if (readOnly) result.dataset.readonlyHistory = "true";
    result.setAttribute(
      "aria-label",
      readOnly
        ? `${action}：${title}。${pathStatus}，只读历史，仍可打开。`
        : `${action}：${title}`,
    );
    const heading = document.createElement("strong");
    heading.textContent = title;
    const meta = document.createElement("span");
    meta.textContent = [
      match.kind === "project" ? "项目" : "会话",
      match.projectName && match.kind !== "project"
        ? `项目：${match.projectName}`
        : "",
      pathStatus,
      readOnly ? "只读历史（仍可打开）" : "",
      fallback ? "当前加载的本地记录" : "元数据",
    ]
      .filter(Boolean)
      .join(" · ");
    const body = document.createElement("small");
    body.textContent =
      metadataDetails(match) || "未记录路径或模型；仍可打开对应会话。";
    if (readOnlyDescription) body.title = readOnlyDescription;
    result.append(heading, meta, body);
    result.addEventListener("click", () => {
      if (opening) return;

      if (match.kind === "project" && match.workdir) {
        const workdir = workspaceKey(match.workdir);
        // The query's existing sidebar filter may have hidden this row. Find
        // it among registered projects first; closing the palette clears that
        // filter before focus is restored.
        const project = projectRows().find(
          (row) => workspaceKey(row.dataset.workdir) === workdir,
        );
        if (project) {
          opening = true;
          actionNotice = { message: "正在定位项目…", failure: false };
          render();
          closePalette(palette, input, project);
          project.scrollIntoView({
            behavior: window.matchMedia("(prefers-reduced-motion: reduce)")
              .matches
              ? "auto"
              : "smooth",
            block: "nearest",
          });
          project.focus({ preventScroll: true });
          return;
        }
        if (!match.conversationId) {
          actionNotice = {
            message:
              "无法在当前侧栏定位该项目。请刷新会话列表后重试，或先打开关联会话。",
            failure: true,
          };
          render();
          return;
        }
      }

      if (!match.conversationId) {
        actionNotice = {
          message: "该项目没有可打开的本地会话。",
          failure: true,
        };
        render();
        return;
      }
      openSession(match.conversationId, match.conversationTitle || title);
    });
    item.append(result);
    results.append(item);
  };

  const contentMatchMetadata = (
    match: HistorySearchMatch,
  ): SessionMetadataMatch | undefined => {
    const nativeMatch = metadataState.matches.find(
      (candidate) =>
        candidate.kind === "session" &&
        candidate.conversationId === match.conversationId,
    );
    if (nativeMatch) return nativeMatch;

    const session = host
      .getSessions()
      .find((candidate) => candidate.id === match.conversationId);
    if (!session) return undefined;
    const workdir = boundedText(session.cwd, MAX_METADATA_PATH_LENGTH);
    const matchingRows = [
      ...document.querySelectorAll<HTMLElement>(".project-row[data-workdir]"),
    ].filter(
      (candidate) =>
        workspaceKey(candidate.dataset.workdir) === workspaceKey(workdir),
    );
    const row =
      matchingRows.find(
        (candidate) =>
          !candidate.closest<HTMLElement>(".project-folder")?.hidden,
      ) ?? matchingRows[0];
    const registered =
      row?.dataset.novaveiWorkspaceKind === "project" ||
      row?.dataset.novaveiProject === "true";
    const workspaceStatus =
      workspaceAccessibilityFromValue(session.workspaceStatus) ??
      workspaceAccessibilityFromValue(row?.dataset.workspaceStatus);
    return {
      kind: "session",
      conversationId: match.conversationId,
      conversationTitle: match.conversationTitle,
      ...(workdir ? { workdir } : {}),
      registered,
      ...(workspaceStatus ? { workspaceStatus } : {}),
    };
  };

  const appendContentResult = (match: HistorySearchMatch) => {
    const item = document.createElement("div");
    item.setAttribute("role", "listitem");
    const result = document.createElement("button");
    result.type = "button";
    result.className = "novavei-history-search-result";
    result.disabled = opening;
    const access = contentMatchMetadata(match);
    const readOnly = access ? isReadOnlyHistory(access) : false;
    const pathStatus = workspaceStatusLabel(access?.workspaceStatus);
    if (access?.workspaceStatus) {
      result.dataset.workspaceStatus = access.workspaceStatus.status;
    }
    if (readOnly) result.dataset.readonlyHistory = "true";
    result.setAttribute(
      "aria-label",
      readOnly
        ? `打开并定位只读历史对话内容：${match.conversationTitle}。${pathStatus || "工作空间尚未登记"}，仍可打开。`
        : `打开并定位对话内容：${match.conversationTitle}`,
    );
    const title = document.createElement("strong");
    title.textContent = match.conversationTitle;
    const meta = document.createElement("span");
    meta.textContent = [
      "正文命中",
      roleLabel(match.role),
      pathStatus,
      readOnly ? "只读历史（仍可打开）" : "",
    ]
      .filter(Boolean)
      .join(" · ");
    const body = document.createElement("small");
    body.textContent = preview(match.text);
    result.append(title, meta, body);
    result.addEventListener("click", () => {
      if (opening) return;
      openSession(
        match.conversationId,
        match.conversationTitle,
        match.messageId,
      );
    });
    item.append(result);
    results.append(item);
  };

  const render = () => {
    section.hidden = false;
    const pending = metadataState.pending || contentState.pending || opening;
    section.setAttribute("aria-busy", String(pending));
    results.replaceChildren();
    for (const match of metadataState.matches)
      appendMetadataResult(match, metadataState.fallback);
    for (const match of contentState.matches) appendContentResult(match);

    if (opening) {
      setStatus(actionNotice?.message || "正在打开本地对话…");
      return;
    }
    if (actionNotice?.failure) {
      setStatus(actionNotice.message, true);
      return;
    }
    if (metadataState.pending && contentState.pending) {
      setStatus("正在搜索项目、路径、会话标题、模型和本地内容…");
      return;
    }
    if (metadataState.pending) {
      setStatus("正在搜索项目、路径、会话标题和模型…");
      return;
    }
    if (contentState.pending) {
      setStatus("正在搜索本地会话内容…");
      return;
    }

    const metadataSummary = metadataState.failure
      ? metadataState.failure
      : metadataState.matches.length
        ? `找到 ${metadataState.matches.length} 条项目或会话元数据匹配项。`
        : "未在项目名、路径、会话标题或模型中找到匹配项。";
    const contentSummary = contentState.failure
      ? contentState.failure
      : contentState.matches.length
        ? `找到 ${contentState.matches.length} 条本地会话内容匹配项。`
        : "未在本地会话内容中找到匹配项。";
    setStatus(
      `${metadataSummary} ${contentSummary}`,
      Boolean(metadataState.failure || contentState.failure),
    );
  };

  const reset = () => {
    if (timer !== undefined) {
      window.clearTimeout(timer);
      timer = undefined;
    }
    request += 1;
    opening = false;
    openingSerial += 1;
    actionNotice = undefined;
    metadataState = emptyMetadataState();
    contentState = emptyContentState();
    section.hidden = true;
    section.setAttribute("aria-busy", "false");
    status.textContent = "";
    results.replaceChildren();
  };

  const openSession = (
    conversationId: string,
    conversationTitle: string,
    messageId?: string,
  ) => {
    const currentOpening = ++openingSerial;
    opening = true;
    actionNotice = {
      message: `正在打开本地对话：${conversationTitle}…`,
      failure: false,
    };
    render();
    void host.selectSession(conversationId).then(
      async () => {
        try {
          if (currentOpening !== openingSerial) return;
          let target = messageId ? renderedMessage(messageId) : undefined;
          if (messageId && !target) {
            actionNotice = {
              message: "正在加载更早的本地消息以定位搜索结果…",
              failure: false,
            };
            render();
            target = await revealMessageInPagedTranscript(
              host,
              conversationId,
              messageId,
              (page) => {
                if (currentOpening !== openingSerial) return;
                actionNotice = {
                  message: `正在加载更早的本地消息（第 ${page} 页）…`,
                  failure: false,
                };
                render();
              },
              () => currentOpening === openingSerial,
            );
          }
          if (currentOpening !== openingSerial) return;
          if (messageId && !target) {
            opening = false;
            actionNotice = {
              message:
                "已打开会话，但未能加载到该搜索结果的位置；消息可能已删除或历史暂不可用。",
              failure: true,
            };
            render();
            return;
          }
          closePalette(palette, input, target);
          if (target) focusMessage(target);
        } catch (error) {
          if (currentOpening !== openingSerial) return;
          opening = false;
          actionNotice = {
            message: `无法定位该搜索结果：${errorText(error)}`,
            failure: true,
          };
          render();
        }
      },
      (error: unknown) => {
        if (currentOpening !== openingSerial) return;
        opening = false;
        actionNotice = {
          message: `无法打开该对话：${errorText(error)}`,
          failure: true,
        };
        render();
      },
    );
  };

  const completeMetadataFallback = (
    current: number,
    query: string,
    reason: string,
  ) => {
    if (current !== request || !input.value.trim()) return;
    const matches = localMetadataMatches(query, host);
    metadataState = {
      pending: false,
      matches,
      fallback: true,
      failure:
        "项目与会话元数据搜索不可用（" +
        reason +
        "）。" +
        (matches.length
          ? "已从当前加载的 " +
            matches.length +
            " 条会话或项目记录中显示匹配项；未加载的项目、路径和模型无法检索。"
          : "当前加载的会话和项目记录中也没有匹配项；未加载的项目、路径和模型无法检索。"),
    };
    render();
  };

  const searchMetadata = (query: string, current: number) => {
    if (!invoke) {
      completeMetadataFallback(current, query, "桌面搜索接口不可用");
      return;
    }
    void invoke<SessionMetadataSearchResponse>("session_metadata_search", {
      args: { query, limit: MAX_RESULTS },
    }).then(
      (response) => {
        if (current !== request || !input.value.trim()) return;
        const parsed = metadataMatches(response);
        if (!parsed.valid) {
          completeMetadataFallback(
            current,
            query,
            "接口返回了无效的元数据响应",
          );
          return;
        }
        metadataState = {
          pending: false,
          matches: parsed.matches,
          fallback: false,
        };
        render();
      },
      (error: unknown) => {
        completeMetadataFallback(current, query, errorText(error));
      },
    );
  };

  const searchContent = (query: string, current: number) => {
    if (!invoke) {
      if (current !== request || !input.value.trim()) return;
      contentState = {
        pending: false,
        matches: [],
        failure: "本地内容搜索需要桌面运行时。",
      };
      render();
      return;
    }
    void invoke<HistorySearchResponse>("chat_history_search", {
      args: { query, limit: MAX_RESULTS },
    }).then(
      (response) => {
        if (current !== request || !input.value.trim()) return;
        contentState = {
          pending: false,
          matches: searchMatches(response),
        };
        render();
      },
      (error: unknown) => {
        if (current !== request || !input.value.trim()) return;
        contentState = {
          pending: false,
          matches: [],
          failure:
            "本地内容搜索不可用（" +
            errorText(error) +
            "）。仍可使用元数据结果和左侧筛选。",
        };
        render();
      },
    );
  };

  const search = (query: string) => {
    const current = ++request;
    opening = false;
    openingSerial += 1;
    actionNotice = undefined;
    metadataState = { pending: true, matches: [], fallback: false };
    contentState = { pending: true, matches: [] };
    render();
    searchMetadata(query, current);
    searchContent(query, current);
  };

  input.addEventListener("input", () => {
    const query = input.value.trim().slice(0, MAX_QUERY_LENGTH);
    if (!query) {
      reset();
      return;
    }
    // Invalidate an earlier response as soon as the user changes the text,
    // not only when the next debounced request begins.
    request += 1;
    opening = false;
    openingSerial += 1;
    actionNotice = undefined;
    metadataState = { pending: true, matches: [], fallback: false };
    contentState = { pending: true, matches: [] };
    section.hidden = false;
    render();
    if (timer !== undefined) window.clearTimeout(timer);
    timer = window.setTimeout(() => {
      timer = undefined;
      search(query);
    }, SEARCH_DEBOUNCE_MS);
  });

  input.addEventListener("keydown", (event) => {
    if (event.key !== "ArrowDown" || section.hidden || opening) return;
    const firstResult = results.querySelector<HTMLButtonElement>(
      "button:not([disabled])",
    );
    if (!firstResult) return;
    event.preventDefault();
    firstResult.focus();
  });

  // This public NativeShell subscription refreshes only the explicitly
  // degraded renderer snapshot. Successful native metadata results remain
  // authoritative until the user changes the query.
  host.onSessionsChanged(() => {
    const query = input.value.trim().slice(0, MAX_QUERY_LENGTH);
    if (!query || !metadataState.fallback || metadataState.pending) return;
    metadataState = {
      ...metadataState,
      matches: localMetadataMatches(query, host),
    };
    render();
  });
}
