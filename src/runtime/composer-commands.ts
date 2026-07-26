import { manualContextCompactionFromNativePayload } from "./pi/context";
import { SESSION_GOAL_UPDATED_EVENT } from "./types";
import {
  listCouncilCommandChoices,
  resolveChatCharacter,
  resolveCouncilExpert,
} from "./workflows";

type Invoke = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

type SkillSummary = {
  name: string;
  description?: string;
  enabled?: boolean;
};

type SkillsListResponse = {
  skills?: SkillSummary[];
};

type SkillReadResponse = {
  skill?: SkillSummary;
  content?: string;
};

export type ComposerCommandSubmission = {
  text: string;
  /** Keep an expanded runtime instruction out of the user-visible transcript. */
  displayText?: string;
};

export type ComposerCommandHandled = {
  /** The command completed locally and must not be forwarded to a provider. */
  handled: true;
  message: string;
};

export type ComposerCommandApi = {
  prepare(
    text: string,
  ): Promise<ComposerCommandSubmission | ComposerCommandHandled | undefined>;
};

type ComposerCommandOptions = {
  form: HTMLFormElement;
  input: HTMLTextAreaElement;
};

type ParsedCommand =
  | { kind: "skill"; raw: string; name: string; task: string }
  | { kind: "expert"; raw: string; name: string; task: string }
  | { kind: "character"; raw: string; name: string; task: string }
  | { kind: "goal"; raw: string; task: string }
  | { kind: "continue"; raw: string; argument: string }
  | {
      kind: "compact";
      raw: string;
      action: "compact" | "restore" | "invalid";
    };

type CommandOption = {
  kind: "skill" | "expert" | "character" | "goal" | "continue" | "compact";
  keyword: string;
  label: string;
  description: string;
  template: string;
  available?: boolean;
};

type ChoiceKind = "skill" | "expert" | "character";

type CommandChoice = {
  label: string;
  description: string;
  selector: string;
};

type ChoiceLoadState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; choices: CommandChoice[] }
  | { status: "error"; message: string };

type CommandMenuElements = {
  menu: HTMLDivElement;
  commandList: HTMLDivElement;
  choicePanel: HTMLElement;
};

const COMMAND_OPTIONS: readonly CommandOption[] = [
  {
    kind: "skill",
    keyword: "skill",
    label: "/skill Skill 名称 任务",
    description: "调用已启用的本机 Skill，并按其指引执行任务",
    template: "/skill ",
  },
  {
    kind: "expert",
    keyword: "expert",
    label: "/expert 专家名称 任务",
    description: "调用 Council 中已配置的单位专家进行聚焦分析",
    template: "/expert ",
  },
  {
    kind: "character",
    keyword: "character",
    label: "/character 角色名称 任务",
    description: "以本机保存的角色人格完成当前消息",
    template: "/character ",
  },
  {
    kind: "goal",
    keyword: "goal",
    label: "/goal 目标任务",
    description: "将目标保存到当前会话，然后立即开始执行",
    template: "/goal ",
  },
  {
    kind: "continue",
    keyword: "continue",
    label: "/continue 在新任务中继续",
    description: "复制当前会话历史和附件到新任务，然后切换过去继续",
    template: "/continue",
  },
  {
    kind: "compact",
    keyword: "compact",
    label: "/compact [restore]",
    description:
      "保存可追溯的连续性摘要并保留最近回合；restore 恢复完整上下文。",
    template: "/compact",
  },
];

function isChoiceKind(kind: CommandOption["kind"]): kind is ChoiceKind {
  return kind === "skill" || kind === "expert" || kind === "character";
}

function selectorForChoice(name: string, id: string) {
  // The slash parser supports quoted selectors but not escaped quotes.  The
  // persisted id is a safe, exact fallback for the rare display name that
  // contains one.
  if (name.includes('"')) return id;
  return /\s/.test(name) ? `"${name}"` : name;
}

function invokeOrUndefined(): Invoke | undefined {
  return window.__TAURI__?.core?.invoke;
}

function normalizeCommandName(value: string) {
  const normalized = value.trim().toLocaleLowerCase();
  // Keep the common spelling mistake working so an existing muscle-memory
  // shortcut cannot silently become a normal chat message.
  return normalized === "skrill" ? "skill" : normalized;
}

function parseCommand(raw: string): ParsedCommand | undefined {
  const match = /^\/([^\s/]+)(?:\s+([\s\S]*))?$/.exec(raw.trim());
  if (!match) return undefined;
  const kind = normalizeCommandName(match[1]);
  const argument = match[2]?.trim() ?? "";
  if (kind === "goal") return { kind, raw, task: argument };
  if (kind === "continue") return { kind, raw, argument };
  if (kind === "compact") {
    const action = argument.toLocaleLowerCase();
    if (!action) return { kind, raw, action: "compact" };
    if (action === "restore") return { kind, raw, action };
    return { kind, raw, action: "invalid" };
  }
  if (kind !== "skill" && kind !== "expert" && kind !== "character")
    return undefined;
  const quoted = /^"([^"\n]+)"(?:\s+([\s\S]*))?$/.exec(argument);
  if (quoted)
    return {
      kind,
      raw,
      name: quoted[1].trim(),
      task: quoted[2]?.trim() ?? "",
    };
  const [name = "", ...task] = argument.split(/\s+/);
  return { kind, raw, name, task: task.join(" ").trim() };
}

function commandError(message: string): never {
  throw new Error(message);
}

function localSkill(response: unknown, requestedName: string) {
  const skills = (response as SkillsListResponse | undefined)?.skills;
  if (!Array.isArray(skills))
    commandError("Skills 服务没有返回可用 Skill 列表");
  const normalized = requestedName.toLocaleLowerCase();
  const skill = skills.find(
    (candidate) => candidate.name?.toLocaleLowerCase() === normalized,
  );
  if (!skill) {
    const available = skills
      .filter((candidate) => candidate.enabled === true)
      .map((candidate) => candidate.name)
      .join("、");
    commandError(
      available
        ? `未找到已启用的 Skill“${requestedName}”。可用：${available}`
        : "当前没有已启用的 Skill。请先在工具 → Skills 中启用或安装。",
    );
  }
  if (skill.enabled !== true)
    commandError(`Skill“${skill.name}”已停用。请先在工具 → Skills 中启用。`);
  return skill;
}

function skillInstruction(name: string, content: string, task: string) {
  return [
    `用户通过 /skill 显式调用了已启用的本机 Skill“${name}”。`,
    "以下内容刚刚从本机受控 SkillRead 边界读取。仅在当前会话权限允许的范围内遵循其适用指引；若指引冲突，遵循系统与用户的更高优先级要求。",
    "<novavei-skill-instruction>",
    content,
    "</novavei-skill-instruction>",
    task ? `任务：${task}` : "请读取该 Skill，并询问用户要用它完成什么任务。",
  ].join("\n\n");
}

function expertInstruction(
  expert: { name: string; prompt: string },
  task: string,
) {
  return [
    `用户通过 /expert 显式调用了 Council 专家“${expert.name}”。`,
    "请以该专家的职责独立完成当前任务，并给出清晰、可执行的结论。此命令不会启动多专家 Council，也不替代其中的交叉评议。",
    "<novavei-expert-instruction>",
    expert.prompt,
    "</novavei-expert-instruction>",
    task || "请说明需要该专家评估的任务。",
  ].join("\n\n");
}

function characterInstruction(
  character: { name: string; prompt: string },
  task: string,
) {
  return [
    `用户通过 /character 显式选择了角色“${character.name}”。`,
    "请在本轮以该角色的语言、职责和边界响应当前任务；这不会更改系统指令、工具权限或后续消息的默认人格。",
    "<novavei-character-instruction>",
    character.prompt,
    "</novavei-character-instruction>",
    task || "请说明希望该角色完成的任务。",
  ].join("\n\n");
}

function goalInstruction(task: string) {
  return [
    `用户通过 /goal 将当前会话目标设为：${task}`,
    "目标已由本机会话存储保存。请围绕该目标开始工作；只有在任务确实完成时，才可调用 goal_progress_update 将它标记为 completed。",
  ].join("\n\n");
}

function createCommandMenu(input: HTMLTextAreaElement): CommandMenuElements {
  const menu = document.createElement("div");
  menu.className = "composer-command-menu";
  menu.id = "composerCommandMenu";
  menu.setAttribute("role", "dialog");
  menu.setAttribute("aria-label", "可用聊天命令");
  menu.hidden = true;
  const commandList = document.createElement("div");
  commandList.className = "composer-command-list";
  commandList.setAttribute("role", "listbox");
  commandList.setAttribute("aria-label", "聊天命令");
  const choicePanel = document.createElement("section");
  choicePanel.className = "composer-command-choices";
  choicePanel.setAttribute("aria-live", "polite");
  menu.append(commandList, choicePanel);
  input.before(menu);
  input.setAttribute("aria-controls", menu.id);
  input.setAttribute("aria-expanded", "false");
  input.setAttribute("aria-haspopup", "dialog");
  return { menu, commandList, choicePanel };
}

/**
 * Add discoverable slash commands to the existing composer without creating a
 * second prompt surface. Each command resolves through its current native
 * authority immediately before the normal Pi submission.
 */
export function installComposerCommands(
  options: ComposerCommandOptions,
): ComposerCommandApi {
  const { form, input } = options;
  const { menu, commandList, choicePanel } = createCommandMenu(input);
  let selectedCommandIndex = 0;
  let selectedChoiceIndex = 0;
  let activePane: "commands" | "choices" = "commands";
  const choiceStates: Record<ChoiceKind, ChoiceLoadState> = {
    skill: { status: "idle" },
    expert: { status: "idle" },
    character: { status: "idle" },
  };

  const matchedOptions = () => {
    const match = /^\/([^\s/]*)\s*$/.exec(input.value.trimStart());
    if (!match) return [];
    const keyword = match[1].toLocaleLowerCase();
    return COMMAND_OPTIONS.filter((option) =>
      option.keyword.startsWith(keyword),
    );
  };

  const closeMenu = () => {
    selectedCommandIndex = 0;
    selectedChoiceIndex = 0;
    activePane = "commands";
    menu.hidden = true;
    commandList.replaceChildren();
    choicePanel.replaceChildren();
    input.setAttribute("aria-expanded", "false");
    input.removeAttribute("aria-activedescendant");
  };

  const applyOption = (option: CommandOption) => {
    if (option.available === false) return;
    input.value = option.template;
    closeMenu();
    input.focus();
  };

  const applyChoice = (kind: ChoiceKind, choice: CommandChoice) => {
    input.value = `/${kind} ${choice.selector} `;
    closeMenu();
    input.focus();
  };

  const choiceHeading = (kind: ChoiceKind) => {
    switch (kind) {
      case "skill":
        return "可选的已启用 Skill";
      case "expert":
        return "可选的专家";
      case "character":
        return "可选的角色";
    }
  };

  const emptyChoiceMessage = (kind: ChoiceKind) => {
    switch (kind) {
      case "skill":
        return "没有已启用的 Skill。请先在工具 → Skills 中启用或安装。";
      case "expert":
        return "还没有可用专家。请先在设置 → 专家中创建或恢复配置。";
      case "character":
        return "还没有创建角色。请先在设置 → 专家 → 角色中创建一个。";
    }
  };

  const renderChoicePanel = (option: CommandOption | undefined) => {
    choicePanel.replaceChildren();
    const heading = document.createElement("strong");
    heading.className = "composer-command-choices-title";

    if (!option || !isChoiceKind(option.kind)) {
      heading.textContent = "选择命令";
      const hint = document.createElement("small");
      hint.className = "composer-command-choices-hint";
      hint.textContent =
        option?.kind === "goal"
          ? "输入目标任务后发送，NovaVei 会保存并开始执行。"
          : option?.kind === "continue"
            ? "发送后会复制当前会话历史和附件并切换到新任务；原任务保持不变。"
            : option?.kind === "compact"
              ? "发送 /compact 保存摘要；使用 /compact restore 恢复完整上下文。"
              : "从左侧选择 Skill、专家或角色，右侧会显示可用项目。";
      choicePanel.append(heading, hint);
      return;
    }

    // `option.kind` is checked above, but capture the narrowed value before
    // it is used by nested callbacks below.
    const choiceKind = option.kind as ChoiceKind;
    heading.textContent = choiceHeading(choiceKind);
    choicePanel.append(heading);
    const state = choiceStates[choiceKind];
    if (state.status === "idle" || state.status === "loading") {
      const loading = document.createElement("small");
      loading.className = "composer-command-choices-hint";
      loading.textContent = "正在读取本机配置…";
      choicePanel.append(loading);
      return;
    }
    if (state.status === "error") {
      const error = document.createElement("small");
      error.className = "composer-command-choices-hint is-error";
      error.textContent = state.message;
      choicePanel.append(error);
      return;
    }
    if (!state.choices.length) {
      const empty = document.createElement("small");
      empty.className = "composer-command-choices-hint";
      empty.textContent = emptyChoiceMessage(choiceKind);
      choicePanel.append(empty);
      return;
    }

    const choiceList = document.createElement("div");
    choiceList.className = "composer-command-choice-list";
    choiceList.setAttribute("role", "listbox");
    choiceList.setAttribute("aria-label", choiceHeading(choiceKind));
    choiceList.append(
      ...state.choices.map((choice, index) => {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "composer-command-choice";
        button.id = `composerCommandChoice-${choiceKind}-${index}`;
        button.setAttribute("role", "option");
        button.setAttribute(
          "aria-selected",
          String(activePane === "choices" && index === selectedChoiceIndex),
        );
        const title = document.createElement("strong");
        title.textContent = choice.label;
        const description = document.createElement("small");
        description.textContent = choice.description;
        button.append(title, description);
        button.addEventListener("mousedown", (event) => event.preventDefault());
        button.addEventListener("click", () => applyChoice(choiceKind, choice));
        return button;
      }),
    );
    choicePanel.append(choiceList);
  };

  const loadChoices = async (kind: ChoiceKind) => {
    if (choiceStates[kind].status !== "idle") return;
    const invoke = invokeOrUndefined();
    if (!invoke) {
      choiceStates[kind] = {
        status: "error",
        message: "可选项只能在 NovaVei 桌面应用中读取。",
      };
      renderMenu();
      return;
    }
    choiceStates[kind] = { status: "loading" };
    renderMenu();
    try {
      let choices: CommandChoice[];
      if (kind === "skill") {
        const response = await invoke<SkillsListResponse>("skills_list");
        if (!Array.isArray(response.skills))
          commandError("Skills 服务没有返回可用 Skill 列表");
        choices = response.skills
          .filter((skill) => skill.enabled === true)
          .map((skill) => ({
            label: skill.name,
            description: skill.description?.trim() || "已启用的本机 Skill",
            selector: selectorForChoice(skill.name, skill.name),
          }));
      } else {
        const library = await listCouncilCommandChoices(invoke);
        const entries =
          kind === "expert" ? library.experts : library.characters;
        choices = entries.map((entry) => ({
          label: entry.name,
          description: `ID: ${entry.id}`,
          selector: selectorForChoice(entry.name, entry.id),
        }));
      }
      choiceStates[kind] = { status: "ready", choices };
    } catch (error) {
      choiceStates[kind] = {
        status: "error",
        message: `无法读取可选项：${error instanceof Error ? error.message : String(error)}`,
      };
    }
    renderMenu();
  };

  const renderMenu = () => {
    const options = matchedOptions();
    if (!options.length) {
      closeMenu();
      return;
    }
    const selectable = options.filter((option) => option.available !== false);
    selectedCommandIndex = Math.min(
      selectedCommandIndex,
      Math.max(0, selectable.length - 1),
    );
    const selected = selectable[selectedCommandIndex];
    const selectedState =
      selected && isChoiceKind(selected.kind)
        ? choiceStates[selected.kind]
        : undefined;
    const choiceCount =
      selectedState?.status === "ready" ? selectedState.choices.length : 0;
    selectedChoiceIndex = Math.min(
      selectedChoiceIndex,
      Math.max(0, choiceCount - 1),
    );
    if (activePane === "choices" && choiceCount === 0) activePane = "commands";

    commandList.replaceChildren(
      ...options.map((option) => {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "composer-command-option";
        button.id = `composerCommandOption-${option.kind}`;
        button.setAttribute("role", "option");
        button.setAttribute("aria-selected", String(option === selected));
        if (option.available === false) {
          button.disabled = true;
          button.setAttribute("aria-disabled", "true");
          button.title = option.description;
        }
        const title = document.createElement("strong");
        title.textContent = option.label;
        const description = document.createElement("small");
        description.textContent = option.description;
        button.append(title, description);
        button.addEventListener("mousedown", (event) => event.preventDefault());
        button.addEventListener("click", () => {
          if (option.available === false) return;
          const index = selectable.indexOf(option);
          selectedCommandIndex = index < 0 ? 0 : index;
          selectedChoiceIndex = 0;
          activePane = "commands";
          if (!isChoiceKind(option.kind)) applyOption(option);
          else renderMenu();
        });
        return button;
      }),
    );
    renderChoicePanel(selected);
    menu.hidden = false;
    input.setAttribute("aria-expanded", "true");
    if (activePane === "choices" && selected && isChoiceKind(selected.kind)) {
      input.setAttribute(
        "aria-activedescendant",
        `composerCommandChoice-${selected.kind}-${selectedChoiceIndex}`,
      );
    } else if (selected) {
      input.setAttribute(
        "aria-activedescendant",
        `composerCommandOption-${selected.kind}`,
      );
    } else {
      input.removeAttribute("aria-activedescendant");
    }
    if (selected && isChoiceKind(selected.kind))
      void loadChoices(selected.kind);
  };

  input.addEventListener("input", () => {
    selectedCommandIndex = 0;
    selectedChoiceIndex = 0;
    activePane = "commands";
    renderMenu();
  });
  input.addEventListener("keydown", (event) => {
    if (event.isComposing) return;
    const options = matchedOptions();
    if (!options.length) return;
    const selectable = options.filter((option) => option.available !== false);
    if (event.key === "Escape") {
      event.preventDefault();
      closeMenu();
      return;
    }
    if (!selectable.length) return;
    const selected = selectable[selectedCommandIndex];
    const choiceState =
      selected && isChoiceKind(selected.kind)
        ? choiceStates[selected.kind]
        : undefined;
    const readyChoices =
      choiceState?.status === "ready" ? choiceState.choices : [];
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (activePane === "choices" && readyChoices.length) {
        selectedChoiceIndex =
          event.key === "ArrowDown"
            ? (selectedChoiceIndex + 1) % readyChoices.length
            : (selectedChoiceIndex - 1 + readyChoices.length) %
              readyChoices.length;
      } else {
        selectedCommandIndex =
          event.key === "ArrowDown"
            ? (selectedCommandIndex + 1) % selectable.length
            : (selectedCommandIndex - 1 + selectable.length) %
              selectable.length;
        selectedChoiceIndex = 0;
      }
      renderMenu();
      return;
    }
    if (event.key === "ArrowRight" && readyChoices.length) {
      event.preventDefault();
      activePane = "choices";
      renderMenu();
      return;
    }
    if (event.key === "ArrowLeft" && activePane === "choices") {
      event.preventDefault();
      activePane = "commands";
      renderMenu();
      return;
    }
    if (
      (event.key === "Enter" && !(event.ctrlKey || event.metaKey)) ||
      event.key === "Tab"
    ) {
      event.preventDefault();
      if (activePane === "choices" && selected && isChoiceKind(selected.kind)) {
        const choice = readyChoices[selectedChoiceIndex];
        if (choice) applyChoice(selected.kind, choice);
        return;
      }
      if (selected && isChoiceKind(selected.kind)) {
        if (readyChoices.length) activePane = "choices";
        else void loadChoices(selected.kind);
        renderMenu();
        return;
      }
      if (selected) applyOption(selected);
    }
  });
  input.addEventListener("blur", () => window.setTimeout(closeMenu, 0));

  const prepare = async (
    raw: string,
  ): Promise<ComposerCommandSubmission | ComposerCommandHandled> => {
    const parsed = parseCommand(raw);
    if (!parsed) return { text: raw };
    const invoke = invokeOrUndefined();
    if (!invoke) commandError("聊天命令只能在 NovaVei 桌面应用中调用");
    form.dataset.novaveiCommandState = "resolving";
    input.setAttribute("aria-busy", "true");
    closeMenu();
    try {
      switch (parsed.kind) {
        case "skill": {
          if (!parsed.name) commandError("用法：/skill <Skill 名称> [任务]");
          const skill = localSkill(await invoke("skills_list"), parsed.name);
          const loaded = await invoke<SkillReadResponse>("skills_read", {
            name: skill.name,
          });
          if (loaded.skill?.enabled !== true || !loaded.content?.trim())
            commandError(`无法读取已启用的 Skill“${skill.name}”`);
          return {
            text: skillInstruction(skill.name, loaded.content, parsed.task),
            displayText: parsed.raw,
          };
        }
        case "expert": {
          if (!parsed.name || !parsed.task)
            commandError(
              "用法：/expert <专家名称或 ID> <任务>；含空格的名称请用双引号包住",
            );
          const expert = await resolveCouncilExpert(invoke, parsed.name);
          return {
            text: expertInstruction(expert, parsed.task),
            displayText: parsed.raw,
          };
        }
        case "character": {
          if (!parsed.name || !parsed.task)
            commandError(
              "用法：/character <角色名称或 ID> <任务>；含空格的名称请用双引号包住",
            );
          const character = await resolveChatCharacter(invoke, parsed.name);
          return {
            text: characterInstruction(character, parsed.task),
            displayText: parsed.raw,
          };
        }
        case "goal": {
          if (!parsed.task) commandError("用法：/goal <目标任务>");
          const sessionId = window.__novaveiHost?.getSessionId?.()?.trim();
          if (!sessionId)
            commandError("请先创建或选择一个本地会话，再使用 /goal 设置目标");
          await invoke("session_goal_set", {
            sessionId,
            text: parsed.task,
            status: "active",
            progress: 0,
          });
          window.dispatchEvent(
            new CustomEvent(SESSION_GOAL_UPDATED_EVENT, {
              detail: { sessionId },
            }),
          );
          return {
            text: goalInstruction(parsed.task),
            displayText: parsed.raw,
          };
        }
        case "continue": {
          if (parsed.argument)
            commandError("用法：/continue（会复制当前任务并切换到新任务）");
          const host = window.__novaveiHost;
          const sessionId = host?.getSessionId?.()?.trim();
          if (!host || !sessionId)
            commandError("请先创建或选择一个本地会话，再在新任务中继续");
          const sourceTitle = host
            .getSessions()
            .find((session) => session.id === sessionId)
            ?.title.trim();
          const branch = await host.branchSession(
            sessionId,
            sourceTitle ? `${sourceTitle}（继续）` : undefined,
          );
          await host.refreshSessions({ loadActive: false });
          await host.selectSession(branch.id);
          return {
            handled: true,
            message: "已复制到新任务，现可在新任务中继续。原任务保持不变。",
          };
        }
        case "compact": {
          const sessionId = window.__novaveiHost?.getSessionId?.()?.trim();
          if (!sessionId)
            commandError("请先创建或选择一个本地会话，再使用 /compact");
          if (parsed.action === "invalid")
            commandError("用法：/compact 或 /compact restore");
          if (parsed.action === "restore") {
            await invoke("session_context_compaction_clear", { sessionId });
            return {
              handled: true,
              message: "已恢复完整会话上下文；原始历史从未删除。",
            };
          }
          const source = await invoke(
            "history_context_compaction_source_load",
            {
              sessionId,
            },
          );
          const compaction = manualContextCompactionFromNativePayload(source);
          if (!compaction)
            commandError(
              "当前历史不足以安全压缩。请至少保留两个完整回合，并在内容增长后重试。",
            );
          await invoke("session_context_compaction_set", {
            sessionId,
            summary: compaction.text,
            metadata: compaction.metadata,
          });
          return {
            handled: true,
            message: `已压缩较早的 ${compaction.metadata.sourceTurns} 个回合，保留最近 ${compaction.retainedTurns} 个完整回合；原始历史仍可阅读和搜索。`,
          };
        }
      }
    } finally {
      delete form.dataset.novaveiCommandState;
      input.removeAttribute("aria-busy");
    }
  };

  // Preserve a normal prompt beginning with an unrecognised slash. The menu is
  // discoverable without taking over file paths, snippets, or prose.
  form.addEventListener("reset", closeMenu);
  return { prepare };
}
