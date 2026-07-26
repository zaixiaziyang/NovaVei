import { requestAppConfirm, requestAppPrompt } from "./app-dialogs";
import { reasoningFromUiValue } from "./dom";
import {
  PiWorkflowRunner,
  type WorkflowModel,
  type WorkflowRunResult,
} from "./workflow-runner";

type UnknownRecord = Record<string, unknown>;
type Invoke = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

type ModelChoice = WorkflowModel & {
  value: string;
};

export type CouncilExpert = {
  id: string;
  name: string;
  prompt: string;
};

/** A reusable single-chat persona, separate from a Council seat. */
export type ChatCharacter = {
  id: string;
  name: string;
  prompt: string;
};

/**
 * A deliberately prompt-free projection for the composer picker.  Choosing an
 * item still resolves the full record immediately before the message runs.
 */
export type CouncilCommandChoice = {
  id: string;
  name: string;
};

export type CouncilCommandChoices = {
  experts: CouncilCommandChoice[];
  characters: CouncilCommandChoice[];
};

type CouncilTeam = {
  id: string;
  name: string;
  expertIds: string[];
  promptId?: string;
};

type CouncilPrompt = {
  id: string;
  name: string;
  prompt: string;
};

type CouncilSettings = {
  version: 1;
  experts: CouncilExpert[];
  characters: ChatCharacter[];
  teams: CouncilTeam[];
  prompts: CouncilPrompt[];
};

type CouncilStage = "independent" | "cross_review" | "questioning";

type CouncilParticipantResult = {
  expert: CouncilExpert;
  model: ModelChoice;
  stages: Partial<Record<CouncilStage, WorkflowRunResult>>;
};

type CouncilRunSnapshot = {
  topic: string;
  startedAt: string;
  participants: Map<string, CouncilParticipantResult>;
  synthesis: string;
};

const COUNCIL_EXPERTS: readonly CouncilExpert[] = [
  {
    id: "architecture",
    name: "架构主席",
    prompt:
      "你是架构评审专家。聚焦系统边界、依赖方向、状态契约、演进成本和可回滚性。给出明确结论与可执行建议。",
  },
  {
    id: "security",
    name: "安全顾问",
    prompt:
      "你是安全评审专家。检查信任边界、凭据、输入验证、权限最小化、进程与文件系统隔离，并按严重度给出修复建议。",
  },
  {
    id: "engineering",
    name: "工程顾问",
    prompt:
      "你是资深工程实现专家。评估落地复杂度、失败模式、测试策略、维护成本和交付顺序，给出具体实现建议。",
  },
  {
    id: "product",
    name: "产品顾问",
    prompt:
      "你是产品评审专家。评估用户价值、工作流完整性、可理解性、误导风险和取舍，给出明确优先级。",
  },
];

const DEFAULT_COUNCIL_SETTINGS: CouncilSettings = {
  version: 1,
  experts: COUNCIL_EXPERTS.map((expert) => ({ ...expert })),
  characters: [],
  teams: [
    {
      id: "architecture-review",
      name: "架构评审团",
      expertIds: ["architecture", "security", "engineering", "product"],
    },
    {
      id: "release-gate",
      name: "发布把关团",
      expertIds: ["security", "engineering", "product"],
    },
  ],
  prompts: [],
};

const COUNCIL_SETTINGS_KIND = "novavei-council-settings";
const MIN_COMPARE_TARGETS = 2;
const MAX_COMPARE_TARGETS = 4;
const MAX_COUNCIL_EVIDENCE = 36_000;
const COUNCIL_STAGES: readonly CouncilStage[] = [
  "independent",
  "cross_review",
  "questioning",
];

function workflowLanguage() {
  return document.documentElement.lang.toLowerCase().startsWith("en")
    ? "en"
    : "zh";
}

function workflowText(
  zh: string,
  en: string,
  values: Record<string, string | number> = {},
) {
  const template = workflowLanguage() === "en" ? en : zh;
  return template.replace(/\{(\w+)\}/g, (token, key) =>
    String(values[key] ?? token),
  );
}

function requiredValue<T>(value: T | null | undefined, label: string): T {
  if (value == null) throw new Error(`Required ${label} is unavailable`);
  return value;
}

function observeWorkflowLanguage(refresh: () => void) {
  const observer = new MutationObserver((changes) => {
    if (
      changes.some(
        (change) =>
          change.type === "attributes" && change.attributeName === "lang",
      )
    )
      refresh();
  });
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["lang"],
  });
  return observer;
}

function councilStageLabel(stage: CouncilStage) {
  switch (stage) {
    case "independent":
      return workflowText("独立陈述", "Independent review");
    case "cross_review":
      return workflowText("交叉评议", "Cross-review");
    case "questioning":
      return workflowText("质询回应", "Questions and responses");
  }
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

function boundedText(value: unknown, max: number) {
  return typeof value === "string" ? value.trim().slice(0, max) : "";
}

function councilId(prefix: string) {
  const suffix =
    globalThis.crypto?.randomUUID?.() ??
    `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  return `${prefix}-${suffix}`;
}

function cloneCouncilSettings(settings: CouncilSettings): CouncilSettings {
  return {
    version: 1,
    experts: settings.experts.map((expert) => ({ ...expert })),
    characters: settings.characters.map((character) => ({ ...character })),
    teams: settings.teams.map((team) => ({
      ...team,
      expertIds: [...team.expertIds],
    })),
    prompts: settings.prompts.map((prompt) => ({ ...prompt })),
  };
}

function normalizeCouncilSettings(value: unknown): CouncilSettings {
  const record = asRecord(value);
  if (!record) return cloneCouncilSettings(DEFAULT_COUNCIL_SETTINGS);

  const experts = (Array.isArray(record.experts) ? record.experts : [])
    .flatMap((value) => {
      const expert = asRecord(value);
      const id = boundedText(expert?.id, 96);
      const name = boundedText(expert?.name, 120);
      const prompt = boundedText(expert?.prompt, 12_000);
      return id && name && prompt ? [{ id, name, prompt }] : [];
    })
    .filter(
      (expert, index, all) =>
        all.findIndex((candidate) => candidate.id === expert.id) === index,
    )
    .slice(0, 24);
  const normalizedExperts = experts.length
    ? experts
    : cloneCouncilSettings(DEFAULT_COUNCIL_SETTINGS).experts;
  const expertIds = new Set(normalizedExperts.map((expert) => expert.id));

  const characters = (Array.isArray(record.characters) ? record.characters : [])
    .flatMap((value) => {
      const character = asRecord(value);
      const id = boundedText(character?.id, 96);
      const name = boundedText(character?.name, 120);
      const prompt = boundedText(character?.prompt, 12_000);
      return id && name && prompt ? [{ id, name, prompt }] : [];
    })
    .filter(
      (character, index, all) =>
        all.findIndex((candidate) => candidate.id === character.id) === index,
    )
    .slice(0, 24);

  const prompts = (Array.isArray(record.prompts) ? record.prompts : [])
    .flatMap((value) => {
      const promptRecord = asRecord(value);
      const id = boundedText(promptRecord?.id, 96);
      const name = boundedText(promptRecord?.name, 120);
      const prompt = boundedText(promptRecord?.prompt, 12_000);
      return id && name && prompt ? [{ id, name, prompt }] : [];
    })
    .filter(
      (prompt, index, all) =>
        all.findIndex((candidate) => candidate.id === prompt.id) === index,
    )
    .slice(0, 24);
  const promptIds = new Set(prompts.map((prompt) => prompt.id));

  const teams = (Array.isArray(record.teams) ? record.teams : [])
    .flatMap((value) => {
      const team = asRecord(value);
      const id = boundedText(team?.id, 96);
      const name = boundedText(team?.name, 120);
      const members = (Array.isArray(team?.expertIds) ? team.expertIds : [])
        .map((member) => boundedText(member, 96))
        .filter(
          (member, index, all) =>
            expertIds.has(member) && all.indexOf(member) === index,
        )
        .slice(0, 12);
      const promptId = boundedText(team?.promptId, 96);
      return id && name && members.length >= 2
        ? [
            {
              id,
              name,
              expertIds: members,
              ...(promptId && promptIds.has(promptId) ? { promptId } : {}),
            },
          ]
        : [];
    })
    .filter(
      (team, index, all) =>
        all.findIndex((candidate) => candidate.id === team.id) === index,
    )
    .slice(0, 24);

  return {
    version: 1,
    experts: normalizedExperts,
    characters,
    teams,
    prompts,
  };
}

function councilSettingsFromAgents(agents: unknown) {
  if (Array.isArray(agents)) {
    const stored = agents
      .map(asRecord)
      .find((item) => item?.kind === COUNCIL_SETTINGS_KIND);
    return normalizeCouncilSettings(stored?.council);
  }
  return normalizeCouncilSettings(asRecord(agents)?.council);
}

function mergeCouncilSettingsIntoAgents(
  agents: unknown,
  council: CouncilSettings,
): unknown {
  const payload = cloneCouncilSettings(council);
  if (Array.isArray(agents)) {
    return [
      ...agents.filter(
        (item) => asRecord(item)?.kind !== COUNCIL_SETTINGS_KIND,
      ),
      {
        id: COUNCIL_SETTINGS_KIND,
        kind: COUNCIL_SETTINGS_KIND,
        council: payload,
      },
    ];
  }
  const record = asRecord(agents);
  return { ...(record ?? {}), council: payload };
}

async function loadCouncilSettings(invoke: Invoke) {
  const settings = await invoke<UnknownRecord>("settings_load_all");
  return councilSettingsFromAgents(settings.agents);
}

/**
 * List the persisted expert and character names for the composer picker
 * without leaking their prompts into the DOM.  The submit path reloads and
 * resolves the selected record from the same source of truth.
 */
export async function listCouncilCommandChoices(
  invoke: Invoke,
): Promise<CouncilCommandChoices> {
  const settings = await loadCouncilSettings(invoke);
  return {
    experts: settings.experts.map(({ id, name }) => ({ id, name })),
    characters: settings.characters.map(({ id, name }) => ({ id, name })),
  };
}

async function saveCouncilSettings(invoke: Invoke, council: CouncilSettings) {
  const latest = await invoke<UnknownRecord>("settings_load_all");
  await invoke("settings_save_agents", {
    payload: mergeCouncilSettingsIntoAgents(latest.agents, council),
  });
}

/**
 * Resolve one configured Council expert for a focused chat command. This
 * reuses the same persisted source as Council instead of keeping a divergent
 * client-side list of expert prompts.
 */
export async function resolveCouncilExpert(
  invoke: Invoke,
  selector: string,
): Promise<CouncilExpert> {
  const normalized = selector.trim().toLocaleLowerCase();
  if (!normalized) throw new Error("请指定专家名称或 ID");
  const settings = await loadCouncilSettings(invoke);
  const matches = settings.experts.filter((expert) =>
    [expert.id, expert.name].some(
      (value) => value.toLocaleLowerCase() === normalized,
    ),
  );
  if (matches.length === 1) return { ...matches[0] };
  const available = settings.experts
    .map((expert) => `${expert.name} (${expert.id})`)
    .join("、");
  if (matches.length > 1)
    throw new Error(`专家名称不唯一，请改用 ID。可用专家：${available}`);
  throw new Error(`未找到专家“${selector}”。可用专家：${available}`);
}

/**
 * Resolve one user-created character for an ordinary focused chat turn.
 * Characters intentionally share the protected local Council settings store
 * with experts, but never join a multi-expert Council unless explicitly
 * recreated as an expert there.
 */
export async function resolveChatCharacter(
  invoke: Invoke,
  selector: string,
): Promise<ChatCharacter> {
  const normalized = selector.trim().toLocaleLowerCase();
  if (!normalized) throw new Error("请指定角色名称或 ID");
  const settings = await loadCouncilSettings(invoke);
  const matches = settings.characters.filter((character) =>
    [character.id, character.name].some(
      (value) => value.toLocaleLowerCase() === normalized,
    ),
  );
  if (matches.length === 1) return { ...matches[0] };
  const available = settings.characters
    .map((character) => `${character.name} (${character.id})`)
    .join("、");
  if (matches.length > 1)
    throw new Error(`角色名称不唯一，请改用 ID。可用角色：${available}`);
  throw new Error(
    available
      ? `未找到角色“${selector}”。可用角色：${available}`
      : "还没有创建角色。请先在设置 → 专家 → 角色中创建一个。",
  );
}

function providerRecords(value: unknown): UnknownRecord[] {
  if (Array.isArray(value))
    return value
      .map(asRecord)
      .filter((item): item is UnknownRecord => Boolean(item));
  const record = asRecord(value);
  if (!record) return [];
  const nested = record.providers ?? record.items ?? record.customProviders;
  if (Array.isArray(nested)) return providerRecords(nested);
  return Object.entries(record).flatMap(([id, item]) => {
    const provider = asRecord(item);
    return provider
      ? [
          {
            ...provider,
            id: readString(provider, "id", "providerId", "provider_id") ?? id,
          },
        ]
      : [];
  });
}

/**
 * Match the native provider resolver's `activeModels` semantics.  Once either
 * spelling is present it is an allowlist, and malformed persisted data must
 * not turn into an implicit allow-all in a workflow picker.
 */
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
  provider: UnknownRecord,
  modelId: string,
  model?: UnknownRecord,
) {
  const boundedModelId = boundedProviderModelId(modelId);
  if (!boundedModelId) return false;
  const active = activeProviderModelIds(provider);
  return (
    active !== null &&
    (!active || active.has(boundedModelId)) &&
    model?.enabled !== false
  );
}

function modelRecords(provider: UnknownRecord) {
  const output: Array<{ id: string; label: string }> = [];
  const hasModels = Object.hasOwn(provider, "models");
  if (!hasModels) {
    const direct = readString(
      provider,
      "defaultModel",
      "default_model",
      "modelId",
      "model_id",
      "model",
    );
    if (direct && providerModelIsEnabled(provider, direct))
      output.push({ id: direct, label: direct });
    return output;
  }

  const models = provider.models;
  if (!Array.isArray(models)) return output;
  for (const value of models) {
    if (
      typeof value === "string" &&
      value.trim() &&
      providerModelIsEnabled(provider, value.trim())
    )
      output.push({ id: value.trim(), label: value.trim() });
    const model = asRecord(value);
    const id = readString(model, "id", "modelId", "model_id", "name");
    if (id && providerModelIsEnabled(provider, id, model))
      output.push({ id, label: readString(model, "label", "name") ?? id });
  }
  return output.filter(
    (item, index, all) =>
      all.findIndex((candidate) => candidate.id === item.id) === index,
  );
}

async function loadModels(invoke: Invoke): Promise<ModelChoice[]> {
  const settings = await invoke<UnknownRecord>("settings_load_all");
  const choices = providerRecords(settings.providers).flatMap((provider) => {
    if (provider.enabled === false) return [];
    const providerId = readString(provider, "id", "providerId", "provider_id");
    if (!providerId) return [];
    const providerLabel = readString(provider, "name", "label") ?? providerId;
    return modelRecords(provider).map((model) => ({
      providerId,
      model: model.id,
      label: `${providerLabel} · ${model.label}`,
      value: JSON.stringify([providerId, model.id]),
    }));
  });
  return choices.filter(
    (item, index, all) =>
      all.findIndex((candidate) => candidate.value === item.value) === index,
  );
}

function invokeOrUndefined() {
  return window.__TAURI__?.core?.invoke as Invoke | undefined;
}

function byId<T extends HTMLElement>(id: string) {
  return document.getElementById(id) as T | null;
}

function toast(message: string) {
  const target = byId<HTMLElement>("toast");
  if (!target) return;
  target.textContent = message;
  target.classList.add("show");
  window.setTimeout(() => target.classList.remove("show"), 2400);
}

function button(label: string, className = "btn") {
  const control = document.createElement("button");
  control.type = "button";
  control.className = className;
  control.textContent = label;
  return control;
}

function enableControl(control: HTMLElement) {
  control.removeAttribute("data-feature-unavailable");
  control.removeAttribute("aria-disabled");
}

function field(label: string, control: HTMLElement) {
  const wrapper = document.createElement("label");
  wrapper.className = "field";
  const title = document.createElement("span");
  title.textContent = label;
  wrapper.append(title, control);
  return wrapper;
}

function makeSelect(ariaLabel: string) {
  const select = document.createElement("select");
  select.className = "inline-select";
  select.setAttribute("aria-label", ariaLabel);
  return select;
}

function populateSelect(
  select: HTMLSelectElement,
  models: readonly ModelChoice[],
  selectedIndex: number,
) {
  // Refreshes are also triggered when the dock is reopened. Keep an explicit
  // user choice whenever its model is still configured instead of silently
  // resetting both sides of a comparison to the first two entries.
  const previousValue = select.value;
  select.replaceChildren();
  for (const model of models) {
    const option = document.createElement("option");
    option.value = model.value;
    option.textContent = model.label;
    select.appendChild(option);
  }
  select.disabled = models.length === 0;
  const selected =
    models.find((model) => model.value === previousValue) ??
    models[Math.min(selectedIndex, models.length - 1)];
  if (selected) select.value = selected.value;
}

function selectedModel(
  select: HTMLSelectElement,
  models: readonly ModelChoice[],
) {
  return models.find((model) => model.value === select.value);
}

function currentWorkdir() {
  return window.__novaveiHost?.getWorkdir()?.trim();
}

function currentReasoning() {
  return reasoningFromUiValue(byId<HTMLInputElement>("reasoningSlider")?.value);
}

function outputPanel(label: string) {
  const wrapper = document.createElement("section");
  wrapper.className = "section";
  const head = document.createElement("div");
  head.className = "section-head";
  const heading = document.createElement("h4");
  heading.textContent = label;
  const status = document.createElement("span");
  status.className = "pill wait";
  status.textContent = workflowText("待命", "Idle");
  status.setAttribute("role", "status");
  status.setAttribute("aria-live", "polite");
  head.append(heading, status);
  const body = document.createElement("pre");
  body.className = "file-preview";
  body.textContent = workflowText("尚无结果", "No results yet");
  body.tabIndex = 0;
  body.setAttribute("aria-label", workflowText("模型输出", "Model output"));
  wrapper.append(head, body);
  return { wrapper, heading, status, body };
}

function installCompare(invoke: Invoke, runner: PiWorkflowRunner) {
  const pane = document.querySelector<HTMLElement>(
    '.dock-pane[data-pane="compare"]',
  );
  if (!pane) return;
  pane.classList.add("model-compare-pane");
  pane.setAttribute("aria-busy", "false");

  const introduction = document.createElement("p");
  introduction.className = "dock-note";
  introduction.textContent = workflowText(
    "2–4 个模型会分别通过嵌入式 Pi 执行同一提示词；比较运行只读且不提供工具。",
    "Two to four models run the same prompt through embedded Pi. Comparison runs are read-only and have no tools.",
  );
  const prompt = document.createElement("textarea");
  prompt.rows = 4;
  prompt.maxLength = 20_000;
  prompt.placeholder = workflowText(
    "输入需要对比的问题",
    "Enter a prompt to compare",
  );
  prompt.setAttribute(
    "aria-label",
    workflowText("对比提示词", "Comparison prompt"),
  );
  const promptField = field(workflowText("提示词", "Prompt"), prompt);
  const targetList = document.createElement("div");
  targetList.className = "council-list";
  const addTarget = button(
    workflowText("添加对比模型", "Add comparison model"),
  );
  const refreshModelsButton = button(
    workflowText("刷新模型", "Refresh models"),
  );
  refreshModelsButton.setAttribute(
    "aria-label",
    workflowText(
      "刷新可用于对比的模型",
      "Refresh models available for comparison",
    ),
  );
  const run = button(
    workflowText("开始对比", "Start comparison"),
    "btn primary",
  );
  const cancel = button(workflowText("取消", "Cancel"));
  cancel.disabled = true;
  const actions = document.createElement("div");
  actions.className = "row-actions";
  actions.append(addTarget, refreshModelsButton, run, cancel);
  const availability = document.createElement("p");
  availability.className = "model-compare-status";
  availability.setAttribute("role", "status");
  availability.setAttribute("aria-live", "polite");
  pane.replaceChildren(
    introduction,
    promptField,
    targetList,
    actions,
    availability,
  );

  type CompareOutputState =
    | "idle"
    | "no_models"
    | "models_unavailable"
    | "starting"
    | "running"
    | "complete"
    | "cancelled"
    | "failed"
    | "cancelling";
  type CompareTarget = {
    id: string;
    wrapper: HTMLElement;
    select: HTMLSelectElement;
    remove: HTMLButtonElement;
    modelField: HTMLElement;
    output: ReturnType<typeof outputPanel>;
    outputState: CompareOutputState;
    emptyResult: boolean;
    outputModelLabel?: string;
  };
  const targets: CompareTarget[] = [];
  let models: ModelChoice[] = [];
  let running = false;
  let modelsLoading = false;
  let modelsLoadFailed = false;
  let refreshSerial = 0;
  let cancellationRequested = false;

  const targetLabel = (index: number) =>
    workflowText("模型 {index}", "Model {index}", { index: index + 1 });
  const renderCompareTargetStatus = (target: CompareTarget) => {
    const status = target.output.status;
    const text = {
      idle: workflowText("待命", "Idle"),
      no_models: workflowText(
        "未配置可运行模型",
        "No runnable models configured",
      ),
      models_unavailable: workflowText(
        "模型列表不可用",
        "Model list unavailable",
      ),
      starting: workflowText("启动中", "Starting"),
      running: workflowText("运行中", "Running"),
      complete: workflowText("完成", "Complete"),
      cancelled: workflowText("已取消", "Cancelled"),
      failed: workflowText("失败", "Failed"),
      cancelling: workflowText("取消中", "Cancelling"),
    }[target.outputState];
    status.textContent = text;
    status.className = ["running", "complete"].includes(target.outputState)
      ? "pill"
      : "pill wait";
  };
  const renderCompareTargetBody = (target: CompareTarget) => {
    if (target.outputState === "idle") {
      target.output.body.textContent = workflowText(
        "尚无结果",
        "No results yet",
      );
    } else if (target.outputState === "no_models") {
      target.output.body.textContent = workflowText(
        "未配置可运行模型",
        "No runnable models configured",
      );
    } else if (target.outputState === "models_unavailable") {
      target.output.body.textContent = workflowText(
        "模型列表不可用",
        "Model list unavailable",
      );
    } else if (target.outputState === "complete" && target.emptyResult) {
      target.output.body.textContent = workflowText(
        "模型未返回文本",
        "The model returned no text",
      );
    }
  };
  const setCompareTargetState = (
    target: CompareTarget,
    state: CompareOutputState,
  ) => {
    target.outputState = state;
    renderCompareTargetStatus(target);
  };

  const selectedTargets = () =>
    targets.map((target) => selectedModel(target.select, models));
  const canRun = () => {
    const selected = selectedTargets();
    return (
      targets.length >= MIN_COMPARE_TARGETS &&
      targets.length <= MAX_COMPARE_TARGETS &&
      selected.every((model): model is ModelChoice => Boolean(model)) &&
      new Set(selected.map((model) => model?.value)).size === targets.length
    );
  };
  const updateAvailability = () => {
    if (modelsLoading) {
      availability.textContent = workflowText(
        "正在读取已保存的模型…",
        "Reading saved models…",
      );
      availability.dataset.state = "loading";
    } else if (modelsLoadFailed) {
      availability.textContent = workflowText(
        "无法读取模型列表；请检查供应商设置后使用“重试读取模型”。",
        "Could not read the model list. Check provider settings, then use Retry model load.",
      );
      availability.dataset.state = "error";
    } else if (models.length < MIN_COMPARE_TARGETS) {
      availability.textContent = workflowText(
        "至少需要配置两个不同的模型。",
        "Configure at least two different models.",
      );
      availability.dataset.state = "empty";
    } else if (!canRun()) {
      availability.textContent = workflowText(
        "请选择 {count} 个不同的模型。",
        "Select {count} different models.",
        { count: targets.length },
      );
      availability.dataset.state = "error";
    } else {
      availability.textContent = workflowText(
        "已就绪：{targets} 路并行，{models} 个可选模型。",
        "Ready: {targets} parallel runs, {models} models available.",
        { targets: targets.length, models: models.length },
      );
      availability.dataset.state = "ready";
    }
  };
  const syncControls = () => {
    prompt.disabled = running;
    addTarget.disabled =
      running ||
      targets.length >= MAX_COMPARE_TARGETS ||
      targets.length >= models.length;
    refreshModelsButton.disabled = running || modelsLoading;
    refreshModelsButton.textContent = modelsLoadFailed
      ? workflowText("重试读取模型", "Retry model load")
      : workflowText("刷新模型", "Refresh models");
    refreshModelsButton.setAttribute(
      "aria-label",
      modelsLoadFailed
        ? workflowText(
            "重试读取可用于对比的模型",
            "Retry loading models available for comparison",
          )
        : workflowText(
            "刷新可用于对比的模型",
            "Refresh models available for comparison",
          ),
    );
    for (const target of targets) {
      target.select.disabled = running || !models.length;
      target.remove.disabled = running || targets.length <= MIN_COMPARE_TARGETS;
    }
    run.disabled = running || modelsLoading || !canRun();
    cancel.disabled = !running;
    pane.setAttribute("aria-busy", String(running || modelsLoading));
    updateAvailability();
  };
  const renumberTargets = () => {
    targets.forEach((target, index) => {
      const label = targetLabel(index);
      requiredValue(
        target.wrapper.querySelector("h4"),
        "comparison target heading",
      ).textContent = label;
      requiredValue(
        target.modelField.querySelector("span"),
        "comparison model label",
      ).textContent = workflowText("运行模型", "Run model");
      target.select.setAttribute("aria-label", label);
      target.remove.textContent = workflowText("移除", "Remove");
      target.remove.setAttribute(
        "aria-label",
        workflowText("移除模型 {index}", "Remove model {index}", {
          index: index + 1,
        }),
      );
      target.remove.title = workflowText(
        "移除模型 {index}",
        "Remove model {index}",
        { index: index + 1 },
      );
      if (!target.outputModelLabel) target.output.heading.textContent = label;
      if (!target.outputModelLabel)
        target.output.body.setAttribute(
          "aria-label",
          workflowText("{model} 输出", "{model} output", { model: label }),
        );
      renderCompareTargetStatus(target);
    });
    syncControls();
  };
  const appendTarget = (selectedIndex = targets.length) => {
    if (targets.length >= MAX_COMPARE_TARGETS) return;
    const wrapper = document.createElement("article");
    wrapper.className = "model-compare-target";
    const head = document.createElement("div");
    head.className = "section-head";
    const heading = document.createElement("h4");
    const remove = button(workflowText("移除", "Remove"));
    head.append(heading, remove);
    const select = makeSelect(workflowText("对比模型", "Comparison model"));
    const output = outputPanel(workflowText("模型", "Model"));
    const modelField = field(workflowText("运行模型", "Run model"), select);
    wrapper.append(head, modelField, output.wrapper);
    const target: CompareTarget = {
      id: councilId("compare"),
      wrapper,
      select,
      remove,
      modelField,
      output,
      outputState: "idle",
      emptyResult: false,
    };
    targets.push(target);
    targetList.appendChild(wrapper);
    populateSelect(select, models, selectedIndex);
    renderCompareTargetStatus(target);
    select.addEventListener("change", syncControls);
    remove.addEventListener("click", () => {
      if (running || targets.length <= MIN_COMPARE_TARGETS) return;
      const index = targets.findIndex(
        (candidate) => candidate.id === target.id,
      );
      if (index >= 0) targets.splice(index, 1);
      wrapper.remove();
      renumberTargets();
    });
    renumberTargets();
  };
  appendTarget(0);
  appendTarget(1);

  const refreshCompareLanguage = () => {
    introduction.textContent = workflowText(
      "2–4 个模型会分别通过嵌入式 Pi 执行同一提示词；比较运行只读且不提供工具。",
      "Two to four models run the same prompt through embedded Pi. Comparison runs are read-only and have no tools.",
    );
    requiredValue(
      promptField.querySelector("span"),
      "comparison prompt label",
    ).textContent = workflowText("提示词", "Prompt");
    prompt.placeholder = workflowText(
      "输入需要对比的问题",
      "Enter a prompt to compare",
    );
    prompt.setAttribute(
      "aria-label",
      workflowText("对比提示词", "Comparison prompt"),
    );
    addTarget.textContent = workflowText(
      "添加对比模型",
      "Add comparison model",
    );
    run.textContent = workflowText("开始对比", "Start comparison");
    cancel.textContent = workflowText("取消", "Cancel");
    targets.forEach((target) => {
      renderCompareTargetStatus(target);
      renderCompareTargetBody(target);
    });
    renumberTargets();
  };

  const refreshModels = async () => {
    const serial = ++refreshSerial;
    modelsLoading = true;
    modelsLoadFailed = false;
    syncControls();
    try {
      const loaded = await loadModels(invoke);
      if (serial !== refreshSerial) return;
      models = loaded;
      targets.forEach((target, index) => {
        populateSelect(target.select, models, index);
        if (!running && !target.outputModelLabel) {
          if (!models.length) {
            target.emptyResult = false;
            setCompareTargetState(target, "no_models");
            renderCompareTargetBody(target);
          } else if (
            ["no_models", "models_unavailable"].includes(target.outputState)
          ) {
            target.emptyResult = false;
            setCompareTargetState(target, "idle");
            renderCompareTargetBody(target);
          }
        }
      });
    } catch (error) {
      console.warn("[NovaVei Pi] unable to load comparison models", error);
      if (serial !== refreshSerial) return;
      models = [];
      modelsLoadFailed = true;
      for (const target of targets) {
        populateSelect(target.select, models, 0);
        if (!running && !target.outputModelLabel) {
          target.emptyResult = false;
          setCompareTargetState(target, "models_unavailable");
          renderCompareTargetBody(target);
        }
      }
    } finally {
      if (serial === refreshSerial) {
        modelsLoading = false;
        syncControls();
      }
    }
  };
  void refreshModels();
  window.addEventListener(
    "novavei:providers-changed",
    () => void refreshModels(),
  );
  const activateCompare = () => {
    if (!prompt.value.trim())
      prompt.value =
        byId<HTMLTextAreaElement>("composerInput")?.value.trim() ?? "";
    void refreshModels();
  };
  window.addEventListener("novavei:dock-pane-activated", (event) => {
    if (event instanceof CustomEvent && event.detail?.pane === "compare")
      activateCompare();
  });
  addTarget.addEventListener("click", () => appendTarget());
  refreshModelsButton.addEventListener("click", () => void refreshModels());

  const startOne = (
    model: ModelChoice,
    target: CompareTarget,
    promptText: string,
    cwd: string,
    reasoning: ReturnType<typeof currentReasoning>,
  ) =>
    runner.run({
      title: `[Compare] ${model.label}`,
      prompt: promptText,
      systemPrompt:
        "You are one side of a model comparison. Answer the user's request directly. Do not mention this comparison and do not use tools.",
      cwd,
      model,
      reasoning,
      onEvent(event) {
        if (event.type === "run_started") {
          setCompareTargetState(target, "running");
          target.output.body.textContent = "";
        }
        if (event.type === "text_delta")
          target.output.body.textContent += event.delta ?? event.text ?? "";
      },
    });

  run.addEventListener("click", () => {
    const selected = selectedTargets();
    const promptText = prompt.value.trim();
    const cwd = currentWorkdir();
    if (selected.some((model) => !model))
      return toast(
        workflowText(
          "请为每一路选择可运行模型",
          "Select a runnable model for every route",
        ),
      );
    if (new Set(selected.map((model) => model?.value)).size !== targets.length)
      return toast(
        workflowText(
          "请选择不同的对比模型",
          "Select different comparison models",
        ),
      );
    if (!promptText)
      return toast(
        workflowText("请输入对比提示词", "Enter a comparison prompt"),
      );
    if (!cwd)
      return toast(
        workflowText("请先打开项目工作区", "Open a project workspace first"),
      );
    const runTargets = targets.map((target, index) => ({
      target,
      model: requiredValue(selected[index], "selected comparison model"),
    }));
    running = true;
    cancellationRequested = false;
    syncControls();
    for (const { target, model } of runTargets) {
      target.outputModelLabel = model.label;
      target.output.heading.textContent = model.label;
      target.output.body.setAttribute(
        "aria-label",
        workflowText("{model} 输出", "{model} output", { model: model.label }),
      );
      target.emptyResult = false;
      setCompareTargetState(target, "starting");
      target.output.body.textContent = "";
    }
    const reasoning = currentReasoning();
    void Promise.allSettled(
      runTargets.map(({ target, model }) =>
        startOne(model, target, promptText, cwd, reasoning),
      ),
    )
      .then((results) => {
        let failures = 0;
        results.forEach((result, index) => {
          const target = runTargets[index].target;
          const output = target.output;
          if (result.status === "fulfilled") {
            target.emptyResult = !result.value.text;
            output.body.textContent =
              result.value.text ||
              workflowText("模型未返回文本", "The model returned no text");
            setCompareTargetState(target, "complete");
          } else {
            failures += 1;
            target.emptyResult = false;
            output.body.textContent =
              result.reason instanceof Error
                ? result.reason.message
                : String(result.reason);
            setCompareTargetState(
              target,
              result.reason instanceof DOMException &&
                result.reason.name === "AbortError"
                ? "cancelled"
                : "failed",
            );
          }
        });
        if (!cancellationRequested) {
          toast(
            failures
              ? workflowText(
                  "模型对比完成，{count} 个运行失败。",
                  "Model comparison complete; {count} run(s) failed.",
                  { count: failures },
                )
              : workflowText("模型对比已完成", "Model comparison complete"),
          );
        }
      })
      .finally(() => {
        running = false;
        syncControls();
        const refresh = window.__novaveiHost?.refreshSessions({
          loadActive: false,
        });
        void refresh?.catch(() => undefined);
      });
  });
  cancel.addEventListener("click", () => {
    if (!running) return;
    cancellationRequested = true;
    cancel.disabled = true;
    for (const target of targets) {
      if (["starting", "running"].includes(target.outputState)) {
        setCompareTargetState(target, "cancelling");
      }
    }
    void runner
      .cancel()
      .then((cancelled) =>
        toast(
          cancelled
            ? workflowText(
                "已请求取消 {count} 个模型运行。",
                "Cancellation requested for {count} model run(s).",
                { count: cancelled },
              )
            : workflowText(
                "没有可取消的模型运行。",
                "There are no model runs to cancel.",
              ),
        ),
      )
      .catch(() =>
        toast(
          workflowText(
            "未能取消模型对比；请等待当前运行结束。",
            "Could not cancel the model comparison. Wait for the current run to finish.",
          ),
        ),
      );
  });
  observeWorkflowLanguage(refreshCompareLanguage);
}

function councilMarkdown(snapshot: CouncilRunSnapshot) {
  const lines = [
    "# NovaVei Council",
    "",
    `## ${workflowText("议题", "Topic")}`,
    "",
    snapshot.topic,
  ];
  for (const participant of snapshot.participants.values()) {
    lines.push(
      "",
      `## ${participant.expert.name}`,
      "",
      `${workflowText("模型", "Model")}: ${participant.model.label}`,
    );
    for (const stage of COUNCIL_STAGES) {
      const result = participant.stages[stage];
      if (result?.text)
        lines.push("", `### ${councilStageLabel(stage)}`, "", result.text);
    }
  }
  if (snapshot.synthesis)
    lines.push(
      "",
      `## ${workflowText("主席综合", "Chair synthesis")}`,
      "",
      snapshot.synthesis,
    );
  return lines.join("\n");
}

function councilJson(snapshot: CouncilRunSnapshot) {
  return JSON.stringify(
    {
      formatVersion: 1,
      topic: snapshot.topic,
      startedAt: snapshot.startedAt,
      participants: [...snapshot.participants.values()].map((participant) => ({
        expert: { id: participant.expert.id, name: participant.expert.name },
        model: participant.model.label,
        stages: Object.fromEntries(
          COUNCIL_STAGES.flatMap((stage) => {
            const result = participant.stages[stage];
            return result?.text ? [[stage, result.text]] : [];
          }),
        ),
      })),
      synthesis: snapshot.synthesis,
    },
    null,
    2,
  );
}

function csvCell(value: string) {
  const protectedValue = /^[=+\-@]/.test(value) ? `'${value}` : value;
  return `"${protectedValue.replace(/"/g, '""')}"`;
}

function councilCsv(snapshot: CouncilRunSnapshot) {
  const rows = [["stage", "expert_id", "expert_name", "model", "text"]];
  for (const participant of snapshot.participants.values()) {
    for (const stage of COUNCIL_STAGES) {
      const result = participant.stages[stage];
      if (result?.text)
        rows.push([
          stage,
          participant.expert.id,
          participant.expert.name,
          participant.model.label,
          result.text,
        ]);
    }
  }
  if (snapshot.synthesis)
    rows.push([
      "synthesis",
      "chair",
      workflowText("主席", "Chair"),
      "",
      snapshot.synthesis,
    ]);
  return rows.map((row) => row.map(csvCell).join(",")).join("\r\n");
}

function downloadCouncil(content: string, extension: string, mime: string) {
  const blob = new Blob([content], { type: `${mime};charset=utf-8` });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = `novavei-council-${new Date().toISOString().slice(0, 10)}.${extension}`;
  document.body.appendChild(link);
  link.click();
  link.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 0);
}

async function promptValue(
  label: string,
  initialValue: string,
  maxLength: number,
) {
  const value = await requestAppPrompt({
    title: label,
    label: workflowText("内容", "Content"),
    initialValue,
    maxLength,
    required: true,
  });
  if (value === null) return null;
  const normalized = value.trim();
  if (!normalized) {
    toast(workflowText("内容不能为空", "Content cannot be empty"));
    return null;
  }
  if (normalized.length > maxLength) {
    toast(
      workflowText(
        "内容不能超过 {count} 个字符",
        "Content cannot exceed {count} characters",
        { count: maxLength },
      ),
    );
    return null;
  }
  return normalized;
}

function installCouncilSettings(
  invoke: Invoke,
  initial: CouncilSettings,
  onChange: (settings: CouncilSettings) => void,
  onLaunch: (teamId?: string) => void,
) {
  const panel = document.querySelector<HTMLElement>(
    '.settings-panel[data-settings="council"]',
  );
  if (!panel) return;
  let current = cloneCouncilSettings(initial);
  let saving = false;
  let selectedTab: "experts" | "characters" | "teams" | "prompts" = "experts";

  const tabs = document.createElement("div");
  tabs.className = "settings-subtabs";
  tabs.setAttribute("role", "tablist");
  tabs.setAttribute("aria-label", workflowText("专家设置", "Expert settings"));
  const content = document.createElement("div");
  const sections = new Map<string, HTMLElement>();
  const tabButtons = new Map<string, HTMLButtonElement>();
  const settingsTabLabel = (
    id: "experts" | "characters" | "teams" | "prompts",
  ) => {
    if (id === "experts") return workflowText("专家", "Experts");
    if (id === "characters") return workflowText("角色", "Characters");
    if (id === "teams") return workflowText("专家团", "Teams");
    return workflowText("自定义提示词", "Custom prompts");
  };
  const selectTab = (id: "experts" | "characters" | "teams" | "prompts") => {
    selectedTab = id;
    for (const [key, control] of tabButtons) {
      const active = key === id;
      control.classList.toggle("on", active);
      control.setAttribute("aria-selected", String(active));
      sections.get(key)?.classList.toggle("on", active);
    }
  };
  for (const id of ["experts", "characters", "teams", "prompts"] as const) {
    const tab = button(settingsTabLabel(id));
    tab.className = id === "experts" ? "on" : "";
    tab.dataset.councilTab = id;
    tab.setAttribute("role", "tab");
    tab.setAttribute("aria-selected", String(id === "experts"));
    const section = document.createElement("div");
    section.className = `settings-subpanel${id === "experts" ? " on" : ""}`;
    section.dataset.councilPanel = id;
    tab.addEventListener("click", () => selectTab(id));
    tabs.appendChild(tab);
    content.appendChild(section);
    tabButtons.set(id, tab);
    sections.set(id, section);
  }
  panel.replaceChildren(tabs, content);

  const persist = async (next: CouncilSettings, message: string) => {
    if (saving)
      return toast(
        workflowText("Council 配置正在保存", "Council settings are saving"),
      );
    saving = true;
    try {
      await saveCouncilSettings(invoke, next);
      current = cloneCouncilSettings(next);
      onChange(cloneCouncilSettings(current));
      render();
      toast(message);
    } catch (error) {
      toast(
        error instanceof Error
          ? error.message
          : workflowText(
              "保存 Council 配置失败",
              "Could not save Council settings",
            ),
      );
    } finally {
      saving = false;
    }
  };

  const toolbar = (
    titleText: string,
    hintText: string,
    actionText: string,
    onAction: () => void,
  ) => {
    const bar = document.createElement("div");
    bar.className = "council-toolbar";
    const lead = document.createElement("div");
    lead.className = "lead";
    const title = document.createElement("h3");
    title.textContent = titleText;
    const hint = document.createElement("p");
    hint.textContent = hintText;
    lead.append(title, hint);
    const action = button(actionText, "btn primary");
    action.addEventListener("click", onAction);
    bar.append(lead, action);
    return bar;
  };

  const requestExpert = async (existing?: CouncilExpert) => {
    const name = await promptValue(
      workflowText("专家名称", "Expert name"),
      existing?.name ?? "",
      120,
    );
    if (!name) return;
    if (
      current.experts.some(
        (item) => item.id !== existing?.id && item.name === name,
      )
    ) {
      return toast(
        workflowText("专家名称不能重复", "Expert names must be unique"),
      );
    }
    const prompt = await promptValue(
      workflowText("专家系统提示词", "Expert system prompt"),
      existing?.prompt ?? "",
      12_000,
    );
    if (!prompt) return;
    const expert = { id: existing?.id ?? councilId("expert"), name, prompt };
    const experts = existing
      ? current.experts.map((item) => (item.id === existing.id ? expert : item))
      : [...current.experts, expert];
    void persist(
      { ...current, experts },
      existing
        ? workflowText("专家已更新", "Expert updated")
        : workflowText("专家已添加", "Expert added"),
    );
  };

  const requestCharacter = async (existing?: ChatCharacter) => {
    const name = await promptValue(
      workflowText("角色名称", "Character name"),
      existing?.name ?? "",
      120,
    );
    if (!name) return;
    const normalizedCharacterName = name.toLocaleLowerCase();
    if (
      current.characters.some(
        (item) =>
          item.id !== existing?.id &&
          item.name.toLocaleLowerCase() === normalizedCharacterName,
      )
    ) {
      return toast(
        workflowText("角色名称不能重复", "Character names must be unique"),
      );
    }
    const prompt = await promptValue(
      workflowText("角色系统提示词", "Character system prompt"),
      existing?.prompt ?? "",
      12_000,
    );
    if (!prompt) return;
    const character = {
      id: existing?.id ?? councilId("character"),
      name,
      prompt,
    };
    const characters = existing
      ? current.characters.map((item) =>
          item.id === existing.id ? character : item,
        )
      : [...current.characters, character];
    void persist(
      { ...current, characters },
      existing
        ? workflowText("角色已更新", "Character updated")
        : workflowText("角色已创建", "Character created"),
    );
  };

  /**
   * Keep characters deliberately one-turn scoped: this only prepares the
   * normal composer command and never writes a persona into session history.
   * The user's current draft becomes the command task so choosing a character
   * from Settings does not discard work already typed in the chat.
   */
  const useCharacterInCurrentChat = (character: ChatCharacter) => {
    const composer = byId<HTMLTextAreaElement>("composerInput");
    if (!composer || composer.disabled || composer.readOnly) {
      toast(
        workflowText(
          "当前无法将角色应用到消息，请先结束正在进行的回复。",
          "The character cannot be applied while the current reply is active.",
        ),
      );
      return;
    }
    const task = composer.value.trim();
    composer.value = `/character ${character.id}${task ? ` ${task}` : " "}`;
    composer.dispatchEvent(new Event("input", { bubbles: true }));
    document
      .querySelector<HTMLButtonElement>("#overlaySettings [data-close-overlay]")
      ?.click();
    window.requestAnimationFrame(() => {
      composer.focus();
      composer.setSelectionRange(composer.value.length, composer.value.length);
    });
    toast(
      workflowText(
        "已将角色“{name}”用于当前消息；发送后仅本轮生效。",
        "Character “{name}” is ready for this message only.",
        { name: character.name },
      ),
    );
  };

  const requestTeam = async (existing?: CouncilTeam) => {
    const name = await promptValue(
      workflowText("专家团名称", "Team name"),
      existing?.name ?? "",
      120,
    );
    if (!name) return;
    if (
      current.teams.some(
        (item) => item.id !== existing?.id && item.name === name,
      )
    ) {
      return toast(
        workflowText("专家团名称不能重复", "Team names must be unique"),
      );
    }
    const initialMembers =
      existing?.expertIds
        .map(
          (id) =>
            current.experts.find((expert) => expert.id === id)?.name ?? id,
        )
        .join(", ") ??
      current.experts
        .slice(0, 4)
        .map((expert) => expert.name)
        .join(", ");
    const memberText = await promptValue(
      workflowText(
        "成员名称或 ID，用逗号分隔。可选：{choices}",
        "Member names or IDs, separated by commas. Available: {choices}",
        {
          choices: current.experts
            .map((expert) => `${expert.name} (${expert.id})`)
            .join("；"),
        },
      ),
      initialMembers,
      2_000,
    );
    if (!memberText) return;
    const tokens = memberText
      .split(/[,，]/)
      .map((value) => value.trim())
      .filter(Boolean);
    const expertIds = tokens
      .flatMap((token) => {
        const expert = current.experts.find(
          (candidate) => candidate.id === token || candidate.name === token,
        );
        return expert ? [expert.id] : [];
      })
      .filter((id, index, all) => all.indexOf(id) === index);
    if (expertIds.length < 2)
      return toast(
        workflowText(
          "专家团至少需要两位有效专家",
          "A team needs at least two valid experts",
        ),
      );
    const promptName = (
      await requestAppPrompt({
        title: workflowText(
          "关联提示词名称或 ID（可留空）。可选：{choices}",
          "Associated prompt name or ID (optional). Available: {choices}",
          {
            choices: current.prompts
              .map((item) => `${item.name} (${item.id})`)
              .join("；"),
          },
        ),
        label: workflowText("提示词", "Prompt"),
        initialValue: existing?.promptId
          ? (current.prompts.find((item) => item.id === existing.promptId)
              ?.name ?? existing.promptId)
          : "",
        required: false,
      })
    )?.trim();
    const promptId = current.prompts.find(
      (item) => item.id === promptName || item.name === promptName,
    )?.id;
    const team: CouncilTeam = {
      id: existing?.id ?? councilId("team"),
      name,
      expertIds,
      ...(promptId ? { promptId } : {}),
    };
    const teams = existing
      ? current.teams.map((item) => (item.id === existing.id ? team : item))
      : [...current.teams, team];
    void persist(
      { ...current, teams },
      existing
        ? workflowText("专家团已更新", "Team updated")
        : workflowText("专家团已创建", "Team created"),
    );
  };

  const requestPrompt = async (existing?: CouncilPrompt, copy = false) => {
    const name = await promptValue(
      workflowText("提示词名称", "Prompt name"),
      copy
        ? `${existing?.name ?? workflowText("提示词", "Prompt")} ${workflowText("副本", "copy")}`
        : (existing?.name ?? ""),
      120,
    );
    if (!name) return;
    if (
      current.prompts.some(
        (item) =>
          (!existing || copy || item.id !== existing.id) && item.name === name,
      )
    ) {
      return toast(
        workflowText("提示词名称不能重复", "Prompt names must be unique"),
      );
    }
    const prompt = await promptValue(
      workflowText("圆桌附加提示词", "Council additional prompt"),
      existing?.prompt ?? "",
      12_000,
    );
    if (!prompt) return;
    const item: CouncilPrompt = {
      id: !copy && existing ? existing.id : councilId("prompt"),
      name,
      prompt,
    };
    const prompts =
      existing && !copy
        ? current.prompts.map((candidate) =>
            candidate.id === existing.id ? item : candidate,
          )
        : [...current.prompts, item];
    void persist(
      { ...current, prompts },
      existing && !copy
        ? workflowText("提示词已更新", "Prompt updated")
        : workflowText("提示词已保存", "Prompt saved"),
    );
  };

  function render() {
    const expertsPanel = requiredValue(
      sections.get("experts"),
      "Council experts panel",
    );
    const expertList = document.createElement("div");
    expertList.className = "council-list";
    for (const expert of current.experts) {
      const row = document.createElement("article");
      row.className = "seat";
      const marker = document.createElement("i");
      marker.setAttribute("aria-hidden", "true");
      const copy = document.createElement("span");
      const name = document.createElement("strong");
      name.textContent = expert.name;
      const detail = document.createElement("span");
      detail.textContent = expert.prompt;
      copy.append(name, detail);
      const actions = document.createElement("div");
      actions.className = "row-actions";
      const edit = button(workflowText("编辑", "Edit"));
      const remove = button(workflowText("删除", "Delete"));
      edit.addEventListener("click", () => {
        void requestExpert(expert);
      });
      remove.addEventListener("click", () => {
        void (async () => {
          if (current.experts.length <= 1) {
            toast(workflowText("至少保留一位专家", "Keep at least one expert"));
            return;
          }
          if (
            !(await requestAppConfirm({
              title: workflowText("删除专家", "Delete expert"),
              message: workflowText(
                "删除专家“{name}”？",
                "Delete expert “{name}”?",
                { name: expert.name },
              ),
              confirmLabel: workflowText("删除", "Delete"),
              cancelLabel: workflowText("取消", "Cancel"),
              danger: true,
            }))
          )
            return;
          const experts = current.experts.filter(
            (item) => item.id !== expert.id,
          );
          const teams = current.teams.flatMap((team) => {
            const expertIds = team.expertIds.filter((id) => id !== expert.id);
            return expertIds.length >= 2 ? [{ ...team, expertIds }] : [];
          });
          void persist(
            { ...current, experts, teams },
            workflowText("专家已删除", "Expert deleted"),
          );
        })();
      });
      actions.append(edit, remove);
      row.append(marker, copy, actions);
      expertList.appendChild(row);
    }
    const launch = button(workflowText("启动圆桌", "Launch Council"));
    launch.addEventListener("click", () => onLaunch());
    expertsPanel.replaceChildren(
      toolbar(
        workflowText("专家", "Experts"),
        workflowText(
          "专家、模型和附加提示词在本机保存；所有圆桌运行均为只读。",
          "Experts, models, and additional prompts are stored locally. Every Council run is read-only.",
        ),
        workflowText("添加专家", "Add expert"),
        () => {
          void requestExpert();
        },
      ),
      expertList,
      launch,
    );

    const charactersPanel = requiredValue(
      sections.get("characters"),
      "Chat characters panel",
    );
    const characterList = document.createElement("div");
    characterList.className = "council-list";
    for (const character of current.characters) {
      const row = document.createElement("article");
      row.className = "seat";
      const marker = document.createElement("i");
      marker.setAttribute("aria-hidden", "true");
      const copy = document.createElement("span");
      const name = document.createElement("strong");
      name.textContent = character.name;
      const detail = document.createElement("span");
      detail.textContent = character.prompt;
      copy.append(name, detail);
      const actions = document.createElement("div");
      actions.className = "row-actions";
      const use = button(
        workflowText("用于当前消息", "Use in current message"),
        "btn primary",
      );
      const edit = button(workflowText("编辑", "Edit"));
      const remove = button(workflowText("删除", "Delete"));
      use.addEventListener("click", () => {
        useCharacterInCurrentChat(character);
      });
      edit.addEventListener("click", () => {
        void requestCharacter(character);
      });
      remove.addEventListener("click", () => {
        void (async () => {
          if (
            !(await requestAppConfirm({
              title: workflowText("删除角色", "Delete character"),
              message: workflowText(
                "删除角色“{name}”？",
                "Delete character “{name}”?",
                { name: character.name },
              ),
              confirmLabel: workflowText("删除", "Delete"),
              cancelLabel: workflowText("取消", "Cancel"),
              danger: true,
            }))
          )
            return;
          void persist(
            {
              ...current,
              characters: current.characters.filter(
                (item) => item.id !== character.id,
              ),
            },
            workflowText("角色已删除", "Character deleted"),
          );
        })();
      });
      actions.append(use, edit, remove);
      row.append(marker, copy, actions);
      characterList.appendChild(row);
    }
    charactersPanel.replaceChildren(
      toolbar(
        workflowText("角色", "Characters"),
        workflowText(
          "角色是本机保存的单聊人格，可用 /character 在当前消息中调用。",
          "Characters are local single-chat personas. Use /character to apply one to the current message.",
        ),
        workflowText("创建角色", "Create character"),
        () => {
          void requestCharacter();
        },
      ),
      characterList,
    );

    const teamsPanel = requiredValue(
      sections.get("teams"),
      "Council teams panel",
    );
    const teamList = document.createElement("div");
    teamList.className = "council-list";
    for (const team of current.teams) {
      const card = document.createElement("article");
      card.className = "hub-card";
      const name = document.createElement("strong");
      name.textContent = team.name;
      const members = document.createElement("small");
      members.textContent = team.expertIds
        .map(
          (id) =>
            current.experts.find((expert) => expert.id === id)?.name ?? id,
        )
        .join(" · ");
      const actions = document.createElement("div");
      actions.className = "row-actions";
      const launchTeam = button(workflowText("使用", "Use"), "btn primary");
      const edit = button(workflowText("编辑", "Edit"));
      const remove = button(workflowText("删除", "Delete"));
      launchTeam.addEventListener("click", () => onLaunch(team.id));
      edit.addEventListener("click", () => {
        void requestTeam(team);
      });
      remove.addEventListener("click", () => {
        void (async () => {
          if (
            !(await requestAppConfirm({
              title: workflowText("删除专家团", "Delete team"),
              message: workflowText(
                "删除专家团“{name}”？",
                "Delete team “{name}”?",
                { name: team.name },
              ),
              confirmLabel: workflowText("删除", "Delete"),
              cancelLabel: workflowText("取消", "Cancel"),
              danger: true,
            }))
          )
            return;
          void persist(
            {
              ...current,
              teams: current.teams.filter((item) => item.id !== team.id),
            },
            workflowText("专家团已删除", "Team deleted"),
          );
        })();
      });
      actions.append(launchTeam, edit, remove);
      card.append(name, members, actions);
      teamList.appendChild(card);
    }
    teamsPanel.replaceChildren(
      toolbar(
        workflowText("专家团", "Teams"),
        workflowText(
          "专家团保存席位组合，也可以关联一条自定义提示词。",
          "Teams save seat combinations and can be linked to a custom prompt.",
        ),
        workflowText("创建专家团", "Create team"),
        () => {
          void requestTeam();
        },
      ),
      teamList,
    );

    const promptsPanel = requiredValue(
      sections.get("prompts"),
      "Council prompts panel",
    );
    const promptList = document.createElement("div");
    promptList.className = "council-list";
    for (const item of current.prompts) {
      const card = document.createElement("article");
      card.className = "prompt-card";
      const head = document.createElement("div");
      head.className = "prompt-card-head";
      const name = document.createElement("strong");
      name.textContent = item.name;
      head.appendChild(name);
      const preview = document.createElement("pre");
      preview.textContent = item.prompt;
      const actions = document.createElement("div");
      actions.className = "prompt-card-actions";
      const edit = button(workflowText("编辑", "Edit"));
      const copy = button(workflowText("复制", "Copy"));
      const remove = button(workflowText("删除", "Delete"));
      edit.addEventListener("click", () => {
        void requestPrompt(item);
      });
      copy.addEventListener("click", () => {
        void requestPrompt(item, true);
      });
      remove.addEventListener("click", () => {
        void (async () => {
          if (
            !(await requestAppConfirm({
              title: workflowText("删除提示词", "Delete prompt"),
              message: workflowText(
                "删除提示词“{name}”？",
                "Delete prompt “{name}”?",
                { name: item.name },
              ),
              confirmLabel: workflowText("删除", "Delete"),
              cancelLabel: workflowText("取消", "Cancel"),
              danger: true,
            }))
          )
            return;
          const prompts = current.prompts.filter(
            (candidate) => candidate.id !== item.id,
          );
          const teams = current.teams.map((team) =>
            team.promptId === item.id ? { ...team, promptId: undefined } : team,
          );
          void persist(
            { ...current, prompts, teams },
            workflowText("提示词已删除", "Prompt deleted"),
          );
        })();
      });
      actions.append(edit, copy, remove);
      card.append(head, preview, actions);
      promptList.appendChild(card);
    }
    promptsPanel.replaceChildren(
      toolbar(
        workflowText("自定义提示词", "Custom prompts"),
        workflowText(
          "附加提示词会进入每位专家的三个评议阶段。",
          "The additional prompt is included in every expert's three review stages.",
        ),
        workflowText("新建提示词", "New prompt"),
        () => {
          void requestPrompt();
        },
      ),
      promptList,
    );
  }

  const refreshCouncilSettingsLanguage = () => {
    tabs.setAttribute(
      "aria-label",
      workflowText("专家设置", "Expert settings"),
    );
    for (const [id, tab] of tabButtons)
      tab.textContent = settingsTabLabel(
        id as "experts" | "characters" | "teams" | "prompts",
      );
    render();
    selectTab(selectedTab);
  };

  render();
  observeWorkflowLanguage(refreshCouncilSettingsLanguage);
}

function installCouncil(invoke: Invoke, runner: PiWorkflowRunner) {
  const overlay = byId<HTMLElement>("overlayCouncil");
  const body = overlay?.querySelector<HTMLElement>(".overlay-body");
  const headActions = overlay?.querySelector<HTMLElement>(
    ".overlay-head .row-actions",
  );
  if (!overlay || !body || !headActions) return;
  const overlayTitle =
    overlay.querySelector<HTMLHeadingElement>(".overlay-head h2");
  const headButtons = [
    ...headActions.querySelectorAll<HTMLButtonElement>(".btn"),
  ];
  const pause =
    headButtons.find((control) => !control.matches("[data-close-overlay]")) ??
    button(workflowText("暂停", "Pause"));
  const exportButton =
    headButtons.find(
      (control) =>
        control !== pause && !control.matches("[data-close-overlay]"),
    ) ?? button(workflowText("导出", "Export"), "btn primary");
  const close = headButtons.find((control) =>
    control.matches("[data-close-overlay]"),
  );
  enableControl(pause);
  enableControl(exportButton);
  pause.textContent = workflowText("暂停", "Pause");
  exportButton.textContent = workflowText("导出", "Export");
  pause.disabled = true;
  exportButton.disabled = true;
  const stop = button(workflowText("取消", "Cancel"));
  stop.disabled = true;
  const exportFormat = makeSelect(workflowText("导出格式", "Export format"));
  for (const [value, label] of [
    ["markdown", "Markdown"],
    ["json", "JSON"],
    ["csv", "CSV"],
  ]) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    exportFormat.appendChild(option);
  }
  exportFormat.disabled = true;
  headActions.insertBefore(stop, exportButton);
  headActions.insertBefore(exportFormat, exportButton);
  if (!close) headActions.appendChild(button("关闭", "btn ghost"));

  const topic = document.createElement("textarea");
  topic.rows = 4;
  topic.maxLength = 20_000;
  topic.placeholder = workflowText("输入圆桌议题", "Enter a Council topic");
  topic.setAttribute("aria-label", workflowText("圆桌议题", "Council topic"));
  const topicField = field(workflowText("议题", "Topic"), topic);
  const teamSelect = makeSelect(workflowText("专家团", "Expert team"));
  const teamField = field(workflowText("专家团", "Team"), teamSelect);
  const promptSelect = makeSelect(
    workflowText("附加提示词", "Additional prompt"),
  );
  const promptField = field(
    workflowText("附加提示词", "Additional prompt"),
    promptSelect,
  );
  const expertRows = document.createElement("div");
  expertRows.className = "council-list";
  const chairModel = makeSelect(workflowText("主席模型", "Chair model"));
  const chairField = field(workflowText("主席模型", "Chair model"), chairModel);
  const start = button(
    workflowText("开始圆桌", "Start Council"),
    "btn primary",
  );
  const actions = document.createElement("div");
  actions.className = "row-actions";
  actions.appendChild(start);
  const stageStatus = document.createElement("p");
  stageStatus.className = "dock-note";
  stageStatus.setAttribute("role", "status");
  stageStatus.setAttribute("aria-live", "polite");
  stageStatus.textContent = workflowText("待命", "Idle");
  const synthesis = outputPanel(workflowText("主席综合", "Chair synthesis"));
  body.replaceChildren(
    topicField,
    teamField,
    promptField,
    expertRows,
    chairField,
    actions,
    stageStatus,
    synthesis.wrapper,
  );

  type ExpertControl = {
    expert: CouncilExpert;
    enabled: HTMLInputElement;
    toggleLabel: Text;
    model: HTMLSelectElement;
    modelField: HTMLElement;
    status: HTMLElement;
    output: HTMLPreElement;
    outputParts: Array<{ stage: CouncilStage; text: string; empty?: boolean }>;
    statusState:
      | "idle"
      | "waiting"
      | "stage"
      | "complete"
      | "cancelled"
      | "failed";
    statusStage?: CouncilStage;
  };
  type CouncilToken = {
    cancelled: boolean;
    paused: boolean;
    resume?: () => void;
  };
  const expertControls = new Map<string, ExpertControl>();
  let councilSettings = cloneCouncilSettings(DEFAULT_COUNCIL_SETTINGS);
  let models: ModelChoice[] = [];
  let running = false;
  let activeToken: CouncilToken | undefined;
  let lastSnapshot: CouncilRunSnapshot | undefined;
  let settingsInstalled = false;
  let stageStatusState:
    | "idle"
    | "paused"
    | "pause_requested"
    | "resumed"
    | "synthesis"
    | "complete"
    | "cancelled"
    | "failed"
    | "cancelling" = "idle";
  let stageStatusStage: CouncilStage | "synthesis" | undefined;
  let synthesisState:
    | "idle"
    | "no_models"
    | "waiting_experts"
    | "running"
    | "complete"
    | "cancelled"
    | "failed" = "idle";
  let synthesisEmptyResult = false;

  const synthesisLabel = () => workflowText("主席综合", "Chair synthesis");
  const statusStageLabel = (stage: CouncilStage | "synthesis") =>
    stage === "synthesis" ? synthesisLabel() : councilStageLabel(stage);
  const renderStageStatus = () => {
    switch (stageStatusState) {
      case "paused":
        stageStatus.textContent = workflowText(
          "已暂停 · 下一阶段：{stage}",
          "Paused · Next stage: {stage}",
          { stage: stageStatusStage ? statusStageLabel(stageStatusStage) : "" },
        );
        return;
      case "pause_requested":
        stageStatus.textContent = workflowText(
          "暂停请求已接收；当前发言完成后暂停",
          "Pause requested; the current response will finish first",
        );
        return;
      case "resumed":
        stageStatus.textContent = workflowText(
          "圆桌会议继续",
          "Council resumed",
        );
        return;
      case "synthesis":
        stageStatus.textContent = workflowText(
          "主席综合进行中",
          "Chair synthesis in progress",
        );
        return;
      case "complete":
        stageStatus.textContent = workflowText(
          "圆桌会议已完成",
          "Council complete",
        );
        return;
      case "cancelled":
        stageStatus.textContent = workflowText(
          "圆桌会议已取消",
          "Council cancelled",
        );
        return;
      case "failed":
        stageStatus.textContent = workflowText(
          "圆桌会议失败",
          "Council failed",
        );
        return;
      case "cancelling":
        stageStatus.textContent = workflowText(
          "正在取消圆桌会议",
          "Cancelling Council",
        );
        return;
      default:
        stageStatus.textContent = stageStatusStage
          ? workflowText("{stage}进行中", "{stage} in progress", {
              stage: statusStageLabel(stageStatusStage),
            })
          : workflowText("待命", "Idle");
    }
  };
  const setStageStatus = (
    state: typeof stageStatusState,
    stage?: CouncilStage | "synthesis",
  ) => {
    stageStatusState = state;
    stageStatusStage = stage;
    renderStageStatus();
  };
  const renderSynthesisState = () => {
    const text = {
      idle: workflowText("待命", "Idle"),
      no_models: workflowText(
        "未配置可运行模型",
        "No runnable models configured",
      ),
      waiting_experts: workflowText("等待专家", "Waiting for experts"),
      running: workflowText("综合中", "Synthesizing"),
      complete: workflowText("完成", "Complete"),
      cancelled: workflowText("已取消", "Cancelled"),
      failed: workflowText("失败", "Failed"),
    }[synthesisState];
    synthesis.status.textContent = text;
    synthesis.status.className = ["running", "complete"].includes(
      synthesisState,
    )
      ? "pill"
      : "pill wait";
  };
  const renderSynthesisBody = () => {
    if (synthesisState === "idle")
      synthesis.body.textContent = workflowText("尚无结果", "No results yet");
    else if (synthesisState === "no_models")
      synthesis.body.textContent = workflowText(
        "未配置可运行模型",
        "No runnable models configured",
      );
    else if (synthesisState === "complete" && synthesisEmptyResult)
      synthesis.body.textContent = workflowText(
        "主席未返回文本",
        "The chair returned no text",
      );
  };
  const setSynthesisState = (state: typeof synthesisState) => {
    synthesisState = state;
    renderSynthesisState();
  };
  const renderExpertStatus = (control: ExpertControl) => {
    let text: string;
    if (control.statusState === "stage" && control.statusStage) {
      text = councilStageLabel(control.statusStage);
    } else if (control.statusState === "waiting") {
      text = workflowText("等待开始", "Waiting to start");
    } else if (control.statusState === "complete") {
      text = workflowText("阶段完成", "Stage complete");
    } else if (control.statusState === "cancelled") {
      text = workflowText("已取消", "Cancelled");
    } else if (control.statusState === "failed") {
      text = workflowText("失败", "Failed");
    } else {
      text = workflowText("待命", "Idle");
    }
    control.status.textContent = text;
    control.status.className = ["stage", "complete"].includes(
      control.statusState,
    )
      ? "pill"
      : "pill wait";
  };
  const setExpertStatus = (
    control: ExpertControl,
    state: ExpertControl["statusState"],
    stage?: CouncilStage,
  ) => {
    control.statusState = state;
    control.statusStage = stage;
    renderExpertStatus(control);
  };
  const renderExpertOutput = (control: ExpertControl) => {
    control.output.textContent = control.outputParts
      .map(
        (part) =>
          `## ${councilStageLabel(part.stage)}\n${part.empty ? workflowText("专家未返回文本", "The expert returned no text") : part.text}`,
      )
      .join("\n\n");
  };

  const syncControls = () => {
    topic.disabled = running;
    teamSelect.disabled = running;
    promptSelect.disabled = running;
    chairModel.disabled = running || !models.length;
    for (const control of expertControls.values()) {
      control.enabled.disabled = running;
      control.model.disabled = running || !models.length;
    }
    start.disabled = running || !models.length;
    pause.disabled = !running;
    stop.disabled = !running;
    exportButton.disabled = running || !lastSnapshot;
    exportFormat.disabled = running || !lastSnapshot;
    overlay.setAttribute("aria-busy", String(running));
  };

  const renderSelectors = (
    preferredTeam = teamSelect.value,
    preferredPrompt = promptSelect.value,
  ) => {
    teamSelect.replaceChildren();
    const custom = document.createElement("option");
    custom.value = "";
    custom.textContent = workflowText("自定义席位", "Custom seats");
    teamSelect.appendChild(custom);
    for (const team of councilSettings.teams) {
      const option = document.createElement("option");
      option.value = team.id;
      option.textContent = team.name;
      teamSelect.appendChild(option);
    }
    teamSelect.value = councilSettings.teams.some(
      (team) => team.id === preferredTeam,
    )
      ? preferredTeam
      : "";

    promptSelect.replaceChildren();
    const none = document.createElement("option");
    none.value = "";
    none.textContent = workflowText("不使用附加提示词", "No additional prompt");
    promptSelect.appendChild(none);
    for (const item of councilSettings.prompts) {
      const option = document.createElement("option");
      option.value = item.id;
      option.textContent = item.name;
      promptSelect.appendChild(option);
    }
    promptSelect.value = councilSettings.prompts.some(
      (item) => item.id === preferredPrompt,
    )
      ? preferredPrompt
      : "";
  };

  const renderExpertControls = () => {
    expertControls.clear();
    expertRows.replaceChildren();
    councilSettings.experts.forEach((expert, index) => {
      const row = document.createElement("section");
      row.className = "section";
      const header = document.createElement("div");
      header.className = "section-head";
      const toggle = document.createElement("input");
      toggle.type = "checkbox";
      toggle.checked = true;
      toggle.setAttribute(
        "aria-label",
        workflowText("启用 {name}", "Enable {name}", { name: expert.name }),
      );
      const title = document.createElement("label");
      const toggleLabel = document.createTextNode(` ${expert.name}`);
      title.append(toggle, toggleLabel);
      const status = document.createElement("span");
      status.className = "pill wait";
      status.textContent = workflowText("待命", "Idle");
      status.setAttribute("role", "status");
      const model = makeSelect(
        workflowText("{name} 模型", "{name} model", { name: expert.name }),
      );
      populateSelect(model, models, index % Math.max(models.length, 1));
      const output = document.createElement("pre");
      output.className = "file-preview";
      output.hidden = true;
      output.tabIndex = 0;
      output.setAttribute(
        "aria-label",
        workflowText("{name} 会议记录", "{name} Council record", {
          name: expert.name,
        }),
      );
      const modelField = field(workflowText("运行模型", "Run model"), model);
      header.append(title, status);
      row.append(header, modelField, output);
      expertRows.appendChild(row);
      const control: ExpertControl = {
        expert,
        enabled: toggle,
        toggleLabel,
        model,
        modelField,
        status,
        output,
        outputParts: [],
        statusState: "idle",
      };
      renderExpertStatus(control);
      expertControls.set(expert.id, control);
    });
    syncControls();
  };

  const applyTeam = (teamId: string) => {
    const team = councilSettings.teams.find(
      (candidate) => candidate.id === teamId,
    );
    if (!team) return;
    for (const [id, control] of expertControls)
      control.enabled.checked = team.expertIds.includes(id);
    if (
      team.promptId &&
      councilSettings.prompts.some((item) => item.id === team.promptId)
    ) {
      promptSelect.value = team.promptId;
    }
  };
  teamSelect.addEventListener("change", () => applyTeam(teamSelect.value));

  const launch = (teamId?: string) => {
    document
      .querySelector<HTMLButtonElement>("#overlaySettings [data-close-overlay]")
      ?.click();
    overlay.classList.add("show");
    if (teamId) {
      teamSelect.value = teamId;
      applyTeam(teamId);
    }
    topic.focus();
  };

  const installSettings = (settings: CouncilSettings) => {
    if (settingsInstalled) return;
    settingsInstalled = true;
    installCouncilSettings(
      invoke,
      settings,
      (next) => {
        councilSettings = cloneCouncilSettings(next);
        renderSelectors();
        if (!running) renderExpertControls();
      },
      launch,
    );
  };

  renderSelectors();
  renderExpertControls();
  void loadCouncilSettings(invoke)
    .then((settings) => {
      councilSettings = settings;
      renderSelectors();
      if (!running) renderExpertControls();
      installSettings(settings);
    })
    .catch((error) => {
      console.warn("[NovaVei Pi] unable to load Council settings", error);
      installSettings(councilSettings);
    });

  const refreshModels = async () => {
    models = await loadModels(invoke).catch(() => []);
    let index = 0;
    for (const control of expertControls.values())
      populateSelect(
        control.model,
        models,
        index++ % Math.max(models.length, 1),
      );
    populateSelect(chairModel, models, 0);
    if (!models.length) {
      synthesisEmptyResult = false;
      setSynthesisState("no_models");
      renderSynthesisBody();
    } else if (!running && synthesisState === "no_models") {
      setSynthesisState("idle");
      renderSynthesisBody();
    }
    syncControls();
  };
  void refreshModels();
  window.addEventListener(
    "novavei:providers-changed",
    () => void refreshModels(),
  );

  const abortError = () =>
    new DOMException(
      workflowText("圆桌会议已取消", "Council cancelled"),
      "AbortError",
    );
  const waitWhilePaused = async (
    token: CouncilToken,
    nextStage: CouncilStage | "synthesis",
  ) => {
    if (token.cancelled) throw abortError();
    if (!token.paused) return;
    setStageStatus("paused", nextStage);
    await new Promise<void>((resolve) => {
      token.resume = resolve;
    });
    token.resume = undefined;
    if (token.cancelled) throw abortError();
  };

  const evidenceFor = (
    participants: Map<string, CouncilParticipantResult>,
    stages: readonly CouncilStage[],
    excludeId?: string,
  ) =>
    [...participants.values()]
      .flatMap((participant) => {
        if (participant.expert.id === excludeId) return [];
        return stages.flatMap((stage) => {
          const text = participant.stages[stage]?.text;
          return text
            ? [
                `## ${participant.expert.name} · ${councilStageLabel(stage)}\n${text}`,
              ]
            : [];
        });
      })
      .join("\n\n")
      .slice(0, MAX_COUNCIL_EVIDENCE);

  const runStage = async (
    stage: CouncilStage,
    selected: Array<{ expert: CouncilExpert; model: ModelChoice }>,
    snapshot: CouncilRunSnapshot,
    cwd: string,
    reasoning: ReturnType<typeof currentReasoning>,
    customPrompt: string,
    token: CouncilToken,
  ) => {
    setStageStatus("idle", stage);
    await Promise.allSettled(
      selected.map(async ({ expert, model }) => {
        if (token.cancelled) throw abortError();
        const controls = requiredValue(
          expertControls.get(expert.id),
          "Council expert controls",
        );
        const participant = requiredValue(
          snapshot.participants.get(expert.id),
          "Council participant",
        );
        setExpertStatus(controls, "stage", stage);
        controls.output.hidden = false;
        const outputPart: ExpertControl["outputParts"][number] = {
          stage,
          text: "",
        };
        controls.outputParts.push(outputPart);
        renderExpertOutput(controls);
        const otherEvidence = evidenceFor(
          snapshot.participants,
          stage === "cross_review"
            ? ["independent"]
            : ["independent", "cross_review"],
          expert.id,
        );
        const stageInstruction =
          stage === "independent"
            ? workflowText(
                "独立分析议题，明确结论、证据、风险和建议。",
                "Analyze the topic independently. State clear conclusions, evidence, risks, and recommendations.",
              )
            : stage === "cross_review"
              ? workflowText(
                  "审阅其他专家的独立意见，指出认同与异议，并向至少一位具名专家提出一个可回答的尖锐问题。",
                  "Review the other experts' independent views. Identify agreements and disagreements, then ask at least one named expert a sharp, answerable question.",
                )
              : workflowText(
                  "回应其他专家提出的质询和异议；说明接受、修正或坚持原判断的理由，并给出收敛建议。",
                  "Respond to other experts' questions and objections. Explain why you accept, revise, or retain your judgment, and recommend how to converge.",
                );
        const promptText = [
          `${workflowText("议题", "Topic")}:\n${snapshot.topic}`,
          customPrompt
            ? `${workflowText("会议附加要求", "Additional meeting requirements")}:\n${customPrompt}`
            : "",
          otherEvidence
            ? `${workflowText("其他专家的已完成发言", "Completed statements from other experts")}:\n${otherEvidence}`
            : "",
          stageInstruction,
        ]
          .filter(Boolean)
          .join("\n\n");
        try {
          const result = await runner.run({
            title: `[Council:${stage}] ${snapshot.topic.slice(0, 44)} · ${expert.name}`,
            prompt: promptText,
            systemPrompt: `${expert.prompt}\n${workflowText(
              "你正在参加 Council 的“{stage}”阶段。只依据可见材料发言，不要使用工具。",
              "You are participating in the Council's “{stage}” stage. Respond only from visible material and do not use tools.",
              { stage: councilStageLabel(stage) },
            )}`,
            cwd,
            model,
            reasoning,
            onEvent(event) {
              if (event.type === "text_delta") {
                outputPart.text += event.delta ?? event.text ?? "";
                renderExpertOutput(controls);
              }
            },
          });
          participant.stages[stage] = result;
          outputPart.text = result.text;
          outputPart.empty = !result.text;
          renderExpertOutput(controls);
          setExpertStatus(controls, "complete");
        } catch (error) {
          outputPart.text += `${outputPart.text ? "\n" : ""}${error instanceof Error ? error.message : String(error)}`;
          renderExpertOutput(controls);
          setExpertStatus(
            controls,
            error instanceof DOMException && error.name === "AbortError"
              ? "cancelled"
              : "failed",
          );
          throw error;
        }
      }),
    );
    if (token.cancelled) throw abortError();
  };

  start.addEventListener("click", () => {
    const topicText = topic.value.trim();
    const cwd = currentWorkdir();
    const selected = councilSettings.experts.flatMap((expert) => {
      const controls = expertControls.get(expert.id);
      const model = controls
        ? selectedModel(controls.model, models)
        : undefined;
      return controls?.enabled.checked && model ? [{ expert, model }] : [];
    });
    const chair = selectedModel(chairModel, models);
    if (!topicText)
      return toast(workflowText("请输入圆桌议题", "Enter a Council topic"));
    if (!cwd)
      return toast(
        workflowText("请先打开项目工作区", "Open a project workspace first"),
      );
    if (selected.length < 2)
      return toast(
        workflowText(
          "交叉评议至少需要两位专家",
          "Cross-review requires at least two experts",
        ),
      );
    if (!chair)
      return toast(workflowText("请选择主席模型", "Select a chair model"));
    const template =
      councilSettings.prompts.find((item) => item.id === promptSelect.value)
        ?.prompt ?? "";
    const token: CouncilToken = { cancelled: false, paused: false };
    activeToken = token;
    running = true;
    pause.textContent = workflowText("暂停", "Pause");
    setStageStatus("idle");
    lastSnapshot = {
      topic: topicText,
      startedAt: new Date().toISOString(),
      participants: new Map(
        selected.map(({ expert, model }) => [
          expert.id,
          { expert, model, stages: {} },
        ]),
      ),
      synthesis: "",
    };
    for (const { expert } of selected) {
      const controls = requiredValue(
        expertControls.get(expert.id),
        "Council expert controls",
      );
      controls.output.hidden = false;
      controls.outputParts = [];
      controls.output.textContent = "";
      setExpertStatus(controls, "waiting");
    }
    synthesisEmptyResult = false;
    setSynthesisState("waiting_experts");
    synthesis.body.textContent = "";
    syncControls();
    const reasoning = currentReasoning();

    void (async () => {
      const snapshot = requiredValue(lastSnapshot, "Council run snapshot");
      await waitWhilePaused(token, "independent");
      await runStage(
        "independent",
        selected,
        snapshot,
        cwd,
        reasoning,
        template,
        token,
      );
      await waitWhilePaused(token, "cross_review");
      await runStage(
        "cross_review",
        selected,
        snapshot,
        cwd,
        reasoning,
        template,
        token,
      );
      await waitWhilePaused(token, "questioning");
      await runStage(
        "questioning",
        selected,
        snapshot,
        cwd,
        reasoning,
        template,
        token,
      );
      await waitWhilePaused(token, "synthesis");
      if (
        ![...snapshot.participants.values()].some(
          (participant) => Object.keys(participant.stages).length,
        )
      ) {
        throw new Error(
          workflowText(
            "专家阶段没有产生可供综合的结果",
            "The expert stages produced no results to synthesize",
          ),
        );
      }
      setStageStatus("synthesis", "synthesis");
      setSynthesisState("running");
      const evidence = evidenceFor(snapshot.participants, [
        "independent",
        "cross_review",
        "questioning",
      ]);
      const result = await runner.run({
        title: `[Council:synthesis] ${topicText.slice(0, 48)}`,
        prompt: `${workflowText("议题", "Topic")}:\n${topicText}\n\n${workflowText("完整会议记录", "Complete meeting record")}:\n${evidence}\n\n${workflowText("请形成最终纪要。", "Produce the final summary.")}`,
        systemPrompt: workflowText(
          "你是 Council 主席。基于独立陈述、交叉评议和质询回应形成结构化纪要：共识、关键异议、风险、决议、待办。不要虚构未提供的发言，不要使用工具。",
          "You are the Council chair. From the independent statements, cross-reviews, and responses, produce a structured summary: consensus, key disagreements, risks, decisions, and follow-ups. Do not invent unseen statements or use tools.",
        ),
        cwd,
        model: chair,
        reasoning,
        onEvent(event) {
          if (event.type === "text_delta")
            synthesis.body.textContent += event.delta ?? event.text ?? "";
        },
      });
      snapshot.synthesis = result.text;
      synthesisEmptyResult = !result.text;
      synthesis.body.textContent =
        result.text ||
        workflowText("主席未返回文本", "The chair returned no text");
      setSynthesisState("complete");
      setStageStatus("complete");
      toast(workflowText("圆桌会议已完成", "Council complete"));
    })()
      .catch((error) => {
        const cancelled =
          error instanceof DOMException && error.name === "AbortError";
        setSynthesisState(cancelled ? "cancelled" : "failed");
        if (!synthesis.body.textContent)
          synthesis.body.textContent =
            error instanceof Error ? error.message : String(error);
        setStageStatus(cancelled ? "cancelled" : "failed");
      })
      .finally(() => {
        if (activeToken !== token) return;
        running = false;
        activeToken = undefined;
        pause.textContent = workflowText("暂停", "Pause");
        syncControls();
        const refresh = window.__novaveiHost?.refreshSessions({
          loadActive: false,
        });
        void refresh?.catch(() => undefined);
      });
  });

  const refreshCouncilLanguage = () => {
    overlay.setAttribute("aria-label", workflowText("圆桌会议", "Council"));
    if (overlayTitle)
      overlayTitle.textContent = workflowText("圆桌会议", "Council");
    pause.textContent = activeToken?.paused
      ? workflowText("继续", "Resume")
      : workflowText("暂停", "Pause");
    exportButton.textContent = workflowText("导出", "Export");
    stop.textContent = workflowText("取消", "Cancel");
    close?.replaceChildren(
      document.createTextNode(workflowText("关闭", "Close")),
    );
    exportFormat.setAttribute(
      "aria-label",
      workflowText("导出格式", "Export format"),
    );
    requiredValue(
      topicField.querySelector("span"),
      "Council topic label",
    ).textContent = workflowText("议题", "Topic");
    topic.placeholder = workflowText("输入圆桌议题", "Enter a Council topic");
    topic.setAttribute("aria-label", workflowText("圆桌议题", "Council topic"));
    requiredValue(
      teamField.querySelector("span"),
      "Council team label",
    ).textContent = workflowText("专家团", "Team");
    teamSelect.setAttribute(
      "aria-label",
      workflowText("专家团", "Expert team"),
    );
    requiredValue(
      promptField.querySelector("span"),
      "Council prompt label",
    ).textContent = workflowText("附加提示词", "Additional prompt");
    promptSelect.setAttribute(
      "aria-label",
      workflowText("附加提示词", "Additional prompt"),
    );
    requiredValue(
      chairField.querySelector("span"),
      "Council chair label",
    ).textContent = workflowText("主席模型", "Chair model");
    chairModel.setAttribute(
      "aria-label",
      workflowText("主席模型", "Chair model"),
    );
    start.textContent = workflowText("开始圆桌", "Start Council");
    synthesis.heading.textContent = synthesisLabel();
    synthesis.body.setAttribute(
      "aria-label",
      workflowText("主席综合输出", "Chair synthesis output"),
    );
    renderSynthesisState();
    renderSynthesisBody();
    renderStageStatus();
    renderSelectors();
    for (const control of expertControls.values()) {
      control.enabled.setAttribute(
        "aria-label",
        workflowText("启用 {name}", "Enable {name}", {
          name: control.expert.name,
        }),
      );
      requiredValue(
        control.modelField.querySelector("span"),
        "Council expert model label",
      ).textContent = workflowText("运行模型", "Run model");
      control.model.setAttribute(
        "aria-label",
        workflowText("{name} 模型", "{name} model", {
          name: control.expert.name,
        }),
      );
      control.output.setAttribute(
        "aria-label",
        workflowText("{name} 会议记录", "{name} Council record", {
          name: control.expert.name,
        }),
      );
      renderExpertStatus(control);
      renderExpertOutput(control);
    }
    syncControls();
  };
  observeWorkflowLanguage(refreshCouncilLanguage);

  pause.addEventListener("click", () => {
    const token = activeToken;
    if (!running || !token) return;
    token.paused = !token.paused;
    pause.textContent = token.paused
      ? workflowText("继续", "Resume")
      : workflowText("暂停", "Pause");
    if (token.paused) {
      setStageStatus("pause_requested");
    } else {
      token.resume?.();
      setStageStatus("resumed");
    }
  });
  stop.addEventListener("click", () => {
    const token = activeToken;
    if (!running || !token) return;
    token.cancelled = true;
    token.paused = false;
    token.resume?.();
    stop.disabled = true;
    setStageStatus("cancelling");
    void runner
      .cancel()
      .catch(() =>
        toast(
          workflowText(
            "未能取消当前发言；后续阶段不会启动",
            "Could not cancel the current response; later stages will not start",
          ),
        ),
      );
  });
  exportButton.addEventListener("click", () => {
    if (!lastSnapshot) return;
    if (exportFormat.value === "json") {
      downloadCouncil(councilJson(lastSnapshot), "json", "application/json");
    } else if (exportFormat.value === "csv") {
      downloadCouncil(councilCsv(lastSnapshot), "csv", "text/csv");
    } else {
      downloadCouncil(councilMarkdown(lastSnapshot), "md", "text/markdown");
    }
    toast(workflowText("圆桌纪要已导出", "Council summary exported"));
  });
}

export function installPiWorkflows() {
  const invoke = invokeOrUndefined();
  if (!invoke) return;
  // Keep cancellation scoped to the workflow surface that requested it. A
  // Compare cancel must never abort a Council phase (or vice versa).
  installCompare(invoke, new PiWorkflowRunner());
  installCouncil(invoke, new PiWorkflowRunner());
}
