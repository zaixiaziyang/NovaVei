import {
  type PiRuntimeSnapshot,
  SESSION_GOAL_UPDATED_EVENT,
  type SessionGoalUpdatedDetail,
} from "./types";

/**
 * Session-scoped goal UI.
 *
 * A goal is persisted by the native `session_goal_*` boundary, never as a
 * browser-side source of truth. This module only keeps a short-lived draft
 * while a real session has not been created or while a native write is in
 * flight; it does not send conversation history, credentials, or arbitrary
 * metadata to native code. Goals are always user-authored through the editor.
 * Ordinary Pi prompts and outputs never infer, create, or complete one; only an
 * explicit goal_progress_update tool call may advance an already-existing goal.
 */

const MAX_GOAL_LENGTH = 600;
const PREVIEW_LENGTH = 88;
const SESSION_POLL_MS = 350;

type GoalStatus = "active" | "completed";

type SessionGoal = {
  text: string;
  status: GoalStatus;
  progress: number;
  updatedAt: number;
};

type GoalElements = {
  bar: HTMLElement;
  summary: HTMLButtonElement;
  complete: HTMLButtonElement;
  clear: HTMLButtonElement;
  editor: HTMLFormElement;
  input: HTMLTextAreaElement;
  progressInput: HTMLInputElement;
  save: HTMLButtonElement;
  cancel: HTMLButtonElement;
  meter: HTMLElement;
  meterFill: HTMLElement;
  progress: HTMLElement;
  runStatus: HTMLElement;
  editorStatus: HTMLElement;
};

type SessionGoalSetDto = {
  sessionId: string;
  text?: string;
  status?: GoalStatus;
  progress?: number;
  clear?: true;
};

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function sanitizeGoal(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const normalized = value.replace(/\s+/g, " ").trim();
  if (!normalized) return undefined;
  return normalized.length > MAX_GOAL_LENGTH
    ? `${normalized.slice(0, MAX_GOAL_LENGTH - 1).trimEnd()}…`
    : normalized;
}

function isRealSessionId(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 128 &&
    /^[A-Za-z0-9_-]+$/.test(value)
  );
}

function currentRealSessionId(): string | undefined {
  // NativeShell owns durable session identity. Never use a prototype row or
  // an in-flight Pi fallback id as a persistence key.
  const candidate = window.__novaveiHost?.getSessionId?.()?.trim();
  return isRealSessionId(candidate) ? candidate : undefined;
}

function readNativeGoal(value: unknown): SessionGoal | undefined {
  if (value === null || value === undefined) return undefined;
  const record = asRecord(value);
  const text = sanitizeGoal(record?.text);
  const status = record?.status;
  const progress = record?.progress;
  const updatedAt = record?.updatedAt ?? record?.updated_at;
  if (
    !text ||
    (status !== "active" && status !== "completed") ||
    typeof progress !== "number" ||
    !Number.isInteger(progress) ||
    typeof updatedAt !== "number" ||
    !Number.isFinite(updatedAt)
  ) {
    return undefined;
  }
  if (
    progress < 0 ||
    progress > 100 ||
    (status === "active" && progress >= 100) ||
    (status === "completed" && progress !== 100)
  ) {
    return undefined;
  }
  return { text, status, progress, updatedAt: Number(updatedAt) };
}

function preview(text: string | undefined): string {
  if (!text) return "当前会话尚未设置目标";
  return text.length > PREVIEW_LENGTH
    ? `${text.slice(0, PREVIEW_LENGTH - 1).trimEnd()}…`
    : text;
}

function runHint(state: PiRuntimeSnapshot | undefined): {
  label: string;
  detail: string;
  state: string;
} {
  switch (state?.status) {
    case "starting":
      return {
        label: "启动中",
        detail: "本轮正在启动；不会自动改变目标进度。",
        state: "starting",
      };
    case "running":
      return {
        label: "运行中",
        detail: "本轮正在运行；不会自动改变目标进度。",
        state: "running",
      };
    case "waiting_permission":
      return {
        label: "等待确认",
        detail: "本轮等待权限确认；目标仍未自动完成。",
        state: "waiting_permission",
      };
    case "completed":
      return {
        label: "本轮完成",
        detail: "本轮响应已完成；请手动标记目标完成。",
        state: "completed",
      };
    case "cancelled":
      return {
        label: "本轮已取消",
        detail: "本轮已取消，目标没有自动完成。",
        state: "cancelled",
      };
    case "error":
      return {
        label: "本轮失败",
        detail: "本轮失败，目标没有自动完成。",
        state: "error",
      };
    default:
      return { label: "—", detail: "当前目标尚未运行。", state: "idle" };
  }
}

function invokeGoal<T>(
  command: "session_goal_get" | "session_goal_set",
  args: Record<string, unknown>,
): Promise<T> {
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) return Promise.reject(new Error("本地目标存储不可用"));
  return invoke<T>(command, args);
}

function createElements(bar: HTMLElement): GoalElements {
  bar.replaceChildren();
  // Stay hidden until there is a goal, an open editor, or an error that needs
  // the bar. An empty "no goal yet" strip is intentionally not shown.
  bar.classList.remove("show");
  bar.hidden = true;
  bar.style.flexWrap = "wrap";

  const label = document.createElement("strong");
  label.id = "novaveiGoalLabel";
  label.textContent = "目标";

  const summary = document.createElement("button");
  summary.type = "button";
  summary.className = "goal-text";
  summary.id = "novaveiGoalSummary";
  summary.style.textAlign = "left";
  summary.style.minWidth = "0";
  summary.setAttribute("aria-controls", "novaveiGoalEditor");
  summary.setAttribute("aria-expanded", "false");

  const meter = document.createElement("div");
  meter.className = "meter";
  meter.id = "novaveiGoalProgress";
  meter.setAttribute("role", "progressbar");
  meter.setAttribute("aria-labelledby", "novaveiGoalLabel");
  meter.setAttribute("aria-valuemin", "0");
  meter.setAttribute("aria-valuemax", "100");
  const meterFill = document.createElement("i");
  meter.appendChild(meterFill);

  const progress = document.createElement("span");
  progress.className = "goal-pct";

  const runStatus = document.createElement("span");
  runStatus.style.color = "var(--subtle)";
  runStatus.style.fontSize = "11.5px";
  runStatus.style.whiteSpace = "nowrap";
  runStatus.setAttribute("role", "status");
  runStatus.setAttribute("aria-live", "polite");

  const complete = document.createElement("button");
  complete.type = "button";
  complete.className = "btn ghost";
  complete.style.height = "30px";
  complete.style.padding = "0 10px";

  const clear = document.createElement("button");
  clear.type = "button";
  clear.className = "btn ghost";
  clear.textContent = "清除";
  clear.style.height = "30px";
  clear.style.padding = "0 10px";
  clear.setAttribute("aria-label", "清除当前会话目标");

  const editor = document.createElement("form");
  editor.id = "novaveiGoalEditor";
  editor.hidden = true;
  editor.style.display = "grid";
  editor.style.gridTemplateColumns =
    "minmax(0, 1fr) minmax(96px, auto) auto auto";
  editor.style.alignItems = "end";
  editor.style.gap = "8px";
  editor.style.width = "100%";
  editor.style.paddingTop = "2px";

  const field = document.createElement("div");
  field.style.minWidth = "0";
  const editorLabel = document.createElement("label");
  editorLabel.htmlFor = "novaveiGoalInput";
  editorLabel.textContent = "当前会话目标";
  editorLabel.style.display = "block";
  editorLabel.style.marginBottom = "4px";
  editorLabel.style.color = "var(--muted)";
  editorLabel.style.fontSize = "12px";
  const input = document.createElement("textarea");
  input.id = "novaveiGoalInput";
  input.rows = 2;
  input.maxLength = MAX_GOAL_LENGTH;
  input.placeholder = "描述这次会话要完成的事情";
  input.style.width = "100%";
  input.style.minHeight = "44px";
  input.style.resize = "vertical";
  input.style.padding = "8px 10px";
  input.style.border = "1px solid var(--line)";
  input.style.borderRadius = "10px";
  input.style.background = "var(--input-deep)";
  input.setAttribute("aria-describedby", "novaveiGoalEditorStatus");
  field.append(editorLabel, input);

  const progressField = document.createElement("div");
  progressField.style.minWidth = "96px";
  const progressLabel = document.createElement("label");
  progressLabel.htmlFor = "novaveiGoalProgressInput";
  progressLabel.textContent = "手动进度";
  progressLabel.style.display = "block";
  progressLabel.style.marginBottom = "4px";
  progressLabel.style.color = "var(--muted)";
  progressLabel.style.fontSize = "12px";
  const progressInput = document.createElement("input");
  progressInput.id = "novaveiGoalProgressInput";
  progressInput.type = "number";
  progressInput.min = "0";
  progressInput.max = "99";
  progressInput.step = "1";
  progressInput.inputMode = "numeric";
  progressInput.style.width = "100%";
  progressInput.style.minHeight = "44px";
  progressInput.style.padding = "8px 10px";
  progressInput.style.border = "1px solid var(--line)";
  progressInput.style.borderRadius = "10px";
  progressInput.style.background = "var(--input-deep)";
  progressInput.setAttribute("aria-describedby", "novaveiGoalEditorStatus");
  progressField.append(progressLabel, progressInput);

  const save = document.createElement("button");
  save.type = "submit";
  save.className = "btn primary";
  save.textContent = "保存";

  const cancel = document.createElement("button");
  cancel.type = "button";
  cancel.className = "btn ghost";
  cancel.textContent = "取消";

  const editorStatus = document.createElement("span");
  editorStatus.id = "novaveiGoalEditorStatus";
  editorStatus.setAttribute("role", "status");
  editorStatus.setAttribute("aria-live", "polite");
  editorStatus.style.gridColumn = "1 / -1";
  editorStatus.style.minHeight = "16px";
  editorStatus.style.color = "var(--subtle)";
  editorStatus.style.fontSize = "11.5px";

  editor.append(field, progressField, save, cancel, editorStatus);
  bar.append(
    label,
    summary,
    meter,
    progress,
    runStatus,
    complete,
    clear,
    editor,
  );

  return {
    bar,
    summary,
    complete,
    clear,
    editor,
    input,
    progressInput,
    save,
    cancel,
    meter,
    meterFill,
    progress,
    runStatus,
    editorStatus,
  };
}

export function installGoals() {
  const bar = document.getElementById("goalBar");
  const runtime = window.__novaveiPiRuntime;
  if (!bar || !runtime) return;

  const elements = createElements(bar);
  let activeSessionId = currentRealSessionId();
  let activeGoal: SessionGoal | undefined;
  let transientGoal: SessionGoal | undefined;
  let lastState = runtime.getState();
  let editorWasOpen = false;
  let saving = false;
  let goalError = "";
  let operation = 0;

  const runtimeRenderKey = (state: PiRuntimeSnapshot) =>
    `${state.requestId ?? ""}:${state.sessionId ?? ""}:${state.status}`;
  let lastRuntimeRenderKey = runtimeRenderKey(lastState);

  const goalForActiveSession = () =>
    activeSessionId ? activeGoal : transientGoal;

  const stateBelongsToActiveSession = () => {
    if (!lastState.requestId) return false;
    if (!activeSessionId) return true;
    return !lastState.sessionId || lastState.sessionId === activeSessionId;
  };

  const render = () => {
    const goal = goalForActiveSession();
    const text = goal?.text;
    // Empty sessions should not paint the goal strip. Show it only when there
    // is a goal, the editor is open, a write is in flight, or a goal error needs
    // a surface.
    const showBar =
      Boolean(goal) || editorWasOpen || saving || Boolean(goalError);
    elements.bar.classList.toggle("show", showBar);
    elements.bar.hidden = !showBar;
    elements.summary.textContent = preview(text);
    elements.summary.title = text || "点击设置当前会话目标";
    elements.summary.setAttribute(
      "aria-label",
      text ? `编辑当前会话目标：${text}` : "设置当前会话目标",
    );
    elements.summary.disabled = saving;
    elements.clear.disabled = !goal || saving;
    elements.complete.disabled = !goal || saving;
    elements.save.disabled = saving;
    elements.input.readOnly = saving;
    elements.progressInput.readOnly = saving;
    elements.complete.textContent =
      goal?.status === "completed" ? "重新打开" : "标记完成";
    elements.complete.setAttribute(
      "aria-label",
      goal?.status === "completed"
        ? "将当前目标重新标记为未完成"
        : "将当前目标标记为完成",
    );

    const progress = goal?.progress ?? 0;
    elements.meterFill.style.width = `${progress}%`;
    elements.meter.setAttribute("aria-valuenow", String(progress));
    elements.meter.setAttribute(
      "aria-valuetext",
      goal
        ? `目标手动进度 ${progress}%${goal.status === "completed" ? "，已完成" : "，未完成"}`
        : "当前会话尚未设置目标",
    );
    elements.progress.textContent = goal ? `${progress}%` : "—";

    const hint = runHint(stateBelongsToActiveSession() ? lastState : undefined);
    elements.runStatus.textContent = goal || editorWasOpen ? hint.label : "";
    elements.runStatus.title = goal || editorWasOpen ? hint.detail : "";
    elements.runStatus.dataset.goalRunState =
      goal || editorWasOpen ? hint.state : "hidden";
    elements.bar.setAttribute("aria-busy", saving ? "true" : "false");
    if (goalError && !editorWasOpen && showBar)
      elements.runStatus.textContent = "目标不可用";
  };

  function closeEditor(restoreFocus: boolean) {
    elements.editor.hidden = true;
    editorWasOpen = false;
    elements.summary.setAttribute("aria-expanded", "false");
    if (restoreFocus) elements.summary.focus();
  }

  const openEditor = () => {
    const goal = goalForActiveSession();
    elements.input.value = goal?.text ?? "";
    elements.progressInput.value = String(
      goal?.status === "completed" ? 99 : (goal?.progress ?? 0),
    );
    elements.editorStatus.textContent = activeSessionId
      ? "保存到当前本地会话。按 Enter 保存；按 Esc 或取消放弃更改。"
      : "会话尚未创建，目标暂存于当前界面。按 Enter 保存；按 Esc 或取消放弃更改。";
    elements.editor.hidden = false;
    editorWasOpen = true;
    elements.summary.setAttribute("aria-expanded", "true");
    window.requestAnimationFrame(() => elements.input.focus());
  };

  const hydrate = async (sessionId = currentRealSessionId()) => {
    if (sessionId !== activeSessionId) return;
    if (!sessionId) {
      activeGoal = undefined;
      render();
      return;
    }
    const request = ++operation;
    goalError = "";
    try {
      const result = await invokeGoal<unknown>("session_goal_get", {
        sessionId,
      });
      if (request !== operation || activeSessionId !== sessionId) return;
      activeGoal = readNativeGoal(result);
    } catch (error) {
      if (request !== operation || activeSessionId !== sessionId) return;
      goalError = error instanceof Error ? error.message : String(error);
      elements.editorStatus.textContent = `无法读取目标：${goalError}`;
    }
    render();
  };

  const writeGoal = async (
    sessionId: string | undefined,
    next: SessionGoal | undefined,
  ): Promise<boolean> => {
    if (!sessionId) {
      transientGoal = next;
      activeGoal = undefined;
      goalError = "";
      render();
      return true;
    }
    const request = ++operation;
    saving = true;
    goalError = "";
    // This is an optimistic visual draft only. Native get/set remains the
    // authority and replaces it as soon as the narrow command resolves.
    if (activeSessionId === sessionId) activeGoal = next;
    render();
    const dto: SessionGoalSetDto = next
      ? {
          sessionId,
          text: next.text,
          status: next.status,
          progress: next.progress,
        }
      : { sessionId, clear: true };
    try {
      const result = await invokeGoal<unknown>("session_goal_set", dto);
      const saved = readNativeGoal(result);
      if (next && !saved) throw new Error("本地目标保存响应无效");
      if (!next && saved) throw new Error("本地目标清除响应无效");
      if (request === operation && activeSessionId === sessionId) {
        activeGoal = saved;
        goalError = "";
      }
      return true;
    } catch (error) {
      if (request === operation && activeSessionId === sessionId) {
        goalError = error instanceof Error ? error.message : String(error);
        elements.editorStatus.textContent = `无法保存目标：${goalError}`;
        window.setTimeout(() => void hydrate(sessionId), 0);
      }
      return false;
    } finally {
      if (request === operation) {
        saving = false;
        render();
      }
    }
  };

  const refreshSession = () => {
    const nextSessionId = currentRealSessionId();
    if (nextSessionId === activeSessionId) return;
    const promoteTransientGoal = !activeSessionId ? transientGoal : undefined;
    activeSessionId = nextSessionId;
    activeGoal = undefined;
    saving = false;
    goalError = "";
    if (editorWasOpen) closeEditor(false);
    render();
    if (nextSessionId && promoteTransientGoal) {
      transientGoal = undefined;
      void writeGoal(nextSessionId, promoteTransientGoal);
    } else {
      void hydrate(nextSessionId);
    }
  };

  const saveEditor = async () => {
    const text = sanitizeGoal(elements.input.value);
    const current = goalForActiveSession();
    const progress = Number(elements.progressInput.value);
    if (
      text &&
      (!Number.isInteger(progress) || progress < 0 || progress > 99)
    ) {
      elements.editorStatus.textContent =
        "手动进度必须是 0 到 99 的整数；完成请使用“标记完成”。";
      elements.progressInput.focus();
      return;
    }
    const next = text
      ? {
          text,
          status: "active" as const,
          progress,
          updatedAt: current?.updatedAt ?? Date.now(),
        }
      : undefined;
    const saved = await writeGoal(activeSessionId, next);
    if (saved) closeEditor(true);
  };

  elements.summary.addEventListener("click", openEditor);
  elements.clear.addEventListener("click", () => {
    void writeGoal(activeSessionId, undefined);
  });
  elements.complete.addEventListener("click", () => {
    const goal = goalForActiveSession();
    if (!goal) return;
    const completed = goal.status !== "completed";
    void writeGoal(activeSessionId, {
      text: goal.text,
      status: completed ? "completed" : "active",
      progress: completed ? 100 : 0,
      updatedAt: goal.updatedAt,
    });
  });
  elements.cancel.addEventListener("click", () => closeEditor(true));
  elements.editor.addEventListener("submit", (event) => {
    event.preventDefault();
    void saveEditor();
  });
  elements.editor.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      closeEditor(true);
      return;
    }
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      void saveEditor();
    }
  });

  const unsubscribe = runtime.subscribe((state) => {
    const nextRenderKey = runtimeRenderKey(state);
    lastState = state;
    if (nextRenderKey === lastRuntimeRenderKey) return;
    lastRuntimeRenderKey = nextRenderKey;
    render();
  });

  const onGoalProgressUpdated = (event: Event) => {
    const detail = asRecord(
      (event as CustomEvent<SessionGoalUpdatedDetail>).detail,
    );
    const sessionId = detail?.sessionId;
    if (!isRealSessionId(sessionId) || sessionId !== activeSessionId) return;
    void hydrate(sessionId);
  };
  window.addEventListener(SESSION_GOAL_UPDATED_EVENT, onGoalProgressUpdated);

  // NativeShell replaces session buttons after loading and handles the actual
  // switch in a document-capture listener. Window capture observes the intent
  // first; the short poll is the reliable fallback for keyboard or programmatic
  // session changes without relying on prototype-only events.
  const requestSessionRefresh = () => window.setTimeout(refreshSession, 0);
  const onSessionIntent = (event: Event) => {
    const target = event.target instanceof Element ? event.target : null;
    if (
      target?.closest(
        ".session[data-session-id], #btnNewChat, .project-row[data-workdir]",
      )
    )
      requestSessionRefresh();
  };
  window.addEventListener("click", onSessionIntent, true);
  const observerTarget = document.getElementById("sessionSidebar");
  const observer = observerTarget
    ? new MutationObserver(requestSessionRefresh)
    : undefined;
  if (observer && observerTarget) {
    observer.observe(observerTarget, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["aria-current", "data-session-id", "class"],
    });
  }
  const sessionPoll = window.setInterval(refreshSession, SESSION_POLL_MS);

  render();
  void hydrate(activeSessionId);
  return () => {
    unsubscribe();
    window.removeEventListener(
      SESSION_GOAL_UPDATED_EVENT,
      onGoalProgressUpdated,
    );
    window.removeEventListener("click", onSessionIntent, true);
    observer?.disconnect();
    window.clearInterval(sessionPoll);
  };
}
