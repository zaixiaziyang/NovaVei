import { requestAppConfirm } from "./app-dialogs";

/**
 * Native-only bindings for Skills, Memory, Cron and MCP.
 *
 * The existing HTML is the application shell.  This module only replaces the
 * service placeholders after a Tauri invoke bridge is present, so browser
 * previews retain their honest "not connected" state.
 */

type UnknownRecord = Record<string, unknown>;
type Invoke = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

type SkillSummary = {
  name: string;
  description: string;
  enabled: boolean;
  builtIn: boolean;
  rootDir: string;
  skillFile: string;
  fileCount: number;
  totalBytes: number;
};

type InvalidSkill = {
  directory: string;
  error: string;
};

type SkillsListResponse = {
  rootDir: string;
  skills: SkillSummary[];
  invalid: InvalidSkill[];
};

type SkillReadResponse = {
  skill: SkillSummary;
  content: string;
};

type SkillsCatalogItem = {
  slug: string;
  reference?: string | null;
  ownerHandle?: string | null;
  displayName: string;
  summary?: string | null;
  latestVersion?: string | null;
  downloads?: number | null;
  updatedAt?: number | null;
};

type SkillsCatalogListResponse = {
  items: SkillsCatalogItem[];
  nextCursor?: string | null;
};

type SkillsCatalogScannerSummary = {
  name: string;
  status?: string | null;
  verdict?: string | null;
  severity?: string | null;
  recommendation?: string | null;
  summary?: string | null;
};

type SkillsCatalogSecuritySummary = {
  status: string;
  hasWarnings: boolean;
  installable: boolean;
  installBlockReason?: string | null;
  scanners: SkillsCatalogScannerSummary[];
};

type SkillsCatalogDetail = {
  reference: string;
  slug: string;
  ownerHandle: string;
  ownerDisplayName?: string | null;
  displayName: string;
  summary?: string | null;
  version: string;
  fileCount: number;
  totalBytes: number;
  localSkillName: string;
  sourceUrl: string;
  security: SkillsCatalogSecuritySummary;
};

type SkillsCatalogOwnerChoice = {
  ownerHandle: string;
  slug: string;
  reference: string;
  sourceUrl: string;
};

type SkillsCatalogBlocked = {
  reference: string;
  slug: string;
  ownerHandle: string;
  ownerDisplayName?: string | null;
  displayName: string;
  summary?: string | null;
  sourceUrl: string;
  security: SkillsCatalogSecuritySummary;
};

type SkillsCatalogDetailResponse =
  | { kind: "found"; data: SkillsCatalogDetail }
  | {
      kind: "ambiguous";
      data: { slug: string; matches: SkillsCatalogOwnerChoice[] };
    }
  | { kind: "blocked"; data: SkillsCatalogBlocked };

type SkillsCatalogInstallResponse = {
  skill: SkillSummary;
  reference: string;
  version: string;
  sourceUrl: string;
  security: SkillsCatalogSecuritySummary;
};

type McpServerSummary = {
  id: string;
  label: string;
  enabled: boolean;
  transport: string;
};

type McpRuntimeStatus = {
  serverId: string;
  running: boolean;
  initialized: boolean;
  transport: string;
  lastError?: string | null;
};

type McpToolInfo = {
  serverId: string;
  serverLabel: string;
  name: string;
  description: string;
  inputSchema: unknown;
};

type McpRuntimeTestResponse = {
  serverId: string;
  ok: boolean;
  phase: string;
  transport: string;
  durationMs: number;
  running: boolean;
  initialized: boolean;
  toolsCount: number;
  tools?: McpToolInfo[];
  error?: string | null;
};

type McpRegistryInput = {
  name: string;
  description?: string | null;
  required: boolean;
  secret: boolean;
};

type McpRegistryRemote = {
  transport: string;
  url: string;
  headers: McpRegistryInput[];
  variables: McpRegistryInput[];
  importable: boolean;
  requiresConfiguration: boolean;
  queryRedacted: boolean;
  incompatibilityReason?: string | null;
};

type McpRegistryPackage = {
  registryType: string;
  identifier: string;
  version?: string | null;
  runtimeHint?: string | null;
  transport?: string | null;
  environmentVariables: McpRegistryInput[];
  packageArguments: Array<{
    argumentType?: string | null;
    name?: string | null;
    valueHint?: string | null;
    description?: string | null;
    required: boolean;
    secret: boolean;
    hasVariables: boolean;
  }>;
  runtimeArguments: Array<{
    argumentType?: string | null;
    name?: string | null;
    valueHint?: string | null;
    description?: string | null;
    required: boolean;
    secret: boolean;
    hasVariables: boolean;
  }>;
  importable: boolean;
  incompatibilityReason?: string | null;
};

type McpRegistryServer = {
  name: string;
  title?: string | null;
  description: string;
  version: string;
  websiteUrl?: string | null;
  remotes: McpRegistryRemote[];
  packages: McpRegistryPackage[];
  status?: string | null;
  isLatest?: boolean | null;
};

type McpRegistryListResponse = {
  servers: McpRegistryServer[];
  nextCursor?: string | null;
  count: number;
};

type McpRegistryRemoteDraft = {
  registryName: string;
  registryVersion: string;
  id: string;
  label: string;
  enabled: boolean;
  transport: "http" | "sse" | string;
  url: string;
  allowRemote: boolean;
  timeoutMs: number;
  headers: McpRegistryInput[];
  variables: McpRegistryInput[];
};

type MemoryScope = "global" | "project";

type MemoryFilter = {
  scope?: MemoryScope;
  workdir?: string;
};

type MemoryEntry = {
  id: string;
  scope: MemoryScope;
  workdir?: string | null;
  type: string;
  title: string;
  content: string;
  createdAt: number;
  updatedAt: number;
};

type MemoryListResponse = {
  items: MemoryEntry[];
  total: number;
  truncated: boolean;
};

type MemorySearchResponse = {
  items: MemoryEntry[];
  backend: string;
  truncated: boolean;
};

type MemoryStatBucket = {
  key: string;
  entries: number;
  bytes: number;
};

type MemoryStats = {
  totalEntries: number;
  totalBytes: number;
  byScope: MemoryStatBucket[];
  byType: MemoryStatBucket[];
  weeklySearches: number;
  weeklyWrites: number;
  weekStartedAt: number;
  trackingStartedAt: number;
  capacity: {
    usedEntries: number;
    maxEntries: number;
    remainingEntries: number;
    usedPercent: number;
  };
};

type MemoryClearResponse = {
  scope: MemoryScope;
  workdir?: string | null;
  removed: number;
  reclaimedBytes: number;
};

type MemoryOrganizeResponse = {
  dryRun: boolean;
  inspected: number;
  duplicateGroups: number;
  duplicateEntries: number;
  removed: number;
  reclaimedBytes: number;
};

type MemoryExportResponse = {
  format: string;
  entries: number;
  bytes: number;
};

type MemoryUsageExportResponse = {
  format: string;
  bytes: number;
  weekStartedAt: number;
};

type KnowledgeBaseFolder = {
  id: string;
  canonicalPath: string;
  displayName: string;
  createdAt: number;
  updatedAt: number;
  lastIndexedAt?: number | null;
  documentCount: number;
  indexedBytes: number;
};

type KnowledgeBaseConsent = {
  providerId: string;
  modelId: string;
};

type KnowledgeBaseListResponse = {
  enabled: boolean;
  consent?: KnowledgeBaseConsent | null;
  folders: KnowledgeBaseFolder[];
};

type KnowledgeBaseIndexResult = {
  folder: KnowledgeBaseFolder;
  indexedFiles: number;
  skippedFiles: number;
  indexedBytes: number;
  truncated: boolean;
};

type KnowledgeBaseSearchItem = {
  documentId: string;
  folderId: string;
  folderName: string;
  title: string;
  relativePath: string;
  snippet: string;
  score: number;
  modifiedAt?: number | null;
};

type KnowledgeBaseSearchResponse = {
  items: KnowledgeBaseSearchItem[];
  backend: string;
  truncated: boolean;
};

type CronJobSummary = {
  id: string;
  name: string;
  type: "prompt" | "shell" | "http" | string;
  schedule: string;
  enabled: boolean;
  nextRunAt?: number | null;
  lastRunAt?: number | null;
  createdAt: number;
  updatedAt: number;
  payloadRedacted: boolean;
};

type CronRunSummary = {
  id: string;
  jobId: string;
  status: string;
  scheduledFor?: number | null;
  startedAt: number;
  completedAt?: number | null;
  hasOutput: boolean;
  hasError: boolean;
};

type CronJobKind = "prompt" | "shell" | "http";

type CronPromptPayload = {
  prompt: string;
  workdir?: string;
  providerId?: string;
  model?: string;
};

type CronShellPayload = {
  command: string;
  workdir: string;
};

type CronHttpPayload = {
  url: string;
  method: string;
  headers: Record<string, string>;
  body?: string;
  timeoutMs: number;
};

type CronUpsertInput = {
  id?: string;
  name: string;
  type: CronJobKind;
  schedule: string;
  payload: CronPromptPayload | CronShellPayload | CronHttpPayload;
  enabled: boolean;
};

type CronRunNowSummary = {
  run: CronRunSummary;
  dispatch?: {
    runId: string;
    jobId: string;
    type: string;
  } | null;
  http?: {
    status?: number | null;
    truncated: boolean;
    success: boolean;
  } | null;
};

type CronSchedulerUpdate = {
  checkedAt: number;
  status: "ok" | "error" | string;
  claimed: number;
  running: number;
  completed: number;
  failed: number;
};

const MAX_PREVIEW_CHARS = 12_000;
const MAX_RENDERED_ITEMS = 100;

function invokeApi(): Invoke | undefined {
  return window.__TAURI__?.core?.invoke as Invoke | undefined;
}

function isEnglish() {
  return document.documentElement.lang.toLowerCase().startsWith("en");
}

function text(zh: string, en: string) {
  return isEnglish() ? en : zh;
}

function element<T extends keyof HTMLElementTagNameMap>(
  tag: T,
  options: {
    className?: string;
    text?: string;
    id?: string;
    attrs?: Record<string, string>;
  } = {},
) {
  const node = document.createElement(tag);
  if (options.className) node.className = options.className;
  if (options.text !== undefined) node.textContent = options.text;
  if (options.id) node.id = options.id;
  for (const [name, value] of Object.entries(options.attrs ?? {}))
    node.setAttribute(name, value);
  return node;
}

function node<T extends HTMLElement>(id: string) {
  return document.getElementById(id) as T | null;
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

function readBoolean(value: unknown, fallback = false) {
  return typeof value === "boolean" ? value : fallback;
}

function isConcreteHttpsRegistryRemote(value: string) {
  const raw = value.trim();
  if (!raw || /[{}]|%7b|%7d/i.test(raw)) return false;
  try {
    const parsed = new URL(raw);
    return (
      parsed.protocol === "https:" &&
      Boolean(parsed.hostname) &&
      !parsed.username &&
      !parsed.password &&
      !parsed.hash &&
      !parsed.search
    );
  } catch {
    return false;
  }
}

/**
 * Registry metadata crosses an IPC boundary before it becomes a clickable
 * link.  Treat it as untrusted presentation data here: only an absolute HTTPS
 * destination without embedded credentials is safe to give to the WebView.
 */
function safeExternalHttpsUrl(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  try {
    const url = new URL(value.trim());
    if (
      url.protocol !== "https:" ||
      !url.hostname ||
      url.username ||
      url.password
    ) {
      return undefined;
    }
    return url.href;
  } catch {
    return undefined;
  }
}

function isPiSafeRegistryId(value: string) {
  return /^[a-z0-9][a-z0-9-]{0,127}$/.test(value);
}

function errorText(error: unknown) {
  const raw =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : text("本机服务请求失败", "Native service request failed");
  const normalized = raw.replace(/\s+/g, " ").trim();
  return (
    normalized || text("本机服务请求失败", "Native service request failed")
  ).slice(0, 360);
}

function toast(message: string) {
  const target = node<HTMLElement>("toast");
  if (!target) {
    console.warn("[NovaVei services]", message);
    return;
  }
  target.setAttribute("role", "status");
  target.setAttribute("aria-live", "polite");
  target.textContent = message;
  target.classList.add("show");
  window.setTimeout(() => target.classList.remove("show"), 2600);
}

function status(
  target: HTMLElement | null,
  message: string,
  kind: "info" | "error" | "success" = "info",
) {
  if (!target) return;
  // Runtime owns this node. Drop static data-i18n so applyI18n cannot restore
  // browser-preview "not connected" copy over live desktop status text.
  target.removeAttribute("data-i18n");
  target.textContent = message;
  target.dataset.serviceStatus = kind;
  target.setAttribute("role", kind === "error" ? "alert" : "status");
  target.setAttribute("aria-live", "polite");
}

function button(label: string, className = "btn", ariaLabel = label) {
  const control = element("button", {
    className,
    text: label,
    attrs: { type: "button", "aria-label": ariaLabel },
  });
  return control;
}

function setBusy(
  control: HTMLButtonElement,
  busy: boolean,
  busyLabel?: string,
) {
  if (busy) {
    control.dataset.serviceLabel ??= control.textContent ?? "";
    control.disabled = true;
    control.setAttribute("aria-busy", "true");
    if (busyLabel) control.textContent = busyLabel;
    return;
  }
  control.disabled = false;
  control.removeAttribute("aria-busy");
  if (control.dataset.serviceLabel)
    control.textContent = control.dataset.serviceLabel;
}

function labelControl(
  control: HTMLButtonElement,
  label: string,
  ariaLabel = label,
) {
  control.dataset.serviceLabel = label;
  if (control.getAttribute("aria-busy") !== "true") {
    control.textContent = label;
  }
  control.setAttribute("aria-label", ariaLabel);
}

/** Coalesce shell + MutationObserver language events into one refresh. */
function onServiceLanguageChange(handler: () => void) {
  let timer = 0;
  const run = () => {
    window.clearTimeout(timer);
    timer = window.setTimeout(handler, 0);
  };
  window.addEventListener("novavei:service-language-changed", run);
  window.addEventListener("novavei:language-changed", run);
}

function truncate(value: string, max = MAX_PREVIEW_CHARS) {
  return value.length > max ? `${value.slice(0, max)}\n\n…` : value;
}

function bytes(value: number) {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const index = Math.min(
    units.length - 1,
    Math.floor(Math.log(value) / Math.log(1024)),
  );
  const amount = value / 1024 ** index;
  return `${amount >= 10 || index === 0 ? amount.toFixed(0) : amount.toFixed(1)} ${units[index]}`;
}

function emptyCard(title: string, detail: string) {
  const card = element("article", {
    className: "hub-card novavei-service-empty",
  });
  card.append(
    element("strong", { text: title }),
    element("small", { text: detail }),
  );
  return card;
}

function installServiceStyles() {
  if (document.getElementById("novaveiLocalServicesStyles")) return;
  const style = element("style", { id: "novaveiLocalServicesStyles" });
  style.textContent = `
    .novavei-service-status { margin: 0; color: var(--subtle); font-size: 12px; line-height: 1.45; }
    .novavei-service-status[data-service-status="error"] { color: var(--danger); }
    .novavei-service-status[data-service-status="success"] { color: var(--success); }
    #overlaySkills .overlay-body, #overlayMcp .overlay-body { display: flex; overflow: hidden; }
    .novavei-service-hub { display: flex; flex: 1 1 auto; flex-direction: column; gap: 12px; min-width: 0; min-height: 0; overflow: auto; overscroll-behavior: contain; }
    .novavei-service-panel { min-width: 0; min-height: 0; }
    .novavei-service-panel-heading { margin: 0; color: var(--text); font-size: 13px; font-weight: 650; letter-spacing: -.01em; }
    .novavei-skills-workbench { display: grid; flex: 1 1 440px; grid-template-columns: minmax(236px, .78fr) minmax(0, 1.42fr); min-height: 360px; overflow: hidden; border: 1px solid var(--line); border-radius: var(--r-md); background: var(--glass); box-shadow: var(--shadow-sm); }
    .novavei-skills-local-panel, .novavei-skills-store-panel { display: flex; flex-direction: column; gap: 12px; min-width: 0; min-height: 0; padding: 14px; }
    .novavei-skills-local-panel { border-right: 1px solid var(--line); background: color-mix(in srgb, var(--panel-deep) 72%, transparent); }
    .novavei-skills-list, .novavei-catalog-results { display: grid; align-content: start; gap: 10px; min-width: 0; }
    .novavei-skills-list { flex: 1 1 auto; overflow: auto; padding-right: 2px; }
    .novavei-catalog-results { grid-template-columns: repeat(auto-fill, minmax(228px, 1fr)); overflow: visible; }
    .novavei-catalog-results > .novavei-service-empty, .novavei-catalog-results > .novavei-catalog-detail { grid-column: 1 / -1; }
    .novavei-catalog-results .hub-card { height: 100%; }
    .novavei-mcp-content { display: flex; flex: 1 0 auto; flex-direction: column; gap: 12px; min-width: 0; }
    .novavei-mcp-workbench { display: grid; grid-template-columns: minmax(220px, 300px) minmax(0, 1fr); min-height: 340px; overflow: hidden; border: 1px solid var(--line); border-radius: var(--r-md); background: var(--glass); box-shadow: var(--shadow-sm); }
    .novavei-mcp-server-panel { display: flex; flex-direction: column; min-width: 0; min-height: 0; border-right: 1px solid var(--line); background: color-mix(in srgb, var(--panel-deep) 72%, transparent); }
    .novavei-mcp-server-panel > header { display: grid; gap: 3px; padding: 13px 14px 10px; border-bottom: 1px solid var(--line); }
    .novavei-mcp-server-panel > header small { color: var(--subtle); font-size: 11px; line-height: 1.4; }
    .novavei-mcp-server-list { display: grid; align-content: start; gap: 7px; min-height: 0; overflow: auto; padding: 9px; }
    .novavei-mcp-list-item { display: grid; grid-template-columns: 8px minmax(0, 1fr); align-items: start; gap: 9px; width: 100%; min-height: 64px; padding: 10px; border: 1px solid var(--line); border-radius: 10px; background: var(--control); color: var(--muted); text-align: left; transition: border-color 160ms var(--ease-out), background-color 160ms var(--ease-out), color 160ms var(--ease-out); }
    .novavei-mcp-list-item:hover:not(:disabled) { border-color: var(--line-strong); background: var(--hover); color: var(--text); }
    .novavei-mcp-list-item.is-selected { border-color: var(--blue-line); background: var(--blue-soft); color: var(--text); box-shadow: inset 3px 0 0 var(--blue); }
    .novavei-mcp-list-item:focus-visible { outline: 3px solid var(--blue-strong); outline-offset: -1px; }
    .novavei-mcp-list-status { width: 8px; height: 8px; margin-top: 4px; border-radius: 999px; background: var(--subtle); }
    .novavei-mcp-list-status.is-ready { background: var(--success); box-shadow: 0 0 8px color-mix(in srgb, var(--success) 55%, transparent); }
    .novavei-mcp-list-status.is-starting { background: var(--warn); box-shadow: 0 0 8px color-mix(in srgb, var(--warn) 48%, transparent); }
    .novavei-mcp-list-status.is-error { background: var(--danger); }
    .novavei-mcp-list-copy { display: grid; gap: 3px; min-width: 0; }
    .novavei-mcp-list-copy strong, .novavei-mcp-list-copy small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .novavei-mcp-list-copy strong { color: var(--text); font-size: 12px; }
    .novavei-mcp-list-copy small { color: var(--subtle); font-size: 11px; }
    .novavei-mcp-detail-panel { min-width: 0; min-height: 0; overflow: auto; padding: 14px; }
    .novavei-mcp-detail-panel > .hub-card { min-height: 100%; }
    .novavei-mcp-registry { margin: 0; }
    .pill.danger { background: color-mix(in srgb, var(--danger) 18%, var(--glass)); color: var(--danger); }
    .novavei-service-actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 4px; }
    .novavei-service-preview { max-height: 300px; overflow: auto; margin: 8px 0 0; padding: 10px; border: 1px solid var(--line); border-radius: 10px; background: var(--code-bg); color: var(--muted); white-space: pre-wrap; overflow-wrap: anywhere; font: 11.5px/1.5 var(--mono); }
    .novavei-service-details { margin-top: 4px; color: var(--muted); font-size: 12px; }
    .novavei-service-details > summary { cursor: pointer; color: var(--blue-strong); }
    .novavei-service-list { display: grid; gap: 10px; }
    .novavei-service-row { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
    .novavei-service-row > div { min-width: 0; display: grid; gap: 3px; }
    .novavei-service-row small { overflow-wrap: anywhere; }
    .novavei-service-note { margin: 0; color: var(--subtle); font-size: 12px; line-height: 1.45; }
    .novavei-catalog-heading { grid-column: 1 / -1; margin: 8px 0 -2px; color: var(--text); font-size: 13px; font-weight: 650; letter-spacing: -.01em; }
    .novavei-catalog-toolbar, .novavei-catalog-detail { grid-column: 1 / -1; display: grid; gap: 10px; }
    .novavei-catalog-toolbar { padding: 12px; border: 1px solid var(--line); border-radius: var(--r-sm); background: color-mix(in srgb, var(--glass) 82%, transparent); }
    .novavei-catalog-toolbar > p { margin: 0; color: var(--subtle); font-size: 12px; line-height: 1.45; }
    .novavei-catalog-form { display: flex; align-items: end; gap: 8px; flex-wrap: wrap; }
    .novavei-catalog-field { min-width: min(100%, 260px); flex: 1 1 260px; display: grid; gap: 6px; color: var(--muted); font-size: 12px; }
    .novavei-catalog-field input { min-width: 0; width: 100%; box-sizing: border-box; border: 1px solid var(--line); border-radius: 10px; background: var(--input-deep); color: var(--text); padding: 8px 10px; }
    .novavei-catalog-meta { display: flex; flex-wrap: wrap; gap: 6px; margin: 2px 0 0; }
    .novavei-catalog-risk { display: grid; gap: 6px; margin: 0; padding: 10px; border: 1px solid var(--line); border-radius: 10px; background: var(--code-bg); color: var(--muted); font-size: 12px; line-height: 1.45; }
    .novavei-catalog-risk[data-risk="blocked"] { border-color: color-mix(in srgb, var(--danger) 62%, var(--line)); color: var(--danger); }
    .novavei-catalog-risk[data-risk="warning"] { border-color: color-mix(in srgb, var(--warning, #F0B429) 55%, var(--line)); }
    .novavei-catalog-source { color: var(--blue-strong); font-size: 12px; overflow-wrap: anywhere; }
    .novavei-catalog-section-status { grid-column: 1 / -1; }
    .novavei-service-dialog { width: min(640px, calc(100vw - 32px)); max-height: min(760px, calc(100dvh - 32px)); overflow: auto; padding: 0; color: var(--text); border: 1px solid var(--line-strong); border-radius: var(--r-md); background: var(--glass-strong); box-shadow: var(--shadow-md); }
    .novavei-service-dialog::backdrop { background: rgb(7 11 20 / 62%); }
    .novavei-service-dialog-body { display: grid; gap: 12px; padding: 18px; }
    .novavei-service-dialog h2 { margin: 0; font-size: 16px; }
    .novavei-service-dialog textarea { width: 100%; min-height: 170px; resize: vertical; box-sizing: border-box; border: 1px solid var(--line); border-radius: 10px; background: var(--input-deep); padding: 10px 12px; color: var(--text); font: 12px/1.5 var(--mono); }
    .novavei-service-dialog .field { padding: 12px; }
    .novavei-service-dialog [role="alert"] { min-height: 1.35em; margin: 0; color: var(--danger); font-size: 12px; }
    .novavei-mcp-editor { width: min(720px, calc(100vw - 32px)); }
    .novavei-mcp-editor .field { display: grid; gap: 7px; }
    .novavei-mcp-editor input:not([type="checkbox"]), .novavei-mcp-editor select { width: 100%; box-sizing: border-box; }
    .novavei-mcp-editor textarea { min-height: 112px; }
    .novavei-mcp-editor [hidden] { display: none !important; }
    .novavei-mcp-required { color: var(--danger); }
    .novavei-mcp-toggle { display: flex; align-items: flex-start; gap: 9px; color: var(--text); font-size: 13px; line-height: 1.45; }
    .novavei-mcp-toggle input { margin-top: 2px; accent-color: var(--blue-strong); }
    .novavei-mcp-warning { margin: 0; padding: 10px 12px; border: 1px solid var(--warning, var(--line-strong)); border-radius: 10px; background: var(--code-bg); color: var(--muted); font-size: 12px; line-height: 1.5; }
    .novavei-mcp-registry { grid-column: 1 / -1; display: grid; gap: 10px; margin-top: 6px; padding: 14px; border: 1px solid var(--line); border-radius: var(--r-sm); background: color-mix(in srgb, var(--glass) 84%, transparent); }
    .novavei-mcp-registry > h3, .novavei-mcp-registry > p { margin: 0; }
    .novavei-mcp-registry > h3 { color: var(--text); font-size: 14px; }
    .novavei-mcp-registry-form { display: flex; align-items: end; flex-wrap: wrap; gap: 8px; }
    .novavei-mcp-registry-form label { min-width: min(100%, 260px); flex: 1 1 260px; display: grid; gap: 6px; color: var(--muted); font-size: 12px; }
    .novavei-mcp-registry-form input { min-width: 0; width: 100%; box-sizing: border-box; border: 1px solid var(--line); border-radius: 10px; background: var(--input-deep); color: var(--text); padding: 8px 10px; }
    .novavei-mcp-registry-results, .novavei-mcp-registry-detail, .novavei-mcp-registry-inputs { display: grid; gap: 8px; }
    .novavei-mcp-registry-item { display: grid; gap: 8px; padding: 10px; border: 1px solid var(--line); border-radius: 10px; background: var(--code-bg); }
    .novavei-mcp-registry-item small, .novavei-mcp-registry-detail small { overflow-wrap: anywhere; }
    .novavei-mcp-registry-inputs { margin: 0; padding-left: 18px; color: var(--subtle); font-size: 12px; }
    .novavei-mcp-registry-package { display: grid; gap: 4px; padding: 8px 0; border-top: 1px solid var(--line); }
    .novavei-mcp-registry-package:first-of-type { border-top: 0; }
    .novavei-cron-editor { width: min(720px, calc(100vw - 32px)); }
    .novavei-cron-editor .field { display: grid; gap: 7px; }
    .novavei-cron-editor input:not([type="checkbox"]), .novavei-cron-editor select { width: 100%; box-sizing: border-box; }
    .novavei-cron-editor textarea { min-height: 108px; }
    .novavei-cron-editor [hidden] { display: none !important; }
    .novavei-cron-required { color: var(--danger); }
    .novavei-cron-toggle { display: flex; align-items: flex-start; gap: 9px; color: var(--text); font-size: 13px; line-height: 1.45; }
    .novavei-cron-toggle input { margin-top: 2px; accent-color: var(--blue-strong); }
    .novavei-cron-warning { margin: 0; padding: 10px 12px; border: 1px solid var(--warning, var(--line-strong)); border-radius: 10px; background: var(--code-bg); color: var(--muted); font-size: 12px; line-height: 1.5; }
    @media (max-width: 760px) {
      #overlaySkills .overlay-body, #overlayMcp .overlay-body { overflow: auto; }
      .novavei-service-hub { flex: 0 0 auto; overflow: visible; }
      .novavei-skills-workbench, .novavei-mcp-workbench { grid-template-columns: minmax(0, 1fr); overflow: visible; }
      .novavei-skills-local-panel, .novavei-mcp-server-panel { border-right: 0; border-bottom: 1px solid var(--line); }
      .novavei-skills-list, .novavei-mcp-server-list, .novavei-mcp-detail-panel { max-height: none; overflow: visible; }
    }
    @media (max-width: 560px) {
      .novavei-service-dialog { width: calc(100vw - 20px); }
      .novavei-catalog-results { grid-template-columns: minmax(0, 1fr); }
    }
  `;
  document.head.append(style);
}

function installSkills() {
  const invoke = invokeApi();
  const overlay = node<HTMLElement>("overlaySkills");
  if (!invoke || !overlay) return;

  const grid = overlay.querySelector<HTMLElement>(".hub-grid");
  const actions = overlay.querySelector<HTMLElement>(
    ".overlay-head .row-actions",
  );
  const settingsPanel = document.querySelector<HTMLElement>(
    '[data-tools-panel="skills"]',
  );
  const settingsRoot =
    settingsPanel?.querySelector<HTMLInputElement>("input[readonly]") ?? null;
  const settingsHint = settingsPanel?.querySelector<HTMLElement>("p") ?? null;
  if (!grid || !actions) return;

  let latest: SkillsListResponse | undefined;
  let catalog: SkillsCatalogListResponse | undefined;
  let catalogQuery = "";
  let catalogState: "idle" | "loading" | "loaded" | "error" = "idle";
  let catalogError = "";
  let selectedCatalogDetail: SkillsCatalogDetailResponse | undefined;
  const overlayStatus = element("p", {
    className: "novavei-service-status",
    attrs: { "aria-live": "polite" },
  });
  const catalogStatus = element("p", {
    className: "novavei-service-status novavei-catalog-section-status",
    attrs: { "aria-live": "polite" },
  });
  const refresh = button(
    text("刷新本机", "Refresh local"),
    "btn",
    text("刷新本机 Skills", "Refresh local Skills"),
  );
  const install = button(
    text("从文件夹安装", "Install folder"),
    "btn primary",
    text("从文件夹安装 Skill", "Install Skill from folder"),
  );
  const close = actions.querySelector<HTMLElement>("[data-close-overlay]");
  actions.replaceChildren(
    refresh,
    install,
    close ?? button(text("关闭", "Close"), "btn ghost"),
  );

  const syncSettings = (response: SkillsListResponse | undefined) => {
    if (!response) return;
    if (settingsRoot) settingsRoot.value = response.rootDir;
    const enabled = response.skills.filter((skill) => skill.enabled).length;
    status(
      settingsHint,
      text(
        `已读取 ${response.skills.length} 个 Skill，其中 ${enabled} 个已启用。`,
        `${response.skills.length} Skills loaded; ${enabled} enabled.`,
      ),
      "success",
    );
  };

  const catalogPill = (security: SkillsCatalogSecuritySummary) => {
    const isBlocked = !security.installable;
    const hasWarnings = security.hasWarnings;
    return element("span", {
      className: isBlocked ? "pill danger" : hasWarnings ? "pill warn" : "pill",
      text: isBlocked
        ? text(
            `不可安装 · ${security.status}`,
            `Not installable · ${security.status}`,
          )
        : hasWarnings
          ? text("已检查 · 有提示", "Checked · warnings")
          : text("已检查 · Clean", "Checked · Clean"),
    });
  };

  const appendCatalogRisk = (
    card: HTMLElement,
    security: SkillsCatalogSecuritySummary,
  ) => {
    const riskKind = !security.installable
      ? "blocked"
      : security.hasWarnings
        ? "warning"
        : "clean";
    const risk = element("div", {
      className: "novavei-catalog-risk",
      attrs: { "data-risk": riskKind },
    });
    const lead = !security.installable
      ? security.installBlockReason ||
        text(
          "原生安全策略已阻止安装。",
          "The native security policy blocked installation.",
        )
      : security.hasWarnings
        ? text(
            "ClawHub 标记了非阻断提示；安装前请阅读下列摘要。",
            "ClawHub reported non-blocking warnings; review the summaries before installing.",
          )
        : text(
            "ClawHub 为此精确版本返回了 clean 状态。",
            "ClawHub returned a clean status for this exact version.",
          );
    risk.append(
      element("strong", { text: text("安全摘要", "Security summary") }),
      element("span", { text: lead }),
    );
    for (const scanner of security.scanners) {
      const parts = [
        scanner.name,
        scanner.status,
        scanner.verdict,
        scanner.severity,
        scanner.recommendation,
      ].filter((value): value is string => Boolean(value));
      const line = element("span", { text: parts.join(" · ") });
      if (scanner.summary) line.title = scanner.summary;
      risk.append(line);
      if (scanner.summary)
        risk.append(element("span", { text: scanner.summary }));
    }
    card.append(risk);
  };

  const appendCatalogSource = (card: HTMLElement, sourceUrl: string) => {
    const href = safeExternalHttpsUrl(sourceUrl);
    if (!href) return;
    card.append(
      element("a", {
        className: "novavei-catalog-source",
        text: text("在 ClawHub 查看源记录", "View source record on ClawHub"),
        attrs: {
          href,
          target: "_blank",
          rel: "noopener noreferrer",
        },
      }),
    );
  };

  const renderCatalogDetail = () => {
    if (!selectedCatalogDetail) return [] as HTMLElement[];
    if (selectedCatalogDetail.kind === "ambiguous") {
      const card = element("article", {
        className: "hub-card novavei-catalog-detail",
      });
      card.append(
        element("strong", { text: text("请选择发布者", "Choose a publisher") }),
        element("small", {
          text: text(
            `“${selectedCatalogDetail.data.slug}”有多个发布者。选择精确来源后才会读取详情。`,
            `“${selectedCatalogDetail.data.slug}” has multiple publishers. Choose an exact source before loading details.`,
          ),
        }),
      );
      const choices = element("div", { className: "novavei-service-actions" });
      for (const choice of selectedCatalogDetail.data.matches) {
        const control = button(
          `@${choice.ownerHandle}/${choice.slug}`,
          "btn",
          text(`查看 ${choice.reference}`, `View ${choice.reference}`),
        );
        control.addEventListener(
          "click",
          () => void loadCatalogDetail(choice.reference),
        );
        choices.append(control);
      }
      card.append(choices);
      return [card];
    }

    const detail = selectedCatalogDetail.data;
    const card = element("article", {
      className: "hub-card novavei-catalog-detail",
    });
    const row = element("div", { className: "novavei-service-row" });
    const copy = element("div");
    copy.append(
      element("strong", { text: detail.displayName }),
      element("small", {
        text: `${detail.reference}${detail.ownerDisplayName ? ` · ${detail.ownerDisplayName}` : ""}`,
      }),
    );
    if (detail.summary) copy.append(element("small", { text: detail.summary }));
    row.append(copy, catalogPill(detail.security));
    const meta = element("div", { className: "novavei-catalog-meta" });
    if ("version" in detail) {
      meta.append(
        element("span", { className: "tag", text: `v${detail.version}` }),
        element("span", {
          className: "tag",
          text: `${detail.fileCount} ${text("个文件", "files")}`,
        }),
        element("span", { className: "tag", text: bytes(detail.totalBytes) }),
        element("span", {
          className: "tag",
          text: text(
            `本机：${detail.localSkillName}`,
            `Local: ${detail.localSkillName}`,
          ),
        }),
      );
    }
    card.append(row, meta);
    appendCatalogRisk(card, detail.security);
    appendCatalogSource(card, detail.sourceUrl);
    if ("version" in detail && detail.security.installable) {
      const actions = element("div", { className: "novavei-service-actions" });
      const installStore = button(
        text("确认并安装", "Confirm & install"),
        "btn primary",
        text(
          `确认安装 ${detail.reference} ${detail.version}`,
          `Confirm install ${detail.reference} ${detail.version}`,
        ),
      );
      installStore.addEventListener("click", () => {
        void (async () => {
          const destination = latest
            ? `${latest.rootDir}\\${detail.localSkillName}`
            : text(
                `本机 Skills 目录中的 ${detail.localSkillName}`,
                `${detail.localSkillName} in the local Skills directory`,
              );
          const confirmation = text(
            `安装 ${detail.reference} 的精确版本 ${detail.version} 到“${destination}”吗？将下载并逐文件校验哈希，然后作为本机 Skill 启用。安全摘要已显示在此处；请仅在你接受这些提示时继续。`,
            `Install exact version ${detail.version} of ${detail.reference} into “${destination}”? NovaVei will download it, verify every file hash, then enable it as a local Skill. The security summary is shown here; continue only if you accept it.`,
          );
          if (
            !(await requestAppConfirm({
              title: text("确认安装 Skill", "Confirm skill install"),
              message: confirmation,
              confirmLabel: text("安装", "Install"),
              cancelLabel: text("取消", "Cancel"),
              danger: false,
            }))
          ) {
            status(
              catalogStatus,
              text("已取消 Store 安装。", "Store installation cancelled."),
            );
            return;
          }
          setBusy(
            installStore,
            true,
            text("校验并安装中…", "Verifying & installing…"),
          );
          status(
            catalogStatus,
            text(
              "正在下载精确版本并逐文件校验…",
              "Downloading the exact version and verifying each file…",
            ),
          );
          try {
            const result = await invoke<SkillsCatalogInstallResponse>(
              "skills_catalog_install",
              {
                input: {
                  reference: detail.reference,
                  version: detail.version,
                  confirmation: `INSTALL_CLAWHUB_SKILL:${detail.reference}@${detail.version}`,
                },
              },
            );
            toast(
              text(
                `已安装 ${result.skill.name}`,
                `Installed ${result.skill.name}`,
              ),
            );
            status(
              catalogStatus,
              text(
                `已安装 ${result.reference} v${result.version}；本机清单已刷新。`,
                `Installed ${result.reference} v${result.version}; local list refreshed.`,
              ),
              "success",
            );
            await loadLocal();
          } catch (error) {
            status(
              catalogStatus,
              `${text("Store 安装失败：", "Store installation failed: ")}${errorText(error)}`,
              "error",
            );
          } finally {
            setBusy(installStore, false);
          }
        })();
      });
      actions.append(installStore);
      card.append(actions);
    }
    return [card];
  };

  const render = () => {
    grid.replaceChildren();
    const hub = element("section", {
      className: "novavei-service-hub novavei-skills-hub",
      attrs: { "aria-label": text("Skills 管理", "Skills management") },
    });
    const workbench = element("div", {
      className: "novavei-skills-workbench",
    });
    const localPanel = element("section", {
      className: "novavei-service-panel novavei-skills-local-panel",
      attrs: {
        "aria-label": text("本机已安装 Skills", "Installed local Skills"),
      },
    });
    const localList = element("div", { className: "novavei-skills-list" });
    const storePanel = element("section", {
      className: "novavei-service-panel novavei-skills-store-panel",
      attrs: { "aria-label": text("ClawHub Store", "ClawHub Store") },
    });
    const catalogResults = element("div", {
      className: "novavei-catalog-results",
      attrs: { "aria-label": text("Store Skills", "Store Skills") },
    });
    localPanel.append(localList);
    workbench.append(localPanel, storePanel);
    hub.append(overlayStatus, workbench);
    grid.append(hub);

    localList.append(
      element("h3", {
        className: "novavei-service-panel-heading",
        text: text("本机已安装", "Installed locally"),
      }),
    );
    if (!latest) {
      localList.append(
        emptyCard(
          text("正在读取 Skills", "Loading Skills"),
          text("正在从本机服务加载。", "Loading from the native service."),
        ),
      );
    } else {
      if (!latest.skills.length) {
        localList.append(
          emptyCard(
            text("尚未安装 Skill", "No Skills installed"),
            text(
              "可用“安装”从本机文件夹导入一个 Skill。",
              "Use Install to import a Skill from a local folder.",
            ),
          ),
        );
      }
      for (const skill of latest.skills.slice(0, MAX_RENDERED_ITEMS)) {
        const card = element("article", { className: "hub-card" });
        const title = element("strong", { text: skill.name });
        const description = element("small", {
          text:
            skill.description ||
            text("此 Skill 未提供描述。", "This Skill has no description."),
        });
        const meta = element("small", {
          text: `${skill.enabled ? text("已启用", "Enabled") : text("已停用", "Disabled")} · ${skill.fileCount} ${text("个文件", "files")} · ${bytes(skill.totalBytes)}`,
        });
        const state = element("span", {
          className: skill.enabled ? "pill" : "pill wait",
          text: skill.enabled
            ? text("已启用", "Enabled")
            : text("已停用", "Disabled"),
        });
        const row = element("div", { className: "novavei-service-row" });
        const copy = element("div");
        copy.append(title, description, meta);
        row.append(copy, state);
        const rowActions = element("div", {
          className: "novavei-service-actions",
        });
        const read = button(
          text("查看", "View"),
          "btn",
          text(`查看 ${skill.name}`, `View ${skill.name}`),
        );
        const toggle = button(
          skill.enabled ? text("停用", "Disable") : text("启用", "Enable"),
          skill.enabled ? "btn" : "btn primary",
          skill.enabled
            ? text(`停用 ${skill.name}`, `Disable ${skill.name}`)
            : text(`启用 ${skill.name}`, `Enable ${skill.name}`),
        );
        read.addEventListener("click", () => {
          void (async () => {
            const existing = card.querySelector<HTMLDetailsElement>(
              "details.novavei-service-details",
            );
            if (existing) {
              existing.open = !existing.open;
              return;
            }
            setBusy(read, true, text("读取中…", "Reading…"));
            try {
              const response = await invoke<SkillReadResponse>("skills_read", {
                name: skill.name,
              });
              const details = element("details", {
                className: "novavei-service-details",
              });
              details.open = true;
              details.append(
                element("summary", {
                  text: text("Skill 内容", "Skill content"),
                }),
              );
              details.append(
                element("pre", {
                  className: "novavei-service-preview",
                  text: truncate(response.content),
                }),
              );
              card.append(details);
            } catch (error) {
              status(
                overlayStatus,
                `${text("无法读取 Skill：", "Unable to read Skill: ")}${errorText(error)}`,
                "error",
              );
            } finally {
              setBusy(read, false);
            }
          })();
        });
        toggle.addEventListener("click", () => {
          void (async () => {
            setBusy(toggle, true, text("保存中…", "Saving…"));
            try {
              await invoke(skill.enabled ? "skills_disable" : "skills_enable", {
                name: skill.name,
              });
              await loadLocal();
            } catch (error) {
              status(
                overlayStatus,
                `${text("无法更新 Skill：", "Unable to update Skill: ")}${errorText(error)}`,
                "error",
              );
            } finally {
              setBusy(toggle, false);
            }
          })();
        });
        rowActions.append(read, toggle);
        card.append(row, rowActions);
        localList.append(card);
      }
      for (const invalid of latest.invalid) {
        const card = emptyCard(
          text("Skill 验证失败", "Skill validation failed"),
          truncate(invalid.error, 220),
        );
        const badge = element("span", {
          className: "pill warn",
          text: text("需修复", "Needs repair"),
        });
        card.append(badge);
        localList.append(card);
      }
    }

    storePanel.append(
      element("h3", {
        className: "novavei-service-panel-heading",
        text: text("ClawHub Store", "ClawHub Store"),
      }),
    );
    const catalogToolbar = element("section", {
      className: "novavei-catalog-toolbar",
      attrs: { "aria-label": text("ClawHub Store", "ClawHub Store") },
    });
    catalogToolbar.append(
      element("p", {
        text: text(
          "只读取官方 ClawHub 公开目录；搜索结果会隐藏被标记为 suspicious 的条目。详情显示精确版本的安全结果，NovaVei 不会跟随下载重定向或 GitHub 交接链接。",
          "Reads only the official public ClawHub catalog; search hides entries marked suspicious. Details show security for the exact version; NovaVei never follows download redirects or GitHub handoff links.",
        ),
      }),
    );
    const catalogForm = element("div", { className: "novavei-catalog-form" });
    const catalogField = element("label", {
      className: "novavei-catalog-field",
      text: text("搜索 ClawHub Skills", "Search ClawHub Skills"),
    });
    const catalogInput = element("input", {
      id: "novaveiSkillsCatalogSearch",
      attrs: {
        type: "search",
        maxlength: "120",
        autocomplete: "off",
        placeholder: text("例如：weather", "For example: weather"),
      },
    });
    catalogInput.value = catalogQuery;
    catalogField.append(catalogInput);
    const searchCatalog = button(
      text("搜索", "Search"),
      "btn",
      text("搜索 ClawHub Skills", "Search ClawHub Skills"),
    );
    const browseCatalog = button(
      text("推荐", "Browse"),
      "btn",
      text("浏览推荐 ClawHub Skills", "Browse recommended ClawHub Skills"),
    );
    const refreshCatalog = button(
      text("刷新 Store", "Refresh Store"),
      "btn",
      text("刷新 ClawHub Store", "Refresh ClawHub Store"),
    );
    catalogForm.append(
      catalogField,
      searchCatalog,
      browseCatalog,
      refreshCatalog,
    );
    catalogToolbar.append(catalogForm);
    storePanel.append(catalogToolbar, catalogStatus, catalogResults);
    searchCatalog.addEventListener("click", () => {
      catalogQuery = catalogInput.value;
      void loadCatalogSearch();
    });
    browseCatalog.addEventListener("click", () => {
      catalogQuery = "";
      void loadCatalogBrowse();
    });
    refreshCatalog.addEventListener("click", () => {
      if (catalogQuery.trim()) void loadCatalogSearch();
      else void loadCatalogBrowse();
    });
    catalogInput.addEventListener("keydown", (event) => {
      if (event.key !== "Enter") return;
      event.preventDefault();
      catalogQuery = catalogInput.value;
      void loadCatalogSearch();
    });

    for (const detail of renderCatalogDetail()) catalogResults.append(detail);
    if (catalogState === "loading" || catalogState === "idle") {
      catalogResults.append(
        emptyCard(
          text("正在读取 Store", "Loading Store"),
          text(
            "正在通过本机服务读取官方 ClawHub 目录。",
            "Loading the official ClawHub catalog through the native service.",
          ),
        ),
      );
    } else if (catalogState === "error") {
      catalogResults.append(
        emptyCard(
          text("Store 暂不可用", "Store unavailable"),
          catalogError || text("请稍后刷新。", "Refresh and try again."),
        ),
      );
    } else if (!catalog?.items.length) {
      catalogResults.append(
        emptyCard(
          text("没有匹配的 Store Skill", "No matching Store Skills"),
          text(
            "尝试不同的搜索词，或返回推荐目录。",
            "Try a different search term or return to the recommended catalog.",
          ),
        ),
      );
    } else {
      for (const item of catalog.items.slice(0, MAX_RENDERED_ITEMS)) {
        const card = element("article", { className: "hub-card" });
        const row = element("div", { className: "novavei-service-row" });
        const copy = element("div");
        copy.append(
          element("strong", { text: item.displayName }),
          element("small", {
            text: item.ownerHandle
              ? `@${item.ownerHandle}/${item.slug}`
              : item.slug,
          }),
        );
        if (item.summary) copy.append(element("small", { text: item.summary }));
        const inspected = element("span", {
          className: "pill wait",
          text: text("待检查", "Not inspected"),
        });
        row.append(copy, inspected);
        const meta = element("div", { className: "novavei-catalog-meta" });
        if (item.latestVersion)
          meta.append(
            element("span", {
              className: "tag",
              text: `v${item.latestVersion}`,
            }),
          );
        if (typeof item.downloads === "number")
          meta.append(
            element("span", {
              className: "tag",
              text: `${item.downloads} ${text("下载", "downloads")}`,
            }),
          );
        const actions = element("div", {
          className: "novavei-service-actions",
        });
        const inspect = button(
          text("查看详情", "Inspect"),
          "btn",
          text(
            `查看 ${item.displayName} 的详情与安全状态`,
            `Inspect ${item.displayName} details and security state`,
          ),
        );
        inspect.addEventListener(
          "click",
          () =>
            void loadCatalogDetail(
              item.reference || item.slug,
              item.latestVersion || undefined,
              inspect,
            ),
        );
        actions.append(inspect);
        card.append(row, meta, actions);
        catalogResults.append(card);
      }
    }
  };

  const loadLocal = async () => {
    setBusy(refresh, true, text("刷新中…", "Refreshing…"));
    status(
      overlayStatus,
      text("正在读取本机 Skills…", "Loading local Skills…"),
    );
    render();
    try {
      latest = await invoke<SkillsListResponse>("skills_list");
      syncSettings(latest);
      status(
        overlayStatus,
        text(
          `已读取 ${latest.skills.length} 个 Skill。`,
          `${latest.skills.length} Skills loaded.`,
        ),
        "success",
      );
    } catch (error) {
      latest = undefined;
      status(
        overlayStatus,
        `${text("Skills 服务不可用：", "Skills service unavailable: ")}${errorText(error)}`,
        "error",
      );
    } finally {
      setBusy(refresh, false);
      render();
    }
  };

  const loadCatalogBrowse = async () => {
    catalogState = "loading";
    catalogError = "";
    selectedCatalogDetail = undefined;
    status(
      catalogStatus,
      text("正在读取推荐 Store Skills…", "Loading recommended Store Skills…"),
    );
    render();
    try {
      catalog = await invoke<SkillsCatalogListResponse>("skills_catalog_list", {
        limit: 12,
        sort: "recommended",
      });
      catalogState = "loaded";
      status(
        catalogStatus,
        text(
          `已读取 ${catalog.items.length} 个 Store Skill；安全状态需在详情中检查。`,
          `${catalog.items.length} Store Skills loaded; inspect details for security status.`,
        ),
        "success",
      );
    } catch (error) {
      catalog = undefined;
      catalogState = "error";
      catalogError = errorText(error);
      status(
        catalogStatus,
        `${text("Store 无法读取：", "Unable to load Store: ")}${catalogError}`,
        "error",
      );
    } finally {
      render();
    }
  };

  const loadCatalogSearch = async () => {
    const query = catalogQuery.trim();
    if (!query) {
      await loadCatalogBrowse();
      return;
    }
    catalogState = "loading";
    catalogError = "";
    selectedCatalogDetail = undefined;
    status(catalogStatus, text("正在搜索 Store…", "Searching Store…"));
    render();
    try {
      catalog = await invoke<SkillsCatalogListResponse>(
        "skills_catalog_search",
        { query, limit: 12 },
      );
      catalogState = "loaded";
      status(
        catalogStatus,
        text(
          `搜索到 ${catalog.items.length} 个 Store Skill；安全状态需在详情中检查。`,
          `${catalog.items.length} Store Skills found; inspect details for security status.`,
        ),
        "success",
      );
    } catch (error) {
      catalog = undefined;
      catalogState = "error";
      catalogError = errorText(error);
      status(
        catalogStatus,
        `${text("Store 搜索失败：", "Store search failed: ")}${catalogError}`,
        "error",
      );
    } finally {
      render();
    }
  };

  const loadCatalogDetail = async (
    reference: string,
    version?: string,
    control?: HTMLButtonElement,
  ) => {
    if (control) setBusy(control, true, text("检查中…", "Inspecting…"));
    status(
      catalogStatus,
      text(
        "正在读取精确版本、安全摘要与本机目标名称…",
        "Loading the exact version, security summary, and local target name…",
      ),
    );
    try {
      selectedCatalogDetail = await invoke<SkillsCatalogDetailResponse>(
        "skills_catalog_detail",
        { reference, version },
      );
      const message =
        selectedCatalogDetail.kind === "found"
          ? text(
              `已检查 ${selectedCatalogDetail.data.reference} v${selectedCatalogDetail.data.version}。`,
              `Inspected ${selectedCatalogDetail.data.reference} v${selectedCatalogDetail.data.version}.`,
            )
          : selectedCatalogDetail.kind === "ambiguous"
            ? text(
                "该 slug 有多个发布者；请选择精确来源。",
                "This slug has multiple publishers; choose an exact source.",
              )
            : text(
                "该精确 Skill 或版本不可安装；请阅读安全摘要。",
                "This exact Skill or version is not installable; read the security summary.",
              );
      status(
        catalogStatus,
        message,
        selectedCatalogDetail.kind === "blocked" ? "error" : "success",
      );
    } catch (error) {
      selectedCatalogDetail = undefined;
      status(
        catalogStatus,
        `${text("无法读取 Store 详情：", "Unable to load Store details: ")}${errorText(error)}`,
        "error",
      );
    } finally {
      if (control) setBusy(control, false);
      render();
    }
  };

  install.addEventListener("click", () => {
    void (async () => {
      setBusy(install, true, text("等待选择…", "Waiting for folder…"));
      try {
        const result = await invoke<{ skill: SkillSummary } | null>(
          "skills_install_pick",
        );
        if (result?.skill)
          toast(
            text(
              `已安装 ${result.skill.name}`,
              `Installed ${result.skill.name}`,
            ),
          );
        await loadLocal();
      } catch (error) {
        status(
          overlayStatus,
          `${text("安装失败：", "Install failed: ")}${errorText(error)}`,
          "error",
        );
      } finally {
        setBusy(install, false);
      }
    })();
  });
  refresh.addEventListener("click", () => void loadLocal());
  document
    .getElementById("openSkillsFromSettings")
    ?.addEventListener("click", () => {
      void loadLocal();
      void loadCatalogBrowse();
    });
  document.addEventListener("click", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest('[data-tools-tab="skills"]')) {
      void loadLocal();
      void loadCatalogBrowse();
    }
  });
  const syncSkillsChrome = () => {
    labelControl(
      refresh,
      text("刷新本机", "Refresh local"),
      text("刷新本机 Skills", "Refresh local Skills"),
    );
    labelControl(
      install,
      text("从文件夹安装", "Install folder"),
      text("从文件夹安装 Skill", "Install Skill from folder"),
    );
  };
  onServiceLanguageChange(() => {
    syncSkillsChrome();
    void loadLocal();
    if (catalogQuery.trim()) void loadCatalogSearch();
    else void loadCatalogBrowse();
  });
  syncSkillsChrome();
  status(
    settingsHint,
    text("正在连接本机 Skills…", "Connecting to native Skills…"),
  );
  void loadLocal();
  void loadCatalogBrowse();
}

const MCP_COLLECTION_KEYS = [
  "servers",
  "items",
  "mcpServers",
  "mcp_servers",
] as const;

type McpServerRecord = {
  id: string;
  record: UnknownRecord;
};

type McpHeaderDraft = {
  name: string;
  value: string;
  source?: UnknownRecord;
};

function cloneJson<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function mcpServerRecords(value: unknown): McpServerRecord[] {
  const entries = (candidate: unknown): McpServerRecord[] => {
    if (Array.isArray(candidate)) {
      return candidate.flatMap((item) => {
        const record = asRecord(item);
        const id = readString(record, "id", "serverId", "server_id");
        return record && id ? [{ id, record }] : [];
      });
    }
    const object = asRecord(candidate);
    if (!object) return [];
    for (const key of MCP_COLLECTION_KEYS) {
      if (object[key] !== undefined) return entries(object[key]);
    }
    const ownId = readString(object, "id", "serverId", "server_id");
    if (ownId) return [{ id: ownId, record: object }];
    return Object.entries(object).flatMap(([fallbackId, item]) => {
      const record = asRecord(item);
      const id =
        readString(record, "id", "serverId", "server_id") ?? fallbackId.trim();
      return record && id ? [{ id, record }] : [];
    });
  };
  const seen = new Set<string>();
  return entries(value).filter(({ id }) => {
    if (seen.has(id)) return false;
    seen.add(id);
    return true;
  });
}

function mcpServers(value: unknown): McpServerSummary[] {
  return mcpServerRecords(value).map(({ id, record }) => ({
    id,
    label: readString(record, "label", "name", "title") ?? id,
    enabled: readBoolean(record.enabled, true),
    transport: readString(record, "transport") ?? "stdio",
  }));
}

function replaceMcpServerSettings(
  raw: unknown,
  serverId: string,
  nextRecord: UnknownRecord,
  create: boolean,
): unknown {
  const value = cloneJson(raw ?? []);
  const replace = (candidate: unknown): boolean => {
    if (Array.isArray(candidate)) {
      const index = candidate.findIndex(
        (item) =>
          readString(asRecord(item), "id", "serverId", "server_id") ===
          serverId,
      );
      if (index >= 0) {
        candidate[index] = nextRecord;
        return true;
      }
      return false;
    }
    const object = asRecord(candidate);
    if (!object) return false;
    if (readString(object, "id", "serverId", "server_id") === serverId) {
      for (const key of Object.keys(object)) delete object[key];
      Object.assign(object, nextRecord);
      return true;
    }
    for (const key of MCP_COLLECTION_KEYS) {
      if (object[key] !== undefined && replace(object[key])) return true;
    }
    if (asRecord(object[serverId])) {
      object[serverId] = nextRecord;
      return true;
    }
    for (const [key, item] of Object.entries(object)) {
      if (
        readString(asRecord(item), "id", "serverId", "server_id") === serverId
      ) {
        object[key] = nextRecord;
        return true;
      }
    }
    return false;
  };
  if (replace(value)) return value;
  if (!create) return value;

  const append = (candidate: unknown): unknown => {
    if (Array.isArray(candidate)) {
      candidate.push(nextRecord);
      return candidate;
    }
    const object = asRecord(candidate);
    if (!object) return [nextRecord];
    for (const key of MCP_COLLECTION_KEYS) {
      if (object[key] !== undefined) {
        object[key] = append(object[key]);
        return object;
      }
    }
    if (readString(object, "id", "serverId", "server_id"))
      return [object, nextRecord];
    object[serverId] = nextRecord;
    return object;
  };
  return append(value);
}

function parseMcpEnvironment(value: string): Record<string, string> {
  const entries: Record<string, string> = {};
  for (const raw of value.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    const separator = line.indexOf("=");
    const name = separator >= 0 ? line.slice(0, separator).trim() : "";
    const secret = separator >= 0 ? line.slice(separator + 1).trim() : "";
    if (!name || /[\r\n\0=]/.test(name))
      throw new Error(
        text(
          "环境变量须使用 NAME=value，一行一个。",
          "Environment variables must use NAME=value, one per line.",
        ),
      );
    if (Object.hasOwn(entries, name))
      throw new Error(
        text(
          `环境变量 ${name} 重复。`,
          `Environment variable ${name} is duplicated.`,
        ),
      );
    entries[name] = secret;
  }
  return entries;
}

function mcpHeaderDrafts(value: unknown): McpHeaderDraft[] {
  if (Array.isArray(value)) {
    return value.flatMap((item) => {
      const source = asRecord(item);
      const name = readString(source, "key", "name");
      if (!source || !name) return [];
      const raw = source.value;
      const value =
        typeof raw === "string" && raw.trim()
          ? "[configured]"
          : source.valueConfigured === true
            ? "[configured]"
            : "";
      return value ? [{ name, value, source }] : [];
    });
  }
  const object = asRecord(value);
  if (!object) return [];
  return Object.entries(object).flatMap(([name, raw]) => {
    if (typeof raw !== "string" || !raw) return [];
    return [{ name, value: "[configured]" }];
  });
}

function parseMcpHeaders(value: string): McpHeaderDraft[] {
  const entries: McpHeaderDraft[] = [];
  const names = new Set<string>();
  for (const raw of value.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    const separator = line.indexOf(":");
    const name = separator >= 0 ? line.slice(0, separator).trim() : "";
    const secret = separator >= 0 ? line.slice(separator + 1).trim() : "";
    if (!name || !secret || /[\r\n\0\s:]/.test(name))
      throw new Error(
        text(
          "请求头须使用 Name: value，一行一个。",
          "Headers must use Name: value, one per line.",
        ),
      );
    const key = name.toLowerCase();
    if (key === "authorization")
      throw new Error(
        text(
          "请使用单独的 Authorization 凭据字段。",
          "Use the separate Authorization credential field.",
        ),
      );
    if (names.has(key))
      throw new Error(
        text(`请求头 ${name} 重复。`, `Header ${name} is duplicated.`),
      );
    names.add(key);
    entries.push({ name, value: secret });
  }
  return entries;
}

function serializeMcpHeaders(
  entries: McpHeaderDraft[],
  original: unknown,
): unknown {
  if (!entries.length) return undefined;
  if (!Array.isArray(original))
    return Object.fromEntries(entries.map(({ name, value }) => [name, value]));
  const existing = mcpHeaderDrafts(original);
  return entries.map((entry) => {
    const source = existing.find(
      (item) => item.name.toLowerCase() === entry.name.toLowerCase(),
    )?.source;
    const result = cloneJson(source ?? {});
    const nameKey =
      typeof result.name === "string" && typeof result.key !== "string"
        ? "name"
        : "key";
    result[nameKey] = entry.name;
    result.value = entry.value;
    delete result.valueConfigured;
    return result;
  });
}

function installMcp() {
  const invoke = invokeApi();
  const overlay = node<HTMLElement>("overlayMcp");
  if (!invoke || !overlay) return;

  const grid = overlay.querySelector<HTMLElement>(".hub-grid");
  const actions = overlay.querySelector<HTMLElement>(
    ".overlay-head .row-actions",
  );
  const settingsPanel = document.querySelector<HTMLElement>(
    '[data-tools-panel="mcp"]',
  );
  const settingsHint = settingsPanel?.querySelector<HTMLElement>("p") ?? null;
  if (!grid || !actions) return;

  const state: {
    servers: McpServerSummary[];
    rawSettings: unknown;
    records: Map<string, UnknownRecord>;
    statuses: Map<string, McpRuntimeStatus>;
    tests: Map<string, McpRuntimeTestResponse>;
    tools: Map<string, McpToolInfo[]>;
    error?: string;
  } = {
    servers: [],
    rawSettings: [],
    records: new Map(),
    statuses: new Map(),
    tests: new Map(),
    tools: new Map(),
  };
  let selectedServerId: string | undefined;
  const overlayStatus = element("p", {
    className: "novavei-service-status",
    attrs: { "aria-live": "polite" },
  });
  const create = button(
    text("新建 Server", "New server"),
    "btn primary",
    text("新建 MCP Server", "Create MCP server"),
  );
  const registryButton = button(
    text("官方 Registry", "Official Registry"),
    "btn",
    text("浏览官方 MCP Registry", "Browse the official MCP Registry"),
  );
  const refresh = button(
    text("刷新", "Refresh"),
    "btn",
    text("刷新 MCP Server", "Refresh MCP servers"),
  );
  const close = actions.querySelector<HTMLElement>("[data-close-overlay]");
  actions.replaceChildren(
    create,
    registryButton,
    refresh,
    close ?? button(text("关闭", "Close"), "btn ghost"),
  );

  const editor = (() => {
    const dialog = element("dialog", {
      className: "novavei-service-dialog novavei-mcp-editor",
      attrs: { "aria-modal": "true" },
    });
    const form = element("form", { className: "novavei-service-dialog-body" });
    form.noValidate = true;
    const heading = element("h2", { id: "novaveiMcpEditorTitle" });
    dialog.setAttribute("aria-labelledby", heading.id);
    const intro = element("p", {
      className: "novavei-service-note",
      text: text(
        "保存只会写入本机 MCP 配置，不会连接服务器或调用 MCP 工具。",
        "Saving only writes local MCP settings. It does not connect to a server or call MCP tools.",
      ),
    });
    const makeField = (
      labelText: string,
      control: HTMLElement,
      required = false,
      hint?: string,
    ) => {
      const field = element("div", { className: "field" });
      const label = element("label", { text: labelText });
      label.htmlFor = control.id;
      if (required) {
        label.append(
          document.createTextNode(" "),
          element("span", {
            className: "novavei-mcp-required",
            text: "*",
            attrs: { "aria-hidden": "true" },
          }),
        );
        control.setAttribute(
          "aria-label",
          `${labelText} ${text("（必填）", "(required)")}`,
        );
      }
      field.append(label, control);
      if (hint)
        field.append(
          element("p", { className: "novavei-service-note", text: hint }),
        );
      return field;
    };
    const makeToggle = (
      input: HTMLInputElement,
      labelText: string,
      hint?: string,
    ) => {
      const field = element("div", { className: "field" });
      const control = element("label", { className: "novavei-mcp-toggle" });
      control.append(input, element("span", { text: labelText }));
      field.append(control);
      if (hint)
        field.append(
          element("p", { className: "novavei-service-note", text: hint }),
        );
      return field;
    };

    const idInput = element("input", {
      id: "novaveiMcpServerId",
      attrs: {
        type: "text",
        maxlength: "128",
        pattern: "[A-Za-z0-9_-]{1,128}",
        required: "",
        autocomplete: "off",
      },
    });
    idInput.type = "text";
    const labelInput = element("input", {
      id: "novaveiMcpServerLabel",
      attrs: { type: "text", maxlength: "160", autocomplete: "off" },
    });
    labelInput.type = "text";
    const enabledInput = element("input", {
      id: "novaveiMcpServerEnabled",
      attrs: { type: "checkbox" },
    });
    enabledInput.type = "checkbox";
    const transport = element("select", {
      id: "novaveiMcpTransport",
      attrs: { "aria-label": text("MCP 传输方式", "MCP transport") },
    });
    for (const [value, zh, en] of [
      ["stdio", "本机 stdio", "Local stdio"],
      ["http", "HTTP（Streamable HTTP）", "HTTP (Streamable HTTP)"],
      ["sse", "SSE（兼容模式）", "SSE (compatibility mode)"],
    ] as const) {
      const option = element("option", { text: text(zh, en) });
      option.value = value;
      transport.append(option);
    }
    const command = element("input", {
      id: "novaveiMcpCommand",
      attrs: { type: "text", maxlength: "32768", autocomplete: "off" },
    });
    command.type = "text";
    const argumentsInput = element("textarea", {
      id: "novaveiMcpArguments",
      attrs: { maxlength: "65536", autocomplete: "off", spellcheck: "false" },
    });
    const cwd = element("input", {
      id: "novaveiMcpCwd",
      attrs: { type: "text", maxlength: "32768", autocomplete: "off" },
    });
    cwd.type = "text";
    const framing = element("select", {
      id: "novaveiMcpFraming",
      attrs: { "aria-label": text("stdio 消息帧", "stdio message framing") },
    });
    for (const [value, zh, en] of [
      ["jsonl", "JSON Lines（默认）", "JSON Lines (default)"],
      [
        "content-length",
        "Content-Length（兼容）",
        "Content-Length (compatibility)",
      ],
    ] as const) {
      const option = element("option", { text: text(zh, en) });
      option.value = value;
      framing.append(option);
    }
    const endpoint = element("input", {
      id: "novaveiMcpEndpoint",
      attrs: { type: "url", maxlength: "4096", autocomplete: "off" },
    });
    endpoint.type = "url";
    const messageUrl = element("input", {
      id: "novaveiMcpMessageUrl",
      attrs: {
        type: "text",
        inputmode: "url",
        maxlength: "4096",
        autocomplete: "off",
      },
    });
    messageUrl.type = "text";
    const allowRemote = element("input", {
      id: "novaveiMcpAllowRemote",
      attrs: { type: "checkbox" },
    });
    allowRemote.type = "checkbox";
    const timeout = element("input", {
      id: "novaveiMcpTimeout",
      attrs: {
        type: "number",
        min: "1",
        max: "600000",
        step: "1",
        required: "",
      },
    });
    timeout.type = "number";
    const authorization = element("input", {
      id: "novaveiMcpAuthorization",
      attrs: {
        type: "password",
        maxlength: "65536",
        autocomplete: "new-password",
      },
    });
    authorization.type = "password";
    const clearAuthorization = element("input", {
      id: "novaveiMcpClearAuthorization",
      attrs: { type: "checkbox" },
    });
    clearAuthorization.type = "checkbox";
    const headers = element("textarea", {
      id: "novaveiMcpHeaders",
      attrs: { maxlength: "65536", autocomplete: "off", spellcheck: "false" },
    });
    const environment = element("textarea", {
      id: "novaveiMcpEnvironment",
      attrs: { maxlength: "65536", autocomplete: "off", spellcheck: "false" },
    });

    const stdioFields = element("div", {
      className: "novavei-mcp-transport-fields",
    });
    stdioFields.append(
      makeField(
        text("启动命令", "Command"),
        command,
        true,
        text(
          "仅 stdio 使用。请勿把密钥放入命令或参数，改用下方环境变量。",
          "Used only by stdio. Keep secrets out of command and arguments; use the environment fields below.",
        ),
      ),
      makeField(
        text("命令参数", "Arguments"),
        argumentsInput,
        false,
        text(
          "每行一个参数；保留同一行内的空格。",
          "One argument per line; spaces within a line are preserved.",
        ),
      ),
      makeField(text("工作目录", "Working directory"), cwd, false),
      makeField(text("stdio 消息帧", "stdio framing"), framing),
    );
    const remoteFields = element("div", {
      className: "novavei-mcp-transport-fields",
    });
    const remoteWarning = element("p", {
      className: "novavei-mcp-warning",
      text: text(
        "默认仅允许 localhost。勾选“允许远程 URL”后，配置可请求局域网或互联网地址；仅在你信任该 MCP Server 时启用。",
        "Only localhost is allowed by default. Enabling remote URLs allows LAN or internet addresses; enable it only for a trusted MCP server.",
      ),
    });
    remoteFields.append(
      makeField(
        text("服务器 URL", "Server URL"),
        endpoint,
        true,
        text(
          "HTTP/SSE 需要完整 URL；默认会被 native 限制为 localhost。",
          "HTTP/SSE requires a full URL and is native-restricted to localhost by default.",
        ),
      ),
      makeField(
        text("SSE 消息 URL", "SSE message URL"),
        messageUrl,
        false,
        text(
          "仅 SSE 可选；可使用完整 URL 或以 / 开头的相对路径。",
          "Optional for SSE only; use a full URL or a relative path beginning with /.",
        ),
      ),
      makeToggle(
        allowRemote,
        text("允许远程 URL（默认关闭）", "Allow remote URL (off by default)"),
      ),
      remoteWarning,
    );
    const credentialWarning = element("p", {
      className: "novavei-mcp-warning",
      text: text(
        "已有密钥绝不回显。`[configured]` 表示 native 已保存的值，原样保留即可；删除该行会在保存时移除对应凭据。新输入的凭据会在保存或关闭后从页面内存清除。",
        "Existing secrets are never shown. `[configured]` means native already holds a value; keep it unchanged to retain it. Removing that line removes the credential on save. Newly entered credentials are cleared from page memory after save or close.",
      ),
    });
    const registryDraftNotice = element("p", {
      className: "novavei-mcp-warning",
      attrs: { "aria-live": "polite" },
    });
    registryDraftNotice.hidden = true;
    const registryDraftConfirmation = element("input", {
      id: "novaveiMcpRegistryDraftConfirm",
      attrs: { type: "checkbox" },
    });
    registryDraftConfirmation.type = "checkbox";
    const registryDraftConfirmationField = makeToggle(
      registryDraftConfirmation,
      text(
        "我已核对该 Registry 草稿，确认保存前不会连接；我会单独决定是否启用及允许远程 URL。",
        "I reviewed this Registry draft. Saving will not connect; I will separately decide whether to enable it and allow remote URLs.",
      ),
    );
    registryDraftConfirmationField.hidden = true;
    const identityField = makeField(
      text("Server ID", "Server ID"),
      idInput,
      true,
      text(
        "仅可使用 ASCII 字母、数字、_ 和 -。ID 是稳定的 native 身份；编辑时不可改名，以免错误继承已保存的凭据。",
        "Use ASCII letters, numbers, _ and - only. The ID is the stable native identity and cannot change during edit, preventing credentials from being inherited by the wrong server.",
      ),
    );
    const labelField = makeField(
      text("显示名称", "Display name"),
      labelInput,
      false,
    );
    const enabledField = makeToggle(
      enabledInput,
      text("启用此 Server", "Enable this server"),
    );
    const transportField = makeField(
      text("传输方式", "Transport"),
      transport,
      true,
    );
    const timeoutField = makeField(
      text("请求超时（毫秒）", "Request timeout (ms)"),
      timeout,
      true,
      text("范围 1–600000；默认 60000。", "Range 1–600000; default is 60000."),
    );
    const authorizationField = makeField(
      text("Authorization 凭据", "Authorization credential"),
      authorization,
      false,
      text(
        "可选；已保存值不会显示。留空会保持现有值，除非勾选下方移除。",
        "Optional; a saved value is never shown. Leave blank to retain it unless removal is selected below.",
      ),
    );
    const clearAuthorizationField = makeToggle(
      clearAuthorization,
      text(
        "移除已保存的 Authorization 凭据",
        "Remove saved Authorization credential",
      ),
    );
    const headersField = makeField(
      text("其他敏感请求头", "Other sensitive request headers"),
      headers,
      false,
      text(
        "每行 `Name: value`。已有值仅显示为 `[configured]`；Authorization 请使用单独字段。",
        "One `Name: value` per line. Existing values appear only as `[configured]`; use the separate field for Authorization.",
      ),
    );
    const environmentField = makeField(
      text("敏感环境变量", "Sensitive environment variables"),
      environment,
      false,
      text(
        "每行 `NAME=value`。已有值仅显示为 `[configured]`。",
        "One `NAME=value` per line. Existing values appear only as `[configured]`.",
      ),
    );
    const alert = element("p", {
      attrs: { role: "alert", "aria-live": "assertive" },
    });
    const actionRow = element("div", { className: "row-actions" });
    const cancel = button(
      text("取消", "Cancel"),
      "btn",
      text("取消 MCP 配置编辑", "Cancel MCP configuration editing"),
    );
    const save = button(
      text("安全保存", "Save safely"),
      "btn primary",
      text("安全保存 MCP 配置", "Save MCP configuration safely"),
    );
    save.type = "submit";
    actionRow.append(cancel, save);
    form.append(
      heading,
      intro,
      registryDraftNotice,
      registryDraftConfirmationField,
      identityField,
      labelField,
      enabledField,
      transportField,
      stdioFields,
      remoteFields,
      timeoutField,
      credentialWarning,
      authorizationField,
      clearAuthorizationField,
      headersField,
      environmentField,
      alert,
      actionRow,
    );
    dialog.append(form);
    document.body.append(dialog);

    let editing: McpServerRecord | undefined;
    let registryDraft: McpRegistryRemoteDraft | undefined;
    let previousFocus: HTMLElement | null = null;
    let saving = false;
    const showError = (control: HTMLElement, message: string) => {
      alert.textContent = message;
      control.focus({ preventScroll: true });
    };
    const mcpSaveError = (error: unknown) => {
      const code = errorText(error).trim().toLowerCase();
      switch (code) {
        case "mcp_settings_invalid_id":
          return {
            control: idInput,
            message: text(
              "MCP Server ID 无效或已存在。请使用一个唯一的 ID。",
              "The MCP Server ID is invalid or already exists. Use a unique ID.",
            ),
          };
        case "mcp_settings_invalid_transport":
          return {
            control: transport,
            message: text(
              "请选择受支持的 MCP 传输方式。",
              "Choose a supported MCP transport.",
            ),
          };
        case "mcp_settings_invalid_url":
          return {
            control: endpoint,
            message: text(
              "请检查服务器 URL；远程地址需要单独允许。",
              "Check the server URL. Remote addresses require separate approval.",
            ),
          };
        case "mcp_settings_invalid_headers":
          return {
            control: headers,
            message: text(
              "请检查请求头名称和值。已保存的凭据仍保持隐藏。",
              "Check header names and values. Saved credentials remain hidden.",
            ),
          };
        case "mcp_settings_invalid_timeout":
          return {
            control: timeout,
            message: text(
              "请求超时必须是 1 到 600000 之间的整数。",
              "Request timeout must be an integer from 1 to 600000.",
            ),
          };
        case "mcp_settings_invalid_command":
          return {
            control: command,
            message: text(
              "请检查 stdio 命令、参数和工作目录。",
              "Check the stdio command, arguments, and working directory.",
            ),
          };
        case "mcp_settings_invalid_environment":
          return {
            control: environment,
            message: text(
              "请检查敏感环境变量的名称和值。",
              "Check sensitive environment variable names and values.",
            ),
          };
        case "mcp_settings_unavailable":
          return {
            control: save,
            message: text(
              "本机 MCP 设置当前不可写。请先恢复本机存储或解锁受保护设置。",
              "Local MCP settings cannot be written. Restore local storage or unlock protected settings first.",
            ),
          };
        case "mcp_stdio_native_confirmation_denied":
          return {
            control: save,
            message: text(
              "已取消本机确认；此 stdio MCP 不会保存或执行。",
              "Native confirmation was cancelled. This stdio MCP was not saved or executed.",
            ),
          };
        case "mcp_stdio_native_confirmation_unavailable":
        case "mcp_stdio_native_confirmation_required":
          return {
            control: save,
            message: text(
              "stdio MCP 必须通过系统原生确认。当前环境无法完成确认，因此配置未保存。",
              "A stdio MCP requires an OS-native confirmation. This environment cannot complete it, so the configuration was not saved.",
            ),
          };
        default:
          return {
            control: save,
            message: text(
              "无法保存 MCP 配置。请检查字段并确认本机设置未锁定。",
              "Unable to save MCP settings. Check the fields and confirm local settings are unlocked.",
            ),
          };
      }
    };
    const clearSensitiveDraft = () => {
      authorization.value = "";
      headers.value = "";
      environment.value = "";
      clearAuthorization.checked = false;
      authorization.removeAttribute("data-configured");
    };
    const renderTransportFields = () => {
      const isStdio = transport.value === "stdio";
      const isSse = transport.value === "sse";
      stdioFields.hidden = !isStdio;
      remoteFields.hidden = isStdio;
      const messageUrlField = messageUrl.closest<HTMLElement>(".field");
      if (!messageUrlField) {
        throw new Error("MCP message URL field is unavailable");
      }
      messageUrlField.hidden = !isSse;
      command.required = isStdio;
      endpoint.required = !isStdio;
    };
    const open = (entry?: McpServerRecord, draft?: McpRegistryRemoteDraft) => {
      editing = entry;
      registryDraft = draft;
      previousFocus =
        document.activeElement instanceof HTMLElement
          ? document.activeElement
          : null;
      const record: UnknownRecord | undefined =
        entry?.record ??
        (draft
          ? {
              id: draft.id,
              label: draft.label,
              enabled: draft.enabled,
              transport: draft.transport,
              url: draft.url,
              allowRemote: draft.allowRemote,
              timeoutMs: draft.timeoutMs,
            }
          : undefined);
      const rawTransport = readString(record, "transport")?.toLocaleLowerCase();
      const selectedTransport =
        rawTransport === "sse" ||
        rawTransport === "legacy-sse" ||
        rawTransport === "legacy_sse"
          ? "sse"
          : rawTransport === "http" ||
              rawTransport === "streamable-http" ||
              rawTransport === "streamablehttp"
            ? "http"
            : "stdio";
      heading.textContent = entry
        ? text("编辑 MCP Server", "Edit MCP server")
        : draft
          ? text("复核 Registry 草稿", "Review Registry draft")
          : text("新建 MCP Server", "New MCP server");
      idInput.value = entry?.id ?? readString(record, "id") ?? "";
      idInput.readOnly = Boolean(entry || draft);
      labelInput.value = readString(record, "label", "name", "title") ?? "";
      enabledInput.checked = readBoolean(record?.enabled, true);
      transport.value = selectedTransport;
      command.value = readString(record, "command") ?? "";
      argumentsInput.value = Array.isArray(record?.args)
        ? record.args
            .filter((item): item is string => typeof item === "string")
            .join("\n")
        : "";
      cwd.value = readString(record, "cwd") ?? "";
      framing.value =
        readString(record, "stdioFraming", "stdio_framing") === "content-length"
          ? "content-length"
          : "jsonl";
      endpoint.value = readString(record, "url") ?? "";
      messageUrl.value = readString(record, "messageUrl", "message_url") ?? "";
      allowRemote.checked = readBoolean(record?.allowRemote, false);
      const timeoutValue =
        typeof record?.timeoutMs === "number" &&
        Number.isFinite(record.timeoutMs)
          ? record.timeoutMs
          : 60_000;
      timeout.value = String(timeoutValue);
      clearSensitiveDraft();
      const headerEntries = mcpHeaderDrafts(record?.headers);
      const auth = headerEntries.find(
        (item) => item.name.toLowerCase() === "authorization",
      );
      authorization.placeholder = auth
        ? text("已配置，留空会保持不变", "Configured; leave blank to retain")
        : text("可选，例如 Bearer …", "Optional, for example Bearer …");
      if (auth) authorization.dataset.configured = "true";
      headers.value = headerEntries
        .filter((item) => item.name.toLowerCase() !== "authorization")
        .map((item) => `${item.name}: ${item.value}`)
        .join("\n");
      const existingEnvironment = asRecord(record?.env);
      environment.value = Object.entries(existingEnvironment ?? {})
        .flatMap(([name, value]) =>
          typeof value === "string" && value.trim()
            ? [`${name}=[configured]`]
            : [],
        )
        .join("\n");
      registryDraftConfirmation.checked = false;
      registryDraftNotice.hidden = !draft;
      registryDraftConfirmationField.hidden = !draft;
      if (draft) {
        const inputs = [...draft.headers, ...draft.variables]
          .map((input) => `${input.name}${input.required ? " *" : ""}`)
          .join(" · ");
        registryDraftNotice.textContent = text(
          `来自官方 Registry：${draft.registryName}@${draft.registryVersion}。此草稿默认停用且不允许远程 URL；保存不会连接。${inputs ? ` 可能需要你自行配置：${inputs}。` : ""}`,
          `From the official Registry: ${draft.registryName}@${draft.registryVersion}. This draft starts disabled with remote URLs denied; saving does not connect.${inputs ? ` You may need to configure: ${inputs}.` : ""}`,
        );
      } else {
        registryDraftNotice.textContent = "";
      }
      alert.textContent = "";
      renderTransportFields();
      if (!dialog.open) dialog.showModal();
      window.setTimeout(
        () => (entry || draft ? labelInput : idInput).focus(),
        0,
      );
    };
    const openRegistryDraft = (draft: McpRegistryRemoteDraft) => {
      if (
        !isPiSafeRegistryId(draft.id) ||
        draft.enabled ||
        draft.allowRemote ||
        !isConcreteHttpsRegistryRemote(draft.url) ||
        !["http", "sse"].includes(draft.transport)
      ) {
        throw new Error(
          "Registry draft did not satisfy the local safety contract",
        );
      }
      open(undefined, draft);
    };
    const buildRecord = () => {
      const id = idInput.value.trim();
      if (!/^[A-Za-z0-9_-]{1,128}$/.test(id)) {
        showError(
          idInput,
          text(
            "Server ID 只能包含 ASCII 字母、数字、_ 和 -，最多 128 个字符。",
            "Server ID may contain only ASCII letters, numbers, _ and -, up to 128 characters.",
          ),
        );
        return undefined;
      }
      if (!editing && state.servers.some((server) => server.id === id)) {
        showError(
          idInput,
          text("该 Server ID 已存在。", "That Server ID already exists."),
        );
        return undefined;
      }
      const timeoutMs = Number(timeout.value);
      if (
        !Number.isSafeInteger(timeoutMs) ||
        timeoutMs < 1 ||
        timeoutMs > 600_000
      ) {
        showError(
          timeout,
          text(
            "请求超时必须是 1 到 600000 之间的整数。",
            "Request timeout must be an integer from 1 to 600000.",
          ),
        );
        return undefined;
      }
      if (transport.value === "stdio" && !command.value.trim()) {
        showError(
          command,
          text("请填写 stdio 启动命令。", "Enter a stdio command."),
        );
        return undefined;
      }
      if (transport.value !== "stdio" && !endpoint.value.trim()) {
        showError(
          endpoint,
          text("请填写 HTTP/SSE 服务器 URL。", "Enter an HTTP/SSE server URL."),
        );
        return undefined;
      }
      if (transport.value !== "stdio" && !endpoint.checkValidity()) {
        showError(
          endpoint,
          text("请输入有效的服务器 URL。", "Enter a valid server URL."),
        );
        return undefined;
      }
      const sseMessagePath = messageUrl.value.trim();
      if (
        transport.value === "sse" &&
        sseMessagePath &&
        !sseMessagePath.startsWith("/") &&
        !/^https?:\/\//i.test(sseMessagePath)
      ) {
        showError(
          messageUrl,
          text(
            "SSE 消息地址须为完整 HTTP(S) URL 或以 / 开头的相对路径。",
            "SSE message URL must be a full HTTP(S) URL or a relative path starting with /.",
          ),
        );
        return undefined;
      }
      let env: Record<string, string>;
      try {
        env = parseMcpEnvironment(environment.value);
      } catch (error) {
        const message =
          error instanceof Error
            ? error.message
            : text("凭据格式无效。", "Credential format is invalid.");
        showError(environment, message);
        return undefined;
      }
      let headerEntries: McpHeaderDraft[];
      try {
        headerEntries = parseMcpHeaders(headers.value);
      } catch (error) {
        const message =
          error instanceof Error
            ? error.message
            : text("凭据格式无效。", "Credential format is invalid.");
        showError(headers, message);
        return undefined;
      }
      const originalHeaders = editing?.record.headers;
      const existingAuthorization = mcpHeaderDrafts(originalHeaders).find(
        (item) => item.name.toLowerCase() === "authorization",
      );
      if (!clearAuthorization.checked) {
        const credential = authorization.value.trim();
        if (credential) {
          headerEntries.push({
            name: existingAuthorization?.name ?? "Authorization",
            value: credential,
          });
        } else if (existingAuthorization) {
          headerEntries.push({
            name: existingAuthorization.name,
            value: "[configured]",
          });
        }
      }
      const record = cloneJson(editing?.record ?? {});
      record.id = id;
      const label = labelInput.value.trim();
      const labelKey =
        typeof record.label === "string"
          ? "label"
          : typeof record.name === "string"
            ? "name"
            : typeof record.title === "string"
              ? "title"
              : "label";
      if (label) record[labelKey] = label;
      else delete record[labelKey];
      record.enabled = enabledInput.checked;
      record.transport = transport.value;
      record.timeoutMs = timeoutMs;
      if (Object.keys(env).length) record.env = env;
      else delete record.env;
      const nextHeaders = serializeMcpHeaders(headerEntries, originalHeaders);
      if (nextHeaders) record.headers = nextHeaders;
      else delete record.headers;
      if (transport.value === "stdio") {
        record.command = command.value.trim();
        // An argument occupies one line. Preserve meaningful leading/trailing
        // whitespace inside that argument; only blank lines are ignored.
        record.args = argumentsInput.value
          .split(/\r?\n/)
          .filter((value) => value.trim().length > 0);
        if (cwd.value.trim()) record.cwd = cwd.value.trim();
        else delete record.cwd;
        record.stdioFraming = framing.value;
        delete record.url;
        delete record.messageUrl;
        record.allowRemote = false;
      } else {
        record.url = endpoint.value.trim();
        if (transport.value === "sse" && messageUrl.value.trim())
          record.messageUrl = messageUrl.value.trim();
        else delete record.messageUrl;
        record.allowRemote = allowRemote.checked;
        delete record.command;
        delete record.args;
        delete record.cwd;
        delete record.stdioFraming;
      }
      return record;
    };
    const saveConfig = async () => {
      const record = buildRecord();
      if (!record) return;
      if (registryDraft) {
        if (!registryDraftConfirmation.checked) {
          showError(
            registryDraftConfirmation,
            text(
              "请先确认你已复核 Registry 草稿；保存不会连接，启用和允许远程 URL 需要你单独决定。",
              "Confirm that you reviewed the Registry draft. Saving will not connect; enabling it and allowing remote URLs are separate decisions.",
            ),
          );
          return;
        }
        const decision =
          record.enabled || record.allowRemote
            ? text(
                "此 Registry 草稿最初是停用且不允许远程 URL。你已修改其中至少一项。保存本身仍不会连接，但后续启用时可能连接远程服务。确认写入本机设置吗？",
                "This Registry draft started disabled with remote URLs denied. You changed at least one of those settings. Saving still does not connect, but a later enable may contact a remote service. Write it to local settings?",
              )
            : text(
                "确认将该 Registry 草稿写入本机设置吗？它会保持停用且不允许远程 URL，保存不会连接或执行任何 MCP。",
                "Write this Registry draft to local settings? It will remain disabled with remote URLs denied; saving does not connect or execute any MCP.",
              );
        if (
          !(await requestAppConfirm({
            title: text("确认保存 MCP 草稿", "Confirm MCP draft save"),
            message: decision,
            confirmLabel: text("保存", "Save"),
            cancelLabel: text("取消", "Cancel"),
            danger: false,
          }))
        ) {
          status(
            alert,
            text(
              "已取消保存 Registry 草稿。",
              "Registry draft save cancelled.",
            ),
          );
          return;
        }
      }
      const serverId = String(record.id);
      const payload = replaceMcpServerSettings(
        state.rawSettings,
        serverId,
        record,
        !editing,
      );
      saving = true;
      setBusy(save, true, text("保存中…", "Saving…"));
      cancel.disabled = true;
      alert.textContent = "";
      try {
        await invoke("settings_save_mcp", { payload });
        await load();
        dialog.close();
        toast(
          editing
            ? text("MCP Server 已更新", "MCP server updated")
            : text("MCP Server 已保存", "MCP server saved"),
        );
      } catch (error) {
        // Native MCP-save failures are closed stable codes. Never render a
        // raw backend error here because it could carry a newly entered secret
        // or a local endpoint.
        const failure = mcpSaveError(error);
        showError(failure.control, failure.message);
      } finally {
        saving = false;
        setBusy(save, false);
        cancel.disabled = false;
      }
    };
    transport.addEventListener("change", renderTransportFields);
    const invalidateRegistryDraftConfirmation = (event: Event) => {
      if (registryDraft && event.target !== registryDraftConfirmation)
        registryDraftConfirmation.checked = false;
    };
    form.addEventListener("input", invalidateRegistryDraftConfirmation);
    form.addEventListener("change", invalidateRegistryDraftConfirmation);
    cancel.addEventListener("click", () => dialog.close());
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      void saveConfig();
    });
    dialog.addEventListener("cancel", (event) => {
      if (saving) event.preventDefault();
    });
    dialog.addEventListener("close", () => {
      clearSensitiveDraft();
      editing = undefined;
      registryDraft = undefined;
      registryDraftConfirmation.checked = false;
      registryDraftNotice.hidden = true;
      registryDraftConfirmationField.hidden = true;
      registryDraftNotice.textContent = "";
      previousFocus?.focus({ preventScroll: true });
    });
    return { open, openRegistryDraft };
  })();

  const registry = (() => {
    const root = element("section", {
      className: "novavei-mcp-registry",
      attrs: {
        "aria-label": text("官方 MCP Registry", "Official MCP Registry"),
      },
    });
    const heading = element("h3", {
      text: text("官方 MCP Registry", "Official MCP Registry"),
    });
    const lead = element("p", {
      className: "novavei-service-note",
      text: text(
        "搜索仅通过原生只读桥接访问官方 Registry。包条目仅供参考，绝不会自动安装、执行或生成命令。",
        "Search uses the native read-only bridge to the official Registry. Package entries are reference-only and never auto-install, execute, or generate commands.",
      ),
    });
    const form = element("form", { className: "novavei-mcp-registry-form" });
    const queryInput = element("input", {
      id: "novaveiMcpRegistrySearch",
      attrs: {
        type: "search",
        maxlength: "200",
        autocomplete: "off",
        placeholder: text("按名称或关键词搜索", "Search by name or keyword"),
      },
    });
    queryInput.type = "search";
    const queryLabel = element("label", {
      text: text("搜索官方 Registry", "Search official Registry"),
    });
    queryLabel.htmlFor = queryInput.id;
    queryLabel.append(queryInput);
    const search = button(
      text("搜索", "Search"),
      "btn primary",
      text("搜索官方 MCP Registry", "Search official MCP Registry"),
    );
    search.type = "submit";
    const clear = button(
      text("清除", "Clear"),
      "btn",
      text("清除 Registry 搜索结果", "Clear Registry search results"),
    );
    form.append(queryLabel, search, clear);
    const registryStatus = element("p", {
      className: "novavei-service-status",
      attrs: { "aria-live": "polite" },
    });
    const resultList = element("div", {
      className: "novavei-mcp-registry-results",
    });
    const detail = element("div", { className: "novavei-mcp-registry-detail" });
    root.append(heading, lead, form, registryStatus, resultList, detail);

    let servers: McpRegistryServer[] = [];
    let nextCursor: string | undefined;
    let selected: McpRegistryServer | undefined;
    let loading = false;
    let detailLoading = false;
    let searched = false;
    let listError = "";

    const renderInputDescriptors = (
      kind: string,
      inputs: McpRegistryInput[],
    ) => {
      if (!inputs.length) return undefined;
      const section = element("div", {
        className: "novavei-mcp-registry-inputs",
      });
      section.append(element("strong", { text: kind }));
      const list = element("ul", { className: "novavei-mcp-registry-inputs" });
      for (const input of inputs.slice(0, MAX_RENDERED_ITEMS)) {
        const item = element("li", {
          text: [
            input.name,
            input.required
              ? text("必填", "required")
              : text("可选", "optional"),
            input.secret
              ? text("值由你在本机填写", "value entered locally")
              : "",
          ]
            .filter(Boolean)
            .join(" · "),
        });
        if (input.description && !input.secret)
          item.append(
            document.createTextNode(` — ${truncate(input.description, 180)}`),
          );
        list.append(item);
      }
      section.append(list);
      return section;
    };

    const safeDraft = (draft: McpRegistryRemoteDraft) =>
      !draft.enabled &&
      !draft.allowRemote &&
      isPiSafeRegistryId(draft.id) &&
      isConcreteHttpsRegistryRemote(draft.url) &&
      ["http", "sse"].includes(draft.transport);

    const createDraft = async (
      server: McpRegistryServer,
      remoteIndex: number,
      control: HTMLButtonElement,
    ) => {
      setBusy(control, true, text("创建草稿…", "Creating draft…"));
      status(
        registryStatus,
        text(
          "正在由 native 复核当前官方记录并创建禁用草稿…",
          "Native is rechecking the current official record and creating a disabled draft…",
        ),
      );
      try {
        const draft = await invoke<McpRegistryRemoteDraft>(
          "mcp_registry_remote_draft",
          { name: server.name, remoteIndex },
        );
        if (!safeDraft(draft))
          throw new Error(
            "Registry draft did not satisfy the disabled local-only contract",
          );
        editor.openRegistryDraft(draft);
        status(
          registryStatus,
          text(
            "禁用草稿已打开；尚未写入设置、连接或执行 MCP。请复核后明确确认保存。",
            "A disabled draft is open. Nothing has been saved, connected, or executed; review it and explicitly confirm saving.",
          ),
          "success",
        );
      } catch (error) {
        status(
          registryStatus,
          `${text("无法创建 Registry 草稿：", "Unable to create Registry draft: ")}${errorText(error)}`,
          "error",
        );
      } finally {
        setBusy(control, false);
      }
    };

    const renderPackages = (packages: McpRegistryPackage[]) => {
      const packagesSection = element("details", {
        className: "novavei-service-details",
      });
      packagesSection.append(
        element("summary", {
          text: text(
            `包元数据（${packages.length}，仅参考）`,
            `Package metadata (${packages.length}, reference only)`,
          ),
        }),
      );
      const list = element("div", {
        className: "novavei-mcp-registry-results",
      });
      if (!packages.length) {
        list.append(
          element("p", {
            className: "novavei-service-note",
            text: text(
              "此 Server 未提供包元数据。",
              "This server has no package metadata.",
            ),
          }),
        );
      }
      for (const pkg of packages.slice(0, MAX_RENDERED_ITEMS)) {
        const item = element("div", {
          className: "novavei-mcp-registry-package",
        });
        const version = pkg.version ? `@${pkg.version}` : "";
        item.append(
          element("strong", {
            text: `${pkg.registryType}: ${pkg.identifier}${version}`,
          }),
          element("span", {
            className: "pill wait",
            text: text(
              "仅参考，不可导入或执行",
              "Reference only; cannot import or execute",
            ),
          }),
          element("small", {
            text:
              pkg.incompatibilityReason ||
              text(
                "包元数据不会转为本机命令。",
                "Package metadata is never converted into a local command.",
              ),
          }),
        );
        list.append(item);
      }
      packagesSection.append(list);
      return packagesSection;
    };

    const renderDetail = () => {
      detail.replaceChildren();
      if (detailLoading) {
        detail.append(
          emptyCard(
            text("正在读取当前版本详情", "Loading current version details"),
            text(
              "详情只由 native 官方 Registry 桥接返回。",
              "Details are returned only by the native official Registry bridge.",
            ),
          ),
        );
        return;
      }
      if (!selected) return;
      const current = selected;
      const title = current.title || current.name;
      const head = element("div", { className: "novavei-service-row" });
      const copy = element("div");
      copy.append(
        element("strong", { text: title }),
        element("small", { text: `${current.name} · ${current.version}` }),
      );
      head.append(
        copy,
        element("span", {
          className: "pill",
          text: text("当前 active", "Current active"),
        }),
      );
      detail.append(head);
      if (current.description)
        detail.append(
          element("p", {
            className: "novavei-service-note",
            text: truncate(current.description, 600),
          }),
        );

      const remoteHeading = element("h4", {
        text: text(
          `远程端点（${current.remotes.length}）`,
          `Remote endpoints (${current.remotes.length})`,
        ),
      });
      detail.append(remoteHeading);
      if (!current.remotes.length) {
        detail.append(
          emptyCard(
            text("没有可用远程端点", "No remote endpoints"),
            text(
              "仅 HTTPS 的 concrete Streamable HTTP 或 SSE 端点才可创建草稿。",
              "Only concrete HTTPS Streamable HTTP or SSE endpoints can create a draft.",
            ),
          ),
        );
      }
      current.remotes.slice(0, MAX_RENDERED_ITEMS).forEach((remote, index) => {
        const card = element("article", {
          className: "novavei-mcp-registry-item",
        });
        const canCreateDraft =
          remote.importable &&
          isConcreteHttpsRegistryRemote(remote.url) &&
          ["streamable-http", "sse"].includes(
            remote.transport.toLocaleLowerCase(),
          );
        const copy = element("div");
        copy.append(
          element("strong", { text: remote.transport }),
          element("small", { text: remote.url }),
          element("small", {
            text: canCreateDraft
              ? text(
                  "会创建禁用、禁止远程 URL 的本机草稿。",
                  "Creates a disabled local-only draft with remote URLs denied.",
                )
              : remote.queryRedacted
                ? text(
                    "端点 query 已由 native 移除，因此不能导入或复制。",
                    "Native removed the endpoint query, so it cannot be imported or copied.",
                  )
                : remote.incompatibilityReason ||
                  text(
                    "此端点不能创建草稿。",
                    "This endpoint cannot create a draft.",
                  ),
          }),
        );
        const create = button(
          text("创建禁用草稿", "Create disabled draft"),
          "btn primary",
          text(
            `为 ${current.name} 的 ${remote.transport} 端点创建禁用草稿`,
            `Create a disabled draft for ${current.name} ${remote.transport} endpoint`,
          ),
        );
        create.disabled = !canCreateDraft;
        if (canCreateDraft)
          create.addEventListener(
            "click",
            () => void createDraft(current, index, create),
          );
        const actions = element("div", {
          className: "novavei-service-actions",
        });
        actions.append(create);
        card.append(copy);
        const headerInputs = renderInputDescriptors(
          text("请求头要求", "Header requirements"),
          remote.headers,
        );
        const variableInputs = renderInputDescriptors(
          text("变量要求", "Variable requirements"),
          remote.variables,
        );
        if (headerInputs) card.append(headerInputs);
        if (variableInputs) card.append(variableInputs);
        card.append(actions);
        detail.append(card);
      });
      detail.append(renderPackages(current.packages));
    };

    const render = () => {
      resultList.replaceChildren();
      if (loading && !servers.length) {
        resultList.append(
          emptyCard(
            text("正在搜索官方 Registry", "Searching official Registry"),
            text(
              "请稍候；未使用浏览器直连。",
              "Please wait; no browser-side request is used.",
            ),
          ),
        );
      } else if (listError) {
        resultList.append(
          emptyCard(
            text("Registry 暂不可用", "Registry temporarily unavailable"),
            text(
              "检查网络后重试搜索；不会伪造任何发现结果。",
              "Check the network and retry; no discovery results are fabricated.",
            ),
          ),
        );
      } else if (searched && !servers.length) {
        resultList.append(
          emptyCard(
            text("没有匹配的 MCP Server", "No matching MCP servers"),
            text(
              "尝试更短的名称、移除筛选词，或稍后再试。",
              "Try a shorter name, remove filters, or retry later.",
            ),
          ),
        );
      } else if (!searched) {
        resultList.append(
          element("p", {
            className: "novavei-service-note",
            text: text(
              "输入关键词后搜索。只展示官方标记为 active 且 latest 的记录。",
              "Enter a keyword to search. Only records marked active and latest by the official Registry are shown.",
            ),
          }),
        );
      } else {
        for (const server of servers.slice(0, MAX_RENDERED_ITEMS)) {
          const card = element("article", {
            className: "novavei-mcp-registry-item",
          });
          const copy = element("div");
          copy.append(
            element("strong", { text: server.title || server.name }),
            element("small", { text: `${server.name} · ${server.version}` }),
            element("small", {
              text: truncate(
                server.description ||
                  text("未提供描述。", "No description provided."),
                300,
              ),
            }),
          );
          const inspect = button(
            text("查看详情", "View details"),
            "btn",
            text(
              `查看 ${server.name} 的官方 Registry 详情`,
              `View official Registry details for ${server.name}`,
            ),
          );
          inspect.addEventListener(
            "click",
            () => void loadDetail(server, inspect),
          );
          const row = element("div", { className: "novavei-service-row" });
          row.append(copy, inspect);
          card.append(row);
          resultList.append(card);
        }
        if (nextCursor) {
          const more = button(
            text("加载更多", "Load more"),
            "btn",
            text(
              "加载更多官方 Registry 结果",
              "Load more official Registry results",
            ),
          );
          more.addEventListener("click", () => void load(nextCursor, more));
          resultList.append(more);
        }
      }
      renderDetail();
    };

    const loadDetail = async (
      summary: McpRegistryServer,
      control: HTMLButtonElement,
    ) => {
      detailLoading = true;
      selected = undefined;
      setBusy(control, true, text("读取中…", "Loading…"));
      status(
        registryStatus,
        text(
          "正在由 native 读取当前 active/latest 版本详情…",
          "Native is loading the current active/latest version details…",
        ),
      );
      render();
      try {
        selected = await invoke<McpRegistryServer>("mcp_registry_get", {
          name: summary.name,
          version: "latest",
        });
        status(
          registryStatus,
          text(
            "已读取当前官方记录。包元数据仅供参考。",
            "Current official record loaded. Package metadata is reference-only.",
          ),
          "success",
        );
      } catch (error) {
        status(
          registryStatus,
          `${text("无法读取 Registry 详情：", "Unable to load Registry details: ")}${errorText(error)}`,
          "error",
        );
      } finally {
        detailLoading = false;
        setBusy(control, false);
        render();
      }
    };

    const load = async (cursor?: string, control?: HTMLButtonElement) => {
      const query = queryInput.value.trim();
      loading = true;
      listError = "";
      if (!cursor) {
        servers = [];
        nextCursor = undefined;
        selected = undefined;
        searched = true;
      }
      setBusy(search, true, text("搜索中…", "Searching…"));
      if (control) setBusy(control, true, text("加载中…", "Loading…"));
      status(
        registryStatus,
        text(
          "正在通过 native 查询官方 Registry…",
          "Querying the official Registry through native…",
        ),
      );
      render();
      try {
        const response = await invoke<McpRegistryListResponse>(
          "mcp_registry_list",
          {
            search: query || null,
            cursor: cursor || null,
            limit: 24,
          },
        );
        const known = new Set(
          servers.map((server) => `${server.name}@${server.version}`),
        );
        for (const server of response.servers) {
          const key = `${server.name}@${server.version}`;
          if (!known.has(key)) {
            known.add(key);
            servers.push(server);
          }
        }
        nextCursor = response.nextCursor || undefined;
        status(
          registryStatus,
          text(
            `已读取 ${servers.length} 个当前官方记录。`,
            `${servers.length} current official records loaded.`,
          ),
          "success",
        );
      } catch (error) {
        listError = errorText(error);
        status(
          registryStatus,
          `${text("Registry 搜索失败：", "Registry search failed: ")}${listError}`,
          "error",
        );
      } finally {
        loading = false;
        setBusy(search, false);
        if (control) setBusy(control, false);
        render();
      }
    };

    const focusAndSearch = () => {
      const behavior: ScrollBehavior = window.matchMedia(
        "(prefers-reduced-motion: reduce)",
      ).matches
        ? "auto"
        : "smooth";
      root.scrollIntoView({ block: "start", behavior });
      queryInput.focus({ preventScroll: true });
      if (!searched && !loading) void load();
    };

    form.addEventListener("submit", (event) => {
      event.preventDefault();
      void load();
    });
    clear.addEventListener("click", () => {
      queryInput.value = "";
      servers = [];
      nextCursor = undefined;
      selected = undefined;
      searched = false;
      listError = "";
      status(
        registryStatus,
        text("已清除 Registry 结果。", "Registry results cleared."),
      );
      render();
      queryInput.focus({ preventScroll: true });
    });
    return { root, render, focusAndSearch };
  })();

  const renderTools = (tools: McpToolInfo[]) => {
    const details = element("details", {
      className: "novavei-service-details",
    });
    details.open = true;
    details.append(
      element("summary", {
        text: text(`工具（${tools.length}）`, `Tools (${tools.length})`),
      }),
    );
    const list = element("ul", { className: "novavei-service-list" });
    for (const tool of tools.slice(0, MAX_RENDERED_ITEMS)) {
      const item = element("li", { className: "novavei-service-note" });
      item.append(element("strong", { text: tool.name }));
      if (tool.description)
        item.append(
          document.createTextNode(` — ${truncate(tool.description, 260)}`),
        );
      list.append(item);
    }
    details.append(list);
    return details;
  };

  const render = () => {
    grid.replaceChildren();
    const hub = element("section", {
      className: "novavei-service-hub novavei-mcp-hub",
      attrs: { "aria-label": text("MCP 管理", "MCP management") },
    });
    const content = element("div", { className: "novavei-mcp-content" });
    hub.append(overlayStatus, content);
    grid.append(hub);
    if (state.error) {
      content.append(
        emptyCard(
          text("MCP 运行时不可用", "MCP runtime unavailable"),
          state.error,
        ),
      );
    } else if (!state.servers.length) {
      content.append(
        emptyCard(
          text("尚未配置 MCP Server", "No MCP servers configured"),
          text(
            "配置保存在本机设置中；连接密钥不会显示在此界面。",
            "Configuration stays in native settings; credentials are never shown here.",
          ),
        ),
      );
    } else {
      if (!state.servers.some((server) => server.id === selectedServerId))
        selectedServerId = state.servers[0]?.id;
      const workbench = element("section", {
        className: "novavei-mcp-workbench",
        attrs: {
          "aria-label": text("MCP Server 工作区", "MCP server workbench"),
        },
      });
      const serverPanel = element("aside", {
        className: "novavei-mcp-server-panel",
        attrs: { "aria-label": text("MCP Server 列表", "MCP server list") },
      });
      const serverPanelHeader = element("header");
      serverPanelHeader.append(
        element("h3", {
          className: "novavei-service-panel-heading",
          text: text("已配置的 Server", "Configured servers"),
        }),
        element("small", {
          text: text(
            "选择一个 Server 查看工具、状态与运行操作。",
            "Select a server to view tools, status, and runtime actions.",
          ),
        }),
      );
      const serverList = element("div", {
        className: "novavei-mcp-server-list",
      });
      const detailPanel = element("section", {
        className: "novavei-mcp-detail-panel",
        attrs: { "aria-live": "polite" },
      });
      serverPanel.append(serverPanelHeader, serverList);
      workbench.append(serverPanel, detailPanel);
      content.append(workbench);
      for (const server of state.servers.slice(0, MAX_RENDERED_ITEMS)) {
        const runtime = state.statuses.get(server.id);
        const isSelected = server.id === selectedServerId;
        const listItem = button(
          server.label,
          `novavei-mcp-list-item${isSelected ? " is-selected" : ""}`,
          text(
            `查看 ${server.label} 的 Server 详情`,
            `View ${server.label} server details`,
          ),
        );
        listItem.setAttribute("aria-current", isSelected ? "true" : "false");
        const listStatus = element("span", {
          className: `novavei-mcp-list-status${
            runtime?.lastError
              ? " is-error"
              : runtime?.running
                ? runtime.initialized
                  ? " is-ready"
                  : " is-starting"
                : ""
          }`,
          attrs: { "aria-hidden": "true" },
        });
        const listCopy = element("span", {
          className: "novavei-mcp-list-copy",
        });
        const listState = runtime?.running
          ? runtime.initialized
            ? text("已连接", "Connected")
            : text("启动中", "Starting")
          : server.enabled
            ? text("未启动", "Stopped")
            : text("已停用", "Disabled");
        listCopy.append(
          element("strong", { text: server.label }),
          element("small", { text: `${server.transport} · ${listState}` }),
        );
        listItem.append(listStatus, listCopy);
        listItem.addEventListener("click", () => {
          selectedServerId = server.id;
          render();
          grid
            .querySelector<HTMLButtonElement>(
              ".novavei-mcp-list-item.is-selected",
            )
            ?.focus({ preventScroll: true });
        });
        serverList.append(listItem);

        const card = element("article", {
          className: "hub-card novavei-mcp-server-detail-card",
        });
        const copy = element("div");
        copy.append(
          element("strong", { text: server.label }),
          element("small", {
            text: `${server.transport} · ${server.enabled ? text("已配置", "Configured") : text("已停用", "Disabled")}`,
          }),
        );
        if (runtime?.lastError)
          copy.append(
            element("small", { text: truncate(runtime.lastError, 220) }),
          );
        const stateLabel = runtime?.running
          ? runtime.initialized
            ? text("已连接", "Connected")
            : text("启动中", "Starting")
          : server.enabled
            ? text("未启动", "Stopped")
            : text("已停用", "Disabled");
        const badge = element("span", {
          className: runtime?.lastError
            ? "pill warn"
            : runtime?.running
              ? "pill"
              : "pill wait",
          text: stateLabel,
        });
        const row = element("div", { className: "novavei-service-row" });
        row.append(copy, badge);
        const rowActions = element("div", {
          className: "novavei-service-actions",
        });
        const edit = button(
          text("编辑", "Edit"),
          "btn",
          text(
            `编辑 ${server.label} 的 MCP 配置`,
            `Edit ${server.label} MCP configuration`,
          ),
        );
        const inspect = button(
          text("工具", "Tools"),
          "btn",
          text(
            `读取 ${server.label} 的工具`,
            `Read tools from ${server.label}`,
          ),
        );
        const test = button(
          text("测试", "Test"),
          "btn primary",
          text(`测试 ${server.label}`, `Test ${server.label}`),
        );
        const restart = button(
          text("重启", "Restart"),
          "btn",
          text(`重启 ${server.label}`, `Restart ${server.label}`),
        );
        const stop = button(
          text("停止", "Stop"),
          "btn",
          text(`停止 ${server.label}`, `Stop ${server.label}`),
        );
        edit.addEventListener("click", () => {
          const record = state.records.get(server.id);
          if (record) editor.open({ id: server.id, record });
          else
            status(
              overlayStatus,
              text(
                "无法读取该 Server 的本机配置。",
                "Unable to read this server's local configuration.",
              ),
              "error",
            );
        });
        if (!server.enabled) {
          inspect.disabled = true;
          test.disabled = true;
          restart.disabled = true;
          stop.disabled = true;
        }
        inspect.addEventListener("click", () => {
          void (async () => {
            setBusy(inspect, true, text("读取中…", "Loading…"));
            try {
              state.tools.set(
                server.id,
                await invoke<McpToolInfo[]>("mcp_list_tools", {
                  serverId: server.id,
                }),
              );
              state.statuses.set(
                server.id,
                await invoke<McpRuntimeStatus>("mcp_runtime_status", {
                  serverId: server.id,
                }),
              );
              render();
            } catch (error) {
              status(
                overlayStatus,
                `${text("无法读取 MCP 工具：", "Unable to read MCP tools: ")}${errorText(error)}`,
                "error",
              );
            } finally {
              setBusy(inspect, false);
            }
          })();
        });
        test.addEventListener("click", () => {
          void (async () => {
            setBusy(test, true, text("测试中…", "Testing…"));
            try {
              const result = await invoke<McpRuntimeTestResponse>(
                "mcp_test_server",
                { serverId: server.id },
              );
              state.tests.set(server.id, result);
              state.statuses.set(server.id, {
                serverId: result.serverId,
                running: result.running,
                initialized: result.initialized,
                transport: result.transport,
                lastError: result.error,
              });
              if (result.tools) state.tools.set(server.id, result.tools);
              status(
                overlayStatus,
                result.ok
                  ? text(
                      `${server.label} 测试通过。`,
                      `${server.label} test passed.`,
                    )
                  : text(
                      `${server.label} 测试失败：${result.error || result.phase}`,
                      `${server.label} test failed: ${result.error || result.phase}`,
                    ),
                result.ok ? "success" : "error",
              );
              render();
            } catch (error) {
              status(
                overlayStatus,
                `${text("MCP 测试失败：", "MCP test failed: ")}${errorText(error)}`,
                "error",
              );
            } finally {
              setBusy(test, false);
            }
          })();
        });
        restart.addEventListener("click", () => {
          void (async () => {
            setBusy(restart, true, text("重启中…", "Restarting…"));
            try {
              state.statuses.set(
                server.id,
                await invoke<McpRuntimeStatus>("mcp_restart_server", {
                  serverId: server.id,
                }),
              );
              status(
                overlayStatus,
                text(`${server.label} 已重启。`, `${server.label} restarted.`),
                "success",
              );
              render();
            } catch (error) {
              status(
                overlayStatus,
                `${text("MCP 重启失败：", "MCP restart failed: ")}${errorText(error)}`,
                "error",
              );
            } finally {
              setBusy(restart, false);
            }
          })();
        });
        stop.addEventListener("click", () => {
          void (async () => {
            setBusy(stop, true, text("停止中…", "Stopping…"));
            try {
              await invoke("mcp_stop_server", { serverId: server.id });
              state.statuses.set(
                server.id,
                await invoke<McpRuntimeStatus>("mcp_runtime_status", {
                  serverId: server.id,
                }),
              );
              status(
                overlayStatus,
                text(`${server.label} 已停止。`, `${server.label} stopped.`),
                "success",
              );
              render();
            } catch (error) {
              status(
                overlayStatus,
                `${text("MCP 停止失败：", "MCP stop failed: ")}${errorText(error)}`,
                "error",
              );
            } finally {
              setBusy(stop, false);
            }
          })();
        });
        rowActions.append(edit, inspect, test, restart, stop);
        card.append(row, rowActions);
        const result = state.tests.get(server.id);
        if (result) {
          card.append(
            element("p", {
              className: "novavei-service-note",
              text: result.ok
                ? text(
                    `最近测试：${result.phase} · ${result.toolsCount} 个工具 · ${result.durationMs} ms`,
                    `Last test: ${result.phase} · ${result.toolsCount} tools · ${result.durationMs} ms`,
                  )
                : text(
                    `最近测试失败：${result.phase}`,
                    `Last test failed: ${result.phase}`,
                  ),
            }),
          );
        }
        const tools = state.tools.get(server.id);
        if (tools) card.append(renderTools(tools));
        card.append(
          element("p", {
            className: "novavei-service-note",
            text: text(
              "工具调用将在 Pi 的权限流程中执行。",
              "Tool calls run through Pi's permission flow.",
            ),
          }),
        );
        if (isSelected) detailPanel.append(card);
      }
    }
    registry.render();
    content.append(registry.root);
  };

  const load = async () => {
    setBusy(refresh, true, text("刷新中…", "Refreshing…"));
    state.error = undefined;
    status(
      overlayStatus,
      text("正在读取本机 MCP 配置…", "Reading native MCP configuration…"),
    );
    render();
    try {
      const settings = await invoke<UnknownRecord>("settings_load_all");
      state.rawSettings = settings.mcp ?? [];
      const records = mcpServerRecords(state.rawSettings);
      state.records = new Map(records.map(({ id, record }) => [id, record]));
      state.servers = mcpServers(state.rawSettings);
      state.statuses.clear();
      await Promise.all(
        state.servers.map(async (server) => {
          try {
            state.statuses.set(
              server.id,
              await invoke<McpRuntimeStatus>("mcp_runtime_status", {
                serverId: server.id,
              }),
            );
          } catch (error) {
            state.statuses.set(server.id, {
              serverId: server.id,
              running: false,
              initialized: false,
              transport: server.transport,
              lastError: errorText(error),
            });
          }
        }),
      );
      status(
        overlayStatus,
        text(
          `已读取 ${state.servers.length} 个 MCP Server。`,
          `${state.servers.length} MCP servers loaded.`,
        ),
        "success",
      );
      status(
        settingsHint,
        text(
          `已读取 ${state.servers.length} 个本机 MCP 配置；凭据始终留在 native 进程。`,
          `${state.servers.length} native MCP configurations loaded; credentials remain in the native process.`,
        ),
        "success",
      );
    } catch (error) {
      state.error = errorText(error);
      status(
        overlayStatus,
        `${text("MCP 配置不可用：", "MCP configuration unavailable: ")}${state.error}`,
        "error",
      );
      status(
        settingsHint,
        `${text("MCP 配置不可用：", "MCP configuration unavailable: ")}${state.error}`,
        "error",
      );
    } finally {
      setBusy(refresh, false);
      render();
    }
  };

  create.addEventListener("click", () => editor.open());
  registryButton.addEventListener("click", () => registry.focusAndSearch());
  refresh.addEventListener("click", () => void load());
  document
    .getElementById("openMcpFromSettings")
    ?.addEventListener("click", () => void load());
  document.addEventListener("click", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest('[data-tools-tab="mcp"]')) void load();
  });
  const syncMcpChrome = () => {
    labelControl(
      create,
      text("新建 Server", "New server"),
      text("新建 MCP Server", "Create MCP server"),
    );
    labelControl(
      registryButton,
      text("官方 Registry", "Official Registry"),
      text("浏览官方 MCP Registry", "Browse the official MCP Registry"),
    );
    labelControl(
      refresh,
      text("刷新", "Refresh"),
      text("刷新 MCP Server", "Refresh MCP servers"),
    );
  };
  onServiceLanguageChange(() => {
    syncMcpChrome();
    void load();
  });
  syncMcpChrome();
  status(
    settingsHint,
    text("正在连接本机 MCP 配置…", "Connecting to native MCP configuration…"),
  );
  void load();
}

function installMemory() {
  const invoke = invokeApi();
  const panel = document.querySelector<HTMLElement>(
    '.settings-panel[data-settings="memory"]',
  );
  if (!invoke || !panel) return;

  type ScopeUi = {
    scope: MemoryScope;
    root: HTMLInputElement;
    query: HTMLInputElement;
    list: HTMLElement;
    message: HTMLElement;
    total: HTMLElement;
    storage: HTMLElement;
    searchButton: HTMLButtonElement;
    refreshButton: HTMLButtonElement;
    createButton: HTMLButtonElement;
    organizeButton: HTMLButtonElement;
    exportButton: HTMLButtonElement;
    exportFormat: HTMLSelectElement;
  };

  const state: {
    entries: Map<MemoryScope, MemoryEntry[]>;
    stats: Map<MemoryScope, MemoryStats>;
    usageStats?: MemoryStats;
  } = { entries: new Map(), stats: new Map() };
  const uis = new Map<MemoryScope, ScopeUi>();
  const projectPanel = panel.querySelector<HTMLElement>(
    '[data-memory-panel="project"]',
  );
  const globalPanel = panel.querySelector<HTMLElement>(
    '[data-memory-panel="longterm"]',
  );
  const usagePanel = panel.querySelector<HTMLElement>(
    '[data-memory-panel="usage"]',
  );
  const knowledgePanel = panel.querySelector<HTMLElement>(
    '[data-memory-panel="knowledge"]',
  );
  if (!projectPanel || !globalPanel || !usagePanel || !knowledgePanel) return;

  const placeholderClear = globalPanel.querySelector<HTMLButtonElement>(
    '[data-i18n="settings.memory.clear"]',
  );
  const placeholderUsageReport = usagePanel.querySelector<HTMLButtonElement>(
    '[data-i18n="settings.memory.usageReport"]',
  );
  const enableMemoryControl = (
    control: HTMLButtonElement | null,
    label: string,
    ariaLabel: string,
  ) => {
    // Clone reused shell placeholders so language rebuild cannot stack click handlers.
    const enabled = control
      ? (control.cloneNode(true) as HTMLButtonElement)
      : button(label, "btn", ariaLabel);
    enabled.type = "button";
    enabled.textContent = label;
    enabled.setAttribute("aria-label", ariaLabel);
    enabled.removeAttribute("data-feature-unavailable");
    enabled.removeAttribute("aria-disabled");
    enabled.disabled = false;
    enabled.className = control?.className?.includes("primary")
      ? control.className
      : enabled.className || "btn";
    if (!enabled.className.trim()) enabled.className = "btn";
    return enabled;
  };

  const usageStatus = element("p", {
    className: "novavei-service-status",
    attrs: { "aria-live": "polite" },
  });
  const usageTotal = element("dd", { text: "—" });
  const usageProject = element("dd", { text: "—" });
  const usageGlobal = element("dd", { text: "—" });
  const usageBytes = element("dd", { text: "—" });
  const usageSearches = element("dd", { text: "—" });
  const usageWrites = element("dd", { text: "—" });
  const usageCapacity = element("p", { className: "tiny", text: "—" });
  const usageTracking = element("p", { className: "tiny", text: "—" });
  const usageMeter = element("div", {
    className: "meter",
    attrs: {
      role: "progressbar",
      "aria-label": text("记忆条目容量", "Memory entry capacity"),
      "aria-valuemin": "0",
      "aria-valuemax": "100",
      "aria-valuenow": "0",
    },
  });
  const usageMeterFill = element("i");
  usageMeterFill.style.width = "0%";
  usageMeter.append(usageMeterFill);

  const currentWorkdir = () => {
    const native = window.__novaveiHost?.getWorkdir?.();
    if (native?.trim()) return native.trim();
    const project = document.querySelector<HTMLElement>(
      '.project-row[aria-current="page"][data-workdir]',
    );
    if (project?.dataset.workdir?.trim()) return project.dataset.workdir.trim();
    // Files dock status is the bare absolute path when a project is open.
    const workdirStatus =
      node<HTMLElement>("workdirStatus")?.textContent?.trim() ?? "";
    if (!workdirStatus) return undefined;
    if (
      /^[A-Za-z]:[\\/]/.test(workdirStatus) ||
      workdirStatus.startsWith("\\\\") ||
      workdirStatus.startsWith("/")
    ) {
      return workdirStatus;
    }
    return workdirStatus.match(/workdir\s*=\s*(.+)$/i)?.[1]?.trim();
  };

  const filterFor = (scope: MemoryScope): MemoryFilter | undefined => {
    if (scope === "global") return { scope: "global" };
    const workdir = currentWorkdir();
    return workdir ? { scope: "project", workdir } : undefined;
  };

  const formatTime = (value: number | undefined) => {
    if (!Number.isFinite(value)) return text("尚未记录", "Not recorded");
    return new Intl.DateTimeFormat(isEnglish() ? "en-GB" : "zh-CN", {
      dateStyle: "short",
      timeStyle: "short",
    }).format(new Date(value as number));
  };

  const updateUsage = () => {
    const stats = state.usageStats;
    if (!stats) {
      for (const target of [
        usageTotal,
        usageProject,
        usageGlobal,
        usageBytes,
        usageSearches,
        usageWrites,
        usageCapacity,
        usageTracking,
      ]) {
        target.textContent = "—";
      }
      usageMeter.setAttribute("aria-valuenow", "0");
      usageMeterFill.style.width = "0%";
      return;
    }
    const projectCount =
      stats.byScope?.find((bucket) => bucket.key === "project")?.entries ?? 0;
    const globalCount =
      stats.byScope?.find((bucket) => bucket.key === "global")?.entries ?? 0;
    const percent = Math.max(
      0,
      Math.min(100, Number(stats.capacity?.usedPercent) || 0),
    );
    usageTotal.textContent = String(stats.totalEntries);
    usageProject.textContent = String(projectCount);
    usageGlobal.textContent = String(globalCount);
    usageBytes.textContent = bytes(stats.totalBytes);
    usageSearches.textContent = String(stats.weeklySearches);
    usageWrites.textContent = String(stats.weeklyWrites);
    usageCapacity.textContent = text(
      `已用 ${stats.capacity.usedEntries} / ${stats.capacity.maxEntries} 条 · 剩余 ${stats.capacity.remainingEntries} 条 · ${percent.toFixed(2)}%`,
      `${stats.capacity.usedEntries} / ${stats.capacity.maxEntries} entries used · ${stats.capacity.remainingEntries} remaining · ${percent.toFixed(2)}%`,
    );
    usageTracking.textContent = text(
      `本周起点 ${formatTime(stats.weekStartedAt)} · 计数始于 ${formatTime(stats.trackingStartedAt)}`,
      `Week starts ${formatTime(stats.weekStartedAt)} · tracking since ${formatTime(stats.trackingStartedAt)}`,
    );
    usageMeter.setAttribute("aria-valuenow", percent.toFixed(2));
    usageMeterFill.style.width = `${percent}%`;
  };

  const usageFilter = (): MemoryFilter => {
    const workdir = currentWorkdir();
    return workdir ? { workdir } : { scope: "global" };
  };

  const refreshUsage = async (
    trigger?: HTMLButtonElement,
  ): Promise<boolean> => {
    if (trigger) setBusy(trigger, true, text("刷新中…", "Refreshing…"));
    status(
      usageStatus,
      text("正在读取真实记忆统计…", "Loading native memory statistics…"),
    );
    try {
      state.usageStats = await invoke<MemoryStats>("memory_stats", {
        filter: usageFilter(),
      });
      updateUsage();
      status(
        usageStatus,
        text("记忆统计已刷新。", "Memory statistics refreshed."),
        "success",
      );
      return true;
    } catch (error) {
      state.usageStats = undefined;
      updateUsage();
      status(
        usageStatus,
        `${text("无法读取记忆统计：", "Unable to load memory statistics: ")}${errorText(error)}`,
        "error",
      );
      return false;
    } finally {
      if (trigger) setBusy(trigger, false);
    }
  };

  const renderEntries = (scope: MemoryScope) => {
    const ui = uis.get(scope);
    if (!ui) return;
    const entries = state.entries.get(scope) ?? [];
    ui.list.replaceChildren();
    if (!entries.length) {
      ui.list.append(
        emptyCard(
          text("暂无记忆条目", "No memory entries"),
          scope === "project"
            ? text(
                "可新建一条与当前项目绑定的记忆。",
                "Create an entry tied to the current project.",
              )
            : text("可新建一条长期记忆。", "Create a long-term memory entry."),
        ),
      );
      return;
    }
    for (const entry of entries.slice(0, MAX_RENDERED_ITEMS)) {
      const card = element("article", { className: "hub-card" });
      const copy = element("div");
      copy.append(
        element("strong", { text: entry.title }),
        element("small", {
          text: `${entry.type} · ${text("更新于", "Updated")} ${formatTime(entry.updatedAt)}`,
        }),
      );
      const badge = element("span", {
        className: "pill wait",
        text:
          entry.scope === "project"
            ? text("项目", "Project")
            : text("长期", "Long-term"),
      });
      const row = element("div", { className: "novavei-service-row" });
      row.append(copy, badge);
      const actions = element("div", { className: "novavei-service-actions" });
      const view = button(
        text("查看", "View"),
        "btn",
        text(`查看 ${entry.title}`, `View ${entry.title}`),
      );
      const edit = button(
        text("编辑", "Edit"),
        "btn",
        text(`编辑 ${entry.title}`, `Edit ${entry.title}`),
      );
      const remove = button(
        text("删除", "Delete"),
        "btn",
        text(`删除 ${entry.title}`, `Delete ${entry.title}`),
      );
      view.addEventListener("click", () => void openEntry(entry, card));
      edit.addEventListener("click", () => void editEntry(entry));
      remove.addEventListener("click", () => void deleteEntry(entry));
      actions.append(view, edit, remove);
      card.append(row, actions);
      ui.list.append(card);
    }
  };

  const refreshScope = async (
    scope: MemoryScope,
    trigger?: HTMLButtonElement,
  ): Promise<boolean> => {
    const ui = uis.get(scope);
    if (!ui) return false;
    const filter = filterFor(scope);
    ui.root.value =
      scope === "project"
        ? (filter?.workdir ??
          text("请先选择工作区", "Select a workspace first"))
        : text("本机全局记忆", "Native global memory");
    if (!filter) {
      state.entries.set(scope, []);
      state.stats.delete(scope);
      ui.total.textContent = "—";
      ui.storage.textContent = "—";
      status(
        ui.message,
        text(
          "项目记忆需要先选择一个可访问的工作区。",
          "Project memory requires an accessible selected workspace.",
        ),
        "error",
      );
      renderEntries(scope);
      updateUsage();
      return false;
    }
    setBusy(trigger ?? ui.refreshButton, true, text("读取中…", "Loading…"));
    status(ui.message, text("正在读取本机记忆…", "Loading native memory…"));
    try {
      const query = ui.query.value.trim();
      const response = query
        ? await invoke<MemorySearchResponse>("memory_search", {
            query,
            filter,
            limit: MAX_RENDERED_ITEMS,
          })
        : await invoke<MemoryListResponse>("memory_list", {
            filter,
            limit: MAX_RENDERED_ITEMS,
            offset: 0,
          });
      const stats = await invoke<MemoryStats>("memory_stats", { filter });
      const items = response.items;
      const total = "total" in response ? response.total : items.length;
      state.entries.set(scope, items);
      state.stats.set(scope, stats);
      ui.total.textContent = String(stats.totalEntries);
      ui.storage.textContent = bytes(stats.totalBytes);
      status(
        ui.message,
        query
          ? text(
              `已找到 ${items.length} 条匹配记忆。`,
              `${items.length} matching memories found.`,
            )
          : text(`已读取 ${total} 条记忆。`, `${total} memories loaded.`),
        "success",
      );
      renderEntries(scope);
      updateUsage();
      return true;
    } catch (error) {
      state.entries.set(scope, []);
      ui.total.textContent = "—";
      ui.storage.textContent = "—";
      status(
        ui.message,
        `${text("记忆服务不可用：", "Memory service unavailable: ")}${errorText(error)}`,
        "error",
      );
      renderEntries(scope);
      updateUsage();
      return false;
    } finally {
      setBusy(trigger ?? ui.refreshButton, false);
    }
  };

  const openEntry = async (entry: MemoryEntry, card: HTMLElement) => {
    const details = card.querySelector<HTMLDetailsElement>(
      "details.novavei-service-details",
    );
    if (details) {
      details.open = !details.open;
      return;
    }
    const filter = filterFor(entry.scope);
    try {
      const latest = await invoke<MemoryEntry>("memory_read", {
        id: entry.id,
        workdir: filter?.workdir,
      });
      const preview = element("details", {
        className: "novavei-service-details",
      });
      preview.open = true;
      preview.append(
        element("summary", { text: text("记忆内容", "Memory content") }),
      );
      preview.append(
        element("pre", {
          className: "novavei-service-preview",
          text: truncate(latest.content),
        }),
      );
      card.append(preview);
    } catch (error) {
      const ui = uis.get(entry.scope);
      status(
        ui?.message ?? null,
        `${text("无法读取记忆：", "Unable to read memory: ")}${errorText(error)}`,
        "error",
      );
    }
  };

  const editor = (() => {
    const dialog = element("dialog", {
      className: "novavei-service-dialog",
      attrs: { "aria-modal": "true" },
    });
    const body = element("form", { className: "novavei-service-dialog-body" });
    body.noValidate = true;
    const heading = element("h2", { id: "novaveiMemoryEditorTitle" });
    dialog.setAttribute("aria-labelledby", heading.id);
    const titleField = element("div", { className: "field" });
    const titleLabel = element("label", { text: text("标题", "Title") });
    const titleInput = element("input", {
      id: "novaveiMemoryTitle",
      attrs: { maxlength: "200", required: "" },
    });
    titleInput.type = "text";
    titleLabel.htmlFor = titleInput.id;
    titleField.append(titleLabel, titleInput);
    const typeField = element("div", { className: "field" });
    const typeLabel = element("label", { text: text("类型", "Type") });
    const kind = element("select", { id: "novaveiMemoryType" });
    typeLabel.htmlFor = kind.id;
    for (const [value, zh, en] of [
      ["user", "用户偏好", "User"],
      ["feedback", "反馈", "Feedback"],
      ["project", "项目", "Project"],
      ["reference", "参考", "Reference"],
      ["daily", "每日", "Daily"],
    ] as const) {
      const option = element("option", { text: text(zh, en) });
      option.value = value;
      kind.append(option);
    }
    typeField.append(typeLabel, kind);
    const contentField = element("div", { className: "field" });
    const contentLabel = element("label", { text: text("内容", "Content") });
    const content = element("textarea", {
      id: "novaveiMemoryContent",
      attrs: { maxlength: "65536" },
    });
    contentLabel.htmlFor = content.id;
    contentField.append(contentLabel, content);
    const alert = element("p", {
      attrs: { role: "alert", "aria-live": "assertive" },
    });
    const actions = element("div", { className: "row-actions" });
    const cancel = button(
      text("取消", "Cancel"),
      "btn",
      text("取消记忆编辑", "Cancel memory editing"),
    );
    const save = button(
      text("保存", "Save"),
      "btn primary",
      text("保存记忆", "Save memory"),
    );
    actions.append(cancel, save);
    body.append(heading, titleField, typeField, contentField, alert, actions);
    dialog.append(body);
    document.body.append(dialog);

    let scope: MemoryScope = "global";
    let editing: MemoryEntry | undefined;
    let previousFocus: HTMLElement | null = null;
    const open = (nextScope: MemoryScope, entry?: MemoryEntry) => {
      scope = nextScope;
      editing = entry;
      previousFocus =
        document.activeElement instanceof HTMLElement
          ? document.activeElement
          : null;
      heading.textContent = entry
        ? text("编辑记忆", "Edit memory")
        : text("新建记忆", "New memory");
      titleInput.value = entry?.title ?? "";
      content.value = entry?.content ?? "";
      kind.value = entry?.type ?? (scope === "project" ? "project" : "user");
      const daily = kind.querySelector<HTMLOptionElement>(
        'option[value="daily"]',
      );
      if (daily) daily.disabled = scope === "project";
      if (scope === "project" && kind.value === "daily") kind.value = "project";
      alert.textContent = "";
      if (!dialog.open) dialog.showModal();
      window.setTimeout(() => titleInput.focus(), 0);
    };
    cancel.addEventListener("click", () => dialog.close());
    body.addEventListener("submit", (event) => {
      event.preventDefault();
      save.click();
    });
    dialog.addEventListener("close", () =>
      previousFocus?.focus({ preventScroll: true }),
    );
    save.addEventListener("click", () => {
      void (async () => {
        const title = titleInput.value.trim();
        if (!title) {
          alert.textContent = text("请填写记忆标题。", "Enter a memory title.");
          titleInput.focus();
          return;
        }
        const filter = filterFor(scope);
        if (!filter) {
          alert.textContent = text(
            "请先选择一个可访问的工作区。",
            "Select an accessible workspace first.",
          );
          return;
        }
        setBusy(save, true, text("保存中…", "Saving…"));
        alert.textContent = "";
        try {
          if (editing) {
            await invoke("memory_update", {
              input: {
                id: editing.id,
                workdir: filter.workdir,
                type: kind.value,
                title,
                content: content.value,
              },
            });
          } else {
            await invoke("memory_create", {
              input: {
                scope,
                workdir: filter.workdir,
                type: kind.value,
                title,
                content: content.value,
              },
            });
          }
          dialog.close();
          toast(
            editing
              ? text("记忆已更新", "Memory updated")
              : text("记忆已保存", "Memory saved"),
          );
          await refreshScope(scope);
          await refreshUsage();
        } catch (error) {
          alert.textContent = `${text("无法保存：", "Unable to save: ")}${errorText(error)}`;
        } finally {
          setBusy(save, false);
        }
      })();
    });
    return { open };
  })();

  const editEntry = async (entry: MemoryEntry) => {
    const filter = filterFor(entry.scope);
    try {
      editor.open(
        entry.scope,
        await invoke<MemoryEntry>("memory_read", {
          id: entry.id,
          workdir: filter?.workdir,
        }),
      );
    } catch (error) {
      const ui = uis.get(entry.scope);
      status(
        ui?.message ?? null,
        `${text("无法读取记忆：", "Unable to read memory: ")}${errorText(error)}`,
        "error",
      );
    }
  };

  const deleteEntry = async (entry: MemoryEntry) => {
    const confirmation = text(
      `确定删除“${entry.title}”吗？此操作无法撤销。`,
      `Delete “${entry.title}”? This cannot be undone.`,
    );
    if (
      !(await requestAppConfirm({
        title: text("删除记忆", "Delete memory"),
        message: confirmation,
        confirmLabel: text("删除", "Delete"),
        cancelLabel: text("取消", "Cancel"),
        danger: true,
      }))
    )
      return;
    const filter = filterFor(entry.scope);
    try {
      await invoke("memory_delete", { id: entry.id, workdir: filter?.workdir });
      toast(text("记忆已删除", "Memory deleted"));
      await refreshScope(entry.scope);
      await refreshUsage();
    } catch (error) {
      const ui = uis.get(entry.scope);
      status(
        ui?.message ?? null,
        `${text("删除失败：", "Delete failed: ")}${errorText(error)}`,
        "error",
      );
    }
  };

  const organize = async (scope: MemoryScope, trigger: HTMLButtonElement) => {
    const ui = uis.get(scope);
    const filter = filterFor(scope);
    if (!ui || !filter) {
      status(
        ui?.message ?? null,
        text(
          "请先选择一个可访问的工作区。",
          "Select an accessible workspace first.",
        ),
        "error",
      );
      return;
    }
    setBusy(trigger, true, text("分析中…", "Analysing…"));
    try {
      const preview = await invoke<MemoryOrganizeResponse>("memory_organize", {
        input: { ...filter, dryRun: true },
      });
      if (!preview.duplicateEntries) {
        status(
          ui.message,
          text(
            `已检查 ${preview.inspected} 条记忆，未发现完全重复项。`,
            `Checked ${preview.inspected} memories; no exact duplicates found.`,
          ),
          "success",
        );
        return;
      }
      const confirmed = await requestAppConfirm({
        title: text("整理记忆", "Organize memory"),
        message: text(
          `发现 ${preview.duplicateEntries} 条完全重复记忆。删除重复项并保留每组第一条吗？`,
          `Found ${preview.duplicateEntries} exact duplicate memories. Remove duplicates and retain the first entry in each group?`,
        ),
        confirmLabel: text("删除重复", "Remove duplicates"),
        cancelLabel: text("取消", "Cancel"),
        danger: true,
      });
      if (!confirmed) {
        status(
          ui.message,
          text(
            "已取消整理，未删除任何记忆。",
            "Organization cancelled; no memories were deleted.",
          ),
        );
        return;
      }
      const result = await invoke<MemoryOrganizeResponse>("memory_organize", {
        input: { ...filter, dryRun: false },
      });
      status(
        ui.message,
        text(
          `已删除 ${result.removed} 条重复记忆，回收 ${bytes(result.reclaimedBytes)}。`,
          `Removed ${result.removed} duplicate memories and reclaimed ${bytes(result.reclaimedBytes)}.`,
        ),
        "success",
      );
      await refreshScope(scope);
      await refreshUsage();
    } catch (error) {
      status(
        ui.message,
        `${text("整理失败：", "Organization failed: ")}${errorText(error)}`,
        "error",
      );
    } finally {
      setBusy(trigger, false);
    }
  };

  const exportScope = async (
    scope: MemoryScope,
    trigger: HTMLButtonElement,
    format: HTMLSelectElement,
  ) => {
    const ui = uis.get(scope);
    const filter = filterFor(scope);
    if (!ui || !filter) {
      status(
        ui?.message ?? null,
        text(
          "请先选择一个可访问的工作区。",
          "Select an accessible workspace first.",
        ),
        "error",
      );
      return;
    }
    setBusy(trigger, true, text("等待保存位置…", "Waiting for save location…"));
    try {
      const result = await invoke<MemoryExportResponse | null>(
        "memory_export",
        { filter, format: format.value },
      );
      if (result)
        status(
          ui.message,
          text(
            `已导出 ${result.entries} 条记忆（${bytes(result.bytes)}）。`,
            `Exported ${result.entries} memories (${bytes(result.bytes)}).`,
          ),
          "success",
        );
      else status(ui.message, text("已取消导出。", "Export cancelled."));
    } catch (error) {
      status(
        ui.message,
        `${text("导出失败：", "Export failed: ")}${errorText(error)}`,
        "error",
      );
    } finally {
      setBusy(trigger, false);
    }
  };

  const clearScope = async (scope: MemoryScope, trigger: HTMLButtonElement) => {
    const ui = uis.get(scope);
    const filter = filterFor(scope);
    if (!ui || !filter) {
      status(
        ui?.message ?? null,
        text(
          "请先选择一个可访问的工作区。",
          "Select an accessible workspace first.",
        ),
        "error",
      );
      return;
    }
    const knownEntries = state.stats.get(scope)?.totalEntries;
    const first = await requestAppConfirm({
      title: text("清空记忆", "Clear memory"),
      message: text(
        `确定清空${scope === "global" ? "全部长期" : "当前项目"}记忆${knownEntries === undefined ? "" : `（${knownEntries} 条）`}吗？此操作无法撤销。`,
        `Clear ${scope === "global" ? "all long-term" : "the current project"} memory${knownEntries === undefined ? "" : ` (${knownEntries} entries)`}? This cannot be undone.`,
      ),
      confirmLabel: text("继续", "Continue"),
      cancelLabel: text("取消", "Cancel"),
      danger: true,
    });
    if (!first) return;
    const second = await requestAppConfirm({
      title: text("再次确认", "Confirm again"),
      message: text(
        "再次确认：原生服务将永久删除这个作用域中的全部记忆。",
        "Confirm again: the native service will permanently delete every memory in this scope.",
      ),
      confirmLabel: text("清空", "Clear"),
      cancelLabel: text("取消", "Cancel"),
      danger: true,
    });
    if (!second) {
      status(
        ui.message,
        text(
          "已取消清空，未删除任何记忆。",
          "Clear cancelled; no memories were deleted.",
        ),
      );
      return;
    }
    setBusy(trigger, true, text("清空中…", "Clearing…"));
    try {
      const confirmation =
        scope === "global" ? "CLEAR_GLOBAL_MEMORY" : "CLEAR_PROJECT_MEMORY";
      const result = await invoke<MemoryClearResponse>("memory_clear", {
        input: { ...filter, confirmation },
      });
      status(
        ui.message,
        text(
          `已清空 ${result.removed} 条记忆，回收 ${bytes(result.reclaimedBytes)}。`,
          `Cleared ${result.removed} memories and reclaimed ${bytes(result.reclaimedBytes)}.`,
        ),
        "success",
      );
      toast(text("长期记忆已清空", "Long-term memory cleared"));
      await refreshScope(scope);
      await refreshUsage();
    } catch (error) {
      status(
        ui.message,
        `${text("清空失败：", "Clear failed: ")}${errorText(error)}`,
        "error",
      );
    } finally {
      setBusy(trigger, false);
    }
  };

  const exportUsage = async (trigger: HTMLButtonElement) => {
    setBusy(trigger, true, text("等待保存位置…", "Waiting for save location…"));
    try {
      const result = await invoke<MemoryUsageExportResponse | null>(
        "memory_usage_export",
        { filter: usageFilter() },
      );
      if (result) {
        status(
          usageStatus,
          text(
            `已导出使用报告（${bytes(result.bytes)}）。`,
            `Usage report exported (${bytes(result.bytes)}).`,
          ),
          "success",
        );
      } else {
        status(usageStatus, text("已取消导出。", "Export cancelled."));
      }
    } catch (error) {
      status(
        usageStatus,
        `${text("报告导出失败：", "Report export failed: ")}${errorText(error)}`,
        "error",
      );
    } finally {
      setBusy(trigger, false);
    }
  };

  const buildScope = (scope: MemoryScope, container: HTMLElement) => {
    container.replaceChildren();
    const title =
      scope === "project"
        ? text("项目记忆", "Project memory")
        : text("长期记忆", "Long-term memory");
    const description =
      scope === "project"
        ? text(
            "记忆只绑定到当前选定工作区。",
            "Memory is bound only to the selected workspace.",
          )
        : text(
            "全局记忆不绑定工作区，可在本机持续使用。",
            "Global memory is not tied to a workspace and persists locally.",
          );
    const lead = element("div", { className: "field" });
    lead.append(
      element("label", { text: title }),
      element("p", { text: description }),
    );
    const rootField = element("div", { className: "field" });
    const rootLabel = element("label", {
      text:
        scope === "project"
          ? text("当前工作区", "Current workspace")
          : text("作用域", "Scope"),
    });
    const root = element("input", {
      attrs: { readonly: "", "aria-label": rootLabel.textContent ?? title },
    });
    rootField.append(rootLabel, root);
    const searchField = element("form", {
      className: "field",
      attrs: { role: "search" },
    });
    const searchLabel = element("label", {
      text: text("搜索记忆", "Search memory"),
    });
    const query = element("input", {
      id: `novaveiMemorySearch-${scope}`,
      attrs: { type: "search", maxlength: "256", autocomplete: "off" },
    });
    query.placeholder = text("输入关键词后搜索", "Enter keywords to search");
    searchLabel.htmlFor = query.id;
    const searchActions = element("div", { className: "row-actions" });
    const searchButton = button(
      text("搜索", "Search"),
      "btn",
      text("搜索记忆", "Search memory"),
    );
    searchButton.type = "submit";
    const clear = button(
      text("清除", "Clear"),
      "btn",
      text("清除搜索", "Clear search"),
    );
    searchActions.append(searchButton, clear);
    searchField.append(searchLabel, query, searchActions);
    const metrics = element("div", { className: "kv" });
    const total = element("dd", { text: "—" });
    const storage = element("dd", { text: "—" });
    const scopeValue = element("dd", {
      text:
        scope === "project" ? text("项目", "Project") : text("全局", "Global"),
    });
    for (const [label, value] of [
      [text("条目", "Entries"), total],
      [text("存储", "Storage"), storage],
      [text("作用域", "Scope"), scopeValue],
    ] as const) {
      const metric = element("div");
      metric.append(element("dt", { text: label }), value);
      metrics.append(metric);
    }
    const message = element("p", {
      className: "novavei-service-status",
      attrs: { "aria-live": "polite" },
    });
    const actions = element("div", { className: "row-actions" });
    const refreshButton = button(
      text("刷新", "Refresh"),
      "btn",
      text("刷新记忆", "Refresh memory"),
    );
    const createButton = button(
      text("新建记忆", "New memory"),
      "btn primary",
      text("新建记忆", "Create memory"),
    );
    const organizeButton = button(
      text("整理重复项", "Organize duplicates"),
      "btn",
      text("预览并整理重复记忆", "Preview and organize duplicate memories"),
    );
    const exportFormat = element("select", {
      attrs: { "aria-label": text("导出格式", "Export format") },
    });
    for (const [value, label] of [
      ["markdown", "Markdown"],
      ["json", "JSON"],
    ] as const) {
      const option = element("option", { text: label });
      option.value = value;
      exportFormat.append(option);
    }
    const exportButton = button(
      text("导出", "Export"),
      "btn",
      text("导出记忆", "Export memory"),
    );
    actions.append(
      refreshButton,
      createButton,
      organizeButton,
      exportFormat,
      exportButton,
    );
    const clearButton =
      scope === "global"
        ? enableMemoryControl(
            placeholderClear,
            text("清空", "Clear"),
            text("清空全部长期记忆", "Clear all long-term memory"),
          )
        : undefined;
    if (clearButton) actions.append(clearButton);
    const list = element("div", {
      className: "novavei-service-list",
      attrs: {
        "aria-live": "polite",
        "aria-label": text(`${title}条目`, `${title} entries`),
      },
    });
    container.append(
      lead,
      rootField,
      searchField,
      metrics,
      message,
      actions,
      list,
    );
    const ui: ScopeUi = {
      scope,
      root,
      query,
      list,
      message,
      total,
      storage,
      searchButton,
      refreshButton,
      createButton,
      organizeButton,
      exportButton,
      exportFormat,
    };
    uis.set(scope, ui);
    searchField.addEventListener("submit", (event) => {
      event.preventDefault();
      void refreshScope(scope, searchButton).then(() => refreshUsage());
    });
    clear.addEventListener("click", () => {
      query.value = "";
      void refreshScope(scope, clear);
    });
    refreshButton.addEventListener(
      "click",
      () => void refreshScope(scope, refreshButton),
    );
    createButton.addEventListener("click", () => editor.open(scope));
    organizeButton.addEventListener(
      "click",
      () => void organize(scope, organizeButton),
    );
    exportButton.addEventListener(
      "click",
      () => void exportScope(scope, exportButton, exportFormat),
    );
    clearButton?.addEventListener(
      "click",
      () => void clearScope(scope, clearButton),
    );
  };

  let knowledgeState: KnowledgeBaseListResponse = {
    enabled: false,
    folders: [],
  };
  let knowledgeRefreshEpoch = 0;
  let knowledgeUi:
    | {
        enabled: HTMLInputElement;
        add: HTMLButtonElement;
        refresh: HTMLButtonElement;
        query: HTMLInputElement;
        search: HTMLButtonElement;
        message: HTMLElement;
        list: HTMLElement;
        results: HTMLElement;
      }
    | undefined;

  const currentKnowledgeProvider = (): KnowledgeBaseConsent | undefined => {
    const picker = document.getElementById("modelPickerName");
    const providerId = picker?.dataset.providerId?.trim() ?? "";
    const modelId = picker?.dataset.modelId?.trim() ?? "";
    return providerId && modelId ? { providerId, modelId } : undefined;
  };

  const knowledgeConsentMatchesCurrentProvider = () => {
    const current = currentKnowledgeProvider();
    return Boolean(
      current &&
        knowledgeState.consent?.providerId === current.providerId &&
        knowledgeState.consent?.modelId === current.modelId,
    );
  };

  const renderKnowledgeFolders = () => {
    if (!knowledgeUi) return;
    const { list } = knowledgeUi;
    list.replaceChildren();
    if (!knowledgeState.folders.length) {
      list.append(
        emptyCard(
          text("尚未添加知识库文件夹", "No knowledge-base folders yet"),
          text(
            "通过“添加文件夹”选择资料目录；NovaVei 只会索引你在系统选择器中明确选中的文件夹。",
            "Choose a source directory with Add folder. NovaVei indexes only folders you explicitly select in the system picker.",
          ),
        ),
      );
      return;
    }
    for (const folder of knowledgeState.folders) {
      const card = element("article", { className: "hub-card" });
      const copy = element("div");
      copy.append(
        element("strong", { text: folder.displayName }),
        element("small", {
          text: `${folder.documentCount} ${text("个文档", "documents")} · ${bytes(folder.indexedBytes)} · ${text("索引于", "Indexed")} ${formatTime(folder.lastIndexedAt ?? undefined)}`,
        }),
        element("small", { text: folder.canonicalPath }),
      );
      const row = element("div", { className: "novavei-service-row" });
      row.append(copy);
      const actions = element("div", { className: "novavei-service-actions" });
      const refresh = button(
        text("重新索引", "Reindex"),
        "btn",
        text(`重新索引 ${folder.displayName}`, `Reindex ${folder.displayName}`),
      );
      const remove = button(
        text("移除", "Remove"),
        "btn",
        text(`移除 ${folder.displayName}`, `Remove ${folder.displayName}`),
      );
      refresh.addEventListener(
        "click",
        () => void refreshKnowledgeFolder(folder, refresh),
      );
      remove.addEventListener(
        "click",
        () => void removeKnowledgeFolder(folder, remove),
      );
      actions.append(refresh, remove);
      card.append(row, actions);
      list.append(card);
    }
  };

  const renderKnowledgeResults = (response?: KnowledgeBaseSearchResponse) => {
    if (!knowledgeUi) return;
    const { results } = knowledgeUi;
    results.replaceChildren();
    if (!response) return;
    if (!response.items.length) {
      results.append(
        emptyCard(
          text("未找到匹配资料", "No matching material"),
          text(
            "可尝试更少或更具体的关键词。",
            "Try fewer or more specific keywords.",
          ),
        ),
      );
      return;
    }
    for (const item of response.items) {
      const card = element("article", { className: "hub-card" });
      card.append(
        element("strong", { text: item.title }),
        element("small", { text: `${item.folderName} · ${item.relativePath}` }),
        element("p", {
          className: "novavei-service-preview",
          text: item.snippet,
        }),
      );
      results.append(card);
    }
  };

  const refreshKnowledge = async (
    trigger?: HTMLButtonElement,
  ): Promise<boolean> => {
    if (!knowledgeUi) return false;
    const epoch = ++knowledgeRefreshEpoch;
    if (trigger) setBusy(trigger, true, text("读取中…", "Loading…"));
    status(
      knowledgeUi.message,
      text(
        "正在读取本机知识库配置…",
        "Loading native knowledge-base configuration…",
      ),
    );
    try {
      knowledgeState = await invoke<KnowledgeBaseListResponse>(
        "knowledge_base_list",
      );
      if (epoch !== knowledgeRefreshEpoch) return true;
      knowledgeUi.enabled.checked =
        knowledgeState.enabled && knowledgeConsentMatchesCurrentProvider();
      knowledgeUi.enabled.disabled = knowledgeState.folders.length === 0;
      knowledgeUi.query.disabled = knowledgeState.folders.length === 0;
      knowledgeUi.search.disabled = knowledgeState.folders.length === 0;
      renderKnowledgeFolders();
      status(
        knowledgeUi.message,
        knowledgeState.enabled && !knowledgeConsentMatchesCurrentProvider()
          ? text(
              "当前模型尚未获准使用知识库；如需启用，请重新确认。",
              "The current model is not approved for knowledge-base use; confirm again to enable it.",
            )
          : knowledgeState.folders.length
            ? text(
                `已加载 ${knowledgeState.folders.length} 个知识库文件夹。`,
                `${knowledgeState.folders.length} knowledge-base folders loaded.`,
              )
            : text(
                "尚未添加知识库文件夹。",
                "No knowledge-base folders have been added.",
              ),
        "success",
      );
      return true;
    } catch (error) {
      if (epoch !== knowledgeRefreshEpoch) return false;
      knowledgeState = { enabled: false, folders: [] };
      knowledgeUi.enabled.checked = false;
      knowledgeUi.enabled.disabled = true;
      knowledgeUi.query.disabled = true;
      knowledgeUi.search.disabled = true;
      renderKnowledgeFolders();
      status(
        knowledgeUi.message,
        `${text("知识库服务不可用：", "Knowledge-base service unavailable: ")}${errorText(error)}`,
        "error",
      );
      return false;
    } finally {
      if (trigger) setBusy(trigger, false);
    }
  };

  const refreshKnowledgeFolder = async (
    folder: KnowledgeBaseFolder,
    trigger: HTMLButtonElement,
  ) => {
    if (!knowledgeUi) return;
    setBusy(trigger, true, text("索引中…", "Indexing…"));
    status(
      knowledgeUi.message,
      text(
        `正在重新索引“${folder.displayName}”…`,
        `Reindexing ${folder.displayName}…`,
      ),
    );
    try {
      const result = await invoke<KnowledgeBaseIndexResult>(
        "knowledge_base_refresh",
        { folderId: folder.id },
      );
      await refreshKnowledge();
      status(
        knowledgeUi.message,
        text(
          `已索引 ${result.indexedFiles} 个文件，跳过 ${result.skippedFiles} 个${result.truncated ? "；已达到本次索引上限。" : "。"}`,
          `${result.indexedFiles} files indexed; ${result.skippedFiles} skipped${result.truncated ? "; this indexing limit was reached." : "."}`,
        ),
        "success",
      );
    } catch (error) {
      status(
        knowledgeUi.message,
        `${text("重新索引失败：", "Reindex failed: ")}${errorText(error)}`,
        "error",
      );
    } finally {
      setBusy(trigger, false);
    }
  };

  const removeKnowledgeFolder = async (
    folder: KnowledgeBaseFolder,
    trigger: HTMLButtonElement,
  ) => {
    if (!knowledgeUi) return;
    const confirmed = await requestAppConfirm({
      title: text("移除知识库", "Remove knowledge base"),
      message: text(
        `移除“${folder.displayName}”只会删除 NovaVei 的本地索引和访问授权，不会删除源文件。`,
        `Removing ${folder.displayName} deletes only NovaVei's local index and access grant, never source files.`,
      ),
      confirmLabel: text("移除", "Remove"),
      danger: true,
    });
    if (!confirmed) return;
    setBusy(trigger, true, text("移除中…", "Removing…"));
    try {
      await invoke("knowledge_base_remove", { folderId: folder.id });
      renderKnowledgeResults();
      await refreshKnowledge();
      status(
        knowledgeUi.message,
        text(
          "知识库文件夹已移除；源文件保持不变。",
          "Knowledge-base folder removed; source files are unchanged.",
        ),
        "success",
      );
    } catch (error) {
      status(
        knowledgeUi.message,
        `${text("移除失败：", "Remove failed: ")}${errorText(error)}`,
        "error",
      );
    } finally {
      setBusy(trigger, false);
    }
  };

  const buildKnowledge = () => {
    knowledgePanel.replaceChildren();
    const lead = element("div", { className: "field" });
    lead.append(
      element("label", { text: text("知识库", "Knowledge base") }),
      element("p", {
        text: text(
          "添加多个本地资料文件夹并建立全文索引。仅支持常见 UTF-8 文本；密钥文件、隐藏环境文件和构建目录会被跳过。",
          "Add multiple local reference folders and build a full-text index. Only common UTF-8 text is supported; secret files, hidden environment files, and build folders are skipped.",
        ),
      }),
    );
    const disclosure = element("p", {
      className: "novavei-service-note",
      text: text(
        "开启“允许聊天使用”后，当前聊天的模型可在回答相关问题时检索并读取受限资料片段；这些片段会发送给你选用的模型提供商。",
        "When Allow chat use is enabled, the current chat model may search and read bounded relevant excerpts; those excerpts are sent to your selected model provider.",
      ),
    });
    const enabled = element("input", {
      id: "novaveiKnowledgeBaseEnabled",
      attrs: { type: "checkbox" },
    });
    enabled.type = "checkbox";
    const enabledLabel = element("label", { className: "novavei-cron-toggle" });
    enabledLabel.htmlFor = enabled.id;
    enabledLabel.append(
      enabled,
      document.createTextNode(
        text("允许聊天使用知识库", "Allow chat use of knowledge bases"),
      ),
    );
    const controls = element("div", { className: "row-actions" });
    const add = button(
      text("添加文件夹", "Add folder"),
      "btn primary",
      text(
        "通过系统选择器添加知识库文件夹",
        "Add a knowledge-base folder with the system picker",
      ),
    );
    const refresh = button(
      text("刷新列表", "Refresh list"),
      "btn",
      text("刷新知识库文件夹列表", "Refresh knowledge-base folders"),
    );
    controls.append(add, refresh);
    const message = element("p", {
      className: "novavei-service-status",
      attrs: { "aria-live": "polite" },
    });
    const list = element("div", {
      className: "novavei-service-list",
      attrs: {
        "aria-live": "polite",
        "aria-label": text("知识库文件夹", "Knowledge-base folders"),
      },
    });
    const searchForm = element("form", {
      className: "field",
      attrs: { role: "search" },
    });
    const searchLabel = element("label", {
      text: text("查询已索引资料", "Search indexed material"),
    });
    const query = element("input", {
      id: "novaveiKnowledgeBaseSearch",
      attrs: { type: "search", maxlength: "256", autocomplete: "off" },
    });
    query.placeholder = text("输入关键词", "Enter keywords");
    searchLabel.htmlFor = query.id;
    const search = button(
      text("查询", "Search"),
      "btn",
      text("查询已索引知识库资料", "Search indexed knowledge-base material"),
    );
    search.type = "submit";
    searchForm.append(searchLabel, query, search);
    const results = element("div", {
      className: "novavei-service-list",
      attrs: {
        "aria-live": "polite",
        "aria-label": text("知识库查询结果", "Knowledge-base search results"),
      },
    });
    knowledgePanel.append(
      lead,
      disclosure,
      enabledLabel,
      controls,
      message,
      list,
      searchForm,
      results,
    );
    knowledgeUi = {
      enabled,
      add,
      refresh,
      query,
      search,
      message,
      list,
      results,
    };

    enabled.addEventListener("change", () => {
      void (async () => {
        if (!knowledgeUi) return;
        const next = enabled.checked;
        if (next && !knowledgeState.folders.length) {
          enabled.checked = false;
          status(
            message,
            text(
              "请先添加至少一个知识库文件夹。",
              "Add at least one knowledge-base folder first.",
            ),
            "error",
          );
          return;
        }
        const consent = next ? currentKnowledgeProvider() : undefined;
        if (next && !consent) {
          enabled.checked = false;
          status(
            message,
            text(
              "请先选择当前聊天模型，再授权知识库使用。",
              "Choose the current chat model before authorizing knowledge-base use.",
            ),
            "error",
          );
          return;
        }
        if (
          next &&
          consent &&
          !(await requestAppConfirm({
            title: text(
              "允许聊天使用知识库",
              "Allow chat use of knowledge bases",
            ),
            message: text(
              `启用后，${consent.providerId} / ${consent.modelId} 可在回答相关问题时读取检索到的本地资料片段。片段会发送给该提供商；新增或移除资料文件夹时需要重新确认。`,
              `When enabled, ${consent.providerId} / ${consent.modelId} may read retrieved local excerpts for relevant answers. Excerpts are sent to that provider; adding or removing source folders requires a new confirmation.`,
            ),
            confirmLabel: text("允许", "Allow"),
            danger: false,
          }))
        ) {
          enabled.checked = false;
          return;
        }
        enabled.disabled = true;
        try {
          knowledgeState = await invoke<KnowledgeBaseListResponse>(
            "knowledge_base_set_enabled",
            { enabled: next, consent },
          );
          enabled.checked =
            knowledgeState.enabled && knowledgeConsentMatchesCurrentProvider();
          query.disabled = knowledgeState.folders.length === 0;
          search.disabled = knowledgeState.folders.length === 0;
          renderKnowledgeFolders();
          status(
            message,
            next
              ? text("聊天知识库已启用。", "Chat knowledge bases enabled.")
              : text("聊天知识库已停用。", "Chat knowledge bases disabled."),
            "success",
          );
        } catch (error) {
          enabled.checked =
            knowledgeState.enabled && knowledgeConsentMatchesCurrentProvider();
          status(
            message,
            `${text("无法保存知识库设置：", "Unable to save knowledge-base setting: ")}${errorText(error)}`,
            "error",
          );
        } finally {
          enabled.disabled = knowledgeState.folders.length === 0;
        }
      })();
    });
    add.addEventListener("click", () => {
      void (async () => {
        setBusy(add, true, text("等待选择…", "Waiting for selection…"));
        status(
          message,
          text(
            "请在系统选择器中选择资料文件夹。",
            "Choose a reference folder in the system picker.",
          ),
        );
        try {
          const result = await invoke<KnowledgeBaseIndexResult | null>(
            "knowledge_base_pick_folder",
          );
          if (!result) {
            status(message, text("已取消选择。", "Selection cancelled."));
            return;
          }
          renderKnowledgeResults();
          await refreshKnowledge();
          status(
            message,
            text(
              `已添加并索引 ${result.indexedFiles} 个文件，跳过 ${result.skippedFiles} 个${result.truncated ? "；已达到本次索引上限。" : "。"}如需聊天使用，请重新确认。`,
              `${result.indexedFiles} files added and indexed; ${result.skippedFiles} skipped${result.truncated ? "; this indexing limit was reached." : "."} Confirm again before allowing chat use.`,
            ),
            "success",
          );
        } catch (error) {
          status(
            message,
            `${text("添加知识库失败：", "Unable to add knowledge base: ")}${errorText(error)}`,
            "error",
          );
        } finally {
          setBusy(add, false);
        }
      })();
    });
    refresh.addEventListener("click", () => void refreshKnowledge(refresh));
    searchForm.addEventListener("submit", (event) => {
      event.preventDefault();
      void (async () => {
        const value = query.value.trim();
        if (!value) {
          renderKnowledgeResults();
          status(
            message,
            text("请输入查询关键词。", "Enter search keywords."),
            "error",
          );
          return;
        }
        setBusy(search, true, text("查询中…", "Searching…"));
        try {
          const response = await invoke<KnowledgeBaseSearchResponse>(
            "knowledge_base_search",
            { query: value, limit: 12 },
          );
          renderKnowledgeResults(response);
          status(
            message,
            text(
              `找到 ${response.items.length} 条匹配资料${response.truncated ? "（结果已截断）。" : "。"}`,
              `${response.items.length} matching items found${response.truncated ? " (results truncated)." : "."}`,
            ),
            "success",
          );
        } catch (error) {
          renderKnowledgeResults();
          status(
            message,
            `${text("知识库查询失败：", "Knowledge-base search failed: ")}${errorText(error)}`,
            "error",
          );
        } finally {
          setBusy(search, false);
          search.disabled = knowledgeState.folders.length === 0;
        }
      })();
    });
  };

  const buildUsage = () => {
    usagePanel.replaceChildren();
    const usageLead = element("div", { className: "field" });
    usageLead.append(
      element("label", { text: text("记忆使用情况", "Memory usage") }),
      element("p", {
        text: text(
          "统计仅来自本机持久化记忆；不会包含模型上下文或供应商凭据。",
          "Statistics cover only locally persisted memory, never model context or provider credentials.",
        ),
      }),
    );
    const usageMetrics = element("div", { className: "kv" });
    for (const [label, value] of [
      [text("总条目", "Total entries"), usageTotal],
      [text("项目", "Project"), usageProject],
      [text("长期", "Long-term"), usageGlobal],
      [text("存储", "Storage"), usageBytes],
      [text("本周检索", "Searches this week"), usageSearches],
      [text("本周写入", "Writes this week"), usageWrites],
    ] as const) {
      const metric = element("div");
      metric.append(element("dt", { text: label }), value);
      usageMetrics.append(metric);
    }
    const capacityField = element("div", { className: "field" });
    capacityField.append(
      element("label", { text: text("条目容量", "Entry capacity") }),
      usageMeter,
      usageCapacity,
      usageTracking,
    );
    const usageActions = element("div", { className: "row-actions" });
    const usageRefresh = button(
      text("刷新统计", "Refresh statistics"),
      "btn primary",
      text("刷新记忆统计", "Refresh memory statistics"),
    );
    const usageReport = enableMemoryControl(
      placeholderUsageReport,
      text("导出报告", "Export report"),
      text("导出记忆使用报告", "Export memory usage report"),
    );
    usageActions.append(usageRefresh, usageReport);
    usagePanel.append(
      usageLead,
      usageMetrics,
      capacityField,
      usageStatus,
      usageActions,
    );
    usageRefresh.addEventListener("click", () => {
      void (async () => {
        setBusy(usageRefresh, true, text("刷新中…", "Refreshing…"));
        status(
          usageStatus,
          text("正在刷新记忆统计…", "Refreshing memory statistics…"),
        );
        const results = await Promise.all([
          refreshScope("global"),
          refreshScope("project"),
          refreshUsage(),
        ]);
        status(
          usageStatus,
          results.every(Boolean)
            ? text("记忆统计已刷新。", "Memory statistics refreshed.")
            : text(
                "部分记忆统计无法读取，请查看对应分区提示。",
                "Some memory statistics could not be read; check the relevant scope message.",
              ),
          results.every(Boolean) ? "success" : "error",
        );
        setBusy(usageRefresh, false);
      })();
    });
    usageReport.addEventListener("click", () => void exportUsage(usageReport));
  };

  const refreshAllMemory = () => {
    void Promise.all([
      refreshScope("global"),
      refreshScope("project"),
      refreshKnowledge(),
    ]).then(() => refreshUsage());
  };

  const remountMemoryChrome = () => {
    buildScope("project", projectPanel);
    buildScope("global", globalPanel);
    buildKnowledge();
    buildUsage();
  };

  remountMemoryChrome();
  window.addEventListener("novavei:model-options-rendered", () => {
    if (!knowledgeUi) return;
    const approvedForCurrentModel = knowledgeConsentMatchesCurrentProvider();
    knowledgeUi.enabled.checked =
      knowledgeState.enabled && approvedForCurrentModel;
    if (knowledgeState.enabled && !approvedForCurrentModel) {
      status(
        knowledgeUi.message,
        text(
          "当前模型尚未获准使用知识库；如需启用，请重新确认。",
          "The current model is not approved for knowledge-base use; confirm again to enable it.",
        ),
      );
    }
  });
  document.addEventListener("click", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest('button[data-settings="memory"], [data-memory-tab]')) {
      refreshAllMemory();
    }
  });
  onServiceLanguageChange(() => {
    remountMemoryChrome();
    refreshAllMemory();
  });
  refreshAllMemory();
}

function installCron() {
  const invoke = invokeApi();
  const listen = window.__TAURI__?.event?.listen;
  const panel = document.querySelector<HTMLElement>(
    '.settings-panel[data-settings="cron"]',
  );
  if (!invoke || !panel) return;

  let jobs: CronJobSummary[] = [];
  const statusLine = element("p", {
    className: "novavei-service-status",
    attrs: { "aria-live": "polite" },
  });
  const total = element("b", { text: "—" });
  const enabled = element("b", { text: "—" });
  const paused = element("b", { text: "—" });
  const list = element("div", {
    className: "cron-list novavei-service-list",
    attrs: {
      "aria-live": "polite",
      "aria-label": text("定时任务列表", "Scheduled task list"),
    },
  });
  const refresh = button(
    text("刷新", "Refresh"),
    "btn",
    text("刷新定时任务", "Refresh scheduled tasks"),
  );

  const formatTime = (value: number | null | undefined) => {
    if (!Number.isFinite(value)) return text("尚未安排", "Not scheduled");
    return new Intl.DateTimeFormat(isEnglish() ? "en-GB" : "zh-CN", {
      dateStyle: "short",
      timeStyle: "short",
    }).format(new Date(value as number));
  };

  const editor = (() => {
    const dialog = element("dialog", {
      className: "novavei-service-dialog novavei-cron-editor",
      attrs: { "aria-modal": "true" },
    });
    const form = element("form", { className: "novavei-service-dialog-body" });
    form.noValidate = true;
    const heading = element("h2", { id: "novaveiCronEditorTitle" });
    dialog.setAttribute("aria-labelledby", heading.id);
    const intro = element("p", {
      className: "novavei-service-note",
      text: text(
        "保存只会把任务和本次输入的 payload 写入本机；任务列表与运行记录从不回显已保存的 payload。",
        "Saving only writes the job and this newly entered payload locally; lists and run history never reveal a saved payload.",
      ),
    });
    const makeField = (
      labelText: string,
      control: HTMLElement,
      required = false,
      hint?: string,
    ) => {
      const field = element("div", { className: "field" });
      const label = element("label", { text: labelText });
      label.htmlFor = control.id;
      if (required) {
        label.append(
          document.createTextNode(" "),
          element("span", {
            className: "novavei-cron-required",
            text: "*",
            attrs: { "aria-hidden": "true" },
          }),
        );
        control.setAttribute(
          "aria-label",
          `${labelText} ${text("（必填）", "(required)")}`,
        );
      }
      field.append(label, control);
      if (hint)
        field.append(
          element("p", { className: "novavei-service-note", text: hint }),
        );
      return field;
    };
    const makeToggle = (
      input: HTMLInputElement,
      labelContent: HTMLElement,
      hint?: string,
    ) => {
      const field = element("div", { className: "field" });
      const control = element("label", { className: "novavei-cron-toggle" });
      control.append(input, labelContent);
      field.append(control);
      if (hint)
        field.append(
          element("p", { className: "novavei-service-note", text: hint }),
        );
      return field;
    };
    const nameInput = element("input", {
      id: "novaveiCronName",
      attrs: {
        type: "text",
        maxlength: "120",
        autocomplete: "off",
        required: "",
      },
    });
    nameInput.type = "text";
    const kindInput = element("select", {
      id: "novaveiCronKind",
      attrs: { "aria-label": text("任务类型", "Task type") },
    });
    for (const [value, zh, en] of [
      ["prompt", "提示词任务", "Prompt task"],
      ["shell", "Shell 命令", "Shell command"],
      ["http", "HTTP 请求", "HTTP request"],
    ] as const) {
      const option = element("option", { text: text(zh, en) });
      option.value = value;
      kindInput.append(option);
    }
    const scheduleInput = element("input", {
      id: "novaveiCronSchedule",
      attrs: {
        type: "text",
        maxlength: "32",
        autocomplete: "off",
        required: "",
      },
    });
    scheduleInput.type = "text";
    scheduleInput.placeholder = "hourly / daily:09:30 / weekly:1:09:30";
    const enabledInput = element("input", {
      id: "novaveiCronEnabled",
      attrs: { type: "checkbox" },
    });
    enabledInput.type = "checkbox";

    const promptInput = element("textarea", {
      id: "novaveiCronPrompt",
      attrs: { maxlength: "65536", autocomplete: "off", spellcheck: "false" },
    });
    const promptWorkdir = element("input", {
      id: "novaveiCronPromptWorkdir",
      attrs: { type: "text", maxlength: "32768", autocomplete: "off" },
    });
    promptWorkdir.type = "text";
    const promptProvider = element("input", {
      id: "novaveiCronPromptProvider",
      attrs: { type: "text", maxlength: "128", autocomplete: "off" },
    });
    promptProvider.type = "text";
    const promptModel = element("input", {
      id: "novaveiCronPromptModel",
      attrs: { type: "text", maxlength: "256", autocomplete: "off" },
    });
    promptModel.type = "text";
    const promptFields = element("div", {
      className: "novavei-cron-payload-fields",
    });
    promptFields.append(
      makeField(
        text("提示词", "Prompt"),
        promptInput,
        true,
        text(
          "NovaVei 打开期间，到期后会在本机用已保存供应商发起一次无工具补全；已保存的文本不会回显到这里。",
          "While NovaVei is open, due jobs run one native tool-less completion with the saved provider; saved text is never shown here.",
        ),
      ),
      makeField(
        text("工作目录（可选）", "Working directory (optional)"),
        promptWorkdir,
      ),
      makeField(
        text("供应商 ID（可选）", "Provider ID (optional)"),
        promptProvider,
      ),
      makeField(text("模型（可选）", "Model (optional)"), promptModel),
    );

    const shellCommand = element("textarea", {
      id: "novaveiCronShellCommand",
      attrs: { maxlength: "16384", autocomplete: "off", spellcheck: "false" },
    });
    const shellWorkdir = element("input", {
      id: "novaveiCronShellWorkdir",
      attrs: { type: "text", maxlength: "32768", autocomplete: "off" },
    });
    shellWorkdir.type = "text";
    const shellFields = element("div", {
      className: "novavei-cron-payload-fields",
    });
    shellFields.append(
      makeField(
        text("Shell 命令", "Shell command"),
        shellCommand,
        true,
        text(
          "命令内容仅在本次保存时提交给 native，之后不会回显。",
          "The command is sent to native only for this save and is never shown again.",
        ),
      ),
      makeField(text("工作目录", "Working directory"), shellWorkdir, true),
    );

    const httpUrl = element("input", {
      id: "novaveiCronHttpUrl",
      attrs: { type: "url", maxlength: "2048", autocomplete: "off" },
    });
    httpUrl.type = "url";
    const httpMethod = element("select", {
      id: "novaveiCronHttpMethod",
      attrs: { "aria-label": text("HTTP 方法", "HTTP method") },
    });
    for (const method of ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"]) {
      const option = element("option", { text: method });
      option.value = method;
      httpMethod.append(option);
    }
    const httpHeaders = element("textarea", {
      id: "novaveiCronHttpHeaders",
      attrs: { maxlength: "65536", autocomplete: "off", spellcheck: "false" },
    });
    const httpBody = element("textarea", {
      id: "novaveiCronHttpBody",
      attrs: { maxlength: "131072", autocomplete: "off", spellcheck: "false" },
    });
    const httpTimeout = element("input", {
      id: "novaveiCronHttpTimeout",
      attrs: {
        type: "number",
        min: "1000",
        max: "30000",
        step: "1",
        required: "",
      },
    });
    httpTimeout.type = "number";
    const httpFields = element("div", {
      className: "novavei-cron-payload-fields",
    });
    httpFields.append(
      makeField(
        text("请求 URL", "Request URL"),
        httpUrl,
        true,
        text(
          "仅允许完整的 HTTP(S) URL；已保存 URL 不会回显。",
          "Only full HTTP(S) URLs are allowed; a saved URL is never shown.",
        ),
      ),
      makeField(text("请求方法", "Request method"), httpMethod, true),
      makeField(
        text("请求头（可选）", "Request headers (optional)"),
        httpHeaders,
        false,
        text(
          "每行 `Name: value`。敏感值只用于本次保存，之后不会回显。",
          "One `Name: value` per line. Sensitive values are only used for this save and never shown again.",
        ),
      ),
      makeField(text("请求体（可选）", "Request body (optional)"), httpBody),
      makeField(
        text("超时（毫秒）", "Timeout (ms)"),
        httpTimeout,
        true,
        text(
          "范围 1000–30000；默认 15000。",
          "Range 1000–30000; default 15000.",
        ),
      ),
    );

    const payloadNotice = element("p", { className: "novavei-service-note" });
    const sensitiveWarning = element("p", {
      className: "novavei-cron-warning",
    });
    const sensitiveConfirmation = element("input", {
      id: "novaveiCronSensitiveConfirm",
      attrs: { type: "checkbox" },
    });
    sensitiveConfirmation.type = "checkbox";
    const sensitiveConfirmationText = element("span");
    const sensitiveConfirmationField = makeToggle(
      sensitiveConfirmation,
      sensitiveConfirmationText,
    );
    const alert = element("p", {
      attrs: { role: "alert", "aria-live": "assertive" },
    });
    const actionRow = element("div", { className: "row-actions" });
    const cancel = button(
      text("取消", "Cancel"),
      "btn",
      text("取消定时任务编辑", "Cancel scheduled task editing"),
    );
    const save = button(
      text("安全保存", "Save safely"),
      "btn primary",
      text("安全保存定时任务", "Save scheduled task safely"),
    );
    save.type = "submit";
    const enabledAfterSavingLabel = element("span", {
      text: text(
        "所有定时任务保存后默认禁用",
        "All scheduled tasks save disabled by default",
      ),
    });
    actionRow.append(cancel, save);
    form.append(
      heading,
      intro,
      makeField(text("任务名称", "Task name"), nameInput, true),
      makeField(text("任务类型", "Task type"), kindInput, true),
      makeField(
        text("日程", "Schedule"),
        scheduleInput,
        true,
        text(
          "支持 `hourly`、`daily:HH:MM` 或 `weekly:D:HH:MM`；星期一为 1，星期日为 7。",
          "Use `hourly`, `daily:HH:MM`, or `weekly:D:HH:MM`; Monday is 1 and Sunday is 7.",
        ),
      ),
      makeToggle(enabledInput, enabledAfterSavingLabel),
      payloadNotice,
      promptFields,
      shellFields,
      httpFields,
      sensitiveWarning,
      sensitiveConfirmationField,
      alert,
      actionRow,
    );
    dialog.append(form);
    document.body.append(dialog);

    let editing: CronJobSummary | undefined;
    let previousFocus: HTMLElement | null = null;
    let saving = false;
    const selectedKind = (): CronJobKind => {
      if (kindInput.value === "shell") return "shell";
      if (kindInput.value === "http") return "http";
      return "prompt";
    };
    const setEditorBusy = (busy: boolean) => {
      for (const control of form.querySelectorAll<
        HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement
      >("input, select, textarea")) {
        control.disabled = busy || control === enabledInput;
      }
      cancel.disabled = busy;
      setBusy(save, busy, text("保存中…", "Saving…"));
    };
    const showError = (control: HTMLElement, message: string) => {
      alert.textContent = message;
      control.focus({ preventScroll: true });
    };
    const clearPayloadDraft = () => {
      promptInput.value = "";
      promptWorkdir.value = "";
      promptProvider.value = "";
      promptModel.value = "";
      shellCommand.value = "";
      shellWorkdir.value = "";
      httpUrl.value = "";
      httpMethod.value = "GET";
      httpHeaders.value = "";
      httpBody.value = "";
      httpTimeout.value = "15000";
      sensitiveConfirmation.checked = false;
    };
    const renderPayloadFields = () => {
      const kind = selectedKind();
      const isPrompt = kind === "prompt";
      const isShell = kind === "shell";
      const isHttp = kind === "http";
      promptFields.hidden = !isPrompt;
      shellFields.hidden = !isShell;
      httpFields.hidden = !isHttp;
      promptInput.required = isPrompt;
      shellCommand.required = isShell;
      shellWorkdir.required = isShell;
      httpUrl.required = isHttp;
      httpTimeout.required = isHttp;
      enabledInput.disabled = true;
      enabledInput.checked = false;
      enabledAfterSavingLabel.textContent = text(
        "所有定时任务保存后默认禁用；请在列表中另行确认启用。",
        "All scheduled tasks save disabled; confirm enable separately from the list.",
      );
      sensitiveWarning.hidden = false;
      sensitiveConfirmationField.hidden = false;
      sensitiveConfirmation.required = true;
      if (isPrompt) {
        sensitiveWarning.textContent = text(
          "Prompt 任务会使用已保存供应商发起可能计费的请求。保存、启用和立即运行都会要求 Windows 原生确认；保存后默认禁用，payload 与凭据不会回显。",
          "A Prompt task can make a billable request through a saved provider. Save, enable, and Run now require Windows-native confirmation; it saves disabled and never reveals its payload or credentials.",
        );
        sensitiveConfirmationText.textContent = text(
          "我确认此 Prompt 任务会使用已保存供应商，且其日程、Provider 和模型是预期的。",
          "I confirm that this Prompt task uses a saved provider and that its schedule, provider, and model are intended.",
        );
      } else if (isShell) {
        sensitiveWarning.textContent = text(
          "Shell 任务会以当前用户权限运行，可能影响本机文件或系统。保存或更改后会默认禁用；请在列表中另行确认启用。命令和输出不会回显到界面。",
          "A Shell task runs with current user permissions and can affect local files or the system. It is saved disabled after creation or changes; enable it separately from the list. The command and output are never shown in the UI.",
        );
        sensitiveConfirmationText.textContent = text(
          "我确认此 Shell 任务会以当前用户权限运行，且其命令与日程是预期的。",
          "I confirm this Shell task runs with current user permissions and that its command and schedule are intended.",
        );
      } else if (isHttp) {
        sensitiveWarning.textContent = text(
          "HTTP 任务只能请求公共 HTTPS 地址；native 会拒绝 localhost、私有网络、回环和本地链路地址。保存或更改后会默认禁用；请在列表中另行确认启用。",
          "An HTTP task may target only public HTTPS addresses; native rejects localhost, private-network, loopback, and link-local addresses. It is saved disabled after creation or changes; enable it separately from the list.",
        );
        sensitiveConfirmationText.textContent = text(
          "我确认此 HTTPS 请求及其日程是预期的，并理解网络目标限制。",
          "I confirm this HTTPS request and its schedule are intended, and understand the network-target restrictions.",
        );
      }
    };
    const isDisallowedCronHttpHostname = (hostname: string) => {
      const normalizedHostname = hostname
        .replace(/^\[|\]$/g, "")
        .replace(/\.$/, "")
        .toLowerCase();
      if (
        [
          "localhost",
          "localhost.localdomain",
          "ip6-localhost",
          "ip6-loopback",
        ].includes(normalizedHostname)
      )
        return true;
      if (
        [
          ".localhost",
          ".local",
          ".localdomain",
          ".lan",
          ".home",
          ".internal",
        ].some((suffix) => normalizedHostname.endsWith(suffix))
      )
        return true;
      const ipv4Parts = normalizedHostname.split(".");
      if (
        ipv4Parts.length === 4 &&
        ipv4Parts.every((part) => /^\d{1,3}$/.test(part))
      ) {
        const octets = ipv4Parts.map(Number);
        if (octets.some((octet) => octet > 255)) return true;
        return (
          octets[0] === 10 ||
          octets[0] === 127 ||
          octets[0] === 0 ||
          (octets[0] === 169 && octets[1] === 254) ||
          (octets[0] === 172 && octets[1] >= 16 && octets[1] <= 31) ||
          (octets[0] === 192 && octets[1] === 168)
        );
      }
      if (
        normalizedHostname === "::" ||
        normalizedHostname === "::1" ||
        /^fe[89ab][0-9a-f:]*$/i.test(normalizedHostname) ||
        /^f[cd][0-9a-f:]*$/i.test(normalizedHostname)
      )
        return true;
      return (
        !normalizedHostname.includes(".") && !normalizedHostname.includes(":")
      );
    };
    const parseHttpHeaders = (): Record<string, string> | undefined => {
      const headers: Record<string, string> = {};
      const names = new Set<string>();
      for (const rawLine of httpHeaders.value.split(/\r?\n/)) {
        if (!rawLine.trim()) continue;
        const divider = rawLine.indexOf(":");
        const name = rawLine.slice(0, divider).trim();
        const value = rawLine.slice(divider + 1).trim();
        if (divider < 1 || !/^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/.test(name)) {
          showError(
            httpHeaders,
            text(
              "请求头须每行使用 `Name: value`，且名称必须有效。",
              "Each request header must use `Name: value` with a valid name.",
            ),
          );
          return undefined;
        }
        const normalized = name.toLowerCase();
        if (names.has(normalized)) {
          showError(
            httpHeaders,
            text(
              "同一个请求头只能填写一次。",
              "Each request header may be entered only once.",
            ),
          );
          return undefined;
        }
        if (
          [
            "host",
            "content-length",
            "connection",
            "transfer-encoding",
          ].includes(normalized)
        ) {
          showError(
            httpHeaders,
            text(
              "请勿填写由 native 管理的传输请求头。",
              "Do not enter transport headers managed by native.",
            ),
          );
          return undefined;
        }
        names.add(normalized);
        if (names.size > 32) {
          showError(
            httpHeaders,
            text(
              "最多可填写 32 个请求头。",
              "At most 32 request headers are allowed.",
            ),
          );
          return undefined;
        }
        headers[name] = value;
      }
      return headers;
    };
    const buildInput = (): CronUpsertInput | undefined => {
      const name = nameInput.value.trim();
      if (!name) {
        showError(nameInput, text("请填写任务名称。", "Enter a task name."));
        return undefined;
      }
      const schedule = scheduleInput.value.trim();
      if (!schedule) {
        showError(scheduleInput, text("请填写日程。", "Enter a schedule."));
        return undefined;
      }
      const type = selectedKind();
      let payload: CronPromptPayload | CronShellPayload | CronHttpPayload;
      if (type === "prompt") {
        const prompt = promptInput.value.trim();
        if (!prompt) {
          showError(promptInput, text("请填写提示词。", "Enter a prompt."));
          return undefined;
        }
        const workdir = promptWorkdir.value.trim();
        const providerId = promptProvider.value.trim();
        const model = promptModel.value.trim();
        payload = {
          prompt,
          ...(workdir ? { workdir } : {}),
          ...(providerId ? { providerId } : {}),
          ...(model ? { model } : {}),
        };
      } else if (type === "shell") {
        const command = shellCommand.value.trim();
        const workdir = shellWorkdir.value.trim();
        if (!command) {
          showError(
            shellCommand,
            text("请填写 Shell 命令。", "Enter a Shell command."),
          );
          return undefined;
        }
        if (!workdir) {
          showError(
            shellWorkdir,
            text("请填写 Shell 工作目录。", "Enter a Shell working directory."),
          );
          return undefined;
        }
        payload = { command, workdir };
      } else {
        const url = httpUrl.value.trim();
        let parsedUrl: URL | undefined;
        try {
          parsedUrl = new URL(url);
        } catch {
          // The inline validation below deliberately does not echo the URL.
        }
        if (
          !url ||
          !httpUrl.checkValidity() ||
          !parsedUrl ||
          parsedUrl.protocol !== "https:" ||
          !parsedUrl.hostname ||
          parsedUrl.username ||
          parsedUrl.password ||
          parsedUrl.hash ||
          isDisallowedCronHttpHostname(parsedUrl.hostname)
        ) {
          showError(
            httpUrl,
            text(
              "请输入公共 HTTPS 请求 URL；native 会拒绝 localhost、私有网络、回环和本地链路地址。",
              "Enter a public HTTPS request URL; native rejects localhost, private-network, loopback, and link-local addresses.",
            ),
          );
          return undefined;
        }
        const timeoutMs = Number(httpTimeout.value);
        if (
          !Number.isSafeInteger(timeoutMs) ||
          timeoutMs < 1_000 ||
          timeoutMs > 30_000
        ) {
          showError(
            httpTimeout,
            text(
              "HTTP 超时必须是 1000 到 30000 之间的整数。",
              "HTTP timeout must be an integer from 1000 to 30000.",
            ),
          );
          return undefined;
        }
        const headers = parseHttpHeaders();
        if (!headers) return undefined;
        payload = {
          url,
          method: httpMethod.value,
          headers,
          ...(httpBody.value.length ? { body: httpBody.value } : {}),
          timeoutMs,
        };
      }
      return {
        ...(editing ? { id: editing.id } : {}),
        name,
        type,
        schedule,
        payload,
        // Native enforces this independently. A Save confirmation only permits
        // persistence; every Cron kind requires a separate native Enable.
        enabled: false,
      };
    };
    const open = (entry?: CronJobSummary) => {
      editing = entry;
      previousFocus =
        document.activeElement instanceof HTMLElement
          ? document.activeElement
          : null;
      const jobKind: CronJobKind =
        entry?.type === "shell" || entry?.type === "http"
          ? entry.type
          : "prompt";
      heading.textContent = entry
        ? text("编辑定时任务", "Edit scheduled task")
        : text("新建定时任务", "New scheduled task");
      nameInput.value = entry?.name ?? "";
      kindInput.value = jobKind;
      scheduleInput.value = entry?.schedule ?? "hourly";
      enabledInput.checked = false;
      clearPayloadDraft();
      payloadNotice.textContent = entry
        ? text(
            "出于安全原因，已保存的 payload 不会显示。要更新此任务，必须重新输入完整 payload。",
            "For security, the saved payload is never shown. To update this task, enter a complete payload again.",
          )
        : text(
            "仅本次输入的 payload 会提交给 native 保存；列表和运行记录只显示脱敏摘要。",
            "Only the payload entered this time is sent to native for saving; lists and run history show redacted summaries only.",
          );
      alert.textContent = "";
      renderPayloadFields();
      if (!dialog.open) dialog.showModal();
      window.setTimeout(() => nameInput.focus(), 0);
    };
    const saveJob = async () => {
      const input = buildInput();
      if (!input) return;
      if (!sensitiveConfirmation.checked) {
        showError(
          sensitiveConfirmation,
          text(
            "请先确认你理解此任务的影响。",
            "Confirm that you understand this task's effect first.",
          ),
        );
        return;
      }
      const saveConfirmation =
        input.type === "shell"
          ? {
              title: text("确认保存 Shell 任务", "Confirm Shell task"),
              message: text(
                "保存不会立即运行命令。Shell 任务会以当前用户权限运行，并会保存但默认禁用；请稍后在列表中另行确认启用。确认保存吗？",
                "Saving does not run the command now. A Shell task runs with current user permissions and will be saved disabled; confirm a separate enable from the list later. Save it?",
              ),
            }
          : input.type === "http"
            ? {
                title: text("确认保存 HTTP 任务", "Confirm HTTP task"),
                message: text(
                  "保存不会立即发送请求。只允许公共 HTTPS 目标；native 会拒绝 localhost、私有网络、回环和本地链路地址。HTTP 任务会保存但默认禁用；请稍后在列表中另行确认启用。确认保存吗？",
                  "Saving does not send a request now. Only public HTTPS targets are allowed; native rejects localhost, private-network, loopback, and link-local addresses. An HTTP task is saved disabled; confirm a separate enable from the list later. Save it?",
                ),
              }
            : {
                title: text("确认保存 Prompt 任务", "Confirm Prompt task"),
                message: text(
                  "保存不会立即调用供应商。Prompt 任务会保存但默认禁用；之后启用或立即运行时仍会出现 Windows 原生确认。确认保存吗？",
                  "Saving does not call the provider now. A Prompt task is saved disabled; enable and Run now will still require Windows-native confirmation. Save it?",
                ),
              };
      if (
        !(await requestAppConfirm({
          ...saveConfirmation,
          confirmLabel: text("保存", "Save"),
          cancelLabel: text("取消", "Cancel"),
          danger: false,
        }))
      )
        return;
      saving = true;
      setEditorBusy(true);
      let phase: "schedule" | "save" = "schedule";
      try {
        await invoke<void>("cron_schedule_validate", {
          schedule: input.schedule,
        });
        phase = "save";
        await invoke<CronJobSummary>("cron_upsert", {
          input,
        });
        dialog.close();
        toast(
          input.type === "http"
            ? text(
                "HTTP 定时任务已保存但默认禁用；本次未发送请求，请在列表中另行确认启用。",
                "HTTP task saved but disabled by default; no request was sent now, and enable it separately from the list.",
              )
            : input.type === "shell"
              ? text(
                  "Shell 定时任务已保存但默认禁用；本次未运行命令，请在列表中另行确认启用。",
                  "Shell task saved but disabled by default; no command ran now, and enable it separately from the list.",
                )
              : text(
                  "Prompt 定时任务已保存但默认禁用；本次未调用供应商，请在列表中另行确认启用。",
                  "Prompt task saved but disabled by default; no provider was called now, and enable it separately from the list.",
                ),
        );
        await load();
      } catch (error) {
        if (phase === "schedule") {
          showError(
            scheduleInput,
            `${text("日程无效：", "Invalid schedule: ")}${errorText(error)}`,
          );
        } else {
          // Do not reflect native write errors verbatim: an implementation may
          // include data derived from the newly entered sensitive payload.
          showError(
            nameInput,
            text(
              "无法保存定时任务。请检查名称、日程和 payload 字段，然后重试。",
              "Unable to save the scheduled task. Check the name, schedule, and payload fields, then try again.",
            ),
          );
        }
      } finally {
        saving = false;
        setEditorBusy(false);
      }
    };
    const invalidateSensitiveConfirmation = () => {
      sensitiveConfirmation.checked = false;
    };
    kindInput.addEventListener("change", () => {
      sensitiveConfirmation.checked = false;
      renderPayloadFields();
    });
    for (const control of [
      nameInput,
      scheduleInput,
      enabledInput,
      shellCommand,
      shellWorkdir,
      httpUrl,
      httpMethod,
      httpHeaders,
      httpBody,
      httpTimeout,
    ]) {
      control.addEventListener("input", invalidateSensitiveConfirmation);
      control.addEventListener("change", invalidateSensitiveConfirmation);
    }
    cancel.addEventListener("click", () => dialog.close());
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      void saveJob();
    });
    dialog.addEventListener("cancel", (event) => {
      if (saving) event.preventDefault();
    });
    dialog.addEventListener("close", () => {
      editing = undefined;
      nameInput.value = "";
      kindInput.value = "prompt";
      scheduleInput.value = "hourly";
      enabledInput.checked = true;
      clearPayloadDraft();
      renderPayloadFields();
      previousFocus?.focus({ preventScroll: true });
    });
    return { open };
  })();

  const showRuns = async (
    job: CronJobSummary,
    control: HTMLButtonElement,
    card: HTMLElement,
  ) => {
    const existing = card.querySelector<HTMLDetailsElement>(
      "details.novavei-service-details",
    );
    if (existing) {
      existing.open = !existing.open;
      return;
    }
    setBusy(control, true, text("读取中…", "Loading…"));
    try {
      const runs = await invoke<CronRunSummary[]>("cron_runs", {
        jobId: job.id,
        limit: 20,
      });
      const details = element("details", {
        className: "novavei-service-details",
      });
      details.open = true;
      details.append(
        element("summary", {
          text: text(
            `无敏感运行记录（${runs.length}）`,
            `Non-sensitive run history (${runs.length})`,
          ),
        }),
      );
      if (!runs.length) {
        details.append(
          element("p", {
            className: "novavei-service-note",
            text: text("尚无运行记录。", "No run history yet."),
          }),
        );
      } else {
        const runList = element("ul", { className: "novavei-service-list" });
        for (const run of runs.slice(0, 20)) {
          const item = element("li", { className: "novavei-service-note" });
          const finished = run.completedAt
            ? formatTime(run.completedAt)
            : text("进行中", "In progress");
          const signals = [
            run.hasOutput
              ? text("有输出", "Has output")
              : text("无输出", "No output"),
            run.hasError
              ? text("标记为错误", "Marked error")
              : text("未标记错误", "No error flag"),
          ];
          item.textContent = `${run.status} · ${finished} · ${signals.join(" · ")}`;
          runList.append(item);
        }
        details.append(runList);
      }
      card.append(details);
    } catch (error) {
      status(
        statusLine,
        `${text("无法读取运行记录：", "Unable to load run history: ")}${errorText(error)}`,
        "error",
      );
    } finally {
      setBusy(control, false);
    }
  };

  const runNow = async (job: CronJobSummary, control: HTMLButtonElement) => {
    const confirmation =
      job.type === "shell"
        ? text(
            "立即运行会在本机执行已保存的 Shell 命令。命令内容与输出不会显示。确定继续吗？",
            "Run now will execute the saved Shell command on this machine. The command and output will not be shown. Continue?",
          )
        : job.type === "http"
          ? text(
              "立即运行会向已保存的 HTTP 目标发送请求。URL、请求头和响应内容都不会显示。确定继续吗？",
              "Run now will send a request to the saved HTTP target. The URL, headers, and response body will not be shown. Continue?",
            )
          : text(
              "立即运行会用已保存供应商发起一次无工具补全。提示词与模型输出不会显示。确定继续吗？",
              "Run now will start one tool-less completion with the saved provider. The prompt and model output will not be shown. Continue?",
            );
    if (
      !(await requestAppConfirm({
        title: text("立即运行", "Run now"),
        message: confirmation,
        confirmLabel: text("运行", "Run"),
        cancelLabel: text("取消", "Cancel"),
        danger: false,
      }))
    ) {
      status(statusLine, text("已取消立即运行。", "Run now cancelled."));
      return;
    }
    setBusy(
      control,
      true,
      job.type === "http"
        ? text("发送中…", "Sending…")
        : text("执行中…", "Running…"),
    );
    try {
      const response = await invoke<CronRunNowSummary>("cron_run_now", {
        jobId: job.id,
      });
      await load();
      if (job.type === "http") {
        const result = response.http;
        const statusCode = result?.status ? ` ${result.status}` : "";
        status(
          statusLine,
          result?.success
            ? text(
                `HTTP 任务已完成${statusCode}；响应内容未显示。`,
                `HTTP task completed${statusCode}; response content is not shown.`,
              )
            : text(
                `HTTP 任务未成功完成${statusCode}；响应内容与错误详情未显示。`,
                `HTTP task did not complete successfully${statusCode}; response content and error detail are not shown.`,
              ),
          result?.success ? "success" : "error",
        );
      } else {
        const label =
          job.type === "shell"
            ? text("Shell", "Shell")
            : text("提示词", "Prompt");
        const succeeded = response.run.status === "completed";
        status(
          statusLine,
          succeeded
            ? text(
                `${label} 任务已在本机完成（${response.run.status}）；输出与 payload 未进入界面。`,
                `${label} task completed natively (${response.run.status}); output and payload never entered the UI.`,
              )
            : text(
                `${label} 任务未成功完成（${response.run.status}）；输出、错误详情与 payload 未进入界面。`,
                `${label} task did not complete successfully (${response.run.status}); output, error detail, and payload never entered the UI.`,
              ),
          succeeded ? "success" : "error",
        );
      }
    } catch (error) {
      status(
        statusLine,
        `${text("立即运行失败：", "Run now failed: ")}${errorText(error)}`,
        "error",
      );
    } finally {
      setBusy(control, false);
    }
  };

  const removeJob = async (job: CronJobSummary, control: HTMLButtonElement) => {
    if (
      !(await requestAppConfirm({
        title: text("删除定时任务", "Delete scheduled task"),
        message: text(
          `确定删除“${job.name}”吗？已保存的 payload 和运行记录也会一并删除，此操作无法撤销。`,
          `Delete “${job.name}”? Its saved payload and run history will also be removed. This cannot be undone.`,
        ),
        confirmLabel: text("删除", "Delete"),
        cancelLabel: text("取消", "Cancel"),
        danger: true,
      }))
    )
      return;
    setBusy(control, true, text("删除中…", "Deleting…"));
    try {
      await invoke<CronJobSummary>("cron_delete", { id: job.id });
      toast(text("定时任务已删除。", "Scheduled task deleted."));
      status(
        statusLine,
        text("定时任务已删除。", "Scheduled task deleted."),
        "success",
      );
      await load();
    } catch (error) {
      status(
        statusLine,
        `${text("删除任务失败：", "Unable to delete task: ")}${errorText(error)}`,
        "error",
      );
    } finally {
      setBusy(control, false);
    }
  };

  const render = () => {
    const active = jobs.filter((job) => job.enabled).length;
    total.textContent = String(jobs.length);
    enabled.textContent = String(active);
    paused.textContent = String(jobs.length - active);
    list.replaceChildren();
    if (!jobs.length) {
      list.append(
        emptyCard(
          text("暂无定时任务", "No scheduled tasks"),
          text(
            "本页只展示脱敏摘要。使用“新建任务”输入 payload；保存后它不会再回显。",
            "This page shows redacted summaries only. Use New task to enter a payload; it will not be shown again after saving.",
          ),
        ),
      );
      return;
    }
    for (const job of jobs.slice(0, MAX_RENDERED_ITEMS)) {
      const card = element("article", { className: "cron-job" });
      const copy = element("div");
      copy.append(
        element("strong", { text: job.name }),
        element("small", { text: `${job.type} · ${job.schedule}` }),
      );
      const badge = element("span", {
        className: job.enabled ? "pill" : "pill wait",
        text: job.enabled
          ? text("已启用", "Enabled")
          : text("已暂停", "Paused"),
      });
      const head = element("div", { className: "novavei-service-row" });
      head.append(copy, badge);
      const meta = element("div", { className: "cron-job-meta" });
      const next = element("span", {
        className: "tag",
        text: `${text("下次", "Next")}: ${job.enabled ? formatTime(job.nextRunAt) : text("已暂停", "Paused")}`,
      });
      const last = element("span", {
        className: "tag",
        text: `${text("上次", "Last")}: ${formatTime(job.lastRunAt)}`,
      });
      const safe = element("span", {
        className: "tag",
        text: text("payload 已脱敏", "Payload redacted"),
      });
      meta.append(next, last, safe);
      const actions = element("div", { className: "cron-job-actions" });
      const toggle = button(
        job.enabled ? text("暂停", "Pause") : text("启用", "Enable"),
        job.enabled ? "btn" : "btn primary",
        job.enabled
          ? text(`暂停 ${job.name}`, `Pause ${job.name}`)
          : text(`启用 ${job.name}`, `Enable ${job.name}`),
      );
      const logs = button(
        text("运行记录", "Run history"),
        "btn",
        text(
          `查看 ${job.name} 的无敏感运行记录`,
          `View non-sensitive run history for ${job.name}`,
        ),
      );
      const run = button(
        text("立即运行", "Run now"),
        "btn",
        text(`立即运行 ${job.name}`, `Run ${job.name} now`),
      );
      const edit = button(
        text("编辑", "Edit"),
        "btn",
        text(`编辑 ${job.name}`, `Edit ${job.name}`),
      );
      const remove = button(
        text("删除", "Delete"),
        "btn",
        text(`删除 ${job.name}`, `Delete ${job.name}`),
      );
      toggle.addEventListener("click", () => {
        void (async () => {
          if (
            !job.enabled &&
            !(await requestAppConfirm({
              title: text("启用定时任务", "Enable scheduled task"),
              message:
                job.type === "shell"
                  ? text(
                      "启用后，NovaVei 打开期间到期的 Shell 日程会以当前用户权限在本机自动运行命令。命令内容不会显示。确定启用吗？",
                      "When enabled, a due Shell schedule executes natively with current user permissions while NovaVei is open. The command will not be shown. Enable it?",
                    )
                  : job.type === "http"
                    ? text(
                        "启用后，NovaVei 打开期间到期的公共 HTTPS HTTP 日程会自动发送。URL 和请求头不会显示。确定启用吗？",
                        "When enabled, a due public HTTPS HTTP schedule is sent automatically while NovaVei is open. The URL and headers will not be shown. Enable it?",
                      )
                    : text(
                        "启用后，NovaVei 打开期间到期的 Prompt 日程会使用已保存供应商自动请求。提示词与模型输出不会显示。确定启用吗？",
                        "When enabled, a due Prompt schedule calls the saved provider while NovaVei is open. The prompt and model output will not be shown. Enable it?",
                      ),
              confirmLabel: text("启用", "Enable"),
              cancelLabel: text("取消", "Cancel"),
              danger: false,
            }))
          ) {
            status(
              statusLine,
              text("已取消启用任务。", "Enable task cancelled."),
            );
            return;
          }
          setBusy(toggle, true, text("保存中…", "Saving…"));
          try {
            await invoke<CronJobSummary>("cron_set_enabled", {
              id: job.id,
              enabled: !job.enabled,
            });
            await load();
            status(
              statusLine,
              job.enabled
                ? text("任务已暂停。", "Task paused.")
                : text("任务已启用。", "Task enabled."),
              "success",
            );
          } catch (error) {
            status(
              statusLine,
              `${text("无法更新任务状态：", "Unable to update task status: ")}${errorText(error)}`,
              "error",
            );
          } finally {
            setBusy(toggle, false);
          }
        })();
      });
      logs.addEventListener("click", () => void showRuns(job, logs, card));
      run.addEventListener("click", () => void runNow(job, run));
      edit.addEventListener("click", () => editor.open(job));
      remove.addEventListener("click", () => void removeJob(job, remove));
      actions.append(toggle, run, edit, logs, remove);
      card.append(head, meta, actions);
      list.append(card);
    }
  };

  const load = async (schedulerUpdate?: CronSchedulerUpdate) => {
    setBusy(refresh, true, text("刷新中…", "Refreshing…"));
    status(
      statusLine,
      text(
        "正在读取脱敏的定时任务摘要…",
        "Loading redacted scheduled task summaries…",
      ),
    );
    try {
      jobs = await invoke<CronJobSummary[]>("cron_list");
      if (schedulerUpdate) {
        status(
          statusLine,
          text(
            `调度器已领取 ${schedulerUpdate.claimed} 个到期任务：运行中 ${schedulerUpdate.running}，完成 ${schedulerUpdate.completed}，失败 ${schedulerUpdate.failed}；敏感内容未进入界面。`,
            `Scheduler claimed ${schedulerUpdate.claimed} due jobs: ${schedulerUpdate.running} running, ${schedulerUpdate.completed} completed, ${schedulerUpdate.failed} failed; sensitive content never entered the UI.`,
          ),
          schedulerUpdate.failed > 0 ? "error" : "success",
        );
      } else {
        status(
          statusLine,
          text(
            `已读取 ${jobs.length} 个任务；payload 从未进入界面。`,
            `${jobs.length} tasks loaded; payload never enters the UI.`,
          ),
          "success",
        );
      }
    } catch (error) {
      jobs = [];
      status(
        statusLine,
        `${text("定时任务服务不可用：", "Scheduled task service unavailable: ")}${errorText(error)}`,
        "error",
      );
    } finally {
      setBusy(refresh, false);
      render();
    }
  };

  const toolbar = element("div", { className: "cron-toolbar" });
  const lead = element("div", { className: "cron-lead" });
  const cronTitle = element("h3", {
    text: text("定时任务", "Scheduled tasks"),
  });
  const cronLeadCopy = element("p", {
    text: text(
      "任务与运行记录仅使用原生返回的脱敏摘要。NovaVei 打开期间，HTTP / Shell / Prompt 日程到期后都会在本机自动执行；已保存的 payload 与运行输出不会显示或回传到界面。",
      "Tasks and run history use native redacted summaries only. While NovaVei is open, due HTTP, Shell, and Prompt schedules all execute natively; saved payload and run output never enter the UI.",
    ),
  });
  lead.append(cronTitle, cronLeadCopy);
  const toolbarActions = element("div", { className: "row-actions" });
  const create = button(
    text("新建任务", "New task"),
    "btn primary",
    text("新建定时任务", "Create scheduled task"),
  );
  toolbarActions.append(create, refresh);
  toolbar.append(lead, toolbarActions);
  const stats = element("div", {
    className: "cron-stats",
    attrs: { "aria-label": text("任务概览", "Task overview") },
  });
  const totalLabel = element("span", { text: text("全部任务", "All tasks") });
  const enabledLabel = element("span", { text: text("已启用", "Enabled") });
  const pausedLabel = element("span", { text: text("已暂停", "Paused") });
  for (const [label, value] of [
    [totalLabel, total],
    [enabledLabel, enabled],
    [pausedLabel, paused],
  ] as const) {
    const metric = element("div", { className: "cron-stat" });
    metric.append(value, label);
    stats.append(metric);
  }
  panel.replaceChildren(toolbar, stats, statusLine, list);
  const syncCronChrome = () => {
    cronTitle.textContent = text("定时任务", "Scheduled tasks");
    cronLeadCopy.textContent = text(
      "任务与运行记录仅使用原生返回的脱敏摘要。NovaVei 打开期间，HTTP / Shell / Prompt 日程到期后都会在本机自动执行；已保存的 payload 与运行输出不会显示或回传到界面。",
      "Tasks and run history use native redacted summaries only. While NovaVei is open, due HTTP, Shell, and Prompt schedules all execute natively; saved payload and run output never enter the UI.",
    );
    labelControl(
      create,
      text("新建任务", "New task"),
      text("新建定时任务", "Create scheduled task"),
    );
    labelControl(
      refresh,
      text("刷新", "Refresh"),
      text("刷新定时任务", "Refresh scheduled tasks"),
    );
    totalLabel.textContent = text("全部任务", "All tasks");
    enabledLabel.textContent = text("已启用", "Enabled");
    pausedLabel.textContent = text("已暂停", "Paused");
    stats.setAttribute("aria-label", text("任务概览", "Task overview"));
    list.setAttribute(
      "aria-label",
      text("定时任务列表", "Scheduled task list"),
    );
  };
  create.addEventListener("click", () => editor.open());
  refresh.addEventListener("click", () => void load());
  document.addEventListener("click", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest('button[data-settings="cron"]')) void load();
  });
  onServiceLanguageChange(() => {
    syncCronChrome();
    void load();
  });
  if (listen) {
    void listen<CronSchedulerUpdate>("cron:scheduler-update", ({ payload }) => {
      if (payload.status === "error") {
        status(
          statusLine,
          text(
            "本机调度器暂时无法检查到期任务。敏感错误详情未进入界面。",
            "The local scheduler could not check due jobs. Sensitive error details did not enter the UI.",
          ),
          "error",
        );
        return;
      }
      if (payload.claimed > 0) void load(payload);
    }).catch(() => undefined);
  }
  syncCronChrome();
  status(
    statusLine,
    text("正在连接本机定时任务…", "Connecting to native scheduled tasks…"),
  );
  void load();
}

function installLanguageObserver() {
  const root = document.documentElement;
  if (root.dataset.novaveiServicesLanguageObserver === "true") return;
  root.dataset.novaveiServicesLanguageObserver = "true";
  let previous = root.lang;
  new MutationObserver(() => {
    if (root.lang === previous) return;
    previous = root.lang;
    window.dispatchEvent(new Event("novavei:service-language-changed"));
  }).observe(root, { attributes: true, attributeFilter: ["lang"] });
}

export function installLocalServices() {
  if (!invokeApi()) return;
  installServiceStyles();
  installLanguageObserver();
  installSkills();
  installMcp();
  installMemory();
  installCron();
}
