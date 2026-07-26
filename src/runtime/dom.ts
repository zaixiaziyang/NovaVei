import type { PiRuntimeController } from "./controller";
import type { ProjectPreferences } from "./host";
import { notifyTranscriptContentChanged } from "./chat-navigation";
import { renderComposerMessageMedia } from "./attachments";
import { renderMarkdown } from "./markdown";
import { displayPath } from "./path-display";
import {
  planExecutionFollowUpText,
  PlanConfirmationCards,
  stripPlanProtocolBlocks,
  type PlanConfirmationCardResult,
} from "./plan-confirmation";
import type {
  LiveTranscriptMessage,
  PermissionDecision,
  PiPlanConfirmation,
  PiPlanConfirmationDecision,
  PiReasoningLevel,
  PiRunInput,
  PiRuntimeState,
  PiSessionRunStateListener,
  PiToolState,
} from "./types";

type UnknownRecord = Record<string, unknown>;

export const LIVE_MARKDOWN_RENDER_INTERVAL_MS = 80;
const LIVE_CHROME_RENDER_INTERVAL_MS = 250;

export function liveMarkdownRenderDelay(
  lastRenderedAt: number,
  now = Date.now(),
) {
  const elapsed = Math.max(0, now - lastRenderedAt);
  return Math.max(0, LIVE_MARKDOWN_RENDER_INTERVAL_MS - elapsed);
}

function element<T extends HTMLElement>(id: string) {
  return document.getElementById(id) as T | null;
}

const SETTINGS_TABLISTS = [
  {
    tabSelector: "[data-system-tab]",
    panelSelector: "[data-system-panel]",
    tabDataKey: "systemTab",
    panelDataKey: "systemPanel",
    idPrefix: "system",
  },
  {
    tabSelector: "[data-tools-tab]",
    panelSelector: "[data-tools-panel]",
    tabDataKey: "toolsTab",
    panelDataKey: "toolsPanel",
    idPrefix: "tools",
  },
  {
    tabSelector: "[data-memory-tab]",
    panelSelector: "[data-memory-panel]",
    tabDataKey: "memoryTab",
    panelDataKey: "memoryPanel",
    idPrefix: "memory",
  },
  {
    tabSelector: "[data-council-tab]",
    panelSelector: "[data-council-panel]",
    tabDataKey: "councilTab",
    panelDataKey: "councilPanel",
    idPrefix: "council",
  },
] as const;

type SettingsTablistConfig = (typeof SETTINGS_TABLISTS)[number];

function tabIdentifierPart(value: string) {
  return value
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean)
    .map((part) => `${part[0]?.toUpperCase() ?? ""}${part.slice(1)}`)
    .join("");
}

function settingsTabValue(
  element: HTMLElement,
  dataKey: SettingsTablistConfig["tabDataKey" | "panelDataKey"],
) {
  return element.dataset[dataKey];
}

function selectSettingsTab(config: SettingsTablistConfig, name: string) {
  for (const tab of document.querySelectorAll<HTMLElement>(
    config.tabSelector,
  )) {
    const selected = settingsTabValue(tab, config.tabDataKey) === name;
    tab.classList.toggle("on", selected);
    tab.setAttribute("aria-selected", String(selected));
    tab.tabIndex = selected ? 0 : -1;
  }
  for (const panel of document.querySelectorAll<HTMLElement>(
    config.panelSelector,
  )) {
    const selected = settingsTabValue(panel, config.panelDataKey) === name;
    panel.classList.toggle("on", selected);
    panel.hidden = !selected;
  }
}

function syncSettingsTablist(config: SettingsTablistConfig) {
  const tabs = [...document.querySelectorAll<HTMLElement>(config.tabSelector)];
  const panels = [
    ...document.querySelectorAll<HTMLElement>(config.panelSelector),
  ];
  if (!tabs.length || !panels.length) return;
  for (const tab of tabs) {
    const name = settingsTabValue(tab, config.tabDataKey);
    if (!name) continue;
    const panel = panels.find(
      (candidate) => settingsTabValue(candidate, config.panelDataKey) === name,
    );
    if (!panel) continue;
    const identifier = tabIdentifierPart(name);
    if (!tab.id) tab.id = `${config.idPrefix}Tab${identifier}`;
    if (!panel.id) panel.id = `${config.idPrefix}Panel${identifier}`;
    tab.setAttribute("role", "tab");
    tab.setAttribute("aria-controls", panel.id);
    panel.setAttribute("role", "tabpanel");
    panel.setAttribute("aria-labelledby", tab.id);
  }
  const selected =
    tabs.find((tab) => tab.getAttribute("aria-selected") === "true") ??
    tabs.find((tab) => tab.classList.contains("on")) ??
    tabs[0];
  const selectedName = selected
    ? settingsTabValue(selected, config.tabDataKey)
    : undefined;
  if (selectedName) selectSettingsTab(config, selectedName);
}

export function installSettingsTablists() {
  const settingsStage = element<HTMLElement>("settingsStage");
  if (!settingsStage || settingsStage.dataset.novaveiTablistsBound === "true")
    return;
  settingsStage.dataset.novaveiTablistsBound = "true";
  const syncAll = () => {
    for (const config of SETTINGS_TABLISTS) syncSettingsTablist(config);
  };
  syncAll();
  settingsStage.addEventListener("click", (event) => {
    const target =
      event.target instanceof Element
        ? event.target.closest<HTMLElement>("[role=tab]")
        : null;
    if (!target) return;
    const config = SETTINGS_TABLISTS.find((candidate) =>
      target.matches(candidate.tabSelector),
    );
    if (!config) return;
    const name = settingsTabValue(target, config.tabDataKey);
    if (name) selectSettingsTab(config, name);
  });
  settingsStage.addEventListener("keydown", (event) => {
    if (event.defaultPrevented) return;
    const target =
      event.target instanceof Element
        ? event.target.closest<HTMLElement>("[role=tab]")
        : null;
    if (!target) return;
    const config = SETTINGS_TABLISTS.find((candidate) =>
      target.matches(candidate.tabSelector),
    );
    if (!config) return;
    const tabs = [
      ...settingsStage.querySelectorAll<HTMLButtonElement>(config.tabSelector),
    ];
    const current = tabs.indexOf(target as HTMLButtonElement);
    if (current < 0 || !tabs.length) return;
    let next = current;
    if (event.key === "ArrowRight") next = (current + 1) % tabs.length;
    else if (event.key === "ArrowLeft")
      next = (current - 1 + tabs.length) % tabs.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = tabs.length - 1;
    else return;
    event.preventDefault();
    const nextTab = tabs[next];
    nextTab.click();
    nextTab.focus();
  });
  new MutationObserver(syncAll).observe(settingsStage, {
    childList: true,
    subtree: true,
  });
}

function toast(message: string) {
  const target = element<HTMLElement>("toast");
  if (!target) {
    console.warn("[NovaVei Pi]", message);
    return;
  }
  target.textContent = message;
  target.classList.add("show");
  window.setTimeout(() => target.classList.remove("show"), 2200);
}

function modelLabel() {
  return (
    element<HTMLElement>("modelPickerName")?.textContent?.trim() || "未选择模型"
  );
}

function permissionLabel() {
  return (
    element<HTMLElement>("permissionLabel")?.textContent?.trim() || "请求批准"
  );
}

function nowLabel(value = new Date()) {
  return value.toLocaleTimeString([], { hour12: false });
}

const REASONING_LABELS: Record<PiReasoningLevel, string> = {
  off: "关闭",
  minimal: "最少",
  low: "轻度",
  medium: "中",
  high: "高",
  xhigh: "极高",
  max: "最高",
};

type AssistantPresentation = {
  model: string;
  reasoning: PiReasoningLevel;
  permission: string;
};

type LiveTurnContext = {
  sessionId: string;
  userMessageId: string;
  assistantMessageId: string;
  createdAt: number;
  displayText: string;
  presentation: AssistantPresentation;
  requestId?: string;
  turnId?: string;
  publishedUser?: {
    content: string;
    requestId?: string;
    turnId?: string;
    status: PiRuntimeState["status"];
  };
};

function reasoningLabel(value: PiReasoningLevel | undefined) {
  return value ? REASONING_LABELS[value] : "未记录";
}

function asRecord(value: unknown): UnknownRecord | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as UnknownRecord)
    : undefined;
}

function readString(record: UnknownRecord | undefined, ...keys: string[]) {
  for (const key of keys) {
    const value = record?.[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return undefined;
}

function positiveTokenCount(value: unknown) {
  const numeric = typeof value === "number" ? value : Number(value);
  return Number.isFinite(numeric) && numeric > 0
    ? Math.floor(numeric)
    : undefined;
}

/** Collapse whitespace and hard-cap length so UI chrome never expands with Bash/args. */
function displaySnippet(
  value: string | undefined | null,
  maxChars = 96,
): string {
  const compact = (value ?? "").replace(/\s+/g, " ").trim();
  if (!compact) return "";
  if (compact.length <= maxChars) return compact;
  if (maxChars <= 1) return "…";
  return `${compact.slice(0, Math.max(1, maxChars - 1))}…`;
}

function safePreview(value: unknown, depth = 0, maxChars = 96): string {
  if (value === undefined || value === null) return "";
  if (typeof value === "string") return displaySnippet(value, maxChars);
  if (typeof value === "number" || typeof value === "boolean")
    return String(value);
  if (depth > 2) return "[object]";
  if (Array.isArray(value)) {
    return displaySnippet(
      value
        .slice(0, 6)
        .map((item) => safePreview(item, depth + 1, Math.min(48, maxChars)))
        .join(", "),
      maxChars,
    );
  }
  const record = asRecord(value);
  if (!record) return displaySnippet(String(value), maxChars);
  const fields = Object.entries(record)
    .slice(0, 4)
    .map(([key, item]) => {
      const redacted =
        /api[-_]?key|authorization|token|secret|password|privateKey/i.test(key);
      return `${key}: ${redacted ? "[redacted]" : safePreview(item, depth + 1, Math.min(64, maxChars))}`;
    });
  return displaySnippet(`{ ${fields.join(", ")} }`, maxChars);
}

function safeRuntimeMessage(value: unknown) {
  // Native surfaces normally redact diagnostics before they reach the WebView,
  // but provider/tool errors can still contain echoed credential-like text.
  // Keep the recovery hint useful without making the UI a second disclosure
  // channel for a secret.
  return safePreview(value, 0, 160)
    .replace(
      /((?:api[-_ ]?key|authorization|token|secret|password)\s*(?:[:=]|bearer\s+))[^\s,;)}\]]+/gi,
      "$1[redacted]",
    )
    .replace(/\bsk-[A-Za-z0-9_-]{8,}\b/g, "[redacted]");
}

function toolArgumentSummary(tool: PiToolState, maxChars = 72) {
  const args = asRecord(tool.arguments);
  if (args) {
    const path = readString(args, "path", "file", "filePath", "file_path");
    if (path) return displaySnippet(displayPath(path), maxChars);
    const command = readString(args, "command", "cmd");
    if (command) return displaySnippet(command, maxChars);
    const query = readString(args, "query", "pattern");
    if (query) return displaySnippet(query, maxChars);
  }
  return safePreview(tool.arguments, 0, maxChars);
}

function toolStatus(tool: PiToolState) {
  const status = String(tool.status ?? "queued").toLowerCase();
  if (status === "completed" || status === "success" || status === "done")
    return "completed";
  if (status === "failed" || status === "error") return "failed";
  if (status === "cancelled" || status === "canceled") return "cancelled";
  if (status === "running" || status === "executing") return "running";
  return "queued";
}

function toolStatusLabel(tool: PiToolState) {
  switch (toolStatus(tool)) {
    case "completed":
      return "完成";
    case "failed":
      return "失败";
    case "cancelled":
      return "已取消";
    case "running":
      return "运行中";
    default:
      return "排队";
  }
}

function sortedTools(state: PiRuntimeState) {
  return Object.values(state.tools).sort((left, right) => {
    const leftTime = left.startedAt ?? Number.MAX_SAFE_INTEGER;
    const rightTime = right.startedAt ?? Number.MAX_SAFE_INTEGER;
    return leftTime - rightTime;
  });
}

function appendUserMessage(
  text: string,
  sessionId?: string,
  messageId?: string,
) {
  const axis =
    element<HTMLElement>("transcriptAxis") ??
    document.querySelector<HTMLElement>(".axis");
  if (!axis) return;
  const id =
    messageId ||
    `floor-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
  const message = document.createElement("div");
  message.className = "msg-user";
  message.dataset.floorId = id;
  message.dataset.messageId = id;
  if (messageId) message.dataset.liveMessageId = messageId;
  renderComposerMessageMedia(message, text, sessionId);
  axis.appendChild(message);
  window.__novaveiFloorNav?.refresh?.();
}

async function copyAssistantText(value: string) {
  const text = value.trim();
  if (!text) throw new Error("没有可复制内容");
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const fallback = document.createElement("textarea");
  fallback.value = text;
  fallback.style.position = "fixed";
  fallback.style.left = "-9999px";
  fallback.setAttribute("aria-hidden", "true");
  document.body.appendChild(fallback);
  try {
    fallback.select();
    if (!document.execCommand("copy")) throw new Error("系统剪贴板不可用");
  } finally {
    fallback.remove();
  }
}

type AssistantMessageOptions = {
  messageId?: string;
  liveMessageId?: string;
  replace?: HTMLElement | null;
};

/**
 * The live and historical transcript use the same accessible, collapsed
 * thinking panel. Historical callers pass the durable terminal value.
 */
export function createAssistantThinkingPanel(thinkingText = "") {
  const thinkingPanel = document.createElement("section");
  thinkingPanel.className = "assistant-thinking";
  thinkingPanel.dataset.piThinkingPanel = "true";
  const thinkingContentId = `pi-thinking-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
  const thinkingToggle = document.createElement("button");
  thinkingToggle.type = "button";
  thinkingToggle.className = "assistant-thinking-toggle";
  thinkingToggle.dataset.piThinkingToggle = "true";
  thinkingToggle.setAttribute("aria-expanded", "false");
  thinkingToggle.setAttribute("aria-controls", thinkingContentId);
  thinkingToggle.setAttribute("aria-label", "展开思考过程");
  const thinkingLabel = document.createElement("span");
  thinkingLabel.textContent = "思考过程";
  const thinkingChevron = document.createElement("span");
  thinkingChevron.className = "assistant-thinking-chevron";
  thinkingChevron.setAttribute("aria-hidden", "true");
  thinkingToggle.append(thinkingLabel, thinkingChevron);
  const thinkingContent = document.createElement("div");
  thinkingContent.className = "assistant-thinking-content";
  thinkingContent.dataset.piThinkingContent = "true";
  thinkingContent.id = thinkingContentId;
  thinkingContent.hidden = true;
  thinkingContent.setAttribute("role", "region");
  thinkingContent.setAttribute("aria-label", "模型思考过程");
  thinkingToggle.addEventListener("click", () => {
    const expanded = thinkingToggle.getAttribute("aria-expanded") === "true";
    thinkingToggle.setAttribute("aria-expanded", expanded ? "false" : "true");
    thinkingToggle.setAttribute(
      "aria-label",
      expanded ? "展开思考过程" : "收起思考过程",
    );
    thinkingContent.hidden = expanded;
  });
  thinkingPanel.append(thinkingToggle, thinkingContent);
  thinkingPanel.hidden = !thinkingText.trim();
  thinkingContent.textContent = thinkingText;
  return thinkingPanel;
}

function createAssistantMessage(
  presentation?: AssistantPresentation,
  options: AssistantMessageOptions = {},
): HTMLElement | null {
  const axis =
    element<HTMLElement>("transcriptAxis") ??
    document.querySelector<HTMLElement>(".axis");
  if (!axis && !options.replace?.isConnected) return null;
  const displayedModel = presentation?.model || modelLabel();
  const displayedReasoning = reasoningLabel(presentation?.reasoning);
  const displayedPermission = presentation?.permission || permissionLabel();
  const article = document.createElement("article");
  article.className = "msg-assistant";
  article.dataset.novaveiRuntime = "pi";
  if (options.messageId) article.dataset.messageId = options.messageId;
  if (options.liveMessageId)
    article.dataset.liveMessageId = options.liveMessageId;
  article.dataset.piModelLabel = displayedModel;
  article.dataset.piReasoningLabel = displayedReasoning;

  const who = document.createElement("div");
  who.className = "who";
  const name = document.createElement("b");
  name.textContent = "NovaVei";
  const badge = document.createElement("span");
  badge.className = "badge-soft";
  badge.dataset.piModel = "true";
  badge.hidden = true;
  badge.setAttribute("aria-hidden", "true");
  badge.textContent = `${displayedModel} · ${displayedPermission}`;
  who.append(name, badge);

  const thinkingPanel = createAssistantThinkingPanel();

  const text = document.createElement("div");
  text.className = "markdown-body";
  text.dataset.piText = "true";
  text.dataset.historyContent = "true";

  const actions = document.createElement("div");
  actions.className = "msg-actions";
  actions.hidden = true;
  actions.setAttribute("aria-hidden", "true");
  const copy = document.createElement("button");
  copy.type = "button";
  copy.textContent = "复制";
  copy.addEventListener("click", () => {
    const source = article.dataset.piSource ?? text.textContent ?? "";
    const previousLabel = copy.textContent;
    copy.disabled = true;
    copy.textContent = "复制中…";
    void copyAssistantText(source)
      .then(
        () => toast("已复制"),
        (error: unknown) => {
          console.warn("[NovaVei Pi] copy assistant response failed", error);
          toast(
            error instanceof Error && error.message === "没有可复制内容"
              ? error.message
              : "复制失败：请检查系统剪贴板",
          );
        },
      )
      .finally(() => {
        if (!copy.isConnected) return;
        copy.disabled = false;
        copy.textContent = previousLabel;
      });
  });
  const trace = document.createElement("button");
  trace.type = "button";
  trace.textContent = "查看轨迹";
  trace.addEventListener("click", () => {
    window.dispatchEvent(
      new CustomEvent("novavei:open-dock-tool", { detail: { pane: "run" } }),
    );
  });
  const retry = document.createElement("button");
  retry.type = "button";
  retry.textContent = "重试";
  retry.addEventListener("click", () => {
    const input = element<HTMLTextAreaElement>("composerInput");
    const prompt = article.dataset.piPrompt;
    if (input && prompt) {
      input.value = prompt;
      input.focus();
    }
  });
  const branch = document.createElement("button");
  branch.type = "button";
  branch.dataset.historyAction = "branch";
  branch.textContent = "分叉新对话";
  const meta = document.createElement("span");
  meta.className = "msg-meta";
  meta.dataset.piMeta = "true";
  meta.title = "模型与思考等级";
  const metaModel = document.createElement("b");
  metaModel.textContent = displayedModel;
  const separator = document.createElement("span");
  separator.className = "sep";
  separator.setAttribute("aria-hidden", "true");
  const metaPermission = document.createElement("span");
  metaPermission.textContent = displayedReasoning;
  const ended = document.createElement("time");
  ended.className = "msg-ended";
  ended.dataset.piEnded = "true";
  ended.textContent = "—";
  meta.append(metaModel, separator, metaPermission);
  actions.append(copy, trace, retry, branch, meta, ended);
  article.append(who, thinkingPanel, text, actions);
  if (options.replace?.isConnected) options.replace.replaceWith(article);
  else axis?.appendChild(article);
  notifyTranscriptContentChanged();
  return article;
}

function isAssistantLivePlaceholderStatus(status: PiRuntimeState["status"]) {
  return ["starting", "running", "waiting_permission", "cancelling"].includes(
    status,
  );
}

function isAssistantCompletedStatus(status: PiRuntimeState["status"]) {
  return status === "completed";
}

function toggleCompletionOnlyAssistantChrome(
  article: HTMLElement,
  assistantCompleted: boolean,
) {
  const actions = article.querySelector<HTMLElement>(".msg-actions");
  if (actions) {
    actions.hidden = !assistantCompleted;
    actions.setAttribute("aria-hidden", assistantCompleted ? "false" : "true");
  }
  const model = article.querySelector<HTMLElement>("[data-pi-model]");
  if (model) {
    model.hidden = !assistantCompleted;
    model.setAttribute("aria-hidden", assistantCompleted ? "false" : "true");
  }
}

const renderedThinkingText = new WeakMap<HTMLElement, string>();

/** Render streamed provider thoughts as plain text to avoid interpreting model output as HTML. */
function renderAssistantThinking(article: HTMLElement, thinkingText: string) {
  const thinkingPanel = article.querySelector<HTMLElement>(
    "[data-pi-thinking-panel]",
  );
  const thinkingToggle = article.querySelector<HTMLButtonElement>(
    "[data-pi-thinking-toggle]",
  );
  const thinkingContent = article.querySelector<HTMLElement>(
    "[data-pi-thinking-content]",
  );
  if (!thinkingPanel || !thinkingToggle || !thinkingContent) return;

  const hasThinking = Boolean(thinkingText.trim());
  thinkingPanel.hidden = !hasThinking;
  if (!hasThinking) {
    if (renderedThinkingText.get(thinkingContent))
      thinkingContent.textContent = "";
    renderedThinkingText.delete(thinkingContent);
    thinkingContent.hidden = true;
    thinkingToggle.setAttribute("aria-expanded", "false");
    thinkingToggle.setAttribute("aria-label", "展开思考过程");
    return;
  }

  const previous = renderedThinkingText.get(thinkingContent);
  if (previous === thinkingText) return;
  if (previous !== undefined && thinkingText.startsWith(previous)) {
    thinkingContent.append(
      document.createTextNode(thinkingText.slice(previous.length)),
    );
  } else {
    thinkingContent.textContent = thinkingText;
  }
  renderedThinkingText.set(thinkingContent, thinkingText);
}

function toolSummary(tools: Record<string, PiToolState>) {
  const entries = Object.values(tools);
  if (!entries.length) return "";
  const running = entries.filter((tool) =>
    ["running", "queued"].includes(toolStatus(tool)),
  ).length;
  const completed = entries.filter(
    (tool) => toolStatus(tool) === "completed",
  ).length;
  const failed = entries.filter((tool) => toolStatus(tool) === "failed").length;
  const cancelled = entries.filter(
    (tool) => toolStatus(tool) === "cancelled",
  ).length;
  const parts = [`${entries.length} 个工具`];
  if (running) parts.push(`${running} 运行中`);
  if (completed) parts.push(`${completed} 完成`);
  if (failed) parts.push(`${failed} 失败`);
  if (cancelled) parts.push(`${cancelled} 已取消`);
  return parts.join(" · ");
}

function createTraceItem(tool: PiToolState) {
  const item = document.createElement("li");
  item.dataset.piToolId = tool.id;
  const marker = document.createElement("i");
  const status = toolStatus(tool);
  if (status === "running") marker.className = "run";
  else if (status === "queued") marker.className = "wait";
  const content = document.createElement("span");
  content.className = "trace-copy";
  const title = document.createElement("b");
  title.textContent = tool.name || "工具";
  const detail = document.createElement("small");
  const argumentSnippet = toolArgumentSummary(tool, 96);
  const failure = tool.error
    ? safeRuntimeMessage(tool.error)
    : status === "failed"
      ? safeRuntimeMessage(tool.result)
      : "";
  const detailText = failure
    ? `${toolStatusLabel(tool)} · ${failure}`
    : `${toolStatusLabel(tool)}${argumentSnippet ? ` · ${argumentSnippet}` : ""}`;
  detail.textContent = detailText;
  detail.title = detailText;
  content.append(title, detail);
  item.append(marker, content);
  return item;
}

function createComposerStep(tool: PiToolState, index: number) {
  const item = document.createElement("li");
  const status = toolStatus(tool);
  item.className = `composer-run-step${status === "completed" ? " done" : status === "running" ? " current" : ""}`;
  item.dataset.piToolId = tool.id;
  const marker = document.createElement("span");
  marker.className = "run-step-marker";
  marker.textContent =
    status === "completed"
      ? "OK"
      : status === "failed"
        ? "!"
        : String(index + 1);
  const title = document.createElement("span");
  title.className = "run-step-title";
  const argumentSnippet = toolArgumentSummary(tool, 64);
  const titleText = `${tool.name || "工具"}${argumentSnippet ? ` · ${argumentSnippet}` : ""}`;
  title.textContent = titleText;
  title.title = titleText;
  const time = document.createElement("time");
  time.textContent = toolStatusLabel(tool);
  item.append(marker, title, time);
  return item;
}

function terminalComposerStep(state: PiRuntimeState) {
  const item = document.createElement("li");
  const cancellationFailed = state.status === "cancel_failed";
  const cancelling = state.status === "cancelling";
  const failed = state.status === "error" || cancellationFailed;
  const cancelled = state.status === "cancelled";
  item.className = `composer-run-step${failed ? " current" : state.status === "completed" ? " done" : ""}`;
  const marker = document.createElement("span");
  marker.className = "run-step-marker";
  marker.textContent = failed
    ? "!"
    : cancelled
      ? "-"
      : state.status === "completed"
        ? "OK"
        : "...";
  const title = document.createElement("span");
  title.className = "run-step-title";
  const titleText = cancellationFailed
    ? state.cancellationError || "停止请求失败，请再次尝试停止当前运行"
    : failed
      ? state.error || "Pi 运行失败"
      : cancelled
        ? "本轮运行已取消"
        : state.status === "completed"
          ? "模型响应完成"
          : cancelling
            ? "正在请求停止当前运行"
            : "模型生成中";
  title.textContent = displaySnippet(titleText, 120);
  title.title = titleText;
  const time = document.createElement("time");
  time.textContent = cancellationFailed
    ? "停止失败"
    : failed
      ? "失败"
      : cancelled
        ? "已取消"
        : state.status === "completed"
          ? "完成"
          : cancelling
            ? "取消中"
            : "进行中";
  item.append(marker, title, time);
  return item;
}

function renderComposerSteps(state: PiRuntimeState) {
  const steps = element<HTMLOListElement>("composerRunSteps");
  if (!steps || !state.requestId) return;
  steps.dataset.piRuntimeRequest = state.requestId;
  steps.replaceChildren(
    ...sortedTools(state).map(createComposerStep),
    terminalComposerStep(state),
  );
  const progress = element<HTMLElement>(
    "composerRun",
  )?.querySelector<HTMLElement>(".composer-run-track");
  const progressBar = progress?.querySelector<HTMLElement>("i");
  const tools = sortedTools(state);
  const completeCount = tools.filter((tool) =>
    ["completed", "failed", "cancelled"].includes(toolStatus(tool)),
  ).length;
  const total = Math.max(1, tools.length + 1);
  const current =
    state.status === "completed" ||
    state.status === "error" ||
    state.status === "cancelled"
      ? total
      : Math.min(total, completeCount + 1);
  if (progressBar)
    progressBar.style.width = `${Math.round((current / total) * 100)}%`;
  if (progress) {
    progress.setAttribute("aria-valuemax", String(total));
    progress.setAttribute("aria-valuenow", String(current));
  }
  const count = element<HTMLElement>("composerRun")?.querySelector<HTMLElement>(
    ".composer-run-count",
  );
  if (count) count.textContent = `第 ${current} / ${total} 步`;
}

function runDock() {
  return document.querySelector<HTMLElement>('.dock-pane[data-pane="run"]');
}

type SubagentTaskSummary = {
  id: string;
  sessionId: string;
  title: string;
  status: string;
  updatedAt: number;
};

type WorktreeReview = {
  taskId: string;
  baseCommit: string;
  digest: string;
  changedPaths: string[];
  patch: string;
};

type WorktreeReviewActions = {
  reviewsByTaskId: ReadonlyMap<string, WorktreeReview>;
  pendingTaskIds: ReadonlySet<string>;
  view: (task: SubagentTaskSummary) => void;
  apply: (task: SubagentTaskSummary) => void;
  discard: (task: SubagentTaskSummary) => void;
};

function subagentTaskSummary(value: unknown): SubagentTaskSummary | undefined {
  const record = asRecord(value);
  const id = readString(record, "id");
  const sessionId = readString(record, "sessionId", "session_id");
  const title = readString(record, "title");
  const status = readString(record, "status");
  const updatedAt = finiteNumber(record?.updatedAt ?? record?.updated_at);
  if (!id || !sessionId || !title || !status || updatedAt === undefined)
    return undefined;
  return { id, sessionId, title, status, updatedAt };
}

function worktreeReview(value: unknown): WorktreeReview | undefined {
  const record = asRecord(value);
  const taskId = readString(record, "taskId", "task_id");
  const baseCommit = readString(record, "baseCommit", "base_commit");
  const digest = readString(record, "digest");
  const patch = typeof record?.patch === "string" ? record.patch : undefined;
  const rawPaths = record?.changedPaths ?? record?.changed_paths;
  const changedPaths = Array.isArray(rawPaths)
    ? rawPaths.filter(
        (path): path is string =>
          typeof path === "string" &&
          Boolean(path.trim()) &&
          !path.startsWith("/") &&
          !/^[A-Za-z]:[\\/]/.test(path) &&
          !path.split(/[\\/]/).includes(".."),
      )
    : [];
  if (!taskId || !baseCommit || !digest || patch === undefined)
    return undefined;
  return { taskId, baseCommit, digest, changedPaths, patch };
}

function subagentTaskStatusLabel(status: string) {
  switch (status) {
    case "queued":
      return "排队";
    case "starting":
      return "启动中";
    case "running":
      return "研究中";
    case "waiting_permission":
      return "等待批准";
    case "awaiting_worktree_approval":
      return "等待工作树批准";
    case "review_ready":
      return "待审阅";
    case "cleanup_pending":
      return "等待清理";
    case "completed":
      return "已完成";
    case "cancelled":
      return "已取消";
    case "failed":
    case "interrupted":
      return "失败";
    default:
      return "处理中";
  }
}

function renderSubagentTasks(
  pane: HTMLElement,
  tasks: readonly SubagentTaskSummary[],
  worktreeActions?: WorktreeReviewActions,
) {
  const section = pane.querySelector<HTMLElement>("[data-subagent-tasks]");
  if (!section) return;
  const heading = document.createElement("div");
  heading.className = "section-head";
  const title = document.createElement("h4");
  title.textContent = "子代理任务";
  const pill = document.createElement("span");
  const activeCount = tasks.filter((task) =>
    ["queued", "starting", "running", "waiting_permission"].includes(
      task.status,
    ),
  ).length;
  const failedCount = tasks.filter((task) =>
    ["failed", "interrupted"].includes(task.status),
  ).length;
  pill.className = `pill${failedCount ? " warn" : activeCount ? "" : " wait"}`;
  pill.textContent = activeCount
    ? `${activeCount} 运行`
    : failedCount
      ? `${failedCount} 失败`
      : tasks.length
        ? `${tasks.length} 已记录`
        : "无记录";
  heading.append(title, pill);
  if (!tasks.length) {
    const empty = document.createElement("p");
    empty.className = "dock-note";
    empty.textContent =
      "只读研究和隔离工作树任务会显示在这里。工作树补丁须经人工审阅与原生确认后才可应用。";
    section.replaceChildren(heading, empty);
    return;
  }
  const rows = [...tasks]
    .sort((left, right) => right.updatedAt - left.updatedAt)
    .slice(0, 12)
    .map((task) => {
      const item = document.createElement("div");
      item.className = "subagent-task";
      const row = document.createElement("div");
      row.className = "file-row";
      row.dataset.subagentTaskId = task.id;
      const marker = document.createElement("span");
      marker.setAttribute("aria-hidden", "true");
      marker.textContent = "·";
      const taskTitle = document.createElement("strong");
      taskTitle.textContent = displaySnippet(task.title, 96);
      const state = document.createElement("span");
      state.className = `delta${["failed", "interrupted"].includes(task.status) ? " minus" : ""}`;
      state.textContent = subagentTaskStatusLabel(task.status);
      row.append(marker, taskTitle, state);
      item.append(row);

      const review = worktreeActions?.reviewsByTaskId.get(task.id);
      const pending = worktreeActions?.pendingTaskIds.has(task.id) ?? false;
      const canReview = task.status === "review_ready";
      const canCleanup = task.status === "cleanup_pending";
      if ((canReview || canCleanup) && worktreeActions) {
        const actions = document.createElement("div");
        actions.className = "row-actions";
        if (canReview) {
          const view = document.createElement("button");
          view.type = "button";
          view.className = "btn ghost";
          view.textContent = review ? "刷新补丁" : "查看补丁";
          view.disabled = pending;
          view.addEventListener("click", () => worktreeActions.view(task));
          const apply = document.createElement("button");
          apply.type = "button";
          apply.className = "btn primary";
          apply.textContent = "应用已审阅补丁";
          apply.disabled = pending || !review || review.taskId !== task.id;
          apply.addEventListener("click", () => worktreeActions.apply(task));
          actions.append(view, apply);
        }
        const discard = document.createElement("button");
        discard.type = "button";
        discard.className = "btn ghost";
        discard.textContent = canCleanup ? "清理工作树" : "丢弃工作树";
        discard.disabled = pending;
        discard.addEventListener("click", () => worktreeActions.discard(task));
        actions.append(discard);
        item.append(actions);
      }
      if (review && canReview) {
        const preview = document.createElement("div");
        preview.className = "code-block";
        const header = document.createElement("header");
        const summary = document.createElement("span");
        summary.textContent = `基准 ${review.baseCommit.slice(0, 12)} · ${review.changedPaths.length} 个文件`;
        const digest = document.createElement("span");
        digest.textContent = `SHA-256 ${review.digest}`;
        header.append(summary, digest);
        const patch = document.createElement("pre");
        // Patch content is an explicit user-requested review artifact. Using
        // textContent prevents diff lines from becoming executable markup.
        patch.textContent = review.patch;
        preview.append(header, patch);
        item.append(preview);
      }
      return item;
    });
  section.replaceChildren(heading, ...rows);
}

function renderRunDock(
  state: PiRuntimeState,
  startedAt: number,
  subagentTasks: readonly SubagentTaskSummary[] = [],
  worktreeActions?: WorktreeReviewActions,
) {
  const pane = runDock();
  if (!pane) return;
  if (!state.requestId) {
    const trace = pane.querySelector<HTMLOListElement>(".trace");
    if (trace) {
      const item = document.createElement("li");
      const marker = document.createElement("i");
      marker.className = "wait";
      const content = document.createElement("span");
      const title = document.createElement("b");
      title.textContent = "尚无运行轨迹";
      const detail = document.createElement("small");
      detail.textContent = "发送消息后显示";
      content.append(title, detail);
      item.append(marker, content);
      trace.replaceChildren(item);
      trace.removeAttribute("data-pi-runtime-request");
    }
    const summary = pane.querySelector<HTMLElement>("[data-run-summary]");
    summary
      ?.querySelector<HTMLElement>(".pill")
      ?.replaceChildren(document.createTextNode("待命"));
    const values = summary
      ? [...summary.querySelectorAll<HTMLElement>("dd")]
      : [];
    if (values[0]) values[0].textContent = "—";
    if (values[1]) values[1].textContent = "—";
    if (values[2]) values[2].textContent = "—";
    renderContextSection(
      pane.querySelector<HTMLElement>("[data-run-context]") ?? undefined,
      state,
    );
    const impactSection = pane.querySelector<HTMLElement>("[data-run-impact]");
    if (impactSection) {
      const heading = document.createElement("div");
      heading.className = "section-head";
      const title = document.createElement("h4");
      title.textContent = "文件影响";
      const pill = document.createElement("span");
      pill.className = "pill wait";
      pill.textContent = "无记录";
      heading.append(title, pill);
      const empty = document.createElement("p");
      empty.className = "dock-note";
      empty.textContent = "本轮尚无文件影响记录";
      impactSection.replaceChildren(heading, empty);
    }
    renderSubagentTasks(pane, subagentTasks, worktreeActions);
    return;
  }
  const trace = pane.querySelector<HTMLOListElement>(".trace");
  if (trace) {
    trace.dataset.piRuntimeRequest = state.requestId;
    const tools = sortedTools(state);
    const rows = tools.map(createTraceItem);
    if (
      !tools.length ||
      [
        "error",
        "cancelled",
        "completed",
        "cancelling",
        "cancel_failed",
      ].includes(state.status)
    ) {
      rows.push(
        createTraceItem({
          id: `${state.requestId}-terminal`,
          name:
            state.status === "error"
              ? "运行失败"
              : state.status === "cancel_failed"
                ? "停止失败"
                : state.status === "cancelling"
                  ? "正在取消"
                  : state.status === "cancelled"
                    ? "取消运行"
                    : "模型响应",
          status:
            state.status === "error" || state.status === "cancel_failed"
              ? "failed"
              : state.status === "cancelled"
                ? "cancelled"
                : state.status === "completed"
                  ? "completed"
                  : "running",
          ...(state.error || state.cancellationError
            ? { error: state.error ?? state.cancellationError }
            : {}),
        }),
      );
    }
    trace.replaceChildren(...rows);
    trace.setAttribute("aria-live", "polite");
  }

  const summary = pane.querySelector<HTMLElement>("[data-run-summary]");
  const tools = sortedTools(state);
  const running = tools.filter((tool) =>
    ["running", "queued"].includes(toolStatus(tool)),
  ).length;
  const completed = tools.filter(
    (tool) => toolStatus(tool) === "completed",
  ).length;
  const failed = tools.filter((tool) => toolStatus(tool) === "failed").length;
  const cancelled = tools.filter(
    (tool) => toolStatus(tool) === "cancelled",
  ).length;
  const summaryPill = summary?.querySelector<HTMLElement>(".pill");
  if (summaryPill) {
    const parts = [`${completed} 完成`];
    if (failed) parts.push(`${failed} 失败`);
    if (cancelled) parts.push(`${cancelled} 已取消`);
    if (running) parts.push(`${running} 运行`);
    summaryPill.replaceChildren(document.createTextNode(parts.join(" / ")));
    summaryPill.classList.toggle(
      "warn",
      failed > 0 ||
        state.status === "error" ||
        state.status === "cancel_failed",
    );
  }
  const values = summary
    ? [...summary.querySelectorAll<HTMLElement>("dd")]
    : [];
  if (values[0])
    values[0].textContent = `${Math.max(0, Math.round((Date.now() - startedAt) / 1000))} s`;
  if (values[1])
    values[1].textContent = state.pendingPermission
      ? "等待批准"
      : permissionLabel();
  const writes = tools.filter((tool) =>
    /^(Write|Edit|Delete|Bash|ManagedProcess)$/i.test(tool.name),
  ).length;
  if (values[2]) values[2].textContent = `${writes} 写`;

  const contextSection =
    pane.querySelector<HTMLElement>("[data-run-context]") ?? undefined;
  renderContextSection(contextSection, state);

  const impactSection = pane.querySelector<HTMLElement>("[data-run-impact]");
  if (impactSection) {
    const fileRows = tools
      .map((tool) => ({ tool, args: asRecord(tool.arguments) }))
      .map(({ tool, args }) => ({
        tool,
        path: readString(args, "path", "file", "filePath", "file_path"),
      }))
      .filter((entry): entry is { tool: PiToolState; path: string } =>
        Boolean(entry.path),
      );
    const heading = document.createElement("div");
    heading.className = "section-head";
    const title = document.createElement("h4");
    title.textContent = "文件影响";
    const pill = document.createElement("span");
    pill.className = `pill${fileRows.length && writes ? " warn" : " wait"}`;
    pill.textContent = fileRows.length
      ? writes
        ? `${writes} 写入`
        : "只读"
      : "无记录";
    heading.append(title, pill);
    const rows = fileRows.map(({ tool, path }) => {
      const row = document.createElement("div");
      row.className = "file-row";
      const marker = document.createElement("span");
      marker.setAttribute("aria-hidden", "true");
      marker.textContent = "·";
      const name = document.createElement("strong");
      name.textContent = displayPath(path);
      const kind = document.createElement("span");
      kind.className = `delta${/^(Write|Edit|Delete)$/i.test(tool.name) ? " minus" : ""}`;
      kind.textContent = /^(Write|Edit|Delete)$/i.test(tool.name)
        ? "write"
        : "read";
      row.append(marker, name, kind);
      return row;
    });
    if (rows.length) impactSection.replaceChildren(heading, ...rows);
    else {
      const empty = document.createElement("p");
      empty.className = "dock-note";
      empty.textContent = "本轮尚无文件影响记录";
      impactSection.replaceChildren(heading, empty);
    }
  }
  renderSubagentTasks(pane, subagentTasks, worktreeActions);
}

function ensurePermissionPrompt(
  onDecision: (decision: PermissionDecision) => Promise<void>,
) {
  const composerRun = element<HTMLElement>("composerRun");
  if (!composerRun) return null;
  const existing = composerRun.querySelector<HTMLElement>(
    "[data-pi-permission-prompt]",
  );
  if (existing) return existing;
  const prompt = document.createElement("div");
  prompt.dataset.piPermissionPrompt = "true";
  prompt.className = "composer-permission-prompt";
  prompt.hidden = true;
  const message = document.createElement("p");
  message.className = "composer-permission-message";
  message.dataset.piPermissionMessage = "true";
  const detail = document.createElement("p");
  detail.className = "composer-permission-detail";
  detail.dataset.piPermissionDetail = "true";
  detail.hidden = true;
  const actions = document.createElement("div");
  actions.className = "row-actions composer-permission-actions";
  actions.dataset.piPermissionActions = "true";
  const choices: Array<[PermissionDecision, string, string]> = [
    ["allow", "允许一次", "primary"],
    ["deny", "拒绝", ""],
    ["cancel", "取消运行", "ghost"],
  ];
  for (const [decision, label, style] of choices) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = `btn${style ? ` ${style}` : ""}`;
    button.dataset.piPermissionDecision = decision;
    button.textContent = label;
    button.addEventListener("click", () => {
      if (permissionDecisionsBlockedByRecovery()) {
        hidePermissionPromptForRecovery(prompt);
        toast(STORAGE_RECOVERY_PERMISSION_MESSAGE);
        return;
      }
      setPermissionDecisionControls(actions, true);
      void onDecision(decision).catch((error) => {
        if (permissionDecisionsBlockedByRecovery())
          hidePermissionPromptForRecovery(prompt);
        else setPermissionDecisionControls(actions, false);
        toast(error instanceof Error ? error.message : String(error));
      });
    });
    actions.appendChild(button);
  }
  prompt.append(message, detail, actions);
  composerRun.appendChild(prompt);
  return prompt;
}

function setPermissionDecisionControls(root: ParentNode, disabled: boolean) {
  for (const button of root.querySelectorAll<HTMLButtonElement>(
    "[data-pi-permission-decision]",
  ))
    button.disabled = disabled;
}

function hidePermissionPromptForRecovery(prompt: HTMLElement | null) {
  if (!prompt) return;
  prompt.hidden = true;
  prompt.setAttribute("aria-hidden", "true");
  prompt.dataset.piPermissionBlocked = "recovery";
  setPermissionDecisionControls(prompt, true);
}

function permissionArgumentSnippet(value: unknown): string {
  const record = asRecord(value);
  if (record) {
    const command = readString(record, "command", "cmd");
    if (command) return displaySnippet(command, 88);
    const path = readString(record, "path", "file", "filePath", "file_path");
    if (path) return displaySnippet(displayPath(path), 88);
  }
  return safePreview(value, 0, 88);
}

function renderPermissionPrompt(
  prompt: HTMLElement | null,
  state: PiRuntimeState,
) {
  if (!prompt) return;
  // Only show the approve/deny chrome while a live turn is actually blocked on
  // a permission answer. Idle / completed runs must never leave the buttons
  // visible under "等待发送消息".
  const pending =
    state.status === "waiting_permission" && state.pendingPermission
      ? state.pendingPermission
      : undefined;
  const blockedByRecovery = permissionDecisionsBlockedByRecovery();
  const shouldShow = Boolean(pending) && !blockedByRecovery;
  prompt.hidden = !shouldShow;
  prompt.setAttribute("aria-hidden", shouldShow ? "false" : "true");
  if (blockedByRecovery) {
    prompt.dataset.piPermissionBlocked = "recovery";
    setPermissionDecisionControls(prompt, true);
    return;
  }
  if (!pending) {
    delete prompt.dataset.piPermissionId;
    delete prompt.dataset.piPermissionBlocked;
    setPermissionDecisionControls(prompt, true);
    const message = prompt.querySelector<HTMLElement>(
      "[data-pi-permission-message]",
    );
    const detailNode = prompt.querySelector<HTMLElement>(
      "[data-pi-permission-detail]",
    );
    if (message) {
      message.textContent = "";
      message.removeAttribute("title");
    }
    if (detailNode) {
      detailNode.hidden = true;
      detailNode.textContent = "";
      detailNode.removeAttribute("title");
    }
    return;
  }
  if (prompt.dataset.piPermissionId !== pending.id) {
    prompt.dataset.piPermissionId = pending.id;
    delete prompt.dataset.piPermissionBlocked;
    setPermissionDecisionControls(prompt, false);
  }
  const message = prompt.querySelector<HTMLElement>(
    "[data-pi-permission-message]",
  );
  const detailNode = prompt.querySelector<HTMLElement>(
    "[data-pi-permission-detail]",
  );
  const tool = pending.toolName || "工具";
  const delegatedGlobalRead =
    ["delegatereadonly", "delegateworktree"].includes(
      tool.trim().toLowerCase(),
    ) &&
    typeof pending.arguments === "object" &&
    pending.arguments !== null &&
    !Array.isArray(pending.arguments) &&
    ((pending.arguments as Record<string, unknown>).allow_global_read ===
      true ||
      (pending.arguments as Record<string, unknown>).allowGlobalRead === true);
  const description = displaySnippet(
    pending.description || "该工具需要访问工作区",
    64,
  );
  const argumentSnippet = permissionArgumentSnippet(pending.arguments);
  const allowButton = prompt.querySelector<HTMLButtonElement>(
    '[data-pi-permission-decision="allow"]',
  );
  if (message) {
    const prompt = delegatedGlobalRead
      ? "子代理请求全局只读访问（仅本次任务）"
      : `${tool} 请求访问本地工作区`;
    message.textContent = prompt;
    message.title = pending.description || prompt;
  }
  if (allowButton) {
    const label = delegatedGlobalRead ? "确认本次全局只读" : "允许一次";
    allowButton.textContent = label;
    allowButton.title = delegatedGlobalRead
      ? "仅允许此子代理任务读取项目外文件，不允许写入"
      : label;
  }
  if (detailNode) {
    if (argumentSnippet) {
      detailNode.hidden = false;
      detailNode.textContent = argumentSnippet;
      detailNode.title = argumentSnippet;
    } else if (description && description !== "该工具需要访问工作区") {
      detailNode.hidden = false;
      detailNode.textContent = description;
      detailNode.title = pending.description || description;
    } else {
      detailNode.hidden = true;
      detailNode.textContent = "";
      detailNode.removeAttribute("title");
    }
  }
  prompt.setAttribute("aria-label", `等待批准：${tool}`);
}

const FALLBACK_MODEL_SELECTIONS: Record<
  string,
  { providerId: string; modelId: string }
> = {
  "0": { providerId: "openai-compat", modelId: "gpt-5.6-sol" },
  "1": { providerId: "openai-compat", modelId: "gpt-5.6-terra" },
  "2": { providerId: "openai-compat", modelId: "gpt-5.6-luna" },
  "3": { providerId: "openai-compat", modelId: "gpt-5.5" },
  "4": { providerId: "openai-compat", modelId: "gpt-5.4" },
  "5": { providerId: "openai-compat", modelId: "gpt-5.4-mini" },
  "6": { providerId: "openai-compat", modelId: "gpt-5.2" },
};

const FALLBACK_MODEL_OPTIONS = [
  { providerId: "openai-compat", modelId: "gpt-5.6-sol", label: "5.6 Sol" },
  { providerId: "openai-compat", modelId: "gpt-5.6-terra", label: "5.6 Terra" },
  { providerId: "openai-compat", modelId: "gpt-5.6-luna", label: "5.6 Luna" },
  { providerId: "openai-compat", modelId: "gpt-5.5", label: "5.5" },
  { providerId: "openai-compat", modelId: "gpt-5.4", label: "5.4" },
  { providerId: "openai-compat", modelId: "gpt-5.4-mini", label: "5.4 Mini" },
  { providerId: "openai-compat", modelId: "gpt-5.2", label: "5.2" },
] as const;

// Keep this aligned with the embedded runner's source-grounded fallback in
// pi/provider.ts. A configured model always overrides these values.
const DEFAULT_MODEL_CONTEXT_WINDOW = 256_000;
const DEFAULT_MODEL_MAX_OUTPUT_TOKENS = 8_192;

/**
 * Browser preview keeps the design's sample model names, but a Tauri window
 * must never turn those labels into a request for an imaginary provider.  The
 * availability state is deliberately independent from the selected option so
 * that a provider refresh cannot briefly make a stale fallback runnable.
 */
type ModelPickerAvailability =
  | "preview"
  | "loading"
  | "ready"
  | "unconfigured"
  | "recovery_required"
  | "error";

let modelPickerAvailability: ModelPickerAvailability = "preview";
let modelPickerAvailabilityError = "";
let localProviderProxyAvailable = true;
let localProviderProxyCanRetry = false;
let localProviderProxyRetrying = false;
// Native settings are deliberately not read until NativeShell has published
// its closed app_health projection. Browser preview remains self-contained.
let appHealthKnown = false;
let providerSettingsReadable = false;
const STORAGE_RECOVERY_PERMISSION_MESSAGE =
  "本地存储需要恢复后才能处理工具权限。";

type ProxyRuntimeStatus = {
  status: "ready" | "unavailable";
  canRetry: boolean;
};

function hasNativeProviderSettings() {
  return Boolean(window.__TAURI__?.core?.invoke);
}

function providerModelIsReady() {
  return (
    !hasNativeProviderSettings() ||
    (providerSettingsReadable &&
      localProviderProxyAvailable &&
      modelPickerAvailability === "ready")
  );
}

function providerAvailabilityMessage() {
  if (hasNativeProviderSettings() && !appHealthKnown)
    return "正在检查本地服务状态，请稍候。";
  if (hasNativeProviderSettings() && !providerSettingsReadable)
    return "本地存储需要恢复后才能读取供应商设置。";
  if (hasNativeProviderSettings() && !localProviderProxyAvailable)
    return localProviderProxyCanRetry
      ? "本地 Provider 代理暂不可用。可重试启动代理，其他本地功能不受影响。"
      : "本地 Provider 代理暂不可用，请稍后重试。";
  switch (modelPickerAvailability) {
    case "loading":
      return "正在读取供应商设置，请稍候。";
    case "recovery_required":
      return "本地存储需要恢复后才能读取供应商设置。";
    case "unconfigured":
      return "尚未配置可用供应商模型。请先在设置中添加供应商和模型。";
    case "error":
      return (
        modelPickerAvailabilityError ||
        "无法读取供应商设置。请在设置中检查供应商后重试。"
      );
    default:
      return "";
  }
}

function applyAppHealth(value: unknown) {
  const health = asRecord(value);
  appHealthKnown = true;
  // Health is a closed native DTO. Require every durable-read predicate here
  // too: this UI guard is not the authority, but it prevents a partial or old
  // host response from triggering settings hydration before recovery is known.
  providerSettingsReadable =
    health?.writes === "enabled" &&
    health.sessionStore === "ready" &&
    health.settings === "ready";
  localProviderProxyAvailable = health?.proxy === "ready";
  // app_health intentionally omits retry mechanics. An explicitly unavailable
  // aggregate state may offer the normal bounded retry; proxy_runtime_status
  // refines that flag immediately after this event.
  localProviderProxyCanRetry = health?.proxy === "unavailable";
}

function permissionDecisionsBlockedByRecovery() {
  return (
    hasNativeProviderSettings() && appHealthKnown && !providerSettingsReadable
  );
}

function applyProxyRuntimeStatus(value: unknown) {
  const status = asRecord(value);
  if (status?.status === "ready" && status.canRetry === false) {
    localProviderProxyAvailable = true;
    localProviderProxyCanRetry = false;
    return;
  }
  if (status?.status === "unavailable" && status.canRetry === true) {
    localProviderProxyAvailable = false;
    localProviderProxyCanRetry = true;
    return;
  }
  // Do not use unknown fields or a malformed DTO as transport metadata.
  // Fail closed for Provider/Pi while retaining a stable recovery message.
  localProviderProxyAvailable = false;
  localProviderProxyCanRetry = false;
}

async function refreshLocalProviderProxyStatus() {
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke || !providerSettingsReadable) return;
  try {
    const status = await invoke<ProxyRuntimeStatus>("proxy_runtime_status");
    applyProxyRuntimeStatus(status);
  } catch {
    // A missing/failed status command is not allowed to turn Provider/Pi back
    // on. The retry command has its own stable failure path below.
    localProviderProxyAvailable = false;
    localProviderProxyCanRetry = true;
  } finally {
    syncComposerProviderAvailability();
  }
}

async function retryLocalProviderProxy() {
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke || localProviderProxyRetrying) return;
  if (!providerSettingsReadable || !localProviderProxyCanRetry) {
    toast(providerAvailabilityMessage());
    return;
  }
  localProviderProxyRetrying = true;
  syncComposerProviderAvailability();
  try {
    const status = await invoke<ProxyRuntimeStatus>("proxy_runtime_retry");
    applyProxyRuntimeStatus(status);
    if (localProviderProxyAvailable && providerSettingsReadable)
      void hydrateModelPickerMetadata();
    else toast("本地 Provider 代理仍不可用，请稍后重试。");
  } catch {
    // Never display native listener, port, or operating-system details.
    localProviderProxyAvailable = false;
    localProviderProxyCanRetry = true;
    toast("无法启动本地 Provider 代理，请稍后重试。");
  } finally {
    localProviderProxyRetrying = false;
    syncComposerProviderAvailability();
  }
}

function openProviderSettings() {
  // The HTML shell already owns the Settings transition. Reuse its real
  // controls instead of duplicating overlay state in the runtime module.
  element<HTMLButtonElement>("btnSettings")?.click();
  window.requestAnimationFrame(() => {
    document
      .querySelector<HTMLButtonElement>(
        ".settings-nav button[data-settings='providers']",
      )
      ?.click();
    window.requestAnimationFrame(() =>
      element<HTMLButtonElement>("btnAddProvider")?.focus(),
    );
  });
}

function syncComposerProviderAvailability() {
  const form = element<HTMLFormElement>("composerForm");
  const send = element<HTMLButtonElement>("btnSend");
  if (!form || !send) return;
  const native = hasNativeProviderSettings();

  const shellState = document.body.dataset.novaveiShellState;
  if (native && shellState && shellState !== "ready") {
    // NativeShell owns the no-project/no-session lock. Provider readiness
    // must never make the Composer look actionable before that boundary is
    // established.
    send.disabled = true;
    send.setAttribute("aria-disabled", "true");
    return;
  }

  const requiresSetup = native && !providerModelIsReady();
  form.dataset.novaveiProviderState = !native
    ? modelPickerAvailability
    : !providerSettingsReadable
      ? "recovery_required"
      : !localProviderProxyAvailable
        ? "proxy_unavailable"
        : modelPickerAvailability;
  form.dataset.novaveiProxyState = !native
    ? "preview"
    : localProviderProxyAvailable
      ? "ready"
      : "unavailable";
  send.dataset.novaveiProviderReady = String(!requiresSetup);

  // Run state owns the same button while a turn is active. Do not replace a
  // visible Stop / Cancelling label with onboarding text mid-turn.
  if (send.dataset.piRunning === "true") return;

  send.disabled =
    native &&
    (!providerSettingsReadable ||
      modelPickerAvailability === "loading" ||
      localProviderProxyRetrying ||
      (!localProviderProxyAvailable && !localProviderProxyCanRetry));
  send.setAttribute("aria-disabled", String(send.disabled));
  send.setAttribute("aria-busy", localProviderProxyRetrying ? "true" : "false");
  if (!requiresSetup) {
    send.textContent = "发送";
    send.setAttribute("aria-label", "发送");
    send.removeAttribute("title");
    return;
  }

  const message = providerAvailabilityMessage();
  send.textContent =
    native && !providerSettingsReadable
      ? "本地存储受限"
      : native && !localProviderProxyAvailable
        ? localProviderProxyRetrying
          ? "重试代理…"
          : localProviderProxyCanRetry
            ? "重试代理"
            : "代理不可用"
        : modelPickerAvailability === "loading"
          ? "读取供应商…"
          : "配置供应商";
  send.setAttribute(
    "aria-label",
    message ||
      (!localProviderProxyAvailable ? "重试本地 Provider 代理" : "配置供应商"),
  );
  if (message) send.title = message;
  else send.removeAttribute("title");
}

function renderProviderModelNotice(
  state: Exclude<ModelPickerAvailability, "preview" | "ready">,
  message: string,
) {
  const root = element<HTMLElement>("modelOptions");
  const picker = element<HTMLElement>("modelPickerName");
  const reasoning = element<HTMLElement>("modelPickerReasoning");
  if (root) {
    const notice = document.createElement("p");
    notice.className = "model-helper";
    notice.setAttribute(
      "role",
      state === "error" || state === "recovery_required" ? "alert" : "status",
    );
    notice.textContent = message;
    root.replaceChildren(notice);

    if (state !== "loading" && state !== "recovery_required") {
      const action = document.createElement("button");
      action.type = "button";
      action.className = "btn";
      action.textContent = "前往供应商设置";
      action.addEventListener("click", openProviderSettings);
      root.appendChild(action);
    }
  }
  if (picker) {
    picker.dataset.novaveiProviderState = state;
    delete picker.dataset.providerId;
    delete picker.dataset.modelId;
    picker.textContent =
      state === "loading"
        ? "读取供应商…"
        : state === "recovery_required"
          ? "本地存储需要恢复"
          : "未配置供应商";
  }
  if (reasoning)
    reasoning.textContent =
      state === "loading"
        ? "等待中"
        : state === "recovery_required"
          ? "已暂停"
          : "需配置";
  syncComposerProviderAvailability();
}

type ModelPickerEntry = {
  providerId: string;
  providerLabel?: string;
  model: {
    id: string;
    label?: string;
    contextWindow?: number;
    maxOutputTokens?: number;
  };
};

type ModelPickerSelection = {
  providerId: string;
  modelId: string;
};

type ModelContextCapacity = {
  contextWindow: number;
  maxOutput: number;
};

type ModelCapacityFallback = {
  contextWindow?: number;
  maxOutputTokens?: number;
};

const MODEL_PROVIDER_ID = /^[A-Za-z0-9._-]{1,128}$/;
const UI_REASONING_LEVELS: readonly PiReasoningLevel[] = [
  "off",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
];
const modelContextCapacities = new Map<string, ModelContextCapacity>();

function boundedModelPickerSelection(
  value: unknown,
): ModelPickerSelection | undefined {
  const selection = asRecord(value);
  if (
    !selection ||
    Object.keys(selection).some(
      (key) => key !== "providerId" && key !== "modelId",
    )
  ) {
    return undefined;
  }
  const providerId =
    typeof selection.providerId === "string" ? selection.providerId.trim() : "";
  const modelId =
    typeof selection.modelId === "string" ? selection.modelId.trim() : "";
  if (
    !MODEL_PROVIDER_ID.test(providerId) ||
    !modelId ||
    /[\u0000-\u001F\u007F]/.test(modelId)
  )
    return undefined;
  if (new TextEncoder().encode(modelId).byteLength > 256) return undefined;
  return { providerId, modelId };
}

function modelPickerOptionSelection(
  option: HTMLElement | null,
): ModelPickerSelection | undefined {
  if (!option) return undefined;
  return boundedModelPickerSelection({
    providerId: option.dataset.providerId,
    modelId: option.dataset.modelId,
  });
}

function sameModelPickerSelection(
  left: ModelPickerSelection | undefined,
  right: ModelPickerSelection | undefined,
) {
  return (
    left?.providerId === right?.providerId && left?.modelId === right?.modelId
  );
}

function boundedProjectPreferences(
  value: unknown,
): ProjectPreferences | undefined {
  const preferences = asRecord(value);
  if (
    !preferences ||
    Object.keys(preferences).some(
      (key) => key !== "model" && key !== "reasoning" && key !== "permission",
    )
  ) {
    return undefined;
  }
  const model = boundedModelPickerSelection(preferences.model);
  if (preferences.model !== undefined && !model) return undefined;
  const reasoning =
    typeof preferences.reasoning === "string"
      ? preferences.reasoning.trim().toLowerCase()
      : undefined;
  if (
    preferences.reasoning !== undefined &&
    !UI_REASONING_LEVELS.includes(reasoning as PiReasoningLevel)
  ) {
    return undefined;
  }
  const permission =
    typeof preferences.permission === "string"
      ? preferences.permission.trim().toLowerCase()
      : undefined;
  if (
    preferences.permission !== undefined &&
    permission !== "readonly" &&
    permission !== "ask" &&
    permission !== "auto-approve"
  ) {
    return undefined;
  }
  if (!model && !reasoning && !permission) return undefined;
  return {
    ...(model ? { model } : {}),
    ...(reasoning ? { reasoning: reasoning as PiReasoningLevel } : {}),
    ...(permission
      ? {
          permission: permission as NonNullable<
            ProjectPreferences["permission"]
          >,
        }
      : {}),
  };
}

function providerRecords(value: unknown): UnknownRecord[] {
  if (Array.isArray(value))
    return value
      .map(asRecord)
      .filter((item): item is UnknownRecord => Boolean(item));
  const object = asRecord(value);
  if (!object) return [];
  const nested = object.providers ?? object.items ?? object.customProviders;
  if (Array.isArray(nested)) return providerRecords(nested);
  return Object.entries(object).flatMap(([id, item]) => {
    const record = asRecord(item);
    return record
      ? [{ ...record, id: readString(record, "id", "providerId") ?? id }]
      : [];
  });
}

function modelCapacity(
  value: UnknownRecord | undefined,
  fallback?: ModelCapacityFallback,
) {
  return {
    contextWindow:
      positiveTokenCount(value?.contextWindow ?? value?.context_window) ??
      fallback?.contextWindow,
    maxOutputTokens:
      positiveTokenCount(
        value?.maxOutputToken ??
          value?.max_output_token ??
          value?.maxOutputTokens ??
          value?.max_output_tokens,
      ) ?? fallback?.maxOutputTokens,
  };
}

function activeProviderModelIds(
  record: UnknownRecord,
): Set<string> | null | undefined {
  const hasCamelCase = Object.hasOwn(record, "activeModels");
  const hasSnakeCase = Object.hasOwn(record, "active_models");
  if (!hasCamelCase && !hasSnakeCase) return undefined;
  const source = hasCamelCase ? record.activeModels : record.active_models;
  if (!Array.isArray(source)) return null;
  return new Set(
    source
      .filter((value): value is string => typeof value === "string")
      .map((value) => value.trim())
      .filter(Boolean),
  );
}

function boundedProviderModelId(value: string) {
  const modelId = value.trim();
  return modelId &&
    new TextEncoder().encode(modelId).byteLength <= 256 &&
    !/[\u0000-\u001F\u007F]/.test(modelId)
    ? modelId
    : undefined;
}

function providerModelIsEnabled(
  record: UnknownRecord,
  modelId: string,
  model?: UnknownRecord,
) {
  const boundedModelId = boundedProviderModelId(modelId);
  if (!boundedModelId) return false;
  const active = activeProviderModelIds(record);
  return (
    active !== null &&
    (!active || active.has(boundedModelId)) &&
    model?.enabled !== false
  );
}

function providerModels(record: UnknownRecord) {
  const providerCapacity = modelCapacity(record);
  const values: Array<{
    id: string;
    label?: string;
    contextWindow?: number;
    maxOutputTokens?: number;
  }> = [];
  const hasModels = Object.hasOwn(record, "models");
  if (!hasModels) {
    const direct = readString(
      record,
      "defaultModel",
      "default_model",
      "modelId",
      "model_id",
      "model",
    );
    if (direct && providerModelIsEnabled(record, direct))
      values.push({ id: direct, label: direct, ...providerCapacity });
    return values;
  }

  const models = record.models;
  if (!Array.isArray(models)) return values;
  for (const item of models) {
    if (
      typeof item === "string" &&
      item.trim() &&
      providerModelIsEnabled(record, item.trim())
    )
      values.push({ id: item.trim(), label: item.trim(), ...providerCapacity });
    else {
      const model = asRecord(item);
      const id = readString(model, "id", "modelId", "model_id", "name");
      if (id && providerModelIsEnabled(record, id, model))
        values.push({
          id,
          label: readString(model, "label", "name") ?? id,
          ...modelCapacity(model, providerCapacity),
        });
    }
  }
  return values.filter(
    (item, index, all) =>
      all.findIndex((candidate) => candidate.id === item.id) === index,
  );
}

function pickerIdentity() {
  const selected = document.querySelector<HTMLElement>(".model-option.on");
  const picker = element<HTMLElement>("modelPickerName");
  return {
    providerId: selected?.dataset.providerId ?? picker?.dataset.providerId,
    modelId: selected?.dataset.modelId ?? picker?.dataset.modelId,
  };
}

function appendModelPickerOption(
  root: HTMLElement,
  index: number,
  input: {
    providerId: string;
    modelId: string;
    label: string;
    title?: string;
    fallback?: boolean;
    contextWindow?: number;
    maxOutputTokens?: number;
  },
  selected: boolean,
) {
  const option = document.createElement("button");
  option.type = "button";
  option.className = "model-option";
  option.dataset.model = String(index);
  option.dataset.providerId = input.providerId;
  option.dataset.modelId = input.modelId;
  option.dataset.modelLabel = input.label;
  if (input.contextWindow)
    option.dataset.contextWindow = String(input.contextWindow);
  if (input.maxOutputTokens)
    option.dataset.maxOutputTokens = String(input.maxOutputTokens);
  option.setAttribute("aria-pressed", String(selected));
  option.setAttribute("aria-label", `选择模型：${input.label}`);
  if (input.title) option.title = input.title;
  if (input.fallback) option.dataset.piFallbackLabel = input.label;
  option.classList.toggle("on", selected);
  // Intentionally use textContent: model metadata comes from a remote provider.
  option.textContent = input.label;
  root.appendChild(option);
}

function renderModelPickerOptions(entries: readonly ModelPickerEntry[]) {
  const root = element<HTMLElement>("modelOptions");
  if (!root) return;
  const previous = pickerIdentity();
  const selectedIndex = Math.max(
    0,
    entries.findIndex(
      (entry) =>
        entry.providerId === previous.providerId &&
        entry.model.id === previous.modelId,
    ),
  );
  root.replaceChildren();
  modelContextCapacities.clear();
  const section = document.createElement("span");
  section.className = "model-section-label";
  section.textContent = "模型";
  root.appendChild(section);
  entries.forEach((entry, index) => {
    const capacity = {
      contextWindow: entry.model.contextWindow ?? DEFAULT_MODEL_CONTEXT_WINDOW,
      maxOutput: entry.model.maxOutputTokens ?? DEFAULT_MODEL_MAX_OUTPUT_TOKENS,
    };
    modelContextCapacities.set(
      `${entry.providerId}\u0000${entry.model.id}`,
      capacity,
    );
    const modelLabel = entry.model.label || entry.model.id;
    const label = entry.providerLabel
      ? `${entry.providerLabel} · ${modelLabel}`
      : modelLabel;
    appendModelPickerOption(
      root,
      index,
      {
        providerId: entry.providerId,
        modelId: entry.model.id,
        label,
        contextWindow: capacity.contextWindow,
        maxOutputTokens: capacity.maxOutput,
        title: entry.providerLabel
          ? `${entry.providerLabel} · ${entry.model.id}`
          : entry.model.id,
      },
      index === selectedIndex,
    );
  });
  syncModelPickerMetadata();
  modelPickerAvailability = "ready";
  modelPickerAvailabilityError = "";
  element<HTMLElement>("modelPickerName")?.removeAttribute(
    "data-novavei-provider-state",
  );
  syncComposerProviderAvailability();
  window.dispatchEvent(
    new CustomEvent("novavei:model-options-rendered", {
      detail: { selectedIndex },
    }),
  );
}

function renderFallbackModelPickerOptions() {
  const root = element<HTMLElement>("modelOptions");
  if (!root) return;
  const previous = pickerIdentity();
  const selectedIndex = Math.max(
    0,
    FALLBACK_MODEL_OPTIONS.findIndex(
      (entry) =>
        entry.providerId === previous.providerId &&
        entry.modelId === previous.modelId,
    ),
  );
  root.replaceChildren();
  modelContextCapacities.clear();
  const section = document.createElement("span");
  section.className = "model-section-label";
  section.textContent = "模型";
  root.appendChild(section);
  FALLBACK_MODEL_OPTIONS.forEach((entry, index) => {
    modelContextCapacities.set(`${entry.providerId}\u0000${entry.modelId}`, {
      contextWindow: DEFAULT_MODEL_CONTEXT_WINDOW,
      maxOutput: DEFAULT_MODEL_MAX_OUTPUT_TOKENS,
    });
    appendModelPickerOption(
      root,
      index,
      {
        providerId: entry.providerId,
        modelId: entry.modelId,
        label: entry.label,
        fallback: true,
        contextWindow: DEFAULT_MODEL_CONTEXT_WINDOW,
        maxOutputTokens: DEFAULT_MODEL_MAX_OUTPUT_TOKENS,
      },
      index === selectedIndex,
    );
  });
  syncModelPickerMetadata();
  modelPickerAvailability = "preview";
  modelPickerAvailabilityError = "";
  element<HTMLElement>("modelPickerName")?.removeAttribute(
    "data-novavei-provider-state",
  );
  syncComposerProviderAvailability();
  window.dispatchEvent(
    new CustomEvent("novavei:model-options-rendered", {
      detail: { selectedIndex },
    }),
  );
}

function syncModelPickerMetadata() {
  const options = [...document.querySelectorAll<HTMLElement>(".model-option")];
  for (const option of options) {
    option.dataset.piFallbackLabel ??=
      option.textContent?.trim().replace(/\s+/g, " ") || "Model";
    const fallback = FALLBACK_MODEL_SELECTIONS[option.dataset.model ?? ""];
    if (fallback && !option.dataset.providerId) {
      option.dataset.providerId = fallback.providerId;
      option.dataset.modelId = fallback.modelId;
    }
    if (option.dataset.piModelBound !== "true") {
      option.dataset.piModelBound = "true";
      option.addEventListener("click", () =>
        queueMicrotask(syncModelPickerMetadata),
      );
    }
  }
  const selected =
    options.find((option) => option.classList.contains("on")) ?? options[0];
  const picker = element<HTMLElement>("modelPickerName");
  if (selected && picker) {
    if (selected.dataset.providerId)
      picker.dataset.providerId = selected.dataset.providerId;
    if (selected.dataset.modelId)
      picker.dataset.modelId = selected.dataset.modelId;
    if (selected.dataset.contextWindow)
      picker.dataset.contextWindow = selected.dataset.contextWindow;
    else delete picker.dataset.contextWindow;
    if (selected.dataset.maxOutputTokens)
      picker.dataset.maxOutputTokens = selected.dataset.maxOutputTokens;
    else delete picker.dataset.maxOutputTokens;
    picker.dataset.modelLabel = picker.textContent?.trim() ?? "";
  }
}

let modelPickerSessionId: string | undefined;
let modelPickerSessionSelection: ModelPickerSelection | undefined;
let unavailableSessionModelKey: string | undefined;
let selectedSessionContextTokenEstimate = 0;
const persistedModelSelections = new Map<string, string>();
const modelSelectionSaveChains = new Map<string, Promise<void>>();
let activeProjectPreferences: ProjectPreferences | undefined;
let projectPreferenceFallbackReasoning: PiReasoningLevel | undefined;
let applyingProjectPreferences = false;

function modelSelectionKey(selection: ModelPickerSelection) {
  return `${selection.providerId}\u0000${selection.modelId}`;
}

function modelPickerOptions() {
  const root = element<HTMLElement>("modelOptions");
  return root ? [...root.querySelectorAll<HTMLElement>(".model-option")] : [];
}

function selectModelPickerOption(option: HTMLElement) {
  const root = element<HTMLElement>("modelOptions");
  if (!root?.contains(option)) return false;
  const options = modelPickerOptions();
  const selectedIndex = Number(option.dataset.model);
  if (
    !Number.isInteger(selectedIndex) ||
    selectedIndex < 0 ||
    !options.includes(option)
  )
    return false;
  for (const candidate of options) {
    const selected = candidate === option;
    candidate.classList.toggle("on", selected);
    candidate.setAttribute("aria-pressed", String(selected));
  }
  syncModelPickerMetadata();
  window.dispatchEvent(
    new CustomEvent("novavei:model-options-rendered", {
      detail: { selectedIndex },
    }),
  );
  return true;
}

function reconcileSessionModelPickerSelection() {
  if (!modelPickerSessionId) return;
  const options = modelPickerOptions();
  const saved = modelPickerSessionSelection;
  const selected = saved
    ? options.find((option) =>
        sameModelPickerSelection(modelPickerOptionSelection(option), saved),
      )
    : options[0];
  if (!selected || !selectModelPickerOption(selected)) return;
  if (
    !saved ||
    sameModelPickerSelection(modelPickerOptionSelection(selected), saved)
  ) {
    unavailableSessionModelKey = undefined;
    return;
  }
  const key = `${modelPickerSessionId}\u0000${modelSelectionKey(saved)}`;
  if (key !== unavailableSessionModelKey) {
    unavailableSessionModelKey = key;
    toast("此会话保存的模型已不可用，已切换到当前列表的默认模型。");
  }
}

function persistModelPickerSelection(
  sessionId: string,
  selection: ModelPickerSelection,
) {
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke || !sessionId) return;
  const key = modelSelectionKey(selection);
  if (persistedModelSelections.get(sessionId) === key) return;
  const previous = modelSelectionSaveChains.get(sessionId) ?? Promise.resolve();
  const task = previous
    .catch(() => undefined)
    .then(async () => {
      if (persistedModelSelections.get(sessionId) === key) return;
      await invoke("chat_history_set_model", {
        id: sessionId,
        selectedModelJson: JSON.stringify(selection),
      });
      persistedModelSelections.set(sessionId, key);
    })
    .catch(() => {
      if (
        modelPickerSessionId === sessionId &&
        sameModelPickerSelection(modelPickerSessionSelection, selection)
      ) {
        toast("未能保存此会话的模型选择，请重新选择模型后重试。");
      }
      console.warn("[NovaVei Pi] unable to persist selected session model");
    });
  modelSelectionSaveChains.set(sessionId, task);
  void task.finally(() => {
    if (modelSelectionSaveChains.get(sessionId) === task)
      modelSelectionSaveChains.delete(sessionId);
  });
}

function installSessionModelPickerPersistence() {
  const root = element<HTMLElement>("modelOptions");
  const onModelClick = (event: MouseEvent) => {
    const target =
      event.target instanceof Element
        ? event.target.closest<HTMLElement>(".model-option")
        : null;
    if (!target || !root?.contains(target) || !target.classList.contains("on"))
      return;
    const selection = modelPickerOptionSelection(target);
    if (!selection) return;
    const sessionId = window.__novaveiHost?.getSessionId()?.trim();
    if (sessionId) {
      modelPickerSessionId = sessionId;
      modelPickerSessionSelection = selection;
      unavailableSessionModelKey = undefined;
      persistModelPickerSelection(sessionId, selection);
    }
    if (!applyingProjectPreferences) {
      const host = window.__novaveiHost;
      if (host) {
        void host
          .saveCurrentProjectPreferences({ model: selection })
          .catch((error) =>
            console.warn(
              "[NovaVei Pi] unable to persist project model preference",
              error,
            ),
          );
      }
    }
    // The static picker updates the active option before this delegated
    // listener. Publish the selected capacity in the same event turn so the
    // dock never renders a stale context window after a manual model change.
    window.dispatchEvent(
      new CustomEvent("novavei:model-options-rendered", {
        detail: { selectedIndex: Number(target.dataset.model) },
      }),
    );
  };
  const onSessionChanged = (event: Event) => {
    const detail =
      event instanceof CustomEvent ? asRecord(event.detail) : undefined;
    const sessionId = readString(detail, "sessionId");
    if (!sessionId) return;
    const selection = boundedModelPickerSelection(detail?.modelSelection);
    modelPickerSessionId = sessionId;
    modelPickerSessionSelection = selection;
    unavailableSessionModelKey = undefined;
    if (selection)
      persistedModelSelections.set(sessionId, modelSelectionKey(selection));
    else persistedModelSelections.delete(sessionId);
    void hydrateModelPickerMetadata();
  };
  root?.addEventListener("click", onModelClick);
  window.addEventListener("novavei:session-changed", onSessionChanged);
  return () => {
    root?.removeEventListener("click", onModelClick);
    window.removeEventListener("novavei:session-changed", onSessionChanged);
  };
}

let modelPickerHydrationSerial = 0;

async function hydrateModelPickerMetadata() {
  const serial = ++modelPickerHydrationSerial;
  syncModelPickerMetadata();
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) {
    renderFallbackModelPickerOptions();
    reconcileSessionModelPickerSelection();
    applyActiveProjectPreferences();
    return;
  }
  if (!providerSettingsReadable) {
    // NativeShell has either not published app_health yet or has entered
    // recovery mode. Never race it with a direct settings_load_all call.
    if (appHealthKnown) {
      modelPickerAvailability = "recovery_required";
      modelPickerAvailabilityError = "";
      renderProviderModelNotice(
        "recovery_required",
        providerAvailabilityMessage(),
      );
    }
    return;
  }
  modelPickerAvailability = "loading";
  modelPickerAvailabilityError = "";
  renderProviderModelNotice("loading", providerAvailabilityMessage());
  try {
    const settings = await invoke<UnknownRecord>("settings_load_all");
    if (serial !== modelPickerHydrationSerial) return;
    const providers = providerRecords(settings.providers);
    const configuredModels = providers.flatMap<ModelPickerEntry>((provider) => {
      if (provider.enabled === false) return [];
      const providerId = readString(
        provider,
        "id",
        "providerId",
        "provider_id",
      );
      if (!providerId) return [];
      const providerLabel = readString(provider, "name", "label") ?? providerId;
      return providerModels(provider).map((model) => ({
        providerId,
        providerLabel,
        model,
      }));
    });
    if (configuredModels.length) renderModelPickerOptions(configuredModels);
    else {
      modelPickerAvailability = "unconfigured";
      renderProviderModelNotice("unconfigured", providerAvailabilityMessage());
    }
    syncModelPickerMetadata();
    reconcileSessionModelPickerSelection();
    applyActiveProjectPreferences();
  } catch {
    if (serial !== modelPickerHydrationSerial) return;
    modelPickerAvailability = "error";
    // Settings failures may carry a path, endpoint, or protected-storage
    // diagnostic. The UI has one actionable recovery route, so keep its
    // message stable rather than reflecting the native error string.
    modelPickerAvailabilityError =
      "无法读取供应商设置。请在设置中检查供应商后重试。";
    renderProviderModelNotice("error", providerAvailabilityMessage());
    reconcileSessionModelPickerSelection();
    applyActiveProjectPreferences();
  }
}

function selectedModelSelection() {
  syncModelPickerMetadata();
  const selected =
    document.querySelector<HTMLElement>(".model-option.on") ??
    document.querySelector<HTMLElement>(".model-option");
  const picker = element<HTMLElement>("modelPickerName");
  const fallback = FALLBACK_MODEL_SELECTIONS[selected?.dataset.model ?? ""];
  return {
    providerId:
      selected?.dataset.providerId ??
      picker?.dataset.providerId ??
      fallback?.providerId,
    modelId:
      selected?.dataset.modelId ?? picker?.dataset.modelId ?? fallback?.modelId,
  };
}

function selectedPermission() {
  const selected = document.querySelector<HTMLElement>(
    ".permission-option.on[data-permission]",
  );
  return selected?.dataset.permission || "ask";
}

export function reasoningFromUiValue(value: unknown): PiReasoningLevel {
  const normalized = typeof value === "string" ? value.trim() : value;
  const numeric =
    typeof normalized === "number"
      ? normalized
      : typeof normalized === "string" && normalized
        ? Number(normalized)
        : Number.NaN;
  const fallbackIndex = Math.max(0, UI_REASONING_LEVELS.indexOf("xhigh"));
  const index = Number.isFinite(numeric)
    ? Math.max(0, Math.min(UI_REASONING_LEVELS.length - 1, Math.round(numeric)))
    : fallbackIndex;
  return UI_REASONING_LEVELS[index];
}

function selectedReasoning(): PiReasoningLevel {
  const slider = element<HTMLInputElement>("reasoningSlider");
  const activeStep = document.querySelector<HTMLElement>(
    ".reasoning-step.on[data-reasoning]",
  );
  return reasoningFromUiValue(slider?.value ?? activeStep?.dataset.reasoning);
}

function setReasoningUiValue(reasoning: PiReasoningLevel) {
  const slider = element<HTMLInputElement>("reasoningSlider");
  const index = UI_REASONING_LEVELS.indexOf(reasoning);
  if (!slider || index < 0) return;
  slider.value = String(index);
  slider.setAttribute("aria-valuetext", REASONING_LABELS[reasoning]);
  slider.dispatchEvent(new Event("input", { bubbles: true }));
}

function applyActiveProjectPreferences() {
  const reasoning =
    activeProjectPreferences?.reasoning ??
    projectPreferenceFallbackReasoning ??
    selectedReasoning();
  applyingProjectPreferences = true;
  try {
    setReasoningUiValue(reasoning);
    // A conversation-level model selection is an explicit override. Keep it
    // intact even when this project's default is refreshed in the background.
    if (modelPickerSessionSelection) return;
    const preferredModel = activeProjectPreferences?.model;
    const preferredOption = preferredModel
      ? modelPickerOptions().find((option) =>
          sameModelPickerSelection(
            modelPickerOptionSelection(option),
            preferredModel,
          ),
        )
      : undefined;
    if (preferredOption && selectModelPickerOption(preferredOption)) return;
    // Missing or disabled project models deliberately fall back through the
    // existing provider default path rather than leaving a stale selection.
    reconcileSessionModelPickerSelection();
  } finally {
    applyingProjectPreferences = false;
  }
}

function installProjectPreferenceBindings() {
  const slider = element<HTMLInputElement>("reasoningSlider");
  const steps = document.querySelector<HTMLElement>(".reasoning-steps");
  projectPreferenceFallbackReasoning ??= selectedReasoning();

  const persistReasoning = () => {
    if (applyingProjectPreferences) return;
    const reasoning = selectedReasoning();
    const host = window.__novaveiHost;
    if (!host) {
      projectPreferenceFallbackReasoning = reasoning;
      return;
    }
    void host.saveCurrentProjectPreferences({ reasoning }).then(
      (saved) => {
        if (!saved) projectPreferenceFallbackReasoning = reasoning;
      },
      (error) =>
        console.warn(
          "[NovaVei Pi] unable to persist project reasoning preference",
          error,
        ),
    );
  };
  const onProjectPreferencesChanged = (event: Event) => {
    const detail =
      event instanceof CustomEvent ? asRecord(event.detail) : undefined;
    activeProjectPreferences = boundedProjectPreferences(detail?.preferences);
    applyActiveProjectPreferences();
  };
  slider?.addEventListener("input", persistReasoning);
  steps?.addEventListener("click", persistReasoning);
  window.addEventListener(
    "novavei:project-preferences-changed",
    onProjectPreferencesChanged,
  );
  activeProjectPreferences = boundedProjectPreferences(
    window.__novaveiHost?.getCurrentProjectPreferences(),
  );
  applyActiveProjectPreferences();
  return () => {
    slider?.removeEventListener("input", persistReasoning);
    steps?.removeEventListener("click", persistReasoning);
    window.removeEventListener(
      "novavei:project-preferences-changed",
      onProjectPreferencesChanged,
    );
  };
}

function selectedWorkdir() {
  const project = document.querySelector<HTMLElement>(
    '.project-row[aria-current="page"][data-workdir]',
  );
  if (project?.dataset.workdir) return project.dataset.workdir;
  // Files dock status is now the bare absolute path (no "workdir =" prefix).
  const status =
    element<HTMLElement>("workdirStatus")?.textContent?.trim() ?? "";
  if (!status) return undefined;
  if (
    /^[A-Za-z]:[\\/]/.test(status) ||
    status.startsWith("\\\\") ||
    status.startsWith("/")
  )
    return status;
  return status.match(/workdir\s*=\s*(.+)$/i)?.[1]?.trim();
}

function finiteNumber(value: unknown) {
  const numeric = typeof value === "number" ? value : Number(value);
  return Number.isFinite(numeric) && numeric >= 0 ? numeric : undefined;
}

function formatTokens(value: number) {
  if (value >= 1_000_000)
    return `${(value / 1_000_000).toFixed(value >= 10_000_000 ? 0 : 1)}M`;
  if (value >= 1_000)
    return `${(value / 1_000).toFixed(value >= 10_000 ? 0 : 1)}k`;
  return String(Math.round(value));
}

type ContextUsage = {
  contextWindow: number;
  maxOutput: number;
  input: number;
  budget: number;
  observedInput: boolean;
  source: "provider" | "runtime" | "estimate";
  cache: number;
  fixedTokens: number;
  keptHistoryTokens: number;
  keptMessages: number;
  droppedMessages: number;
  trimmed: boolean;
  compaction?: ContextCompactionUsage;
};

type ContextCompactionUsage = {
  version: 1;
  summaryId: string;
  generatedAt?: number;
  sourceFingerprint: string;
  sourceMessageStart: number;
  sourceMessageEnd: number;
  sourceTurnStart: number;
  sourceTurnEnd: number;
};

type ContextInspector = {
  render: (state: PiRuntimeState) => void;
  dispose: () => void;
};

function selectedModelContextCapacity(): ModelContextCapacity | undefined {
  const selection = modelPickerSessionSelection ?? selectedModelSelection();
  if (selection) {
    const configured = modelContextCapacities.get(
      `${selection.providerId}\u0000${selection.modelId}`,
    );
    if (configured) return configured;
  }
  const selected =
    document.querySelector<HTMLElement>(".model-option.on") ??
    document.querySelector<HTMLElement>(".model-option");
  const picker = element<HTMLElement>("modelPickerName");
  const contextWindow = positiveTokenCount(
    selected?.dataset.contextWindow ?? picker?.dataset.contextWindow,
  );
  const maxOutput = positiveTokenCount(
    selected?.dataset.maxOutputTokens ?? picker?.dataset.maxOutputTokens,
  );
  if (!selection && !selected) return undefined;
  return {
    contextWindow: contextWindow ?? DEFAULT_MODEL_CONTEXT_WINDOW,
    maxOutput: maxOutput ?? DEFAULT_MODEL_MAX_OUTPUT_TOKENS,
  };
}

function firstFiniteNumber(...values: unknown[]) {
  for (const value of values) {
    const numeric = finiteNumber(value);
    if (numeric !== undefined) return numeric;
  }
  return undefined;
}

function contextCompactionUsage(
  context: Record<string, unknown> | undefined,
): ContextCompactionUsage | undefined {
  const raw = asRecord(context?.compaction);
  if (!raw || finiteNumber(raw.version) !== 1) return undefined;
  const sourceMessageStart = finiteNumber(raw.sourceMessageStart);
  const sourceMessageEnd = finiteNumber(raw.sourceMessageEnd);
  const sourceTurnStart = finiteNumber(raw.sourceTurnStart);
  const sourceTurnEnd = finiteNumber(raw.sourceTurnEnd);
  const summaryId =
    typeof raw.summaryId === "string" && raw.summaryId.length <= 256
      ? raw.summaryId
      : "";
  const sourceFingerprint =
    typeof raw.sourceFingerprint === "string" &&
    raw.sourceFingerprint.length <= 128
      ? raw.sourceFingerprint
      : "";
  if (
    !summaryId ||
    !sourceFingerprint ||
    sourceMessageStart === undefined ||
    sourceMessageEnd === undefined ||
    sourceTurnStart === undefined ||
    sourceTurnEnd === undefined ||
    sourceMessageStart < 1 ||
    sourceMessageEnd < sourceMessageStart ||
    sourceTurnStart < 1 ||
    sourceTurnEnd < sourceTurnStart
  )
    return undefined;
  const generatedAt = finiteNumber(raw.generatedAt);
  return {
    version: 1,
    summaryId,
    ...(generatedAt && generatedAt > 0 ? { generatedAt } : {}),
    sourceFingerprint,
    sourceMessageStart,
    sourceMessageEnd,
    sourceTurnStart,
    sourceTurnEnd,
  };
}

function contextCompactionTime(value: number | undefined) {
  if (!value) return "本轮生成";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "本轮生成" : date.toLocaleString();
}

function contextUsage(state: PiRuntimeState): ContextUsage | undefined {
  const context = state.contextTrim;
  const modelCapacity = selectedModelContextCapacity();
  const contextWindow =
    finiteNumber(context?.contextWindow) ?? modelCapacity?.contextWindow;
  if (!contextWindow) return undefined;
  const maxOutput = Math.min(
    contextWindow,
    finiteNumber(context?.maxOutputTokens) ?? modelCapacity?.maxOutput ?? 0,
  );
  const observedInput = firstFiniteNumber(
    state.usage?.input,
    state.usage?.inputTokens,
    state.usage?.input_tokens,
  );
  const fixedTokens = finiteNumber(context?.fixedTokens) ?? 0;
  const keptHistoryTokens = finiteNumber(context?.keptHistoryTokens) ?? 0;
  const runtimeEstimate = fixedTokens + keptHistoryTokens;
  const hasRuntimeEstimate = context !== undefined;
  const input =
    observedInput ??
    (hasRuntimeEstimate
      ? runtimeEstimate
      : selectedSessionContextTokenEstimate);
  const cacheRead =
    firstFiniteNumber(
      state.usage?.cacheRead,
      state.usage?.cache_read,
      state.usage?.cachedTokens,
      state.usage?.cached_tokens,
    ) ?? 0;
  const cacheWrite =
    firstFiniteNumber(state.usage?.cacheWrite, state.usage?.cache_write) ?? 0;
  return {
    contextWindow,
    maxOutput,
    input,
    budget: Math.max(1, contextWindow - maxOutput),
    observedInput: observedInput !== undefined,
    source:
      observedInput !== undefined
        ? "provider"
        : hasRuntimeEstimate
          ? "runtime"
          : "estimate",
    cache: cacheRead + cacheWrite,
    fixedTokens,
    keptHistoryTokens,
    keptMessages: finiteNumber(context?.keptMessages) ?? 0,
    droppedMessages: finiteNumber(context?.droppedMessages) ?? 0,
    trimmed: context?.trimmed === true,
    compaction: contextCompactionUsage(context),
  };
}

function contextSourceLabel(source: ContextUsage["source"]) {
  switch (source) {
    case "provider":
      return "供应商统计";
    case "runtime":
      return "本地运行统计";
    default:
      return "会话估算";
  }
}

function contextMetric(label: string, value: string) {
  const item = document.createElement("span");
  const name = document.createElement("b");
  name.textContent = label;
  const amount = document.createElement("strong");
  amount.textContent = value;
  item.append(name, amount);
  return item;
}

function renderContextSection(
  contextSection: HTMLElement | undefined,
  state: PiRuntimeState,
) {
  if (!contextSection) return;
  const usage = contextUsage(state);
  const heading = contextSection.querySelector<HTMLElement>(
    ".section-head strong",
  );
  const meter = contextSection.querySelector<HTMLElement>(".meter");
  const fill = meter?.querySelector<HTMLElement>("i");
  const note = contextSection.querySelector<HTMLElement>(".tiny");
  let breakdown = contextSection.querySelector<HTMLElement>(
    "[data-context-breakdown]",
  );
  if (!breakdown) {
    breakdown = document.createElement("div");
    breakdown.className = "context-breakdown";
    breakdown.dataset.contextBreakdown = "true";
    contextSection.appendChild(breakdown);
  }
  if (!usage) {
    if (heading) heading.textContent = "—";
    if (fill) fill.style.width = "0%";
    if (meter) {
      meter.setAttribute("role", "progressbar");
      meter.setAttribute("aria-label", "当前上下文使用率");
      meter.setAttribute("aria-valuemin", "0");
      meter.setAttribute("aria-valuemax", "100");
      meter.setAttribute("aria-valuenow", "0");
    }
    if (note) note.textContent = "等待当前模型的上下文容量";
    breakdown.replaceChildren(
      contextMetric("已使用", "—"),
      contextMetric("预留", "—"),
      contextMetric("缓存", "—"),
    );
    return;
  }

  const percent = Math.round(
    Math.max(0, Math.min(1, usage.input / usage.contextWindow)) * 100,
  );
  if (heading)
    heading.textContent = `${formatTokens(usage.input)} / ${formatTokens(usage.contextWindow)} tokens`;
  if (fill) fill.style.width = `${percent}%`;
  if (meter) {
    meter.setAttribute("role", "progressbar");
    meter.setAttribute("aria-label", "当前上下文使用率");
    meter.setAttribute("aria-valuemin", "0");
    meter.setAttribute("aria-valuemax", String(usage.contextWindow));
    meter.setAttribute(
      "aria-valuenow",
      String(Math.min(usage.input, usage.contextWindow)),
    );
    meter.setAttribute(
      "aria-valuetext",
      `已使用 ${formatTokens(usage.input)}，总上下文 ${formatTokens(usage.contextWindow)} tokens`,
    );
  }
  if (note) {
    const compactionNote = usage.compaction
      ? ` · 已生成受控摘要 v${usage.compaction.version}（原始历史仍可阅读）`
      : usage.trimmed
        ? ` · 已裁剪 ${Math.round(usage.droppedMessages)} 条消息`
        : "";
    note.textContent = `${contextSourceLabel(usage.source)} · 可用 ${formatTokens(
      Math.max(0, usage.contextWindow - usage.input - usage.maxOutput),
    )} tokens${compactionNote}`;
  }
  breakdown.replaceChildren(
    contextMetric("已使用", formatTokens(usage.input)),
    contextMetric("预留", formatTokens(usage.maxOutput)),
    contextMetric("缓存", formatTokens(usage.cache)),
  );
}

function contextDetail(label: string, value: string) {
  const item = document.createElement("div");
  const term = document.createElement("dt");
  term.textContent = label;
  const definition = document.createElement("dd");
  definition.textContent = value;
  item.append(term, definition);
  return item;
}

function installContextInspector(): ContextInspector | undefined {
  const ring = element<HTMLButtonElement>("btnContextRing");
  const parent = ring?.parentElement;
  if (!ring || !parent || document.getElementById("novaveiContextInspector"))
    return undefined;

  // The visual ring remains compact, while its transparent button target is
  // large enough to be comfortably operable in the composer.
  const control = document.createElement("div");
  control.className = "novavei-context-control";
  Object.assign(control.style, {
    position: "relative",
    display: "grid",
    placeItems: "center",
    flex: "0 0 40px",
    width: "40px",
    height: "40px",
  });
  parent.insertBefore(control, ring);
  control.appendChild(ring);
  ring.style.width = "40px";
  ring.style.height = "40px";
  ring.style.padding = "12px";
  ring.style.flex = "0 0 40px";

  const panel = document.createElement("section");
  panel.id = "novaveiContextInspector";
  panel.className = "model-popover";
  panel.setAttribute("role", "region");
  panel.setAttribute("aria-label", "本轮上下文统计");
  panel.style.width = "min(320px, calc(100vw - 32px))";
  panel.style.gap = "10px";
  panel.style.zIndex = "35";

  const head = document.createElement("div");
  head.className = "model-popover-head";
  const label = document.createElement("span");
  label.textContent = "本轮上下文";
  const summary = document.createElement("strong");
  summary.textContent = "等待统计";
  head.append(label, summary);

  const details = document.createElement("dl");
  details.className = "kv";
  details.style.gridTemplateColumns = "repeat(2, minmax(0, 1fr))";
  details.style.margin = "0";

  const note = document.createElement("p");
  note.className = "dock-note";
  note.style.margin = "0";
  panel.append(head, details, note);
  control.appendChild(panel);

  ring.setAttribute("aria-controls", panel.id);
  ring.setAttribute("aria-expanded", "false");

  const close = (restoreFocus = false) => {
    panel.classList.remove("show");
    ring.setAttribute("aria-expanded", "false");
    if (restoreFocus) ring.focus();
  };
  const onRingClick = () => {
    const willOpen = !panel.classList.contains("show");
    if (!willOpen) {
      close();
      return;
    }
    panel.classList.add("show");
    ring.setAttribute("aria-expanded", "true");
  };
  const onPointerDown = (event: PointerEvent) => {
    const target = event.target;
    if (target instanceof Node && !control.contains(target)) close();
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key !== "Escape" || !panel.classList.contains("show")) return;
    event.preventDefault();
    close(true);
  };
  ring.addEventListener("click", onRingClick);
  document.addEventListener("pointerdown", onPointerDown, true);
  document.addEventListener("keydown", onKeyDown, true);

  return {
    render: (state) => {
      const usage = contextUsage(state);
      details.replaceChildren();
      if (!usage) {
        summary.textContent = "等待统计";
        note.textContent =
          "发送消息后会显示本轮可用上下文、已保留历史和裁剪情况。";
        return;
      }
      summary.textContent = `${formatTokens(usage.input)} / ${formatTokens(usage.contextWindow)}`;
      details.append(
        contextDetail("已使用", formatTokens(usage.input)),
        contextDetail("预留输出", formatTokens(usage.maxOutput)),
        contextDetail("缓存", formatTokens(usage.cache)),
        contextDetail(
          "可用",
          formatTokens(
            Math.max(0, usage.contextWindow - usage.input - usage.maxOutput),
          ),
        ),
        contextDetail("保留消息", String(Math.round(usage.keptMessages))),
        contextDetail("裁剪消息", String(Math.round(usage.droppedMessages))),
      );
      if (usage.compaction) {
        details.append(
          contextDetail("摘要版本", `v${usage.compaction.version}`),
          contextDetail(
            "来源范围",
            `T${usage.compaction.sourceTurnStart}–T${usage.compaction.sourceTurnEnd}`,
          ),
          contextDetail(
            "生成时间",
            contextCompactionTime(usage.compaction.generatedAt),
          ),
        );
      }
      const estimate =
        usage.source === "provider"
          ? "已使用供应商返回的输入统计。"
          : usage.source === "runtime"
            ? `当前为本地运行估算：固定 ${formatTokens(usage.fixedTokens)} + 历史 ${formatTokens(usage.keptHistoryTokens)}。`
            : "当前为已加载会话的本地估算。";
      note.textContent = usage.compaction
        ? `${estimate} 已生成受控摘要 v${usage.compaction.version}，覆盖消息 M${usage.compaction.sourceMessageStart}–M${usage.compaction.sourceMessageEnd}；完整原始历史未删除，可继续在当前会话中阅读或搜索。追溯标识：${usage.compaction.sourceFingerprint}。`
        : usage.trimmed
          ? `${estimate} 为适配窗口，较早的历史未生成摘要；完整原始历史仍可在会话中阅读或搜索。`
          : `${estimate} 本轮历史尚未被裁剪。`;
    },
    dispose: () => {
      ring.removeEventListener("click", onRingClick);
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown, true);
      parent.insertBefore(ring, control);
      control.remove();
    },
  };
}

function updateContextRing(state: PiRuntimeState) {
  const ring = element<HTMLButtonElement>("btnContextRing");
  if (!ring) return;
  const usage = contextUsage(state);
  if (!usage) {
    ring.style.setProperty("--context-pct", "0");
    ring.title = "本轮尚未获得上下文统计";
    ring.setAttribute("aria-label", ring.title);
    return;
  }
  const percent = Math.max(0, Math.min(1, usage.input / usage.contextWindow));
  const label = `上下文 ${formatTokens(usage.input)} / ${formatTokens(usage.contextWindow)} (${Math.round(percent * 100)}%) · 预留 ${formatTokens(usage.maxOutput)} · 缓存 ${formatTokens(usage.cache)}${usage.compaction ? ` · 受控摘要 v${usage.compaction.version}` : usage.trimmed ? " · 已裁剪历史" : ""}`;
  ring.style.setProperty("--context-pct", percent.toFixed(4));
  ring.title = label;
  ring.setAttribute("aria-label", label);
  ring.removeAttribute("data-feature-unavailable");
}

function looksLikeDisplayModel(value: string | undefined) {
  if (!value) return true;
  const pickerLabel =
    element<HTMLElement>("modelPickerName")?.textContent?.trim();
  return (
    value === pickerLabel ||
    [...document.querySelectorAll<HTMLElement>(".model-option")].some(
      (option) => {
        const label =
          option.dataset.piModelLabel ||
          option.textContent?.trim().replace(/\s+/g, " ");
        return label === value;
      },
    )
  );
}

/** Surface composer progress only while Pi still has a live request to process. */
function shouldShowComposerRun(state: PiRuntimeState) {
  if (!state.requestId) return false;
  return ["starting", "running", "waiting_permission", "cancelling"].includes(
    state.status,
  );
}

// Collapsed by default: only the head row is visible unless the user expands,
// or a live permission prompt needs the full body.
let composerRunDetailsExpanded = false;
let composerRunTrackedRequestId: string | undefined;
let composerRunPermissionForcedExpandId: string | undefined;

function applyComposerRunExpanded() {
  const composerRun = element<HTMLElement>("composerRun");
  const toggle = element<HTMLButtonElement>("btnRunDetails");
  if (!composerRun) return;
  composerRun.dataset.piExpanded = composerRunDetailsExpanded
    ? "true"
    : "false";
  if (!toggle) return;
  toggle.setAttribute(
    "aria-expanded",
    composerRunDetailsExpanded ? "true" : "false",
  );
  toggle.setAttribute(
    "aria-label",
    composerRunDetailsExpanded ? "收起执行步骤" : "展开执行步骤",
  );
  toggle.title = composerRunDetailsExpanded ? "收起执行步骤" : "展开执行步骤";
}

function installComposerRunToggle() {
  const toggle = element<HTMLButtonElement>("btnRunDetails");
  if (!toggle || toggle.dataset.piRunToggle === "true") return;
  toggle.dataset.piRunToggle = "true";
  toggle.addEventListener("click", () => {
    composerRunDetailsExpanded = !composerRunDetailsExpanded;
    applyComposerRunExpanded();
  });
  applyComposerRunExpanded();
}

function updateRunChrome(
  state: PiRuntimeState,
  contextInspector?: ContextInspector,
) {
  installComposerRunToggle();
  const composerRun = element<HTMLElement>("composerRun");
  const status = element<HTMLElement>("composerRunStatus");
  const title = element<HTMLElement>("composerRunTitle");
  const send = element<HTMLButtonElement>("btnSend");
  const run = [
    "starting",
    "running",
    "waiting_permission",
    "cancelling",
    "cancel_failed",
  ].includes(state.status);
  const showRunChrome = shouldShowComposerRun(state);
  // Each new turn starts collapsed so steps do not occupy the composer by default.
  if (state.requestId && state.requestId !== composerRunTrackedRequestId) {
    composerRunTrackedRequestId = state.requestId;
    composerRunDetailsExpanded = false;
    composerRunPermissionForcedExpandId = undefined;
  }
  // Auto-expand once per permission id so allow/deny controls are reachable,
  // without re-expanding if the user collapses mid-prompt.
  const pendingPermissionId =
    state.status === "waiting_permission" && state.pendingPermission
      ? state.pendingPermission.id
      : undefined;
  if (
    pendingPermissionId &&
    pendingPermissionId !== composerRunPermissionForcedExpandId
  ) {
    composerRunPermissionForcedExpandId = pendingPermissionId;
    composerRunDetailsExpanded = true;
  }
  updateContextRing(state);
  contextInspector?.render(state);
  if (status) {
    const label =
      state.status === "waiting_permission"
        ? "等待批准"
        : state.status === "starting"
          ? "启动中"
          : state.status === "running"
            ? "运行中"
            : state.status === "cancelling"
              ? "取消中"
              : state.status === "cancel_failed"
                ? "停止失败"
                : state.status === "cancelled"
                  ? "已取消"
                  : state.status === "error"
                    ? "失败"
                    : state.status === "completed"
                      ? "完成"
                      : "待命";
    // Keep the status pill structure (dot + label) so CSS can restyle by state.
    const marker = document.createElement("i");
    marker.setAttribute("aria-hidden", "true");
    status.replaceChildren(marker, document.createTextNode(label));
    status.dataset.piStatus = state.status;
  }
  if (title) {
    const toolText = toolSummary(state.tools);
    const pendingTool = state.pendingPermission?.toolName?.trim();
    const fullTitle =
      state.status === "waiting_permission"
        ? pendingTool
          ? `工具 ${pendingTool} 请求访问本地工作区`
          : "等待工具权限确认"
        : state.status === "error"
          ? state.error || "Pi 运行失败"
          : state.status === "cancel_failed"
            ? state.cancellationError || "停止请求失败，请重试停止"
            : state.status === "cancelling"
              ? "正在请求停止当前运行"
              : toolText ||
                (state.status === "completed"
                  ? "本轮响应已完成"
                  : state.status === "idle"
                    ? "等待发送消息"
                    : "Pi 正在处理你的请求");
    title.textContent = displaySnippet(fullTitle, 96);
    title.title = fullTitle;
    if (state.status === "error" || state.status === "cancel_failed") {
      title.setAttribute("role", "alert");
      title.setAttribute("aria-live", "assertive");
    } else {
      title.removeAttribute("role");
      title.removeAttribute("aria-live");
    }
  }
  if (composerRun) {
    composerRun.hidden = !showRunChrome;
    composerRun.setAttribute("aria-hidden", showRunChrome ? "false" : "true");
    delete composerRun.dataset.piPermissionOnly;
    composerRun.dataset.piRequestId = state.requestId ?? "";
    composerRun.dataset.piStatus = state.status;
    composerRun.setAttribute(
      "aria-busy",
      run && showRunChrome ? "true" : "false",
    );
    applyComposerRunExpanded();
  }
  if (send) {
    send.dataset.piRunning = run ? "true" : "false";
    const cancelLabel =
      state.status === "cancelling"
        ? "取消中…"
        : state.status === "cancel_failed"
          ? "重试停止"
          : "停止";
    send.textContent = run ? cancelLabel : "发送";
    send.setAttribute(
      "aria-label",
      run
        ? state.status === "cancelling"
          ? "正在停止当前运行"
          : state.status === "cancel_failed"
            ? "停止请求失败，点击重试"
            : "停止当前运行"
        : "发送",
    );
    if (run)
      send.title =
        state.status === "cancel_failed"
          ? safeRuntimeMessage(
              state.cancellationError || "停止请求失败，点击重试",
            )
          : state.status === "cancelling"
            ? "正在请求停止当前运行"
            : "停止当前运行";
    // Cancellation is already in flight. Keep the control visibly disabled
    // rather than leaving a clickable-looking button whose only safe action
    // is to ignore repeated stop requests.
    if (run) send.disabled = state.status === "cancelling";
    send.classList.toggle("stop", run);
    if (!run) syncComposerProviderAvailability();
  }
}

export function createDomPiRuntime(controller: PiRuntimeController) {
  installSettingsTablists();
  let activeNode: HTMLElement | null = null;
  let activeRequestId: string | undefined;
  let activeLiveMessageId: string | undefined;
  let sessionViewInvalidated = false;
  let activeStartedAt = Date.now();
  let renderedError = "";
  let renderedTerminal = "";
  let activePresentation: AssistantPresentation | undefined;
  let pendingPresentation: AssistantPresentation | undefined;
  const presentationsByRequest = new Map<string, AssistantPresentation>();
  const startedAtByRequest = new Map<string, number>();
  const liveTurnsBySession = new Map<string, LiveTurnContext>();
  let pendingLiveAssistantText = "";
  let pendingLiveAssistantThinking = "";
  let pendingLiveAssistantNode: HTMLElement | null = null;
  let pendingLiveAssistantTimer: number | undefined;
  let lastLiveAssistantRenderedAt = 0;
  let lastLiveChromeRenderedAt = 0;
  let lastRenderedStreamEvent: PiRuntimeState["lastEvent"];
  const planConfirmationCards = new PlanConfirmationCards();
  const planConfirmationContexts = new Map<
    string,
    { requestId?: string; terminal: boolean; resolved: boolean }
  >();
  const subagentTasksBySession = new Map<
    string,
    Map<string, SubagentTaskSummary>
  >();
  const reviewedWorktreesByTaskId = new Map<string, WorktreeReview>();
  const pendingWorktreeTaskIds = new Set<string>();
  let disposeSubagentTaskListener: (() => void) | undefined;
  const subagentTasksForSession = (sessionId: string | undefined) => {
    if (!sessionId) return [];
    return [...(subagentTasksBySession.get(sessionId)?.values() ?? [])];
  };
  const subagentTasksForState = (state: PiRuntimeState) =>
    subagentTasksForSession(
      state.sessionId ?? window.__novaveiHost?.getSessionId(),
    );
  const renderSubagentTasksForCurrentState = () => {
    const state = controller.getState();
    renderRunDock(
      state,
      activeStartedAt,
      subagentTasksForState(state),
      worktreeReviewActions,
    );
  };
  const storeSubagentTask = (task: SubagentTaskSummary) => {
    const tasks = subagentTasksBySession.get(task.sessionId) ?? new Map();
    tasks.set(task.id, task);
    subagentTasksBySession.set(task.sessionId, tasks);
    if (task.status !== "review_ready")
      reviewedWorktreesByTaskId.delete(task.id);
  };
  const invokeWorktreeAction = async (
    command: string,
    task: SubagentTaskSummary,
    extra: UnknownRecord = {},
  ) => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) throw new Error("native_unavailable");
    return invoke<unknown>(command, {
      sessionId: task.sessionId,
      taskId: task.id,
      ...extra,
    });
  };
  const viewWorktreePatch = async (task: SubagentTaskSummary) => {
    if (pendingWorktreeTaskIds.has(task.id)) return;
    pendingWorktreeTaskIds.add(task.id);
    renderSubagentTasksForCurrentState();
    try {
      const review = worktreeReview(
        await invokeWorktreeAction("worktree_task_review_get", task),
      );
      if (!review || review.taskId !== task.id)
        throw new Error("invalid_review");
      reviewedWorktreesByTaskId.set(task.id, review);
    } catch {
      // Native Git errors can contain managed paths. Keep renderer feedback
      // generic rather than turning a dock toast into a path disclosure.
      toast("无法加载工作树补丁，请确认任务仍待审阅后重试。");
    } finally {
      pendingWorktreeTaskIds.delete(task.id);
      renderSubagentTasksForCurrentState();
    }
  };
  const applyWorktreePatch = async (task: SubagentTaskSummary) => {
    const review = reviewedWorktreesByTaskId.get(task.id);
    if (!review || review.taskId !== task.id) {
      toast("请先查看当前补丁，再应用该精确版本。");
      return;
    }
    if (pendingWorktreeTaskIds.has(task.id)) return;
    pendingWorktreeTaskIds.add(task.id);
    renderSubagentTasksForCurrentState();
    try {
      const updated = subagentTaskSummary(
        await invokeWorktreeAction("worktree_task_apply", task, {
          digest: review.digest,
        }),
      );
      if (!updated || updated.id !== task.id)
        throw new Error("invalid_task_update");
      storeSubagentTask(updated);
      toast("补丁已应用；请在任务列表中显式清理隔离工作树。");
    } catch {
      toast("未能应用补丁。请检查审阅状态并在原生确认窗口中确认。");
    } finally {
      pendingWorktreeTaskIds.delete(task.id);
      renderSubagentTasksForCurrentState();
    }
  };
  const discardWorktree = async (task: SubagentTaskSummary) => {
    if (pendingWorktreeTaskIds.has(task.id)) return;
    pendingWorktreeTaskIds.add(task.id);
    renderSubagentTasksForCurrentState();
    try {
      const updated = subagentTaskSummary(
        await invokeWorktreeAction("worktree_task_discard", task),
      );
      if (!updated || updated.id !== task.id)
        throw new Error("invalid_task_update");
      storeSubagentTask(updated);
      toast(
        task.status === "cleanup_pending"
          ? "隔离工作树已清理。"
          : "隔离工作树和未应用补丁已丢弃。",
      );
    } catch {
      toast("未能清理隔离工作树。请在原生确认窗口中确认后重试。");
    } finally {
      pendingWorktreeTaskIds.delete(task.id);
      renderSubagentTasksForCurrentState();
    }
  };
  const worktreeReviewActions: WorktreeReviewActions = {
    reviewsByTaskId: reviewedWorktreesByTaskId,
    pendingTaskIds: pendingWorktreeTaskIds,
    view: (task) => void viewWorktreePatch(task),
    apply: (task) => void applyWorktreePatch(task),
    discard: (task) => void discardWorktree(task),
  };
  const hydrateSubagentTasks = async (sessionId: string | undefined) => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke || !sessionId) return;
    try {
      const tasks = await invoke<unknown[]>("subagent_tasks_list", {
        sessionId,
        limit: 30,
      });
      const summaries = tasks
        .map(subagentTaskSummary)
        .filter((task): task is SubagentTaskSummary => Boolean(task));
      const stored = new Map(summaries.map((task) => [task.id, task]));
      subagentTasksBySession.set(sessionId, stored);
      if (
        (controller.getState().sessionId ??
          window.__novaveiHost?.getSessionId()) === sessionId
      ) {
        renderSubagentTasksForCurrentState();
      }
    } catch {
      // Task history is auxiliary observability; a missing command on an
      // older host must not disrupt normal chat rendering.
    }
  };
  let permissionPrompt: HTMLElement | null = null;
  const answerPermission = async (decision: PermissionDecision) => {
    if (permissionDecisionsBlockedByRecovery()) {
      hidePermissionPromptForRecovery(permissionPrompt);
      throw new Error(STORAGE_RECOVERY_PERMISSION_MESSAGE);
    }
    return controller.answerPermission(decision);
  };
  permissionPrompt = ensurePermissionPrompt(answerPermission);
  const contextInspector = installContextInspector();
  const disposeSessionModelPickerPersistence =
    installSessionModelPickerPersistence();
  const disposeProjectPreferenceBindings = installProjectPreferenceBindings();
  if (hasNativeProviderSettings()) {
    // NativeShell publishes app_health after installing this runtime. Keep the
    // prototype fallback hidden until that durable-read gate arrives.
    modelPickerAvailability = "loading";
    renderProviderModelNotice("loading", "正在检查本地服务状态，请稍候。");
  } else {
    void hydrateModelPickerMetadata();
  }
  const onProvidersChanged = () => void hydrateModelPickerMetadata();
  const onHostStateChanged = () => syncComposerProviderAvailability();
  const onAppHealthChanged = (event: Event) => {
    const detail = event instanceof CustomEvent ? event.detail : undefined;
    applyAppHealth(detail);
    // A Pi run can already be waiting for a permission decision when storage
    // becomes unavailable. The controller will not necessarily emit another
    // state update, so hide and disable its action surface immediately.
    renderPermissionPrompt(permissionPrompt, controller.getState());
    if (!providerSettingsReadable) {
      // Invalidate any in-flight pre-recovery hydration before replacing the
      // picker with a safe, non-actionable recovery notice.
      modelPickerHydrationSerial += 1;
      modelPickerAvailability = "recovery_required";
      modelPickerAvailabilityError = "";
      renderProviderModelNotice(
        "recovery_required",
        providerAvailabilityMessage(),
      );
      return;
    }
    void refreshLocalProviderProxyStatus().then(() => {
      if (providerSettingsReadable) void hydrateModelPickerMetadata();
    });
  };
  const onModelOptionsRendered = () => {
    const state = controller.getState();
    updateRunChrome(state, contextInspector);
    renderRunDock(
      state,
      activeStartedAt,
      subagentTasksForState(state),
      worktreeReviewActions,
    );
  };
  const onSessionChanged = (event: Event) => {
    const detail =
      event instanceof CustomEvent ? asRecord(event.detail) : undefined;
    const sessionId = readString(detail, "sessionId");
    if (!sessionId) return;
    sessionViewInvalidated = false;
    selectedSessionContextTokenEstimate =
      finiteNumber(detail?.contextTokenEstimate) ?? 0;
    // The transcript has already been reloaded by the native shell. Forget
    // the previous view node before the controller emits the newly selected
    // session's state, otherwise a background stream could paint into it.
    cancelPendingLiveAssistantRender();
    planConfirmationCards.invalidateAll();
    planConfirmationContexts.clear();
    activeNode = null;
    activeRequestId = undefined;
    activeLiveMessageId = undefined;
    activePresentation = undefined;
    renderedError = "";
    renderedTerminal = "";
    controller.selectSession(sessionId);
    void hydrateSubagentTasks(sessionId);
  };
  const onSessionViewInvalidated = () => {
    // Native navigation invalidates the visible view before its async load
    // starts. Background stream state still persists through Host, but it may
    // not create or toast into the outgoing transcript.
    sessionViewInvalidated = true;
    cancelPendingLiveAssistantRender();
    planConfirmationCards.invalidateAll();
    planConfirmationContexts.clear();
    activeNode = null;
    activeRequestId = undefined;
    activeLiveMessageId = undefined;
    activePresentation = undefined;
    renderedError = "";
    renderedTerminal = "";
  };
  const onTranscriptWindowRendered = (event: Event) => {
    const detail =
      event instanceof CustomEvent ? asRecord(event.detail) : undefined;
    const sessionId = readString(detail, "sessionId");
    const state = controller.getState();
    if (
      sessionViewInvalidated ||
      !sessionId ||
      !state.requestId ||
      state.sessionId !== sessionId
    )
      return;
    // Host has just rebuilt the virtual window. Rebind the active stream to
    // its stable live id so the next delta never targets a detached article.
    renderState(state);
  };
  window.addEventListener("novavei:providers-changed", onProvidersChanged);
  window.addEventListener("novavei:host-state-changed", onHostStateChanged);
  window.addEventListener("novavei:app-health-changed", onAppHealthChanged);
  window.addEventListener(
    "novavei:model-options-rendered",
    onModelOptionsRendered,
  );
  window.addEventListener("novavei:session-changed", onSessionChanged);
  window.addEventListener(
    "novavei:session-view-invalidated",
    onSessionViewInvalidated,
  );
  window.addEventListener(
    "novavei:transcript-window-rendered",
    onTranscriptWindowRendered,
  );
  const listen = window.__TAURI__?.event?.listen;
  if (listen) {
    void listen<unknown>("subagent:task-update", ({ payload }) => {
      const task = subagentTaskSummary(payload);
      if (!task) return;
      storeSubagentTask(task);
      renderSubagentTasksForCurrentState();
    })
      .then((unlisten) => {
        disposeSubagentTaskListener = unlisten;
      })
      .catch(() => undefined);
  }
  void hydrateSubagentTasks(window.__novaveiHost?.getSessionId());

  function cancelPendingLiveAssistantRender() {
    if (pendingLiveAssistantTimer !== undefined) {
      window.clearTimeout(pendingLiveAssistantTimer);
      pendingLiveAssistantTimer = undefined;
    }
    lastLiveAssistantRenderedAt = 0;
    pendingLiveAssistantText = "";
    pendingLiveAssistantThinking = "";
    pendingLiveAssistantNode = null;
  }

  function liveTurnKey(sessionId: string | undefined) {
    const normalized = sessionId?.trim();
    return normalized || "__novavei-unbound-session__";
  }

  function createLiveMessageId(prefix: string) {
    try {
      if (typeof crypto?.randomUUID === "function")
        return `${prefix}:${crypto.randomUUID()}`;
    } catch {
      // Older WebViews can expose crypto without randomUUID.
    }
    return `${prefix}:${Date.now().toString(36)}:${Math.random().toString(36).slice(2, 10)}`;
  }

  function publishLiveTranscriptMessage(message: LiveTranscriptMessage) {
    const host = window.__novaveiHost;
    if (!host?.upsertLiveTranscriptMessage) return false;
    host.upsertLiveTranscriptMessage(message);
    return true;
  }

  function liveTurnForState(state: PiRuntimeState) {
    const sessionId = (
      state.sessionId ?? window.__novaveiHost?.getSessionId()
    )?.trim();
    if (!sessionId) return undefined;
    const key = liveTurnKey(sessionId);
    const requestId = state.requestId?.trim();
    let turn = liveTurnsBySession.get(key);
    if (
      !turn ||
      (requestId && turn.requestId && turn.requestId !== requestId)
    ) {
      const createdAt = Date.now();
      const seed = requestId ?? createLiveMessageId("turn");
      const presentation = (requestId
        ? presentationsByRequest.get(requestId)
        : undefined) ??
        pendingPresentation ?? {
          model: modelLabel(),
          reasoning: selectedReasoning(),
          permission: permissionLabel(),
        };
      turn = {
        sessionId,
        userMessageId: `live:user:${seed}`,
        assistantMessageId: `live:assistant:${seed}`,
        createdAt,
        displayText: state.prompt || "",
        presentation,
      };
      liveTurnsBySession.set(key, turn);
    }
    if (requestId) {
      turn.requestId = requestId;
      if (!presentationsByRequest.has(requestId))
        presentationsByRequest.set(requestId, turn.presentation);
      if (pendingPresentation === turn.presentation)
        pendingPresentation = undefined;
    }
    if (state.turnId) turn.turnId = state.turnId;
    return turn;
  }

  function syncLiveTranscriptFromState(state: PiRuntimeState) {
    const turn = liveTurnForState(state);
    if (!turn) return undefined;
    const presentation =
      (state.requestId
        ? presentationsByRequest.get(state.requestId)
        : undefined) ?? turn.presentation;
    const userContent = turn.displayText || state.prompt || "[附件]";
    const publishedUser = turn.publishedUser;
    if (
      !publishedUser ||
      publishedUser.content !== userContent ||
      publishedUser.requestId !== turn.requestId ||
      publishedUser.turnId !== turn.turnId ||
      publishedUser.status !== state.status
    ) {
      const published = publishLiveTranscriptMessage({
        id: turn.userMessageId,
        sessionId: turn.sessionId,
        role: "user",
        content: userContent,
        createdAt: turn.createdAt,
        requestId: turn.requestId,
        turnId: turn.turnId,
        status: state.status,
      });
      if (published) {
        turn.publishedUser = {
          content: userContent,
          requestId: turn.requestId,
          turnId: turn.turnId,
          status: state.status,
        };
      }
    }
    publishLiveTranscriptMessage({
      id: turn.assistantMessageId,
      sessionId: turn.sessionId,
      role: "assistant",
      content: state.assistantText,
      createdAt: turn.createdAt + 1,
      requestId: turn.requestId,
      turnId: turn.turnId,
      model: presentation?.model,
      reasoning: presentation?.reasoning,
      status: state.status,
      prompt: state.prompt,
    });
    return turn;
  }

  function liveAssistantPlaceholder(liveMessageId: string) {
    return [
      ...document.querySelectorAll<HTMLElement>("[data-live-message-id]"),
    ].find((item) => item.dataset.liveMessageId === liveMessageId);
  }

  function rebindActiveAssistantNode(state: PiRuntimeState) {
    const liveMessageId = activeLiveMessageId;
    if (!liveMessageId || !state.requestId) {
      activeNode = null;
      return null;
    }
    if (
      activeNode?.isConnected &&
      activeNode.dataset.liveMessageId === liveMessageId
    ) {
      return activeNode;
    }
    const placeholder = liveAssistantPlaceholder(liveMessageId);
    if (placeholder?.dataset.novaveiRuntime === "pi") {
      activeNode = placeholder;
      return activeNode;
    }
    if (placeholder) {
      activeNode = createAssistantMessage(activePresentation, {
        messageId: placeholder.dataset.messageId,
        liveMessageId,
        replace: placeholder,
      });
      if (activeNode) activeNode.dataset.piPrompt = state.prompt ?? "";
      return activeNode;
    }
    // Browser preview has no NativeShell state bridge. Preserve its previous
    // direct rendering path without making it a second source of truth in the
    // desktop WebView.
    if (!window.__novaveiHost?.upsertLiveTranscriptMessage) {
      activeNode = createAssistantMessage(activePresentation, {
        liveMessageId,
      });
      if (activeNode) activeNode.dataset.piPrompt = state.prompt ?? "";
      return activeNode;
    }
    activeNode = null;
    return null;
  }

  async function handlePlanConfirmation(
    plan: PiPlanConfirmation,
    decision: PiPlanConfirmationDecision,
  ): Promise<PlanConfirmationCardResult> {
    const context = planConfirmationContexts.get(plan.id);
    if (!context) return "invalidated";

    if (decision === "execute") {
      if (context.terminal) {
        // A provider can finish after emitting a plan-only reply. Execute then
        // starts a clearly labeled follow-up turn rather than pretending an
        // old completed Agent can be resumed.
        if (!providerModelIsReady()) {
          const message = providerAvailabilityMessage();
          if (!localProviderProxyAvailable) void retryLocalProviderProxy();
          else if (modelPickerAvailability !== "loading")
            openProviderSettings();
          toast(message || "请先在设置中配置供应商。");
          return "retry";
        }
        const approval = controller.issuePlanContinuationApproval(plan.id);
        if (!approval) return "retry";
        context.resolved = true;
        void submit({
          text: planExecutionFollowUpText(plan),
          displayText: "执行已确认的计划",
          planApproval: approval,
        }).then((result) => {
          if (!result) {
            controller.invalidatePlanConfirmation(plan.id, context.requestId);
            planConfirmationCards.setStatus(
              context.requestId,
              plan.id,
              "invalidated",
            );
          }
        });
        return "executing";
      }
      const accepted = await controller.answerPlanConfirmation("execute");
      if (accepted) context.resolved = true;
      return accepted ? "approved" : "retry";
    }

    if (!context.terminal) {
      const accepted = await controller.answerPlanConfirmation(decision);
      if (!accepted) return "invalidated";
      context.resolved = true;
    }
    if (context.terminal) {
      context.resolved = true;
      // A deferred/modified terminal plan must not retain a grant that could
      // later be minted through a stale DOM reference.
      controller.invalidatePlanConfirmation(plan.id);
    }
    if (decision === "modify") {
      const composer = element<HTMLTextAreaElement>("composerInput");
      if (composer) {
        composer.value = "请修改上述执行计划：";
        composer.focus();
      }
      return "modify_requested";
    }
    return "deferred";
  }

  /**
   * Provider events can arrive token-by-token. Coalesce the growing Markdown
   * source to a bounded cadence while immediately flushing a terminal response.
   */
  function renderLiveAssistantText(
    assistantText: string,
    thinkingText: string,
    flushImmediately: boolean,
  ) {
    if (!activeNode) return;
    pendingLiveAssistantText = stripPlanProtocolBlocks(assistantText);
    pendingLiveAssistantThinking = thinkingText;
    pendingLiveAssistantNode = activeNode;

    const flush = () => {
      const targetNode = pendingLiveAssistantNode;
      const latestText = pendingLiveAssistantText;
      const latestThinking = pendingLiveAssistantThinking;
      pendingLiveAssistantTimer = undefined;
      if (!targetNode?.isConnected) return;
      renderAssistantThinking(targetNode, latestThinking);
      const text = targetNode.querySelector<HTMLElement>("[data-pi-text]");
      if (text) renderMarkdown(text, latestText);
      lastLiveAssistantRenderedAt = Date.now();
    };

    if (flushImmediately) {
      cancelPendingLiveAssistantRender();
      pendingLiveAssistantText = stripPlanProtocolBlocks(assistantText);
      pendingLiveAssistantThinking = thinkingText;
      pendingLiveAssistantNode = activeNode;
      flush();
      return;
    }
    if (pendingLiveAssistantTimer === undefined) {
      pendingLiveAssistantTimer = window.setTimeout(
        flush,
        liveMarkdownRenderDelay(lastLiveAssistantRenderedAt),
      );
    }
  }

  const renderState = (state: PiRuntimeState) => {
    if (sessionViewInvalidated) {
      if (state.requestId) syncLiveTranscriptFromState(state);
      return;
    }
    const streamEvent = state.lastEvent;
    const hasNewStreamEvent = streamEvent !== lastRenderedStreamEvent;
    lastRenderedStreamEvent = streamEvent;

    // Thinking changes neither run chrome nor the visible answer. Skip the
    // expensive transcript, dock, and permission refreshes; the shared frame
    // coalescer below updates the disclosure at most once per paint. Checking
    // event identity prevents a later non-stream action from inheriting this
    // fast path through the reducer's retained `lastEvent` field.
    if (
      hasNewStreamEvent &&
      streamEvent?.type === "thinking_delta" &&
      state.requestId === activeRequestId &&
      activeNode?.isConnected
    ) {
      renderLiveAssistantText(state.assistantText, state.thinkingText, false);
      return;
    }

    // Text deltas still update the cached live transcript and Markdown source,
    // but status chrome and the run dock do not need token-level DOM writes.
    // Refresh those surfaces at a human-visible cadence while keeping tool,
    // permission, cancellation, and terminal events immediate.
    if (
      hasNewStreamEvent &&
      streamEvent?.type === "text_delta" &&
      state.requestId === activeRequestId &&
      activeNode?.isConnected &&
      Date.now() - lastLiveChromeRenderedAt < LIVE_CHROME_RENDER_INTERVAL_MS
    ) {
      syncLiveTranscriptFromState(state);
      renderLiveAssistantText(state.assistantText, state.thinkingText, false);
      activeNode.dataset.piSource = stripPlanProtocolBlocks(
        state.assistantText,
      );
      return;
    }
    lastLiveChromeRenderedAt = Date.now();
    updateRunChrome(state, contextInspector);
    const liveTurn = state.requestId ? liveTurnForState(state) : undefined;
    if (state.requestId && state.requestId !== activeRequestId) {
      const previousRequestId = activeRequestId;
      if (previousRequestId) {
        for (const [planId, context] of planConfirmationContexts) {
          if (context.requestId !== previousRequestId) continue;
          if (!context.resolved)
            planConfirmationCards.invalidateRequest(previousRequestId);
          planConfirmationContexts.delete(planId);
        }
      }
      cancelPendingLiveAssistantRender();
      activeRequestId = state.requestId;
      activeStartedAt = startedAtByRequest.get(state.requestId) ?? Date.now();
      startedAtByRequest.set(state.requestId, activeStartedAt);
      renderedError = "";
      renderedTerminal = "";
      const storedPresentation = presentationsByRequest.get(state.requestId);
      activePresentation = storedPresentation ??
        pendingPresentation ?? {
          model: modelLabel(),
          reasoning: selectedReasoning(),
          permission: permissionLabel(),
        };
      if (!storedPresentation) {
        presentationsByRequest.set(state.requestId, activePresentation);
        pendingPresentation = undefined;
      }
      activeLiveMessageId = liveTurn?.assistantMessageId;
      syncLiveTranscriptFromState(state);
      rebindActiveAssistantNode(state);
    } else if (state.requestId) {
      if (!activeLiveMessageId)
        activeLiveMessageId = liveTurn?.assistantMessageId;
      syncLiveTranscriptFromState(state);
      if (!activeNode?.isConnected) rebindActiveAssistantNode(state);
    }
    renderPermissionPrompt(permissionPrompt, state);
    if (state.requestId && shouldShowComposerRun(state))
      renderComposerSteps(state);
    renderRunDock(
      state,
      activeStartedAt,
      subagentTasksForState(state),
      worktreeReviewActions,
    );
    if (!activeNode) return;
    activeNode.dataset.piStatus = state.status;
    activeNode.dataset.piToolCount = String(Object.keys(state.tools).length);
    const assistantCompleted = isAssistantCompletedStatus(state.status);
    const assistantTerminal =
      assistantCompleted ||
      ["cancelled", "error", "cancel_failed"].includes(state.status);
    const assistantWaiting = isAssistantLivePlaceholderStatus(state.status);
    toggleCompletionOnlyAssistantChrome(activeNode, assistantCompleted);
    const text = activeNode.querySelector<HTMLElement>("[data-pi-text]");
    if (text) {
      text.toggleAttribute("aria-busy", assistantWaiting);
      text.setAttribute(
        "aria-label",
        assistantWaiting ? "NovaVei 正在生成回复" : "助手回复",
      );
      renderLiveAssistantText(
        state.assistantText,
        state.thinkingText,
        assistantTerminal,
      );
    }
    activeNode.dataset.piSource = stripPlanProtocolBlocks(state.assistantText);
    if (state.pendingPlan && state.requestId) {
      // A replacement protocol block is a new reviewed object. Retire the
      // earlier card immediately so it cannot look actionable while the gate
      // has already revoked its decision.
      for (const [planId, context] of planConfirmationContexts) {
        if (
          context.requestId !== state.requestId ||
          planId === state.pendingPlan.id
        )
          continue;
        planConfirmationCards.setStatus(
          context.requestId,
          planId,
          "invalidated",
        );
        planConfirmationContexts.delete(planId);
      }
      const previous = planConfirmationContexts.get(state.pendingPlan.id);
      planConfirmationContexts.set(state.pendingPlan.id, {
        requestId: state.requestId,
        terminal: state.status === "completed",
        resolved: previous?.resolved ?? state.pendingPlan.status !== "pending",
      });
      planConfirmationCards.render(
        activeNode,
        state.requestId,
        state.pendingPlan,
        handlePlanConfirmation,
      );
    } else if (
      state.requestId &&
      ["cancelled", "error", "cancel_failed"].includes(state.status)
    ) {
      planConfirmationCards.invalidateRequest(state.requestId);
      for (const [planId, context] of planConfirmationContexts) {
        if (context.requestId === state.requestId)
          planConfirmationContexts.delete(planId);
      }
    }
    const runtimeIssue =
      state.status === "cancel_failed"
        ? state.cancellationError
        : state.status === "error"
          ? state.error
          : undefined;
    let issue = activeNode.querySelector<HTMLElement>(
      "[data-pi-runtime-issue]",
    );
    if (runtimeIssue) {
      if (!issue) {
        issue = document.createElement("p");
        issue.className = "dock-note";
        issue.dataset.piRuntimeIssue = "true";
        issue.setAttribute("role", "alert");
        const actions = activeNode.querySelector<HTMLElement>(".msg-actions");
        if (actions) actions.before(issue);
        else activeNode.appendChild(issue);
      }
      issue.textContent =
        state.status === "cancel_failed"
          ? `停止请求失败：${safeRuntimeMessage(runtimeIssue)}。可点击“重试停止”继续终止本轮。`
          : `本轮运行失败：${safeRuntimeMessage(runtimeIssue)}。可查看轨迹，或将原始问题重新发送。`;
    } else {
      issue?.remove();
    }
    const model = activeNode.querySelector<HTMLElement>("[data-pi-model]");
    if (model && activePresentation && assistantCompleted)
      model.textContent = `${activePresentation.model} · ${activePresentation.permission}`;
    const meta = activeNode.querySelector<HTMLElement>("[data-pi-meta]");
    if (meta && activePresentation && assistantCompleted) {
      const modelMeta = meta.querySelector("b");
      if (modelMeta) modelMeta.textContent = activePresentation.model;
      const permissionMeta = meta.querySelector("span:not(.sep)");
      if (permissionMeta)
        permissionMeta.textContent = reasoningLabel(
          activePresentation.reasoning,
        );
    }
    const ended = activeNode.querySelector<HTMLElement>("[data-pi-ended]");
    if (
      ended instanceof HTMLTimeElement &&
      assistantCompleted &&
      !ended.dateTime
    ) {
      const finished = new Date();
      ended.textContent = nowLabel(finished);
      ended.dateTime = finished.toISOString();
    }
    if (
      (state.status === "error" || state.status === "cancel_failed") &&
      runtimeIssue &&
      `${state.requestId}:${state.status}:${runtimeIssue}` !== renderedError
    ) {
      renderedError = `${state.requestId}:${state.status}:${runtimeIssue}`;
      toast(safeRuntimeMessage(runtimeIssue));
    }
    if (
      ["completed", "cancelled"].includes(state.status) &&
      `${state.requestId}:${state.status}` !== renderedTerminal
    ) {
      renderedTerminal = `${state.requestId}:${state.status}`;
      toast(state.status === "completed" ? "已完成" : "已取消");
    }
    window.__novaveiFloorNav?.refresh?.();
  };

  const unsubscribe = controller.subscribe(renderState);
  const submit = async (input: Omit<PiRunInput, "requestId">) => {
    if (!providerModelIsReady()) {
      const message = providerAvailabilityMessage();
      if (!localProviderProxyAvailable) void retryLocalProviderProxy();
      else if (modelPickerAvailability !== "loading") openProviderSettings();
      toast(message || "请先在设置中配置供应商。");
      return undefined;
    }
    const selection = selectedModelSelection();
    const model = looksLikeDisplayModel(input.model)
      ? (selection.modelId ?? input.model)
      : input.model;
    const nativeSessionId = window.__novaveiHost?.getSessionId();
    const nativeWorkdir = window.__novaveiHost?.getWorkdir();
    const originalText = input.text;
    const composerAttachments = window.__novaveiComposerAttachments;
    let attachmentPayload:
      | ReturnType<NonNullable<typeof composerAttachments>["prepare"]>
      | undefined;
    let runtimeText = originalText;
    try {
      attachmentPayload = composerAttachments?.has()
        ? composerAttachments.prepare(originalText)
        : undefined;
      runtimeText =
        attachmentPayload?.text ??
        composerAttachments?.augment(originalText) ??
        originalText;
    } catch (error) {
      toast(error instanceof Error ? error.message : String(error));
      return undefined;
    }
    const displayText =
      attachmentPayload?.displayText ?? input.displayText ?? input.text;
    const request: Omit<PiRunInput, "requestId"> = {
      ...input,
      text: runtimeText,
      displayText,
      images: attachmentPayload?.images ?? input.images,
      providerId: input.providerId || selection.providerId,
      model,
      reasoning: input.reasoning ?? selectedReasoning(),
      permission: selectedPermission(),
      sessionId: nativeSessionId || input.sessionId,
      cwd: nativeWorkdir || input.cwd || selectedWorkdir(),
    };
    const submissionPresentation: AssistantPresentation = {
      model: modelLabel(),
      reasoning: request.reasoning ?? selectedReasoning(),
      permission: permissionLabel(),
    };
    pendingPresentation = submissionPresentation;
    const optimisticSessionId = request.sessionId?.trim();
    const optimisticText = displayText.trim() || "[附件]";
    const optimisticTurn: LiveTurnContext | undefined = optimisticSessionId
      ? {
          sessionId: optimisticSessionId,
          userMessageId: `live:user:${createLiveMessageId("turn")}`,
          assistantMessageId: "",
          createdAt: Date.now(),
          displayText: optimisticText,
          presentation: submissionPresentation,
        }
      : undefined;
    if (optimisticTurn) {
      optimisticTurn.assistantMessageId = optimisticTurn.userMessageId.replace(
        "live:user:",
        "live:assistant:",
      );
      liveTurnsBySession.set(
        liveTurnKey(optimisticTurn.sessionId),
        optimisticTurn,
      );
    }
    // A normal host session-change event selects this already. Selecting here
    // as well closes the short bootstrap race where a ready Composer can send
    // before the initial session notification has reached this module.
    controller.selectSession(request.sessionId);
    const hostAcceptedOptimisticMessage =
      optimisticTurn &&
      publishLiveTranscriptMessage({
        id: optimisticTurn.userMessageId,
        sessionId: optimisticTurn.sessionId,
        role: "user",
        content: optimisticTurn.displayText,
        createdAt: optimisticTurn.createdAt,
      });
    if (!hostAcceptedOptimisticMessage)
      appendUserMessage(
        optimisticText,
        request.sessionId,
        optimisticTurn?.userMessageId,
      );
    const composer = element<HTMLTextAreaElement>("composerInput");
    // The optimistic transcript can be rendered before a transport accepts the
    // turn. Keep the exact visible draft long enough to recover it if that
    // acceptance fails, without overwriting text the user starts typing while
    // the request is in flight.
    const composerDraft = composer?.value ?? "";
    let composerEditedAfterClear = false;
    const noteComposerEdit = () => {
      composerEditedAfterClear = true;
    };
    if (composer) {
      composer.addEventListener("input", noteComposerEdit);
      composer.value = "";
    }
    const restoreComposerDraft = () => {
      composer?.removeEventListener("input", noteComposerEdit);
      const currentSessionId =
        window.__novaveiHost?.getSessionId?.()?.trim() ?? "";
      const requestSessionId = request.sessionId?.trim() ?? "";
      if (
        !composer?.isConnected ||
        !composerDraft ||
        composerEditedAfterClear ||
        composer.value ||
        currentSessionId !== requestSessionId
      ) {
        return false;
      }
      composer.value = composerDraft;
      // Re-run the existing command-menu projection for a restored slash
      // command, rather than leaving its ARIA state stale.
      composer.dispatchEvent(new Event("input", { bubbles: true }));
      return true;
    };
    const failureMessage = (message: string, restoredDraft: boolean) => {
      if (!restoredDraft) return message;
      return document.documentElement.lang.toLowerCase().startsWith("en")
        ? `${message} Your draft was kept so you can review and retry.`
        : `${message} 输入草稿已保留，可检查后重试。`;
    };
    const renderSubmissionFailure = (message: string) => {
      if (
        optimisticTurn &&
        publishLiveTranscriptMessage({
          id: optimisticTurn.assistantMessageId,
          sessionId: optimisticTurn.sessionId,
          role: "assistant",
          content: message,
          createdAt: optimisticTurn.createdAt + 1,
          requestId: optimisticTurn.requestId,
          turnId: optimisticTurn.turnId,
          model: submissionPresentation.model,
          reasoning: submissionPresentation.reasoning,
          status: "error",
          prompt: request.text,
        })
      )
        return;
      activeLiveMessageId = optimisticTurn?.assistantMessageId;
      if (!activeNode)
        activeNode = createAssistantMessage(submissionPresentation, {
          liveMessageId: activeLiveMessageId,
        });
      if (activeNode) {
        activeNode.dataset.piStatus = "error";
        const text = activeNode.querySelector<HTMLElement>("[data-pi-text]");
        if (text) renderMarkdown(text, message);
      }
    };
    const attachmentSubmissionId = attachmentPayload
      ? composerAttachments?.beginSubmission()
      : undefined;
    let attachmentSubmissionSettled = false;
    const settleAttachmentSubmission = (accepted: boolean) => {
      if (attachmentSubmissionSettled) return;
      attachmentSubmissionSettled = true;
      composerAttachments?.settleSubmission(attachmentSubmissionId, accepted);
    };
    try {
      const result = await controller.submit(request);
      settleAttachmentSubmission(Boolean(result));
      if (result) composerAttachments?.clear();
      if (result && optimisticTurn) {
        if (!presentationsByRequest.has(result.requestId))
          presentationsByRequest.set(result.requestId, submissionPresentation);
        optimisticTurn.requestId = result.requestId;
        optimisticTurn.turnId = result.turnId;
        const state = controller.getState();
        if (
          state.sessionId === optimisticTurn.sessionId &&
          state.requestId === result.requestId
        ) {
          syncLiveTranscriptFromState(state);
        } else {
          publishLiveTranscriptMessage({
            id: optimisticTurn.userMessageId,
            sessionId: optimisticTurn.sessionId,
            role: "user",
            content: optimisticTurn.displayText,
            createdAt: optimisticTurn.createdAt,
            requestId: optimisticTurn.requestId,
            turnId: optimisticTurn.turnId,
          });
          publishLiveTranscriptMessage({
            id: optimisticTurn.assistantMessageId,
            sessionId: optimisticTurn.sessionId,
            role: "assistant",
            content: result.assistantText ?? "",
            createdAt: optimisticTurn.createdAt + 1,
            requestId: optimisticTurn.requestId,
            turnId: optimisticTurn.turnId,
            model: submissionPresentation.model,
            reasoning: submissionPresentation.reasoning,
            status: "running",
            prompt: request.text,
          });
        }
      }
      if (!result) {
        const state = controller.getState();
        const message = safeRuntimeMessage(
          state.error || "Pi 运行时未连接，消息尚未发送",
        );
        if (optimisticTurn && state.requestId === optimisticTurn.requestId)
          optimisticTurn.turnId = state.turnId;
        renderSubmissionFailure(message);
        toast(failureMessage(message, restoreComposerDraft()));
      }
      return result;
    } catch (error) {
      settleAttachmentSubmission(false);
      const message = safeRuntimeMessage(error);
      renderSubmissionFailure(message);
      toast(failureMessage(message, restoreComposerDraft()));
      return undefined;
    } finally {
      settleAttachmentSubmission(false);
      composer?.removeEventListener("input", noteComposerEdit);
      if (pendingPresentation === submissionPresentation)
        pendingPresentation = undefined;
      // A transport that rejects before allocating a request leaves only the
      // rendered failure row. Do not let the next programmatic run in this
      // session claim that failed optimistic turn's stable live IDs.
      if (
        optimisticTurn &&
        !optimisticTurn.requestId &&
        liveTurnsBySession.get(liveTurnKey(optimisticTurn.sessionId)) ===
          optimisticTurn
      ) {
        liveTurnsBySession.delete(liveTurnKey(optimisticTurn.sessionId));
      }
    }
  };

  const cancel = () => controller.cancel();

  const send = element<HTMLButtonElement>("btnSend");
  const onSendClick = (event: MouseEvent) => {
    if (send?.dataset.piRunning !== "true") {
      if (!providerModelIsReady()) {
        event.preventDefault();
        event.stopImmediatePropagation();
        const message = providerAvailabilityMessage();
        if (!localProviderProxyAvailable) void retryLocalProviderProxy();
        else if (modelPickerAvailability !== "loading") openProviderSettings();
        toast(message || "请先在设置中配置供应商。");
      }
      return;
    }
    event.preventDefault();
    event.stopImmediatePropagation();
    if (controller.getState().status === "cancelling") return;
    void cancel().catch((error) => toast(safeRuntimeMessage(error)));
  };
  send?.addEventListener("click", onSendClick);

  return {
    submit,
    cancel,
    answerPermission,
    answerPlanConfirmation: (decision: PiPlanConfirmationDecision) =>
      controller.answerPlanConfirmation(decision),
    issuePlanContinuationApproval: (planId: string) =>
      controller.issuePlanContinuationApproval(planId),
    invalidatePlanConfirmation: (planId: string, requestId?: string) =>
      controller.invalidatePlanConfirmation(planId, requestId),
    getState: () => controller.getState(),
    subscribe: (listener: (state: PiRuntimeState) => void) =>
      controller.subscribe(listener),
    subscribeSessionState: (listener: PiSessionRunStateListener) =>
      controller.subscribeSessionState(listener),
    ready: controller.ready,
    dispose: () => {
      unsubscribe();
      send?.removeEventListener("click", onSendClick);
      window.removeEventListener(
        "novavei:providers-changed",
        onProvidersChanged,
      );
      window.removeEventListener(
        "novavei:host-state-changed",
        onHostStateChanged,
      );
      window.removeEventListener(
        "novavei:app-health-changed",
        onAppHealthChanged,
      );
      window.removeEventListener(
        "novavei:model-options-rendered",
        onModelOptionsRendered,
      );
      window.removeEventListener("novavei:session-changed", onSessionChanged);
      window.removeEventListener(
        "novavei:session-view-invalidated",
        onSessionViewInvalidated,
      );
      window.removeEventListener(
        "novavei:transcript-window-rendered",
        onTranscriptWindowRendered,
      );
      disposeSubagentTaskListener?.();
      disposeSessionModelPickerPersistence();
      disposeProjectPreferenceBindings();
      planConfirmationCards.dispose();
      planConfirmationContexts.clear();
      contextInspector?.dispose();
    },
  };
}
