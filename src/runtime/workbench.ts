import {
  requestAppConfirm,
  requestAppPrompt,
  showAppError,
} from "./app-dialogs";
import type { NativeShellApi } from "./host";
import {
  applyFullMessageTimestampPreference,
  currentFullMessageTimestampPreference,
  normalizeFullMessageTimestampPreference,
} from "./message-time";
import { displayPath } from "./path-display";

type UnknownRecord = Record<string, unknown>;
type Invoke = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

type FileEntry = {
  path?: string;
  name?: string;
  kind?: string;
  sizeBytes?: number;
  size_bytes?: number;
};

type FileSearchMatch = {
  path?: string;
  line?: number;
  text?: string;
};

type FileSearchResponse = {
  pattern?: string;
  matchCount?: number;
  fileCount?: number;
  hasMore?: boolean;
  matches?: FileSearchMatch[];
};

type FileReadResponse = {
  content?: string;
  truncated?: boolean;
  startLine?: number;
  numLines?: number;
  totalLines?: number;
};

type GitStatusEntry = {
  path?: string;
  indexStatus?: string;
  worktreeStatus?: string;
};

type GitStatusResponse = {
  isRepository?: boolean;
  repositoryRoot?: string;
  branch?: string;
  ahead?: number;
  behind?: number;
  entries?: GitStatusEntry[];
  stagedCount?: number;
  unstagedCount?: number;
  untrackedCount?: number;
  clean?: boolean;
  unavailableReason?: string;
};

type GitCommitResponse = {
  commitId?: string;
  committedFiles?: number;
};

type GitCommitCapabilityResponse = {
  grantToken?: string;
  workdir?: string;
  stagedCount?: number;
  expiresAtMs?: number;
};

type RuntimeState = {
  status?: string;
  requestId?: string;
};

type HistoryTraceTool = {
  name: string;
  status: string;
  startedAt?: number;
  finishedAt?: number;
};

type HistoryTraceResponse = {
  sessionId: string;
  turnId: string;
  status: string;
  startedAt: number;
  finishedAt?: number;
  tools: HistoryTraceTool[];
};

type MessageLike = {
  role?: string;
  content?: unknown;
  text?: unknown;
};

const MAX_SESSION_TITLE_LENGTH = 200;
const MAX_GLOBAL_SYSTEM_PROMPT_CODE_POINTS = 32_000;
const FILE_CONTENT_SEARCH_LIMIT = 80;
const FILE_CONTENT_SEARCH_PATTERN_MAX_LENGTH = 240;
const FILE_CONTENT_SEARCH_LINE_PREVIEW_MAX_LENGTH = 640;
const renamingSessionIds = new Set<string>();

const SESSION_RENAME_COPY = {
  zh: {
    empty: "请输入对话标题。",
    tooLong: `对话标题最多 ${MAX_SESSION_TITLE_LENGTH} 个字符。`,
    save: "保存",
    saving: "正在保存…",
    busy: "该对话正在保存重命名。",
    renamed: "对话已重命名。",
  },
  en: {
    empty: "Enter a conversation title.",
    tooLong: `Conversation titles can be at most ${MAX_SESSION_TITLE_LENGTH} characters.`,
    save: "Save",
    saving: "Saving…",
    busy: "This conversation is already being renamed.",
    renamed: "Conversation renamed.",
  },
} as const;

const SESSION_DELETE_COPY = {
  zh: {
    title: "删除对话",
    message: (sessionTitle: string) =>
      `确定删除会话“${sessionTitle}”？此操作不可撤销。`,
    confirm: "删除",
    cancel: "取消",
    deleted: "会话已删除",
  },
  en: {
    title: "Delete conversation",
    message: (sessionTitle: string) =>
      `Delete conversation “${sessionTitle}”? This cannot be undone.`,
    confirm: "Delete",
    cancel: "Cancel",
    deleted: "Conversation deleted",
  },
} as const;

type ChatHistoryBatchMutationResult = {
  affectedIds?: string[];
  affected_ids?: string[];
};

const SHORTCUT_HINT_COPY = {
  zh: {
    composerPlaceholder: "描述目标，或粘贴上下文…（Ctrl+Enter 发送）",
    composerPlaceholderWithoutShortcut: "描述目标，或粘贴上下文…",
  },
  en: {
    composerPlaceholder:
      "Describe a goal or paste context... (Ctrl+Enter to send)",
    composerPlaceholderWithoutShortcut: "Describe a goal or paste context...",
  },
} as const;

function sessionDeleteCopy() {
  const locale = document.documentElement.lang.toLowerCase().startsWith("en")
    ? "en"
    : "zh";
  return SESSION_DELETE_COPY[locale];
}

const ARCHIVED_SETTINGS_COPY = {
  zh: {
    count: (count: number) => `${count} 个归档对话`,
    empty: "暂无已归档对话。",
    open: "打开",
    restore: "恢复归档",
    delete: "删除",
    unavailableDate: "归档时间未知",
    unknownProject: "未关联项目",
    opened: "已打开归档对话",
    restored: "已恢复归档",
    deleted: "已删除归档对话",
    busy: "该对话正在处理中。",
    deleteTitle: "删除归档对话",
    deleteMessage: (title: string) =>
      `确定永久删除会话“${title}”？此操作不可撤销。`,
    cancel: "取消",
    listAria: "已归档对话列表",
  },
  en: {
    count: (count: number) => `${count} archived conversations`,
    empty: "No archived conversations.",
    open: "Open",
    restore: "Restore",
    delete: "Delete",
    unavailableDate: "Archive time unavailable",
    unknownProject: "No linked project",
    opened: "Opened archived conversation",
    restored: "Conversation restored",
    deleted: "Archived conversation deleted",
    busy: "This conversation is already being updated.",
    deleteTitle: "Delete archived conversation",
    deleteMessage: (title: string) =>
      `Permanently delete conversation “${title}”? This cannot be undone.`,
    cancel: "Cancel",
    listAria: "Archived conversations list",
  },
} as const;

function archivedSettingsCopy() {
  const locale = document.documentElement.lang.toLowerCase().startsWith("en")
    ? "en"
    : "zh";
  return ARCHIVED_SETTINGS_COPY[locale];
}

function invokeApi(): Invoke | undefined {
  return window.__TAURI__?.core?.invoke as Invoke | undefined;
}

function hostApi(): NativeShellApi | undefined {
  return window.__novaveiHost;
}

function element<T extends HTMLElement>(id: string): T | null {
  return document.getElementById(id) as T | null;
}

function shortcutHintCopy() {
  const locale = document.documentElement.lang.toLowerCase().startsWith("en")
    ? "en"
    : "zh";
  return SHORTCUT_HINT_COPY[locale];
}

function shortcutHintToggles() {
  return [
    ...document.querySelectorAll<HTMLInputElement>(
      "[data-shortcut-hints-toggle]",
    ),
  ];
}

function currentShortcutHintVisibility() {
  const primary = element<HTMLInputElement>("showShortcutHints");
  if (primary) return primary.checked;
  const toggles = shortcutHintToggles();
  return toggles[0]?.checked ?? true;
}

function applyShortcutHintVisibility(showShortcutHints: boolean) {
  document.documentElement.dataset.showShortcutHints =
    String(showShortcutHints);
  for (const toggle of shortcutHintToggles())
    toggle.checked = showShortcutHints;
  const composerInput = element<HTMLTextAreaElement>("composerInput");
  if (!composerInput) return;
  const copy = shortcutHintCopy();
  composerInput.placeholder = showShortcutHints
    ? copy.composerPlaceholder
    : copy.composerPlaceholderWithoutShortcut;
}

function messageTimestampToggles() {
  return [
    ...document.querySelectorAll<HTMLInputElement>(
      "[data-message-timestamp-toggle]",
    ),
  ];
}

function currentMessageTimestampPreference() {
  const primary = element<HTMLInputElement>("showFullMessageTimestamp");
  if (primary) return primary.checked;
  const toggles = messageTimestampToggles();
  return toggles[0]?.checked ?? currentFullMessageTimestampPreference();
}

function applyMessageTimestampPreference(showFull: boolean) {
  applyFullMessageTimestampPreference(showFull);
  for (const toggle of messageTimestampToggles()) toggle.checked = showFull;
}

function toast(message: string) {
  const target = element<HTMLElement>("toast");
  if (!target) {
    console.warn("[NovaVei workbench]", message);
    return;
  }
  target.textContent = message;
  target.classList.add("show");
  window.setTimeout(() => target.classList.remove("show"), 2200);
}

function errorText(error: unknown) {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error.trim()) return error;
  try {
    return JSON.stringify(error);
  } catch {
    return "操作失败";
  }
}

type SessionRenameDialogParts = {
  dialog: HTMLDialogElement;
  form: HTMLFormElement;
  input: HTMLInputElement;
  error: HTMLElement;
  cancel: HTMLButtonElement;
  save: HTMLButtonElement;
};

function sessionRenameText(key: keyof typeof SESSION_RENAME_COPY.zh) {
  const locale = document.documentElement.lang.toLowerCase().startsWith("en")
    ? "en"
    : "zh";
  return SESSION_RENAME_COPY[locale][key];
}

function sessionRenameDialogParts(): SessionRenameDialogParts | undefined {
  const dialog = element<HTMLDialogElement>("sessionRenameDialog");
  const form = element<HTMLFormElement>("sessionRenameForm");
  const input = element<HTMLInputElement>("sessionRenameInput");
  const error = element<HTMLElement>("sessionRenameError");
  const cancel = element<HTMLButtonElement>("sessionRenameCancel");
  const save = element<HTMLButtonElement>("sessionRenameSave");
  if (!dialog || !form || !input || !error || !cancel || !save)
    return undefined;
  return { dialog, form, input, error, cancel, save };
}

function sessionTitleValidationError(title: string): string | undefined {
  if (!title) return sessionRenameText("empty");
  if (Array.from(title).length > MAX_SESSION_TITLE_LENGTH)
    return sessionRenameText("tooLong");
  return undefined;
}

async function requestSessionRename(
  initialTitle: string,
  persist: (title: string) => Promise<void>,
): Promise<void> {
  const parts = sessionRenameDialogParts();
  if (!parts || typeof parts.dialog.showModal !== "function") {
    const entered = await requestAppPrompt({
      title: document.documentElement.lang.toLowerCase().startsWith("en")
        ? "Rename conversation"
        : "重命名对话",
      label: document.documentElement.lang.toLowerCase().startsWith("en")
        ? "Title"
        : "对话标题",
      initialValue: initialTitle,
      maxLength: MAX_SESSION_TITLE_LENGTH,
    });
    if (entered === null) return;
    const title = entered.trim();
    const validationError = sessionTitleValidationError(title);
    if (validationError) {
      toast(validationError);
      return;
    }
    await persist(title);
    return;
  }
  const { dialog, form, input, error, cancel, save } = parts;
  if (dialog.open) {
    toast(sessionRenameText("busy"));
    return;
  }
  await new Promise<void>((resolve) => {
    let finished = false;
    let saving = false;
    const clearError = () => {
      error.hidden = true;
      error.textContent = "";
      input.removeAttribute("aria-invalid");
    };
    const showError = (message: string) => {
      error.textContent = message;
      error.hidden = false;
      input.setAttribute("aria-invalid", "true");
    };
    const setSaving = (active: boolean) => {
      saving = active;
      input.disabled = active;
      cancel.disabled = active;
      save.disabled = active;
      save.setAttribute("aria-busy", String(active));
      save.textContent = active
        ? sessionRenameText("saving")
        : sessionRenameText("save");
    };
    const cleanup = () => {
      form.removeEventListener("submit", onSubmit);
      cancel.removeEventListener("click", onCancelClick);
      dialog.removeEventListener("cancel", onDialogCancel);
      dialog.removeEventListener("close", onDialogClose);
    };
    const finish = () => {
      if (finished) return;
      finished = true;
      cleanup();
      if (dialog.open) dialog.close();
      resolve();
    };
    const onSubmit = (event: Event) => {
      event.preventDefault();
      if (saving) return;
      const title = input.value.trim();
      const validationError = sessionTitleValidationError(title);
      if (validationError) {
        showError(validationError);
        toast(validationError);
        input.focus();
        return;
      }
      clearError();
      setSaving(true);
      void persist(title).then(
        () => finish(),
        (reason: unknown) => {
          const message = errorText(reason);
          toast(message);
          showError(message);
          setSaving(false);
          input.focus();
        },
      );
    };
    const onCancelClick = () => {
      if (!saving) finish();
    };
    const onDialogCancel = (event: Event) => {
      if (saving) event.preventDefault();
    };
    const onDialogClose = () => finish();

    input.value = initialTitle;
    clearError();
    setSaving(false);
    form.addEventListener("submit", onSubmit);
    cancel.addEventListener("click", onCancelClick);
    dialog.addEventListener("cancel", onDialogCancel);
    dialog.addEventListener("close", onDialogClose);
    dialog.showModal();
    window.requestAnimationFrame(() => {
      if (finished) return;
      input.focus();
      input.select();
    });
  });
}

function requireDesktop() {
  const invoke = invokeApi();
  const host = hostApi();
  if (!invoke || !host) throw new Error("该操作需要 NovaVei 桌面运行时");
  return { invoke, host };
}

function hideContextMenus() {
  document
    .querySelectorAll<HTMLElement>(".sidebar-ctx-menu")
    .forEach((menu) => {
      menu.hidden = true;
    });
  document.querySelectorAll<HTMLElement>(".is-ctx-target").forEach((target) => {
    target.classList.remove("is-ctx-target");
  });
}

async function copyText(value: string) {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value);
    return;
  }
  const input = document.createElement("textarea");
  input.value = value;
  input.style.position = "fixed";
  input.style.left = "-9999px";
  document.body.appendChild(input);
  input.select();
  if (!document.execCommand("copy")) throw new Error("系统剪贴板不可用");
  input.remove();
}

function asRecord(value: unknown): UnknownRecord | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as UnknownRecord)
    : undefined;
}

function parseJson(value: unknown): unknown {
  if (typeof value !== "string") return value;
  try {
    return JSON.parse(value) as unknown;
  } catch {
    return value;
  }
}

function messageText(value: unknown): string {
  if (typeof value === "string") return value;
  if (Array.isArray(value)) {
    return value
      .map((item) => {
        const record = asRecord(item);
        return typeof record?.text === "string"
          ? record.text
          : typeof record?.content === "string"
            ? record.content
            : "";
      })
      .join("");
  }
  return "";
}

function collectMessages(value: unknown): MessageLike[] {
  const parsed = parseJson(value);
  if (Array.isArray(parsed))
    return parsed.flatMap((item) => collectMessages(item));
  const record = asRecord(parsed);
  if (!record) return [];
  if (Array.isArray(record.messages))
    return record.messages.flatMap((item) => collectMessages(item));
  if (Array.isArray(record.items))
    return record.items.flatMap((item) => collectMessages(item));
  if (Array.isArray(record.segments))
    return record.segments.flatMap((item) => collectMessages(item));
  if (record.messagesJson !== undefined)
    return collectMessages(record.messagesJson);
  if (typeof record.role === "string") {
    return [{ role: record.role, content: record.content, text: record.text }];
  }
  return [];
}

function historyExport(value: unknown, fallbackTitle: string) {
  const root = asRecord(value);
  const title =
    typeof root?.title === "string" && root.title.trim()
      ? root.title.trim()
      : fallbackTitle;
  const messages = collectMessages(root?.segments ?? root?.messages ?? value);
  const lines = [title];
  for (const message of messages) {
    const role = (message.role ?? "assistant").toLowerCase();
    const label =
      role === "user" ? "User" : role.includes("tool") ? "Tool" : "Assistant";
    const text = messageText(message.content ?? message.text).trim();
    if (text) lines.push("", `${label}: ${text}`);
  }
  return lines.join("\n").trim();
}

function historyArticle(target: Element) {
  return target.closest<HTMLElement>(".msg-assistant");
}

function assistantText(article: HTMLElement) {
  const explicit = article
    .querySelector<HTMLElement>("[data-history-content]")
    ?.textContent?.trim();
  if (explicit) return explicit;
  return [...article.children]
    .filter(
      (child) =>
        !child.classList.contains("who") &&
        !child.classList.contains("msg-actions"),
    )
    .map((child) => child.textContent?.trim() || "")
    .filter(Boolean)
    .join("\n\n");
}

function nearestUserPrompt(article: HTMLElement) {
  let cursor = article.previousElementSibling;
  while (cursor) {
    if (cursor.matches(".msg-user")) return cursor.textContent?.trim() || "";
    cursor = cursor.previousElementSibling;
  }
  return "";
}

function traceStatusLabel(status: string) {
  switch (status) {
    case "completed":
      return "完成";
    case "failed":
      return "失败";
    case "cancelled":
      return "已取消";
    case "interrupted":
      return "已中断";
    case "waiting_permission":
      return "等待批准";
    case "starting":
      return "启动中";
    case "running":
      return "运行中";
    default:
      return "状态未知";
  }
}

function finiteTime(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) && value > 0
    ? value
    : undefined;
}

function safeTraceLabel(value: unknown, fallback: string, maxLength: number) {
  if (typeof value !== "string") return fallback;
  const text = value.replace(/[\u0000-\u001F\u007F]/g, " ").trim();
  return text ? text.slice(0, maxLength) : fallback;
}

function normalizeTraceStatus(value: unknown) {
  const status = typeof value === "string" ? value.trim().toLowerCase() : "";
  return [
    "completed",
    "failed",
    "cancelled",
    "interrupted",
    "waiting_permission",
    "starting",
    "running",
  ].includes(status)
    ? status
    : "unknown";
}

function traceTiming(
  startedAt: number | undefined,
  finishedAt: number | undefined,
) {
  if (startedAt && finishedAt && finishedAt >= startedAt) {
    const milliseconds = finishedAt - startedAt;
    return milliseconds < 1000
      ? `${milliseconds} ms`
      : `${(milliseconds / 1000).toFixed(milliseconds < 10_000 ? 1 : 0)} s`;
  }
  if (finishedAt) return "已结束";
  if (startedAt) return "进行中";
  return "未记录时间";
}

function parseHistoryTrace(value: unknown): HistoryTraceResponse | undefined {
  const record = asRecord(value);
  const sessionId =
    typeof record?.sessionId === "string" ? record.sessionId.trim() : "";
  const turnId = typeof record?.turnId === "string" ? record.turnId.trim() : "";
  const startedAt = finiteTime(record?.startedAt);
  if (!sessionId || !turnId || !startedAt) return undefined;
  const tools = Array.isArray(record?.tools)
    ? record.tools.slice(0, 128).map((candidate) => {
        const tool = asRecord(candidate);
        return {
          name: safeTraceLabel(tool?.name, "工具", 160),
          status: normalizeTraceStatus(tool?.status),
          startedAt: finiteTime(tool?.startedAt),
          finishedAt: finiteTime(tool?.finishedAt),
        };
      })
    : [];
  return {
    sessionId,
    turnId,
    status: normalizeTraceStatus(record?.status),
    startedAt,
    finishedAt: finiteTime(record?.finishedAt),
    tools,
  };
}

function traceItem(
  name: string,
  status: string,
  startedAt?: number,
  finishedAt?: number,
): HTMLLIElement {
  const normalizedStatus = normalizeTraceStatus(status);
  const item = document.createElement("li");
  item.dataset.traceStatus = normalizedStatus;
  const marker = document.createElement("i");
  marker.setAttribute("aria-hidden", "true");
  if (
    normalizedStatus === "running" ||
    normalizedStatus === "starting" ||
    normalizedStatus === "waiting_permission"
  )
    marker.className = "run";
  else if (normalizedStatus === "unknown") marker.className = "wait";
  const content = document.createElement("span");
  content.className = "history-trace-content";
  const title = document.createElement("b");
  title.textContent = name;
  const meta = document.createElement("span");
  meta.className = "history-trace-meta";
  const statusLabel = document.createElement("small");
  statusLabel.className = "history-trace-status";
  statusLabel.textContent = traceStatusLabel(normalizedStatus);
  const timing = document.createElement("small");
  timing.className = "history-trace-time";
  timing.textContent = traceTiming(startedAt, finishedAt);
  meta.append(statusLabel, timing);
  content.append(title, meta);
  item.append(marker, content);
  return item;
}

function historyTraceList(article: HTMLElement): HTMLOListElement {
  const existing = article.querySelector<HTMLOListElement>(
    "[data-history-trace]",
  );
  if (existing) return existing;
  const list = document.createElement("ol");
  list.className = "trace history-trace";
  list.dataset.historyTrace = "true";
  list.tabIndex = -1;
  list.setAttribute("aria-label", "本回复的已保存运行轨迹");
  list.setAttribute("aria-live", "polite");
  article.appendChild(list);
  return list;
}

function focusTrace(list: HTMLOListElement) {
  list.scrollIntoView({
    behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches
      ? "auto"
      : "smooth",
    block: "nearest",
  });
  list.focus({ preventScroll: true });
}

async function showHistoricalTrace(
  article: HTMLElement,
  button: HTMLButtonElement,
) {
  const invoke = invokeApi();
  const host = hostApi();
  if (!invoke || !host) {
    throw new Error("运行轨迹仅在已保存的桌面会话中提供");
  }

  const sessionId = article.dataset.historySessionId?.trim();
  const messageId = article.dataset.historyMessageId?.trim();
  const turnId = article.dataset.historyTurnId?.trim();
  if (!sessionId || !messageId || !turnId) {
    throw new Error("该历史回复未保存可验证的运行标识，无法安全显示轨迹");
  }
  if (host.getSessionId()?.trim() !== sessionId) {
    throw new Error("当前会话已切换；不会显示其他会话的轨迹");
  }

  const list = historyTraceList(article);
  const previousLabel = button.textContent;
  button.disabled = true;
  button.textContent = "读取轨迹…";
  button.setAttribute("aria-expanded", "true");
  list.setAttribute("aria-busy", "true");
  list.replaceChildren(traceItem("正在读取已保存轨迹", "running"));
  try {
    const raw = await invoke<unknown>("chat_history_trace_get", {
      input: { sessionId, messageId, turnId },
    });
    const trace = parseHistoryTrace(raw);
    if (!trace || trace.sessionId !== sessionId || trace.turnId !== turnId) {
      throw new Error("历史轨迹响应无法验证");
    }
    if (!article.isConnected || host.getSessionId()?.trim() !== sessionId) {
      list.remove();
      toast("会话已切换，未显示其他会话的轨迹");
      return;
    }
    const rows = trace.tools.map((tool) =>
      traceItem(tool.name, tool.status, tool.startedAt, tool.finishedAt),
    );
    if (!rows.length)
      rows.push(
        traceItem(
          "本轮没有保存工具调用",
          trace.status,
          trace.startedAt,
          trace.finishedAt,
        ),
      );
    else
      rows.push(
        traceItem("本轮运行", trace.status, trace.startedAt, trace.finishedAt),
      );
    list.replaceChildren(...rows);
    list.dataset.historyTraceTurnId = turnId;
    list.removeAttribute("aria-busy");
    focusTrace(list);
    toast(`已显示 ${trace.tools.length} 条本回复的工具轨迹`);
  } catch (error) {
    list.remove();
    button.setAttribute("aria-expanded", "false");
    console.warn("[NovaVei workbench] historical trace load failed", error);
    throw new Error("无法读取本回复的已保存轨迹；不会显示其他运行的轨迹");
  } finally {
    if (button.isConnected) {
      button.disabled = false;
      button.textContent = previousLabel;
    }
  }
}

async function handleHistoryAction(action: string, target: HTMLElement) {
  if (action === "copy-code") {
    const code =
      target.closest(".code-block")?.querySelector("pre")?.textContent || "";
    if (!code.trim()) throw new Error("没有可复制的代码");
    await copyText(code);
    toast("已复制代码");
    return;
  }
  const article = historyArticle(target);
  if (!article) throw new Error("找不到对应的历史回复");
  if (action === "copy") {
    const content = assistantText(article);
    if (!content) throw new Error("该回复没有可复制内容");
    await copyText(content);
    toast("已复制回复");
    return;
  }
  if (action === "trace") {
    const button = target instanceof HTMLButtonElement ? target : undefined;
    if (!button) throw new Error("历史轨迹按钮不可用");
    await showHistoricalTrace(article, button);
    return;
  }
  if (action === "retry") {
    const prompt = nearestUserPrompt(article);
    if (!prompt) throw new Error("找不到该回复对应的用户消息");
    const input = element<HTMLTextAreaElement>("composerInput");
    if (!input) throw new Error("消息输入框不可用");
    input.value = prompt;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
    toast("已回填原问题，可修改后重新发送");
    return;
  }
  if (action === "branch") {
    const sessionId = hostApi()?.getSessionId();
    if (!sessionId) throw new Error("当前没有可分叉的真实会话");
    await activateBranch(sessionId);
    toast("已创建分支会话");
    return;
  }
  throw new Error(`不支持的历史操作: ${action}`);
}

function sessionTarget(target: Element | null) {
  return (
    target?.closest<HTMLElement>(".session[data-session-id]") ??
    document.querySelector<HTMLElement>(
      ".session.is-ctx-target[data-session-id]",
    )
  );
}

async function renameSession(
  id: string,
  currentTitle: string,
  invoke: Invoke,
  host: NativeShellApi,
) {
  if (renamingSessionIds.has(id)) {
    toast(sessionRenameText("busy"));
    return;
  }
  await requestSessionRename(currentTitle, async (title) => {
    if (title === currentTitle) return;
    if (renamingSessionIds.has(id)) throw new Error(sessionRenameText("busy"));
    renamingSessionIds.add(id);
    try {
      await invoke("chat_history_rename", { id, title });
      await host.refreshSessions({ loadActive: false });
      if (host.getSessionId() === id) {
        const heading = element<HTMLElement>("chatTitle");
        if (heading) heading.textContent = title;
      }
      toast(sessionRenameText("renamed"));
    } finally {
      renamingSessionIds.delete(id);
    }
  });
}

async function handleSessionAction(action: string, target: HTMLElement) {
  const id = target.dataset.sessionId?.trim();
  if (!id) throw new Error("当前会话没有持久化 ID");
  const { invoke, host } = requireDesktop();
  const title =
    (
      target.dataset.title ||
      target.querySelector("strong")?.textContent ||
      "当前会话"
    ).trim() || "当前会话";
  switch (action) {
    case "rename":
      await renameSession(id, title, invoke, host);
      return;
    case "pin":
    case "unpin":
      await invoke("chat_history_set_pinned", {
        id,
        isPinned: action === "pin",
      });
      await host.refreshSessions({ loadActive: false });
      toast(action === "pin" ? "会话已置顶" : "会话已取消置顶");
      return;
    case "archive":
    case "unarchive":
      await invoke("chat_history_set_archived", {
        id,
        isArchived: action === "archive",
      });
      await host.refreshSessions({ loadActive: false });
      toast(action === "archive" ? "会话已归档" : "会话已取消归档");
      return;
    case "delete": {
      const deleteCopy = sessionDeleteCopy();
      const confirmed = await requestAppConfirm({
        title: deleteCopy.title,
        message: deleteCopy.message(title),
        confirmLabel: deleteCopy.confirm,
        cancelLabel: deleteCopy.cancel,
        danger: true,
      });
      if (!confirmed) return;
      await invoke("chat_history_delete", { id });
      await host.refreshSessions({ loadActive: true });
      toast(deleteCopy.deleted);
      return;
    }
    case "copy": {
      const history = await invoke("chat_history_get", { id });
      await copyText(historyExport(history, title));
      toast("已复制会话");
      return;
    }
    case "duplicate": {
      await activateBranch(id, `${title} (副本)`);
      toast("已创建会话副本");
      return;
    }
    case "branch": {
      await activateBranch(id);
      toast("已创建分支会话");
      return;
    }
    default:
      throw new Error(`不支持的会话操作: ${action}`);
  }
}

async function activateBranch(id: string, title?: string) {
  const host = requireDesktop().host;
  const branch = await host.branchSession(id, title);
  await host.refreshSessions({ loadActive: false });
  await host.selectSession(branch.id);
}

function installSessionActions() {
  document.addEventListener(
    "click",
    (event) => {
      const target = event.target instanceof Element ? event.target : null;
      const actionButton = target?.closest<HTMLElement>(
        "[data-session-action]",
      );
      const historyAction = target?.closest<HTMLElement>(
        "[data-history-action]",
      );
      if (actionButton) {
        const action = actionButton.dataset.sessionAction;
        if (!action) return;
        const session = sessionTarget(actionButton);
        // Capture only hydrated native sessions; browser markup never owns
        // durable conversation actions.
        if (!session || !invokeApi() || !hostApi()) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        hideContextMenus();
        void handleSessionAction(action, session).catch((error) =>
          toast(errorText(error)),
        );
        return;
      }
      if (historyAction) {
        const action = historyAction.dataset.historyAction;
        if (!action) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        void handleHistoryAction(action, historyAction).catch((error) =>
          toast(errorText(error)),
        );
      }
    },
    true,
  );
}

function selectedConversationCount(
  result: ChatHistoryBatchMutationResult | undefined,
  fallback: number,
) {
  const affectedIds = result?.affectedIds ?? result?.affected_ids;
  return Array.isArray(affectedIds) ? affectedIds.length : fallback;
}

async function applyProjectConversationBatchAction(
  action: "archive" | "delete",
) {
  const { invoke, host } = requireDesktop();
  const selection = host.getProjectConversationSelection();
  if (!selection || selection.sessionIds.length === 0) return;

  const selectedCount = selection.sessionIds.length;
  const isDelete = action === "delete";
  const confirmed = await requestAppConfirm({
    title: isDelete ? "删除所选对话" : "归档所选对话",
    message: isDelete
      ? `确定删除已选的 ${selectedCount} 个会话吗？此操作不可撤销。`
      : `确定归档已选的 ${selectedCount} 个会话吗？归档后可在归档列表中查看。`,
    confirmLabel: isDelete ? "删除" : "归档",
    cancelLabel: "取消",
    danger: isDelete,
  });
  if (!confirmed) return;

  host.setProjectConversationSelectionBusy(true);
  try {
    const result = isDelete
      ? await invoke<ChatHistoryBatchMutationResult>(
          "chat_history_bulk_delete",
          {
            ids: selection.sessionIds,
            workdir: selection.workdir,
          },
        )
      : await invoke<ChatHistoryBatchMutationResult>(
          "chat_history_bulk_set_archived",
          {
            ids: selection.sessionIds,
            isArchived: true,
            workdir: selection.workdir,
          },
        );
    const affectedCount = selectedConversationCount(result, selectedCount);
    host.clearProjectConversationSelection();
    await host.refreshSessions({ loadActive: isDelete });
    host.restoreProjectConversationSelectionFocus();
    toast(
      isDelete
        ? `已删除 ${affectedCount} 个会话`
        : `已归档 ${affectedCount} 个会话`,
    );
  } catch (error) {
    // A rejected active run intentionally leaves the selected rows intact so
    // the user can stop the run or revise the exact batch and retry.
    await showAppError({
      title: isDelete ? "无法删除所选对话" : "无法归档所选对话",
      message: "批量操作未完成，未应用任何部分更改。",
      detail: errorText(error),
    });
  } finally {
    host.setProjectConversationSelectionBusy(false);
  }
}

function installProjectConversationBulkActions() {
  document.addEventListener(
    "click",
    (event) => {
      const target = event.target instanceof Element ? event.target : null;
      const archive = target?.closest<HTMLButtonElement>(
        "#btnArchiveSelectedConversations",
      );
      const remove = target?.closest<HTMLButtonElement>(
        "#btnDeleteSelectedConversations",
      );
      if (!archive && !remove) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      void applyProjectConversationBatchAction(remove ? "delete" : "archive");
    },
    true,
  );
}

function installUnavailableFeatures() {
  document
    .querySelectorAll<HTMLElement>("[data-feature-unavailable]")
    .forEach((control) => {
      control.setAttribute("aria-disabled", "true");
    });
  document.addEventListener(
    "click",
    (event) => {
      const target = event.target instanceof Element ? event.target : null;
      const control = target?.closest<HTMLElement>(
        "[data-feature-unavailable]",
      );
      if (!control) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      const english = document.documentElement.lang
        .toLowerCase()
        .startsWith("en");
      toast(
        english
          ? "This feature is not connected yet."
          : "此功能尚未接入桌面运行时",
      );
    },
    true,
  );
}

function filePane() {
  return document.querySelector<HTMLElement>('.dock-pane[data-pane="files"]');
}

function fileIcon(kind: string) {
  const icon = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  icon.setAttribute("viewBox", "0 0 24 24");
  icon.setAttribute("class", "file-icon");
  icon.setAttribute("aria-hidden", "true");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute(
    "d",
    kind === "dir"
      ? "M3.5 7.5A2.5 2.5 0 0 1 6 5h4.1l2 2H18a2.5 2.5 0 0 1 2.5 2.5v7A2.5 2.5 0 0 1 18 19H6a2.5 2.5 0 0 1-2.5-2.5z"
      : "M6.5 3.5h7l4 4v13H6.5a2 2 0 0 1-2-2v-13a2 2 0 0 1 2-2Z M13.5 3.5v4h4",
  );
  icon.appendChild(path);
  return icon;
}

function uiText(zh: string, en: string) {
  return document.documentElement.lang.toLowerCase().startsWith("en") ? en : zh;
}

function normaliseWorkspacePath(value: string | undefined) {
  const segments: string[] = [];
  for (const segment of (value ?? "").replace(/\\/g, "/").split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === "..") {
      // The native boundary remains authoritative, but make breadcrumbs and
      // back navigation accurately reflect a relative path without allowing
      // the UI to walk above the project root.
      segments.pop();
      continue;
    }
    segments.push(segment);
  }
  return segments.join("/") || ".";
}

function parentWorkspacePath(value: string) {
  const segments = normaliseWorkspacePath(value)
    .split("/")
    .filter((segment) => segment !== ".");
  segments.pop();
  return segments.join("/") || ".";
}

function formatFileSize(value: unknown) {
  const bytes = Number(value);
  if (!Number.isFinite(bytes) || bytes <= 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let unit = -1;
  let amount = bytes;
  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024;
    unit += 1;
  }
  return `${amount >= 10 ? amount.toFixed(0) : amount.toFixed(1)} ${units[unit]}`;
}

function fileEntryRow(
  entry: FileEntry,
  load: (entry: FileEntry) => void,
  selected = false,
) {
  const row = document.createElement("button");
  row.type = "button";
  row.className = selected ? "tree-row is-selected" : "tree-row";
  const path = normaliseWorkspacePath(entry.path || entry.name);
  const kind = entry.kind === "dir" ? "dir" : "file";
  row.dataset.fsPath = path;
  row.dataset.fsKind = kind;
  const name = document.createElement("strong");
  name.textContent = entry.name || displayPath(entry.path) || "(unnamed)";
  const size = document.createElement("span");
  size.textContent =
    kind === "dir" ? "" : formatFileSize(entry.sizeBytes ?? entry.size_bytes);
  row.setAttribute(
    "aria-label",
    `${kind === "dir" ? uiText("打开文件夹", "Open folder") : uiText("预览文件", "Preview file")} ${name.textContent}`,
  );
  if (selected) row.setAttribute("aria-current", "true");
  row.append(fileIcon(kind), name, size);
  row.addEventListener("click", () => load(entry));
  return row;
}

function fileMessage(message: string, kind: "info" | "error" = "info") {
  const notice = document.createElement("p");
  notice.className =
    kind === "error" ? "file-browser-message is-error" : "file-browser-message";
  notice.setAttribute("role", kind === "error" ? "alert" : "status");
  notice.setAttribute("aria-live", kind === "error" ? "assertive" : "polite");
  notice.textContent = message;
  return notice;
}

function renderFileError(
  section: HTMLElement,
  message: string,
  toolbar?: HTMLElement,
) {
  section.replaceChildren(...(toolbar ? [toolbar] : []));
  const error = document.createElement("p");
  error.className = "file-browser-message is-error";
  error.setAttribute("role", "alert");
  error.setAttribute("aria-live", "assertive");
  error.textContent = `${uiText("无法读取文件：", "Could not read files: ")}${message}`;
  section.appendChild(error);
}

function installFiles() {
  // Browser preview keeps its static File Dock surface. Native capability
  // issuance is meaningful only after the Tauri host has been installed.
  if (!invokeApi() || !hostApi()) return;

  let serial = 0;
  let currentDirectory = ".";
  let previewEntry: FileEntry | undefined;
  let previewContent:
    | {
        content: string;
        truncated: boolean;
        startLine?: number;
        numLines?: number;
        totalLines?: number;
      }
    | undefined;
  let currentEntries: FileEntry[] = [];
  let fileFilter = "";
  let contentSearch = "";
  let contentSearchResult: FileSearchResponse | undefined;
  let contentSearchError = "";
  let contentSearchPending = false;
  let contentSearchSerial = 0;
  let hasLoaded = false;

  const sectionFor = () => filePane()?.querySelector<HTMLElement>(".section");

  const clearContentSearch = () => {
    contentSearchSerial += 1;
    contentSearch = "";
    contentSearchResult = undefined;
    contentSearchError = "";
    contentSearchPending = false;
  };

  const fileAccessMessage = () => {
    switch (document.body.dataset.novaveiShellState) {
      case "loading":
        return uiText(
          "正在读取项目与会话，准备完成后即可查看文件。",
          "Loading projects and conversations. Files will be available when ready.",
        );
      case "needs_workspace":
        return uiText(
          "请先打开一个项目文件夹，再查看项目文件。",
          "Open a project folder before browsing project files.",
        );
      case "needs_session":
        return uiText(
          "请先创建或打开一个对话，再查看项目文件。",
          "Create or open a conversation before browsing project files.",
        );
      case "storage_recovery":
        return uiText(
          "本地存储需要恢复后才能查看项目文件。",
          "Recover local storage before browsing project files.",
        );
      case "error":
        return uiText(
          "本地项目暂不可用。请在对话区重试加载后再查看文件。",
          "Local project data is unavailable. Retry loading from the conversation area first.",
        );
      default:
        return "";
    }
  };

  const canBrowseFiles = (section: HTMLElement) => {
    const message = fileAccessMessage();
    if (!message) return true;
    section.replaceChildren(fileMessage(message));
    return false;
  };

  const renderToolbar = (
    section: HTMLElement,
    options: {
      loading?: boolean;
      refresh: () => void;
      navigate: (path: string) => void;
      goBack: () => void;
    },
  ) => {
    const toolbar = document.createElement("div");
    toolbar.className = "file-browser-toolbar";
    const head = document.createElement("div");
    head.className = "section-head";
    const title = document.createElement("h4");
    title.textContent = previewEntry
      ? uiText("文件预览", "File preview")
      : uiText("项目文件", "Project files");
    const state = document.createElement("span");
    state.className = "pill wait";
    state.textContent = options.loading
      ? uiText("加载中", "Loading")
      : previewEntry
        ? uiText("预览", "Preview")
        : uiText("目录", "Directory");
    head.append(title, state);

    const breadcrumb = document.createElement("nav");
    breadcrumb.className = "file-breadcrumb";
    breadcrumb.setAttribute(
      "aria-label",
      uiText("当前文件夹", "Current folder"),
    );
    const appendCrumb = (label: string, path: string, current: boolean) => {
      const crumb = document.createElement("button");
      crumb.type = "button";
      crumb.textContent = label;
      crumb.disabled = current || Boolean(options.loading);
      if (current) crumb.setAttribute("aria-current", "page");
      else crumb.addEventListener("click", () => options.navigate(path));
      breadcrumb.appendChild(crumb);
    };
    appendCrumb(
      uiText("根目录", "Root"),
      ".",
      currentDirectory === "." && !previewEntry,
    );
    const segments = currentDirectory
      .split("/")
      .filter((segment) => segment !== ".");
    let path = "";
    for (const [index, segment] of segments.entries()) {
      path = path ? `${path}/${segment}` : segment;
      const separator = document.createElement("span");
      separator.setAttribute("aria-hidden", "true");
      separator.textContent = "/";
      breadcrumb.appendChild(separator);
      appendCrumb(
        segment,
        path,
        index === segments.length - 1 && !previewEntry,
      );
    }
    if (previewEntry) {
      const separator = document.createElement("span");
      separator.setAttribute("aria-hidden", "true");
      separator.textContent = "/";
      const fileName = document.createElement("span");
      fileName.className = "file-breadcrumb-current";
      fileName.textContent =
        previewEntry.name || previewEntry.path || uiText("文件", "File");
      fileName.title = previewEntry.path || previewEntry.name || "";
      breadcrumb.append(separator, fileName);
    }

    const actions = document.createElement("div");
    actions.className = "row-actions";
    const back = document.createElement("button");
    back.type = "button";
    back.className = "btn";
    back.textContent = previewEntry
      ? uiText("返回列表", "Back to list")
      : uiText("上一级", "Up one level");
    back.disabled =
      options.loading || (!previewEntry && currentDirectory === ".");
    back.addEventListener("click", options.goBack);
    const refresh = document.createElement("button");
    refresh.type = "button";
    refresh.className = "btn";
    refresh.textContent = uiText("刷新", "Refresh");
    refresh.disabled = Boolean(options.loading);
    refresh.addEventListener("click", options.refresh);
    actions.append(back, refresh);
    toolbar.append(head, breadcrumb, actions);
    section.replaceChildren(toolbar);
    return toolbar;
  };

  const renderFileBrowser = (section: HTMLElement) => {
    const layout = document.createElement("div");
    layout.className = "file-browser-layout";

    const tree = document.createElement("section");
    tree.className = "file-browser-tree";
    const treeHead = document.createElement("div");
    treeHead.className = "file-browser-tree-head";
    const treeTitle = document.createElement("strong");
    treeTitle.textContent = uiText("资源", "Resources");
    const treeCount = document.createElement("span");
    treeCount.setAttribute("aria-live", "polite");
    treeHead.append(treeTitle, treeCount);

    const filter = document.createElement("input");
    filter.type = "search";
    filter.className = "file-browser-filter";
    filter.value = fileFilter;
    filter.placeholder = uiText("筛选当前目录", "Filter this folder");
    filter.setAttribute(
      "aria-label",
      uiText("筛选当前目录", "Filter this folder"),
    );

    const contentSearchForm = document.createElement("form");
    contentSearchForm.className = "row-actions";
    contentSearchForm.noValidate = true;
    const contentSearchInput = document.createElement("input");
    contentSearchInput.type = "search";
    contentSearchInput.className = "file-browser-filter";
    contentSearchInput.value = contentSearch;
    contentSearchInput.maxLength = FILE_CONTENT_SEARCH_PATTERN_MAX_LENGTH;
    contentSearchInput.placeholder = uiText(
      "内容搜索（正则，不区分大小写）",
      "Search content (regex, case-insensitive)",
    );
    contentSearchInput.setAttribute(
      "aria-label",
      uiText(
        "在当前文件夹内容中搜索正则",
        "Search current folder content with a regular expression",
      ),
    );
    const contentSearchButton = document.createElement("button");
    contentSearchButton.type = "submit";
    contentSearchButton.className = "btn";
    const clearContentSearchButton = document.createElement("button");
    clearContentSearchButton.type = "button";
    clearContentSearchButton.className = "btn";
    contentSearchForm.append(
      contentSearchInput,
      contentSearchButton,
      clearContentSearchButton,
    );
    const contentSearchStatus = document.createElement("p");
    contentSearchStatus.className = "file-browser-message";
    contentSearchStatus.setAttribute("role", "status");
    contentSearchStatus.setAttribute("aria-live", "polite");
    const contentSearchResults = document.createElement("div");
    contentSearchResults.className = "file-browser-list";

    const compactSearchLine = (value: unknown) => {
      const line =
        typeof value === "string" ? value.replace(/\s+/g, " ").trim() : "";
      return line.length > FILE_CONTENT_SEARCH_LINE_PREVIEW_MAX_LENGTH
        ? `${line.slice(0, FILE_CONTENT_SEARCH_LINE_PREVIEW_MAX_LENGTH)}…`
        : line;
    };
    const openContentSearchMatch = (match: FileSearchMatch) => {
      const rawPath = match.path?.trim();
      if (!rawPath) return;
      const path = normaliseWorkspacePath(rawPath);
      if (path === ".") return;
      const line =
        Number.isSafeInteger(match.line) && Number(match.line) > 0
          ? Number(match.line)
          : 1;
      clearContentSearch();
      void readFile(
        {
          path,
          name: path.split("/").pop(),
          kind: "file",
        },
        Math.max(1, line - 4),
      );
    };
    const renderContentSearch = () => {
      contentSearchButton.disabled = contentSearchPending;
      contentSearchButton.textContent = contentSearchPending
        ? uiText("搜索中…", "Searching…")
        : uiText("搜索", "Search");
      clearContentSearchButton.disabled =
        contentSearchPending && !contentSearchInput.value;
      clearContentSearchButton.textContent = uiText("清除", "Clear");
      contentSearchStatus.setAttribute(
        "role",
        contentSearchError ? "alert" : "status",
      );
      contentSearchStatus.setAttribute(
        "aria-live",
        contentSearchError ? "assertive" : "polite",
      );
      contentSearchResults.replaceChildren();
      if (contentSearchPending) {
        contentSearchStatus.textContent = uiText(
          "正在搜索当前文件夹及其子目录…",
          "Searching this folder and its descendants…",
        );
        return;
      }
      if (contentSearchError) {
        contentSearchStatus.textContent = contentSearchError;
        return;
      }
      const query = contentSearch.trim();
      if (!query) {
        contentSearchStatus.textContent = uiText(
          "内容搜索使用 Rust 正则；最多显示 80 个匹配。隐藏、敏感和部分生成目录不会被搜索。",
          "Content search uses Rust regex and shows at most 80 matches. Hidden, sensitive, and some generated directories are skipped.",
        );
        return;
      }
      const response = contentSearchResult;
      if (!response) {
        contentSearchStatus.textContent = uiText(
          "输入正则后按“搜索”。",
          "Enter a regular expression, then choose Search.",
        );
        return;
      }
      const matches = Array.isArray(response.matches) ? response.matches : [];
      const matchCount = Math.max(
        matches.length,
        Number(response.matchCount) || 0,
      );
      const fileCount = Math.max(0, Number(response.fileCount) || 0);
      if (!matches.length && matchCount === 0) {
        contentSearchStatus.textContent = uiText(
          "没有匹配项。请检查正则，或使用更宽泛的表达式。",
          "No matches. Check the regex or try a broader expression.",
        );
        return;
      }
      contentSearchStatus.textContent = response.hasMore
        ? uiText(
            `显示前 ${matches.length} / ${matchCount} 个匹配（${fileCount} 个文件）；请细化正则。`,
            `Showing the first ${matches.length} of ${matchCount} matches in ${fileCount} files; refine the regex.`,
          )
        : uiText(
            `找到 ${matchCount} 个匹配，涉及 ${fileCount} 个文件。`,
            `Found ${matchCount} matches in ${fileCount} files.`,
          );
      if (!matches.length) return;
      const fragment = document.createDocumentFragment();
      for (const match of matches) {
        const rawPath = match.path?.trim();
        if (!rawPath) continue;
        const path = normaliseWorkspacePath(rawPath);
        if (path === ".") continue;
        const line =
          Number.isSafeInteger(match.line) && Number(match.line) > 0
            ? Number(match.line)
            : 1;
        const result = document.createElement("button");
        result.type = "button";
        result.className = "tree-row";
        result.setAttribute(
          "aria-label",
          uiText(
            `打开 ${displayPath(path)} 第 ${line} 行`,
            `Open ${displayPath(path)} at line ${line}`,
          ),
        );
        const label = document.createElement("strong");
        label.textContent = `${displayPath(path)}:${line}`;
        const preview = document.createElement("span");
        preview.textContent = compactSearchLine(match.text);
        result.append(label, preview);
        result.addEventListener("click", () => openContentSearchMatch(match));
        fragment.appendChild(result);
      }
      contentSearchResults.appendChild(fragment);
    };
    const runContentSearch = async () => {
      const query = contentSearch.trim();
      if (!query) {
        contentSearchResult = undefined;
        contentSearchError = uiText(
          "请输入要搜索的正则表达式。",
          "Enter a regular expression to search.",
        );
        renderContentSearch();
        contentSearchInput.focus();
        return;
      }
      const request = ++contentSearchSerial;
      const searchDirectory = currentDirectory;
      contentSearchPending = true;
      contentSearchResult = undefined;
      contentSearchError = "";
      renderContentSearch();
      try {
        const { invoke, host } = requireDesktop();
        const capability = await host.issueWorkspaceCapability();
        const result = await invoke<FileSearchResponse>("fs_grep", {
          workdir: capability.workdir,
          path: searchDirectory,
          pattern: query,
          ignore_case: true,
          output_mode: "content",
          head_limit: FILE_CONTENT_SEARCH_LIMIT,
          offset: 0,
          context: 0,
          multiline: false,
          include_hidden: false,
          capability_token: capability.capabilityToken,
        });
        if (
          request !== contentSearchSerial ||
          searchDirectory !== currentDirectory
        )
          return;
        contentSearchResult = result;
      } catch (error) {
        if (
          request !== contentSearchSerial ||
          searchDirectory !== currentDirectory
        )
          return;
        contentSearchError = `${uiText("内容搜索失败：", "Content search failed: ")}${errorText(error).replace(/\s+/g, " ").trim().slice(0, 240)}`;
      } finally {
        if (
          request === contentSearchSerial &&
          searchDirectory === currentDirectory
        ) {
          contentSearchPending = false;
          if (contentSearchStatus.isConnected) renderContentSearch();
        }
      }
    };
    contentSearchInput.addEventListener("input", () => {
      contentSearch = contentSearchInput.value;
      contentSearchResult = undefined;
      contentSearchError = "";
      renderContentSearch();
    });
    contentSearchForm.addEventListener("submit", (event) => {
      event.preventDefault();
      contentSearch = contentSearchInput.value;
      void runContentSearch();
    });
    clearContentSearchButton.addEventListener("click", () => {
      clearContentSearch();
      contentSearchInput.value = "";
      renderContentSearch();
      contentSearchInput.focus();
    });

    const list = document.createElement("div");
    list.className = "file-browser-list";
    const rowCache = new Map<string, HTMLButtonElement>();
    let filterTimer: number | undefined;
    const rowForEntry = (entry: FileEntry, selected: boolean) => {
      const path = normaliseWorkspacePath(entry.path || entry.name);
      let row = rowCache.get(path);
      if (!row) {
        row = fileEntryRow(
          entry,
          (opened) => {
            if (opened.kind === "dir")
              void loadDirectory(opened.path || opened.name || ".");
            else void readFile(opened);
          },
          selected,
        );
        rowCache.set(path, row);
      } else {
        row.className = selected ? "tree-row is-selected" : "tree-row";
        if (selected) row.setAttribute("aria-current", "true");
        else row.removeAttribute("aria-current");
      }
      return row;
    };
    const updateList = () => {
      const needle = fileFilter.trim().toLocaleLowerCase();
      const entries = needle
        ? currentEntries.filter((entry) =>
            (entry.name || entry.path || "")
              .toLocaleLowerCase()
              .includes(needle),
          )
        : currentEntries;
      const folderCount = entries.filter(
        (entry) => entry.kind === "dir",
      ).length;
      treeCount.textContent = uiText(
        `${entries.length} 项 · ${folderCount} 个文件夹`,
        `${entries.length} items · ${folderCount} folders`,
      );
      list.replaceChildren(
        ...entries.map((entry) =>
          rowForEntry(
            entry,
            entry.kind !== "dir" &&
              normaliseWorkspacePath(entry.path || entry.name) ===
                normaliseWorkspacePath(previewEntry?.path || ""),
          ),
        ),
      );
      if (!entries.length) {
        const empty = document.createElement("p");
        empty.className = "file-browser-empty";
        empty.textContent = needle
          ? uiText(
              "当前目录没有匹配的资源。",
              "No matching resources in this folder.",
            )
          : uiText("当前文件夹为空。", "This folder is empty.");
        list.appendChild(empty);
      }
    };
    filter.addEventListener("input", () => {
      fileFilter = filter.value;
      if (filterTimer !== undefined) window.clearTimeout(filterTimer);
      filterTimer = window.setTimeout(() => {
        filterTimer = undefined;
        updateList();
      }, 100);
    });
    tree.append(
      treeHead,
      filter,
      contentSearchForm,
      contentSearchStatus,
      contentSearchResults,
      list,
    );

    const preview = document.createElement("section");
    preview.className = "file-browser-preview";
    const previewHead = document.createElement("div");
    previewHead.className = "file-browser-preview-head";
    const previewTitle = document.createElement("strong");
    previewTitle.textContent = previewEntry
      ? previewEntry.name || displayPath(previewEntry.path)
      : uiText("文件预览", "File preview");
    const previewState = document.createElement("span");
    previewState.textContent = previewEntry
      ? uiText("已打开", "Open")
      : uiText("未选择", "No selection");
    previewHead.append(previewTitle, previewState);
    if (previewEntry && previewContent) {
      const content = document.createElement("pre");
      content.className = "file-preview";
      const startLine = previewContent.startLine ?? 1;
      const numLines = previewContent.numLines ?? 0;
      const endLine = Math.max(startLine, startLine + numLines - 1);
      const location = previewContent.totalLines
        ? uiText(
            `第 ${startLine}–${endLine} 行，共 ${previewContent.totalLines} 行`,
            `Lines ${startLine}–${endLine} of ${previewContent.totalLines}`,
          )
        : uiText(`从第 ${startLine} 行开始`, `Starting at line ${startLine}`);
      content.textContent = `${displayPath(previewEntry.path || previewEntry.name || "")} · ${location}\n\n${previewContent.content}${previewContent.truncated ? `\n\n[${uiText("内容已截断", "Content truncated")}]` : ""}`;
      preview.append(previewHead, content);
    } else {
      const empty = document.createElement("p");
      empty.className = "file-browser-empty";
      empty.textContent = uiText(
        "从资源列表中选择一个文本文件以查看预览。",
        "Choose a text file from the resource list to preview it.",
      );
      preview.append(previewHead, empty);
    }
    updateList();
    renderContentSearch();
    layout.append(tree, preview);
    section.appendChild(layout);
  };

  const loadDirectory = async (relativePath = currentDirectory) => {
    currentDirectory = normaliseWorkspacePath(relativePath);
    previewEntry = undefined;
    previewContent = undefined;
    currentEntries = [];
    fileFilter = "";
    clearContentSearch();
    const current = ++serial;
    const pane = filePane();
    const section = sectionFor();
    if (!pane || !section) return;
    if (!canBrowseFiles(section)) return;
    renderToolbar(section, {
      loading: true,
      refresh: () => void loadDirectory(currentDirectory),
      navigate: (path) => void loadDirectory(path),
      goBack: () => void loadDirectory(parentWorkspacePath(currentDirectory)),
    });
    section.appendChild(
      fileMessage(uiText("正在读取当前项目文件…", "Reading project files…")),
    );
    try {
      const { invoke, host } = requireDesktop();
      const capability = await host.issueWorkspaceCapability();
      const entries = await invoke<FileEntry[]>("fs_list", {
        workdir: capability.workdir,
        path: currentDirectory,
        include_hidden: false,
        capability_token: capability.capabilityToken,
      });
      if (current !== serial) return;
      hasLoaded = true;
      const ordered = (Array.isArray(entries) ? entries : [])
        .slice()
        .sort((left, right) => {
          const kind =
            Number(left.kind !== "dir") - Number(right.kind !== "dir");
          if (kind) return kind;
          return (left.name || left.path || "").localeCompare(
            right.name || right.path || "",
            undefined,
            { sensitivity: "base" },
          );
        });
      currentEntries = ordered;
      renderToolbar(section, {
        refresh: () => void loadDirectory(currentDirectory),
        navigate: (path) => void loadDirectory(path),
        goBack: () => void loadDirectory(parentWorkspacePath(currentDirectory)),
      });
      renderFileBrowser(section);
    } catch (error) {
      if (current === serial) {
        const settledToolbar = renderToolbar(section, {
          refresh: () => void loadDirectory(currentDirectory),
          navigate: (path) => void loadDirectory(path),
          goBack: () =>
            void loadDirectory(parentWorkspacePath(currentDirectory)),
        });
        renderFileError(section, errorText(error), settledToolbar);
      }
    }
  };
  const readFile = async (entry: FileEntry, requestedStartLine = 1) => {
    const path = normaliseWorkspacePath(entry.path || entry.name);
    const section = sectionFor();
    if (!section) return;
    if (!canBrowseFiles(section)) return;
    const startLine =
      Number.isSafeInteger(requestedStartLine) && requestedStartLine > 0
        ? requestedStartLine
        : 1;
    currentDirectory = parentWorkspacePath(path);
    previewEntry = { ...entry, path };
    previewContent = undefined;
    const current = ++serial;
    const refreshPreview = () =>
      void readFile(
        previewEntry ?? entry,
        previewContent?.startLine ?? startLine,
      );
    renderToolbar(section, {
      loading: true,
      refresh: refreshPreview,
      navigate: (nextPath) => void loadDirectory(nextPath),
      goBack: () => void loadDirectory(currentDirectory),
    });
    section.appendChild(
      fileMessage(uiText("正在读取文件预览…", "Reading file preview…")),
    );
    try {
      const { invoke, host } = requireDesktop();
      const capability = await host.issueWorkspaceCapability();
      const result = await invoke<FileReadResponse>("fs_read_text", {
        workdir: capability.workdir,
        path,
        start_line: startLine,
        limit: 240,
        capability_token: capability.capabilityToken,
      });
      if (current !== serial) return;
      const responseStartLine =
        typeof result.startLine === "number" &&
        Number.isSafeInteger(result.startLine) &&
        result.startLine > 0
          ? result.startLine
          : startLine;
      const responseNumLines =
        typeof result.numLines === "number" &&
        Number.isSafeInteger(result.numLines) &&
        result.numLines >= 0
          ? result.numLines
          : undefined;
      const responseTotalLines =
        typeof result.totalLines === "number" &&
        Number.isSafeInteger(result.totalLines) &&
        result.totalLines >= 0
          ? result.totalLines
          : undefined;
      previewContent = {
        content: typeof result.content === "string" ? result.content : "",
        truncated: Boolean(result.truncated),
        startLine: responseStartLine,
        numLines: responseNumLines,
        totalLines: responseTotalLines,
      };
      renderToolbar(section, {
        refresh: refreshPreview,
        navigate: (nextPath) => void loadDirectory(nextPath),
        goBack: () => void loadDirectory(currentDirectory),
      });
      renderFileBrowser(section);
    } catch (error) {
      if (current !== serial) return;
      const settledToolbar = renderToolbar(section, {
        refresh: refreshPreview,
        navigate: (nextPath) => void loadDirectory(nextPath),
        goBack: () => void loadDirectory(currentDirectory),
      });
      renderFileError(section, errorText(error), settledToolbar);
    }
  };
  const activateFiles = () => {
    if (!hasLoaded)
      window.setTimeout(() => void loadDirectory(currentDirectory), 0);
  };
  document.addEventListener("click", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    if (
      target?.closest<HTMLElement>(
        ".session[data-novavei-session], .project-row[data-workdir]",
      )
    ) {
      currentDirectory = ".";
      previewEntry = undefined;
      previewContent = undefined;
      currentEntries = [];
      fileFilter = "";
      clearContentSearch();
      hasLoaded = false;
      window.setTimeout(() => void loadDirectory("."), 0);
    }
  });
  window.addEventListener("novavei:dock-pane-activated", (event) => {
    if (!(event instanceof CustomEvent) || event.detail?.pane !== "files")
      return;
    activateFiles();
  });

  const resetForWorkdirChange = () => {
    const pane = filePane();
    clearContentSearch();
    if (!pane?.classList.contains("on")) return;
    hasLoaded = false;
    // A session/project switch can make the old relative directory invalid.
    // Return to the new project's root instead of issuing a stale path read.
    currentDirectory = ".";
    previewEntry = undefined;
    previewContent = undefined;
    currentEntries = [];
    fileFilter = "";
    window.setTimeout(() => void loadDirectory("."), 0);
  };
  window.addEventListener("novavei:workdir-changed", resetForWorkdirChange);
  window.addEventListener("novavei:host-state-changed", () => {
    // `workdir-changed` handles a successful directory switch. Keep this
    // separate event for initial hydration and unavailable host states.
    if (document.body.dataset.novaveiShellState !== "ready" || !hasLoaded)
      resetForWorkdirChange();
  });
  window.addEventListener("novavei:language-changed", () => {
    const pane = filePane();
    if (!pane?.classList.contains("on")) return;
    if (previewEntry)
      window.setTimeout(
        () =>
          void readFile(
            previewEntry as FileEntry,
            previewContent?.startLine ?? 1,
          ),
        0,
      );
    else window.setTimeout(() => void loadDirectory(currentDirectory), 0);
  });
  if (filePane()?.classList.contains("on")) activateFiles();
}

function gitPane() {
  return document.querySelector<HTMLElement>('.dock-pane[data-pane="git"]');
}

let gitRefreshSerial = 0;
let gitRefreshInFlight = false;
let gitCommitInFlight = false;
let gitCanCommit = false;

function gitControls(pane = gitPane()) {
  return {
    commit: pane?.querySelector<HTMLButtonElement>("[data-git-commit]"),
    refresh: pane?.querySelector<HTMLButtonElement>("[data-git-refresh]"),
  };
}

function updateGitControls() {
  const { commit, refresh } = gitControls();
  if (commit) {
    commit.disabled = gitRefreshInFlight || gitCommitInFlight || !gitCanCommit;
    commit.textContent = gitCommitInFlight
      ? uiText("正在提交…", "Committing…")
      : uiText("提交", "Commit");
  }
  if (refresh) {
    refresh.disabled = gitRefreshInFlight || gitCommitInFlight;
    refresh.textContent = gitRefreshInFlight
      ? uiText("正在刷新…", "Refreshing…")
      : uiText("刷新", "Refresh");
  }
}

function hideGitFeature() {
  gitRefreshSerial += 1;
  gitRefreshInFlight = false;
  gitCommitInFlight = false;
  gitCanCommit = false;
  const pane = gitPane();
  const tab = document.getElementById(
    "dock-tab-git",
  ) as HTMLButtonElement | null;
  const menuChoice = document.querySelector<HTMLElement>(
    '[data-dock-tool="git"]',
  );
  const wasActive = tab?.classList.contains("on") === true;
  if (tab) {
    tab.hidden = true;
    tab.disabled = true;
    tab.tabIndex = -1;
    tab.setAttribute("aria-hidden", "true");
  }
  if (menuChoice) menuChoice.hidden = true;
  if (pane) {
    pane.hidden = true;
    pane.classList.remove("on");
  }
  if (wasActive)
    document
      .getElementById("dock-tab-run")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
}

function gitUnavailableMessage(status: GitStatusResponse) {
  switch (status.unavailableReason) {
    case "repository_outside_workspace":
      return uiText(
        "当前打开的是仓库的子文件夹。为避免读取或提交项目外的改动，请从仓库根目录打开项目。",
        "This project is a subfolder of a repository. Open the repository root so NovaVei never reviews or commits sibling changes.",
      );
    case "not_repository":
      return uiText(
        "当前项目不是 Git 仓库。切换到 Git 项目后可在这里查看状态。",
        "This project is not a Git repository. Open a Git project to review its status here.",
      );
    default:
      return uiText("Git 当前不可用。", "Git is currently unavailable.");
  }
}

function gitStatusLabel(entry: GitStatusEntry) {
  const index = entry.indexStatus?.trim() || "";
  const worktree = entry.worktreeStatus?.trim() || "";
  if (index === "?" && worktree === "?") return uiText("未跟踪", "Untracked");
  const parts: string[] = [];
  if (index) parts.push(uiText(`已暂存 ${index}`, `Staged ${index}`));
  if (worktree)
    parts.push(uiText(`未暂存 ${worktree}`, `Unstaged ${worktree}`));
  return parts.join(" · ") || uiText("已变更", "Changed");
}

function renderGitStatus(pane: HTMLElement, status: GitStatusResponse) {
  const section = pane.querySelector<HTMLElement>(".section");
  const note = pane.querySelector<HTMLElement>(".dock-note");
  if (!section || !note) return;
  if (status.isRepository !== true) {
    note.textContent = uiText(
      "Git Review · 不可用",
      "Git Review · Unavailable",
    );
    section.replaceChildren(fileMessage(gitUnavailableMessage(status)));
    return;
  }
  const branch =
    status.branch?.trim() || uiText("未命名分支", "Unnamed branch");
  const ahead = Math.max(0, status.ahead ?? 0);
  const behind = Math.max(0, status.behind ?? 0);
  const staged = Math.max(0, status.stagedCount ?? 0);
  const unstaged = Math.max(0, status.unstagedCount ?? 0);
  const untracked = Math.max(0, status.untrackedCount ?? 0);
  const entries = Array.isArray(status.entries) ? status.entries : [];
  note.textContent = uiText("Git Review · 已连接", "Git Review · Connected");

  const summary = document.createElement("div");
  summary.className = "target";
  const badge = document.createElement("div");
  badge.className = "avatar";
  badge.textContent = "Git";
  const text = document.createElement("div");
  const title = document.createElement("strong");
  title.textContent = branch;
  const detail = document.createElement("span");
  const tracking = [
    ahead ? uiText(`领先 ${ahead}`, `Ahead ${ahead}`) : "",
    behind ? uiText(`落后 ${behind}`, `Behind ${behind}`) : "",
  ].filter(Boolean);
  detail.textContent = status.clean
    ? uiText("工作区干净", "Working tree clean")
    : uiText(
        `已暂存 ${staged} · 未暂存 ${unstaged} · 未跟踪 ${untracked}`,
        `Staged ${staged} · Unstaged ${unstaged} · Untracked ${untracked}`,
      );
  if (tracking.length) detail.textContent += ` · ${tracking.join(" · ")}`;
  text.append(title, detail);
  const state = document.createElement("span");
  state.className = `pill ${status.clean ? "ok" : "wait"}`;
  state.textContent = status.clean
    ? uiText("干净", "Clean")
    : uiText("有改动", "Changes");
  summary.append(badge, text, state);

  if (status.clean) {
    section.replaceChildren(
      summary,
      fileMessage(
        uiText("没有待提交的改动。", "There are no pending changes."),
      ),
    );
    return;
  }
  const list = document.createElement("ol");
  list.className = "trace";
  list.setAttribute("aria-label", uiText("Git 改动列表", "Git changes"));
  for (const entry of entries) {
    const item = document.createElement("li");
    const marker = document.createElement("i");
    marker.className = entry.indexStatus?.trim() ? "run" : "wait";
    marker.setAttribute("aria-hidden", "true");
    const body = document.createElement("span");
    const path = document.createElement("b");
    path.textContent = entry.path || uiText("未知路径", "Unknown path");
    const statusText = document.createElement("small");
    statusText.textContent = gitStatusLabel(entry);
    body.append(path, statusText);
    item.append(marker, body);
    list.appendChild(item);
  }
  section.replaceChildren(summary, list);
}

function isGitExecutableUnavailable(error: unknown) {
  return errorText(error)
    .toLocaleLowerCase()
    .includes("git executable is unavailable");
}

function gitAccessMessage() {
  switch (document.body.dataset.novaveiShellState) {
    case "loading":
      return uiText(
        "正在读取项目与会话，准备完成后即可查看 Git 状态。",
        "Loading projects and conversations. Git will be available when ready.",
      );
    case "needs_workspace":
      return uiText(
        "请先打开一个项目文件夹，再查看 Git 状态。",
        "Open a project folder before reviewing Git status.",
      );
    case "needs_session":
      return uiText(
        "请先创建或打开一个对话，再查看 Git 状态。",
        "Create or open a conversation before reviewing Git status.",
      );
    case "storage_recovery":
      return uiText(
        "本地存储需要恢复后才能执行 Git 操作。",
        "Recover local storage before running Git actions.",
      );
    case "error":
      return uiText(
        "本地项目暂不可用。请在对话区重试加载后再查看 Git。",
        "Local project data is unavailable. Retry loading from the conversation area first.",
      );
    default:
      return undefined;
  }
}

async function refreshGit() {
  const pane = gitPane();
  const section = pane?.querySelector<HTMLElement>(".section");
  const note = pane?.querySelector<HTMLElement>(".dock-note");
  if (!pane || !section || !note) return;
  const serial = ++gitRefreshSerial;
  gitRefreshInFlight = true;
  gitCanCommit = false;
  updateGitControls();
  // Match File Dock: never claim Git readiness before the host has a real
  // project/session. Browser preview keeps the static Git surface.
  if (!invokeApi() || !hostApi()) {
    gitRefreshInFlight = false;
    updateGitControls();
    return;
  }
  const unavailable = gitAccessMessage();
  if (unavailable) {
    note.textContent = "Git Review";
    section.replaceChildren(fileMessage(unavailable));
    gitRefreshInFlight = false;
    updateGitControls();
    return;
  }
  note.textContent = uiText("Git Review · 正在读取", "Git Review · Reading");
  section.replaceChildren(
    fileMessage(
      uiText(
        "正在读取当前项目的 Git 状态…",
        "Reading this project's Git status…",
      ),
    ),
  );
  try {
    const { invoke, host } = requireDesktop();
    const capability = await host.issueWorkspaceCapability();
    const status = await invoke<GitStatusResponse>("git_status", {
      workdir: capability.workdir,
      capability_token: capability.capabilityToken,
    });
    if (serial !== gitRefreshSerial || !pane.isConnected) return;
    gitCanCommit =
      status.isRepository === true && (status.stagedCount ?? 0) > 0;
    renderGitStatus(pane, status);
  } catch (error) {
    if (serial !== gitRefreshSerial || !pane.isConnected) return;
    if (isGitExecutableUnavailable(error)) {
      hideGitFeature();
      toast(
        uiText(
          "未检测到 Git，已隐藏 Git 模块。安装 Git 并重启 NovaVei 后会重新显示。",
          "Git is not installed, so the Git module was hidden. Install Git and restart NovaVei to restore it.",
        ),
      );
      return;
    }
    note.textContent = uiText(
      "Git Review · 无法读取",
      "Git Review · Unavailable",
    );
    section.replaceChildren(
      fileMessage(
        uiText(
          "无法读取当前项目的 Git 状态。",
          "Could not read this project's Git status.",
        ),
      ),
    );
    console.warn("[NovaVei Git] status request failed", error);
  } finally {
    if (serial === gitRefreshSerial) {
      gitRefreshInFlight = false;
      updateGitControls();
    }
  }
}

async function commitGit() {
  const unavailable = gitAccessMessage();
  if (unavailable) {
    await showAppError(unavailable, "Git 当前不可用");
    return;
  }
  if (!invokeApi() || !hostApi() || gitCommitInFlight) return;
  try {
    const { invoke, host } = requireDesktop();
    const capability = await host.issueWorkspaceCapability();
    const status = await invoke<GitStatusResponse>("git_status", {
      workdir: capability.workdir,
      capability_token: capability.capabilityToken,
    });
    if (status.isRepository !== true) {
      await refreshGit();
      await showAppError(gitUnavailableMessage(status), "Git 当前不可用");
      return;
    }
    if ((status.stagedCount ?? 0) < 1) {
      await showAppError(
        uiText(
          "没有已暂存的改动可以提交。Git 模块只会提交你已明确暂存的文件。",
          "There are no staged changes to commit. Git Review commits only files you explicitly staged.",
        ),
        uiText("没有可提交的改动", "Nothing to commit"),
      );
      return;
    }
    const message = await requestAppPrompt({
      title: uiText("创建 Git 提交", "Create Git commit"),
      message: uiText(
        "只会提交当前暂存区的文件。该项目配置的 Git hooks 可能会在提交期间运行。",
        "Only currently staged files will be committed. Git hooks configured by this project may run during the commit.",
      ),
      label: uiText("提交说明", "Commit message"),
      placeholder: uiText("说明这次改动", "Describe this change"),
      confirmLabel: uiText("继续", "Continue"),
      maxLength: 4000,
      multiline: true,
    });
    if (message === null) return;
    if (
      capability.sessionId &&
      host.getSessionId()?.trim() !== capability.sessionId
    ) {
      throw new Error("会话已切换；不会提交其他项目的暂存改动");
    }
    const commitGrant = await invoke<GitCommitCapabilityResponse>(
      "git_commit_capability_issue",
      {
        workdir: capability.workdir,
        message,
        capability_token: capability.capabilityToken,
      },
    );
    if (!commitGrant.grantToken) {
      throw new Error("Git 提交确认无效");
    }
    gitCommitInFlight = true;
    updateGitControls();
    const result = await invoke<GitCommitResponse>("git_commit", {
      workdir: commitGrant.workdir ?? capability.workdir,
      message,
      commit_token: commitGrant.grantToken,
    });
    toast(
      result.commitId
        ? uiText(
            `已创建提交 ${result.commitId}`,
            `Created commit ${result.commitId}`,
          )
        : uiText("已创建 Git 提交", "Created Git commit"),
    );
    await refreshGit();
  } catch (error) {
    console.warn("[NovaVei Git] commit request failed", error);
    await showAppError(
      errorText(error),
      uiText("Git 提交失败", "Git commit failed"),
    );
  } finally {
    gitCommitInFlight = false;
    updateGitControls();
  }
}

function installGit() {
  document.addEventListener(
    "click",
    (event) => {
      const target = event.target instanceof Element ? event.target : null;
      if (target?.closest("[data-git-refresh]")) {
        event.preventDefault();
        event.stopImmediatePropagation();
        void refreshGit();
      } else if (target?.closest("[data-git-commit]")) {
        event.preventDefault();
        event.stopImmediatePropagation();
        void commitGit();
      }
    },
    true,
  );
  const activateGit = () => window.setTimeout(() => void refreshGit(), 0);
  window.addEventListener("novavei:dock-pane-activated", (event) => {
    if (event instanceof CustomEvent && event.detail?.pane === "git")
      activateGit();
  });
  window.addEventListener("novavei:host-state-changed", () => {
    const pane = gitPane();
    if (!pane?.classList.contains("on")) return;
    window.setTimeout(() => void refreshGit(), 0);
  });
  window.addEventListener("novavei:workdir-changed", () => {
    const pane = gitPane();
    if (!pane?.classList.contains("on")) return;
    window.setTimeout(() => void refreshGit(), 0);
  });
  window.addEventListener("novavei:language-changed", () => {
    const pane = gitPane();
    if (!pane?.classList.contains("on")) return;
    window.setTimeout(() => void refreshGit(), 0);
  });
  if (gitPane()?.classList.contains("on")) activateGit();
}

function object(value: unknown): UnknownRecord {
  return asRecord(value) || {};
}

function selectedData(selector: string, attribute: string) {
  return document.querySelector<HTMLElement>(`${selector}.on`)?.dataset[
    attribute
  ];
}

function codePointCount(value: string) {
  return Array.from(value).length;
}

function truncateToCodePoints(value: string, maximum: number) {
  return Array.from(value).slice(0, maximum).join("");
}

function globalPromptCounterText(current: number) {
  const language = document.documentElement.lang.toLowerCase().startsWith("en")
    ? "en"
    : "zh";
  return language === "en"
    ? `${current} / ${MAX_GLOBAL_SYSTEM_PROMPT_CODE_POINTS} characters`
    : `${current} / ${MAX_GLOBAL_SYSTEM_PROMPT_CODE_POINTS} 个字符`;
}

const HISTORY_MESSAGE_PAGE_SIZES = new Set([40, 80, 120, 200]);
const DEFAULT_HISTORY_MESSAGE_PAGE_SIZE = 80;
const HISTORY_MESSAGE_PAGE_SIZE_STORAGE_KEY = "novavei.historyMessagePageSize";
const SECONDARY_LAUNCH_FOCUS_EXISTING = "focus-existing";
const SECONDARY_LAUNCH_NEW_WINDOW = "new-window";

function normalizeSecondaryLaunchBehavior(value: unknown) {
  return value === SECONDARY_LAUNCH_NEW_WINDOW
    ? SECONDARY_LAUNCH_NEW_WINDOW
    : SECONDARY_LAUNCH_FOCUS_EXISTING;
}

function applySecondaryLaunchBehavior(value: unknown) {
  const behavior = normalizeSecondaryLaunchBehavior(value);
  document
    .querySelectorAll<HTMLButtonElement>("[data-secondary-launch-behavior]")
    .forEach((button) => {
      const selected = button.dataset.secondaryLaunchBehavior === behavior;
      button.classList.toggle("on", selected);
      button.setAttribute("aria-checked", String(selected));
      button.tabIndex = selected ? 0 : -1;
    });
}

function normalizeHistoryMessagePageSize(value: unknown): number {
  const parsed =
    typeof value === "number"
      ? value
      : typeof value === "string"
        ? Number(value)
        : Number.NaN;
  if (
    Number.isFinite(parsed) &&
    HISTORY_MESSAGE_PAGE_SIZES.has(Math.trunc(parsed))
  ) {
    return Math.trunc(parsed);
  }
  return DEFAULT_HISTORY_MESSAGE_PAGE_SIZE;
}

function publishHistoryMessagePageSize(pageSize: number) {
  const normalized = normalizeHistoryMessagePageSize(pageSize);
  try {
    window.localStorage?.setItem(
      HISTORY_MESSAGE_PAGE_SIZE_STORAGE_KEY,
      String(normalized),
    );
  } catch {
    // localStorage may be unavailable in restricted WebViews.
  }
  window.dispatchEvent(
    new CustomEvent("novavei:history-page-size-changed", {
      detail: { pageSize: normalized },
    }),
  );
}

function currentSystemPayload(previous: UnknownRecord) {
  const picker = element<HTMLInputElement>("userColorPicker");
  const uiFont = element<HTMLSelectElement>("settingsUiFont");
  const codeFont = element<HTMLSelectElement>("settingsCodeFont");
  const globalSystemPrompt = element<HTMLTextAreaElement>("globalSystemPrompt");
  const historyMessagePageSize = element<HTMLSelectElement>(
    "historyMessagePageSize",
  );
  // The native boundary intentionally supports only exact session project
  // roots. Drop an old snake_case/"extra" value while saving so the settings
  // UI cannot recreate an unsupported filesystem policy.
  // Permission state is owned by permission-picker.ts. Leave it out of this
  // appearance patch so the native locked merge always retains the latest
  // permission-picker state.
  const system = { ...previous };
  delete system.workdir_policy;
  delete system.defaultPermissionTier;
  delete system.default_permission_tier;
  delete system.fullPermissionConfirmedRoots;
  delete system.secondary_launch_behavior;
  const scale = selectedData("[data-ui-scale]", "uiScale");
  const secondaryLaunchBehavior = normalizeSecondaryLaunchBehavior(
    selectedData(
      "[data-secondary-launch-behavior]",
      "secondaryLaunchBehavior",
    ) ?? system.secondaryLaunchBehavior,
  );
  const pageSize = normalizeHistoryMessagePageSize(
    historyMessagePageSize?.value ?? system.historyMessagePageSize,
  );
  return {
    ...system,
    theme:
      selectedData("[data-theme-opt]", "themeOpt") ||
      document.documentElement.dataset.theme ||
      "dark",
    language:
      selectedData("[data-lang-opt]", "langOpt") ||
      document.documentElement.lang ||
      "zh-CN",
    uiScale: Number(scale || 100),
    showShortcutHints: currentShortcutHintVisibility(),
    showFullMessageTimestamp: currentMessageTimestampPreference(),
    userAccent: picker?.value || "#0A84FF",
    uiFont: uiFont?.value || "system",
    codeFont: codeFont?.value || "system",
    workdirPolicy: "project",
    historyMessagePageSize: pageSize,
    globalSystemPrompt: globalSystemPrompt?.value ?? "",
    secondaryLaunchBehavior,
  };
}

function installSettings() {
  const invoke = invokeApi();
  if (!invoke) return;
  let previous: UnknownRecord = {};
  let hydrating = false;
  let saveTimer: number | undefined;
  let localSaveRevision = 0;
  let mergedSystemRevision = 0;
  // The desktop shell publishes app health before it treats local settings as
  // trustworthy. Keep this false until that authoritative write projection is
  // explicitly enabled; browser preview returns above and retains its static
  // controls unchanged.
  let settingsWritable = false;
  let settingsHydrated = false;
  let settingsLoadInFlight = false;
  const initialLocalSaveRevision = localSaveRevision;
  const initialMergedSystemRevision = mergedSystemRevision;
  const persistentSettingsControlSelector = [
    "[data-theme-opt]",
    "[data-lang-opt]",
    "[data-ui-scale]",
    "[data-user-color]",
    "[data-shortcut-hints-toggle]",
    "[data-message-timestamp-toggle]",
    "#userColorPicker",
    "#settingsUiFont",
    "#settingsCodeFont",
    "#workdirPolicy",
    "#historyMessagePageSize",
    "#globalSystemPrompt",
    "[data-secondary-launch-behavior]",
  ].join(", ");
  const setPersistentSettingsControlsEnabled = (enabled: boolean) => {
    document
      .querySelectorAll<
        | HTMLButtonElement
        | HTMLInputElement
        | HTMLSelectElement
        | HTMLTextAreaElement
      >(persistentSettingsControlSelector)
      .forEach((control) => {
        control.disabled = !enabled;
      });
  };
  const cancelPendingSave = () => {
    if (saveTimer === undefined) return;
    window.clearTimeout(saveTimer);
    saveTimer = undefined;
  };
  const updateGlobalPromptCounter = () => {
    const globalSystemPrompt =
      element<HTMLTextAreaElement>("globalSystemPrompt");
    const globalSystemPromptCount = element<HTMLElement>(
      "globalSystemPromptCount",
    );
    if (!globalSystemPrompt || !globalSystemPromptCount) return;
    if (
      codePointCount(globalSystemPrompt.value) >
      MAX_GLOBAL_SYSTEM_PROMPT_CODE_POINTS
    ) {
      globalSystemPrompt.value = truncateToCodePoints(
        globalSystemPrompt.value,
        MAX_GLOBAL_SYSTEM_PROMPT_CODE_POINTS,
      );
    }
    globalSystemPromptCount.textContent = globalPromptCounterText(
      codePointCount(globalSystemPrompt.value),
    );
  };
  window.addEventListener("novavei:language-changed", () => {
    applyShortcutHintVisibility(currentShortcutHintVisibility());
    updateGlobalPromptCounter();
  });
  window.addEventListener("novavei:system-settings-merged", (event) => {
    const detail = event instanceof CustomEvent ? object(event.detail) : {};
    previous = { ...previous, ...detail };
    mergedSystemRevision += 1;
  });
  const scheduleSave = () => {
    if (hydrating || !settingsWritable) return;
    cancelPendingSave();
    saveTimer = window.setTimeout(() => {
      saveTimer = undefined;
      if (!settingsWritable) return;
      const payload = currentSystemPayload(previous);
      previous = payload;
      localSaveRevision += 1;
      publishHistoryMessagePageSize(
        Number(payload.historyMessagePageSize) ||
          DEFAULT_HISTORY_MESSAGE_PAGE_SIZE,
      );
      void invoke("settings_save_system", { payload }).catch((error) => {
        if (settingsWritable) toast(errorText(error));
      });
    }, 180);
  };
  document.addEventListener("click", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    const launchBehaviorChoice = target?.closest<HTMLButtonElement>(
      "[data-secondary-launch-behavior]",
    );
    if (launchBehaviorChoice) {
      applySecondaryLaunchBehavior(
        launchBehaviorChoice.dataset.secondaryLaunchBehavior,
      );
      if (!hydrating) scheduleSave();
      return;
    }
    if (
      target?.closest(
        "[data-theme-opt], [data-lang-opt], [data-ui-scale], [data-user-color]",
      )
    ) {
      if (hydrating) return;
      window.setTimeout(scheduleSave, 0);
    }
  });
  document.addEventListener("keydown", (event) => {
    const target = event.target;
    if (!(target instanceof HTMLButtonElement)) return;
    if (!target.matches("[data-secondary-launch-behavior]")) return;
    const controls = [
      ...document.querySelectorAll<HTMLButtonElement>(
        "[data-secondary-launch-behavior]",
      ),
    ];
    const current = controls.indexOf(target);
    if (current < 0 || !controls.length) return;
    let next = current;
    if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      next = (current - 1 + controls.length) % controls.length;
    } else if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      next = (current + 1) % controls.length;
    } else if (event.key === "Home") {
      next = 0;
    } else if (event.key === "End") {
      next = controls.length - 1;
    } else {
      return;
    }
    event.preventDefault();
    controls[next]?.focus();
    controls[next]?.click();
  });
  document.addEventListener("change", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    if (target?.matches("[data-shortcut-hints-toggle]")) {
      const checked =
        target instanceof HTMLInputElement
          ? target.checked
          : currentShortcutHintVisibility();
      applyShortcutHintVisibility(checked);
      scheduleSave();
    } else if (target?.matches("[data-message-timestamp-toggle]")) {
      const checked =
        target instanceof HTMLInputElement
          ? target.checked
          : currentMessageTimestampPreference();
      applyMessageTimestampPreference(checked);
      scheduleSave();
    } else if (
      target?.matches(
        "#userColorPicker, #settingsUiFont, #settingsCodeFont, #workdirPolicy, #historyMessagePageSize",
      )
    ) {
      scheduleSave();
    }
  });
  document.addEventListener("input", (event) => {
    const target =
      event.target instanceof HTMLTextAreaElement ? event.target : null;
    if (target?.id !== "globalSystemPrompt") return;
    updateGlobalPromptCounter();
    scheduleSave();
  });
  const hydrateSettings = () => {
    if (!settingsWritable || settingsHydrated || settingsLoadInFlight) return;
    settingsLoadInFlight = true;
    void invoke<UnknownRecord>("settings_load_all")
      .then((response) => {
        // A recovery event can arrive while the native read is in flight. Do
        // not apply an untrusted in-memory response after the UI has failed
        // closed.
        if (!settingsWritable) return;
        const loaded = object(response?.system);
        // A permission change can finish while this initial settings request is
        // in flight. Preserve the newer in-memory payload so a later appearance
        // save cannot silently drop newer permission-picker fields.
        previous =
          localSaveRevision !== initialLocalSaveRevision ||
          mergedSystemRevision !== initialMergedSystemRevision
            ? { ...loaded, ...previous }
            : loaded;
        const system = previous;
        hydrating = true;
        const theme = String(
          system.themePreference ??
            system.theme_pref ??
            system.theme ??
            "system",
        );
        document
          .querySelector<HTMLElement>(`[data-theme-opt="${CSS.escape(theme)}"]`)
          ?.click();
        const language = String(system.language ?? system.lang ?? "zh")
          .toLowerCase()
          .startsWith("en")
          ? "en"
          : "zh";
        document
          .querySelector<HTMLElement>(`[data-lang-opt="${language}"]`)
          ?.click();
        const scale = Number(system.uiScale ?? system.ui_scale ?? 100);
        document
          .querySelector<HTMLElement>(
            `[data-ui-scale="${Number.isFinite(scale) ? scale : 100}"]`,
          )
          ?.click();
        const showShortcutHints = system.showShortcutHints !== false;
        applyShortcutHintVisibility(showShortcutHints);
        const fullMessageTimestamp =
          system.showFullMessageTimestamp ??
          system.show_full_message_timestamp ??
          system.messageTimestampFormat ??
          system.message_timestamp_format;
        applyMessageTimestampPreference(
          fullMessageTimestamp === undefined
            ? currentFullMessageTimestampPreference()
            : normalizeFullMessageTimestampPreference(fullMessageTimestamp),
        );
        const accent =
          typeof system.userAccent === "string"
            ? system.userAccent
            : typeof system.accent === "string"
              ? system.accent
              : "#0A84FF";
        const picker = element<HTMLInputElement>("userColorPicker");
        if (picker && /^#[0-9a-f]{6}$/i.test(accent)) {
          picker.value = accent;
          picker.dispatchEvent(new Event("change", { bubbles: true }));
        }
        const uiFont = element<HTMLSelectElement>("settingsUiFont");
        if (uiFont && typeof system.uiFont === "string") {
          uiFont.value = system.uiFont;
          uiFont.dispatchEvent(new Event("change", { bubbles: true }));
        }
        const codeFont = element<HTMLSelectElement>("settingsCodeFont");
        if (codeFont && typeof system.codeFont === "string") {
          codeFont.value = system.codeFont;
          codeFont.dispatchEvent(new Event("change", { bubbles: true }));
        }
        const storedWorkdirPolicy =
          typeof system.workdirPolicy === "string"
            ? system.workdirPolicy.trim()
            : typeof system.workdir_policy === "string"
              ? system.workdir_policy.trim()
              : undefined;
        const needsWorkdirPolicyMigration = Boolean(
          storedWorkdirPolicy && storedWorkdirPolicy !== "project",
        );
        const policy = element<HTMLSelectElement>("workdirPolicy");
        if (policy) policy.value = "project";
        const historyMessagePageSizeSelect = element<HTMLSelectElement>(
          "historyMessagePageSize",
        );
        const historyPageSize = normalizeHistoryMessagePageSize(
          system.historyMessagePageSize ?? system.history_message_page_size,
        );
        if (historyMessagePageSizeSelect) {
          historyMessagePageSizeSelect.value = String(historyPageSize);
        }
        publishHistoryMessagePageSize(historyPageSize);
        const globalSystemPrompt =
          element<HTMLTextAreaElement>("globalSystemPrompt");
        if (globalSystemPrompt) {
          globalSystemPrompt.value =
            typeof system.globalSystemPrompt === "string"
              ? truncateToCodePoints(
                  system.globalSystemPrompt,
                  MAX_GLOBAL_SYSTEM_PROMPT_CODE_POINTS,
                )
              : "";
        }
        applySecondaryLaunchBehavior(system.secondaryLaunchBehavior);
        updateGlobalPromptCounter();
        hydrating = false;
        settingsHydrated = true;
        if (needsWorkdirPolicyMigration && settingsWritable) {
          // `extra` was once a UI-only setting. Convert it immediately rather
          // than leaving a saved value that native workspace commands reject.
          const payload = currentSystemPayload(previous);
          previous = payload;
          void invoke("settings_save_system", { payload })
            .then(() => {
              toast(
                document.documentElement.lang.toLowerCase().startsWith("en")
                  ? "Extra workspace paths are not supported; the policy was reset to project root only."
                  : "额外路径尚未启用，已恢复为“限制在项目根”。",
              );
            })
            .catch((error) => {
              if (settingsWritable) toast(errorText(error));
            });
        }
      })
      .catch((error) => {
        if (settingsWritable) toast(errorText(error));
      })
      .finally(() => {
        settingsLoadInFlight = false;
      });
  };
  const onAppHealthChanged = (event: Event) => {
    const health = event instanceof CustomEvent ? object(event.detail) : {};
    settingsWritable = health.writes === "enabled";
    setPersistentSettingsControlsEnabled(settingsWritable);
    if (!settingsWritable) {
      cancelPendingSave();
      settingsHydrated = false;
      return;
    }
    hydrateSettings();
  };
  // Native hydration is asynchronous. Start disabled so no settings load or
  // debounced save can race the app-health recovery decision.
  setPersistentSettingsControlsEnabled(false);
  window.addEventListener("novavei:app-health-changed", onAppHealthChanged);
}

function installTurnRefresh() {
  let lastTerminal = "";
  const runtime = window.__novaveiPiRuntime;
  if (!runtime?.subscribe) return;
  runtime.subscribe((state) => {
    const status = String((state as RuntimeState).status || "");
    const requestId = String((state as RuntimeState).requestId || "");
    if (!requestId || !["completed", "cancelled", "error"].includes(status))
      return;
    const key = `${requestId}:${status}`;
    if (key === lastTerminal) return;
    lastTerminal = key;
    void hostApi()
      ?.refreshSessions({ loadActive: false })
      .catch((error) => toast(errorText(error)));
  });
}

function formatArchivedTimestamp(
  updatedAt: number | undefined,
  fallback: string,
) {
  if (
    typeof updatedAt !== "number" ||
    !Number.isFinite(updatedAt) ||
    updatedAt <= 0
  )
    return fallback;
  try {
    const locale = document.documentElement.lang.toLowerCase().startsWith("en")
      ? "en-GB"
      : "zh-CN";
    return new Intl.DateTimeFormat(locale, {
      year: "numeric",
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    }).format(new Date(updatedAt));
  } catch {
    return fallback;
  }
}

function closeSettingsOverlay() {
  const closeButton = document.querySelector<HTMLButtonElement>(
    "#overlaySettings [data-close-overlay]",
  );
  if (closeButton) {
    closeButton.click();
    return;
  }
  document.getElementById("overlaySettings")?.classList.remove("show");
}

function installArchivedSettings() {
  // Keep Settings page bound to data-settings="archived" panel markup.
  if (!document.querySelector('.settings-panel[data-settings="archived"]'))
    return;
  const list = element<HTMLElement>("archivedConversationsList");
  const empty = element<HTMLElement>("archivedConversationsEmpty");
  const count = element<HTMLElement>("archivedConversationsCount");
  if (!list || !empty || !count) return;

  const archivedPendingIds = new Set<string>();

  const projectLabelFor = (cwd: string) => {
    const match = [
      ...document.querySelectorAll<HTMLElement>(".project-row[data-workdir]"),
    ].find((row) => (row.dataset.workdir || "") === cwd);
    return (
      match?.dataset.project ||
      match?.querySelector("strong")?.textContent?.trim() ||
      cwd ||
      archivedSettingsCopy().unknownProject
    );
  };

  const renderArchivedSettings = () => {
    const copy = archivedSettingsCopy();
    const host = hostApi();
    const sessions = host ? host.getSessions() : [];
    const archived = sessions.filter((session) => session.isArchived === true);
    count.textContent = copy.count(archived.length);
    list.setAttribute("aria-label", copy.listAria);
    list.replaceChildren();

    if (!archived.length) {
      empty.hidden = false;
      empty.textContent = copy.empty;
      return;
    }
    empty.hidden = true;

    for (const session of archived) {
      const row = document.createElement("article");
      row.className = "archived-row";
      row.dataset.sessionId = session.id;

      const copyBlock = document.createElement("div");
      copyBlock.className = "archived-row-copy";
      const title = document.createElement("strong");
      title.textContent = session.title || "新建对话";
      const project = document.createElement("small");
      project.textContent = projectLabelFor(session.cwd);
      const meta = document.createElement("span");
      meta.textContent = formatArchivedTimestamp(
        session.updatedAt,
        copy.unavailableDate,
      );
      copyBlock.append(title, project, meta);

      const actions = document.createElement("div");
      actions.className = "archived-row-actions";
      const pending = archivedPendingIds.has(session.id);

      const openButton = document.createElement("button");
      openButton.type = "button";
      openButton.className = "btn";
      openButton.setAttribute("data-archived-action", "open"); // data-archived-action="open"
      openButton.dataset.sessionId = session.id;
      openButton.textContent = copy.open;
      openButton.disabled = pending;

      const restoreButton = document.createElement("button");
      restoreButton.type = "button";
      restoreButton.className = "btn primary";
      restoreButton.setAttribute("data-archived-action", "restore"); // data-archived-action="restore"
      restoreButton.dataset.sessionId = session.id;
      restoreButton.textContent = copy.restore;
      restoreButton.disabled = pending;

      const deleteButton = document.createElement("button");
      deleteButton.type = "button";
      deleteButton.className = "btn danger";
      deleteButton.setAttribute("data-archived-action", "delete"); // data-archived-action="delete"
      deleteButton.dataset.sessionId = session.id;
      deleteButton.textContent = copy.delete;
      deleteButton.disabled = pending;

      actions.append(openButton, restoreButton, deleteButton);
      row.append(copyBlock, actions);
      list.appendChild(row);
    }
  };

  const runArchivedAction = async (action: string, id: string) => {
    const copy = archivedSettingsCopy();
    if (archivedPendingIds.has(id)) {
      toast(copy.busy);
      return;
    }
    const host = hostApi();
    const invoke = invokeApi();
    if (!host || !invoke) {
      toast(
        document.documentElement.lang.toLowerCase().startsWith("en")
          ? "Desktop runtime is required."
          : "需要桌面运行时",
      );
      return;
    }
    const session = host.getSessions().find((item) => item.id === id);
    const title = session?.title?.trim() || "当前会话";

    if (action === "open") {
      await host.selectSession(id);
      closeSettingsOverlay();
      toast(copy.opened);
      return;
    }

    archivedPendingIds.add(id);
    renderArchivedSettings();
    try {
      if (action === "restore") {
        await invoke("chat_history_set_archived", { id, isArchived: false });
        await host.refreshSessions({ loadActive: false });
        toast(copy.restored);
        return;
      }
      if (action === "delete") {
        const confirmed = await requestAppConfirm({
          title: copy.deleteTitle,
          message: copy.deleteMessage(title),
          confirmLabel: copy.delete,
          cancelLabel: copy.cancel,
          danger: true,
        });
        if (!confirmed) return;
        await invoke("chat_history_delete", { id });
        await host.refreshSessions({ loadActive: true });
        toast(copy.deleted);
      }
    } finally {
      archivedPendingIds.delete(id);
      renderArchivedSettings();
    }
  };

  document.addEventListener(
    "click",
    (event) => {
      const target = event.target instanceof Element ? event.target : null;
      const actionButton = target?.closest<HTMLElement>(
        "[data-archived-action]",
      );
      if (!actionButton) return;
      if (!actionButton.closest("#archivedConversationsList")) return;
      const action = actionButton.dataset.archivedAction?.trim();
      const id = actionButton.dataset.sessionId?.trim();
      if (!action || !id) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      void runArchivedAction(action, id).catch(async (error) => {
        await showAppError(error, archivedSettingsCopy().deleteTitle);
      });
    },
    true,
  );

  window.addEventListener("novavei:sessions-changed", () => {
    renderArchivedSettings();
  });
  window.addEventListener("novavei:language-changed", () => {
    renderArchivedSettings();
  });
  const unsubscribe = hostApi()?.onSessionsChanged?.(() => {
    renderArchivedSettings();
  });
  if (!unsubscribe) {
    renderArchivedSettings();
  }
}

export function installWorkbench() {
  installUnavailableFeatures();
  installSessionActions();
  installProjectConversationBulkActions();
  installArchivedSettings();
  installFiles();
  installGit();
  installSettings();
  installTurnRefresh();
}
