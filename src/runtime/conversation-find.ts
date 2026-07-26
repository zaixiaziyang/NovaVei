type Invoke = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

type ConversationMessage = {
  id: string;
  role: "user" | "assistant";
  text: string;
  createdAt?: number;
};

type ConversationPage = {
  messages: ConversationMessage[];
  hasMoreBefore: boolean;
};

type ConversationCursor = {
  createdAt: number;
  id: string;
};

type ConversationCache = {
  sessionId: string;
  messages: ConversationMessage[];
  cursor?: ConversationCursor;
  hasMoreBefore: boolean;
};

type FindMatch = ConversationMessage;

type FindPhase = "idle" | "searching" | "ready" | "error" | "navigating";

type FindState = {
  sessionId?: string;
  query: string;
  matches: FindMatch[];
  currentIndex: number;
  phase: FindPhase;
  loadedMessages: number;
  hasMoreBefore: boolean;
  error?: string;
};

type Copy = {
  label: string;
  placeholder: string;
  previous: string;
  next: string;
  earlier: string;
  close: string;
  empty: string;
  searching: (count: number) => string;
  noResults: string;
  partialNoResults: (count: number) => string;
  count: (current: number, total: number) => string;
  partialCount: (current: number, total: number, count: number) => string;
  locating: (current: number, total: number, step: number) => string;
  unavailable: string;
  interrupted: string;
  revealFailed: string;
};

const SEARCH_DEBOUNCE_MS = 140;
const MESSAGE_PAGE_SIZE = 200;
const REVEAL_WAIT_MS = 4_500;

const COPY: Record<"zh" | "en", Copy> = {
  zh: {
    label: "在当前会话中查找",
    placeholder: "查找当前会话",
    previous: "上一处",
    next: "下一处",
    earlier: "继续搜索更早消息",
    close: "关闭",
    empty: "输入关键词以搜索当前会话",
    searching: (count) =>
      count > 0
        ? `正在搜索当前会话…已读取 ${count} 条消息`
        : "正在搜索当前会话…",
    noResults: "当前会话中没有匹配项",
    partialNoResults: (count) =>
      `已搜索最近 ${count} 条消息，暂无匹配；还可继续搜索更早消息`,
    count: (current, total) => `${current} / ${total} 条匹配`,
    partialCount: (current, total, count) =>
      `${current} / ${total} 条匹配（已搜索最近 ${count} 条）`,
    locating: (current, total, step) =>
      `正在定位 ${current} / ${total} 条匹配…（加载历史 ${step}）`,
    unavailable: "当前会话历史暂不可搜索",
    interrupted: "会话已切换，已停止查找",
    revealFailed: "已找到匹配项，但未能加载到对应的会话位置",
  },
  en: {
    label: "Find in this conversation",
    placeholder: "Find in this conversation",
    previous: "Previous",
    next: "Next",
    earlier: "Search earlier messages",
    close: "Close",
    empty: "Type to search this conversation",
    searching: (count) =>
      count > 0
        ? `Searching this conversation… ${count} messages loaded`
        : "Searching this conversation…",
    noResults: "No matches in this conversation",
    partialNoResults: (count) =>
      `No matches in the latest ${count} messages; earlier messages remain`,
    count: (current, total) => `${current} / ${total} matches`,
    partialCount: (current, total, count) =>
      `${current} / ${total} matches (${count} latest messages searched)`,
    locating: (current, total, step) =>
      `Locating ${current} / ${total}… (loading history ${step})`,
    unavailable: "This conversation history is not available for search",
    interrupted: "Conversation changed; search stopped",
    revealFailed:
      "A match was found, but its conversation position could not be loaded",
  },
};

function invokeApi(): Invoke | undefined {
  return window.__TAURI__?.core?.invoke as Invoke | undefined;
}

function element<T extends HTMLElement>(id: string): T | null {
  return document.getElementById(id) as T | null;
}

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function cleanText(value: unknown) {
  return typeof value === "string" ? value.replace(/\s+/gu, " ").trim() : "";
}

function validMessageId(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 256 &&
    !/[\u0000-\u001F\u007F]/u.test(value)
  );
}

function currentLanguage(): "zh" | "en" {
  return document.documentElement.lang.toLowerCase().startsWith("en")
    ? "en"
    : "zh";
}

function normalize(value: string) {
  return value.normalize("NFKC").toLocaleLowerCase();
}

function messageFromValue(value: unknown): ConversationMessage | undefined {
  const source = record(value);
  const id = source?.id;
  const role =
    typeof source?.role === "string" ? source.role.toLowerCase() : "";
  const text = cleanText(source?.content ?? source?.text);
  if (!validMessageId(id) || !text || (role !== "user" && role !== "assistant"))
    return undefined;
  const createdAt =
    typeof source?.createdAt === "number" && Number.isFinite(source.createdAt)
      ? source.createdAt
      : typeof source?.created_at === "number" &&
          Number.isFinite(source.created_at)
        ? source.created_at
        : undefined;
  return { id, role, text, createdAt };
}

function pageFromValue(value: unknown): ConversationPage | undefined {
  const source = record(value);
  if (!source || !Array.isArray(source.messages)) return undefined;
  const seen = new Set<string>();
  const messages: ConversationMessage[] = [];
  for (const value of source.messages) {
    const message = messageFromValue(value);
    if (!message || seen.has(message.id)) continue;
    seen.add(message.id);
    messages.push(message);
  }
  return {
    messages,
    hasMoreBefore:
      source.hasMoreBefore === true || source.has_more_before === true,
  };
}

function pageCursor(message: ConversationMessage | undefined) {
  if (!message || !Number.isFinite(message.createdAt)) return undefined;
  return { createdAt: message.createdAt as number, id: message.id };
}

function mergeMessages(
  older: ConversationMessage[],
  newer: ConversationMessage[],
) {
  const seen = new Set<string>();
  return [...older, ...newer].filter(
    (message) => !seen.has(message.id) && Boolean(seen.add(message.id)),
  );
}

function renderedMessage(id: string) {
  return [...document.querySelectorAll<HTMLElement>("[data-message-id]")].find(
    (candidate) => candidate.dataset.messageId === id,
  );
}

function removeCurrentMark() {
  document
    .querySelectorAll<HTMLElement>("[data-novavei-conversation-find-current]")
    .forEach((node) =>
      node.removeAttribute("data-novavei-conversation-find-current"),
    );
}

function focusMatch(target: HTMLElement) {
  removeCurrentMark();
  target.dataset.novaveiConversationFindCurrent = "true";
  target.scrollIntoView({
    block: "center",
    behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches
      ? "auto"
      : "smooth",
  });
}

function waitForAxisMutation(axis: HTMLElement) {
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
    const timeout = window.setTimeout(() => finish(false), REVEAL_WAIT_MS);
    observer.observe(axis, { childList: true, subtree: true });
  });
}

function installStyles() {
  if (document.getElementById("novaveiConversationFindStyles")) return;
  const style = document.createElement("style");
  style.id = "novaveiConversationFindStyles";
  style.textContent = `
    .novavei-conversation-find {
      position: sticky; top: 8px; z-index: 22; display: grid;
      grid-template-columns: minmax(0, 1fr) auto; gap: 8px; align-items: center;
      margin: 8px 10px; padding: 8px; border: 1px solid var(--line);
      border-radius: 12px; background: color-mix(in srgb, var(--card) 94%, transparent);
      box-shadow: var(--shadow-sm); backdrop-filter: blur(12px);
    }
    .novavei-conversation-find[hidden] { display: none; }
    .novavei-conversation-find-form { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px; align-items: center; }
    .novavei-conversation-find input {
      min-width: 0; height: 34px; padding: 0 10px; border: 1px solid var(--line);
      border-radius: 8px; color: var(--text); background: var(--input, var(--card));
      font: inherit;
    }
    .novavei-conversation-find input:focus-visible,
    .novavei-conversation-find button:focus-visible {
      outline: 2px solid rgb(107 181 255 / 80%); outline-offset: 2px;
    }
    .novavei-conversation-find-actions { display: inline-flex; flex-wrap: wrap; gap: 5px; align-items: center; justify-content: flex-end; }
    .novavei-conversation-find button {
      min-height: 32px; border: 1px solid var(--line); border-radius: 8px;
      padding: 0 9px; color: var(--text); background: transparent; cursor: pointer;
      font: inherit; font-size: 12px;
    }
    .novavei-conversation-find button:hover:not(:disabled) { background: var(--hover); }
    .novavei-conversation-find button:disabled { opacity: .48; cursor: default; }
    .novavei-conversation-find button[hidden] { display: none; }
    .novavei-conversation-find-status {
      grid-column: 1 / -1; margin: 0; color: var(--muted); font-size: 12px; line-height: 1.35;
    }
    [data-novavei-conversation-find-current="true"] {
      outline: 2px solid color-mix(in srgb, var(--blue-strong, #6bb5ff) 78%, transparent);
      outline-offset: 4px; border-radius: 10px;
    }
    @media (max-width: 620px) {
      .novavei-conversation-find { grid-template-columns: 1fr; }
      .novavei-conversation-find-form { grid-template-columns: 1fr; }
      .novavei-conversation-find-actions { justify-content: flex-end; }
    }
  `;
  document.head.appendChild(style);
}

/**
 * Current-conversation find deliberately sits beside (rather than inside) the
 * Ctrl/Cmd+K global history palette. It searches only the selected session and
 * uses the native paged transcript API, so virtualized DOM windows do not make
 * older results disappear from the count.
 */
export function installConversationFind() {
  if (document.getElementById("novaveiConversationFind")) return;
  const transcript = element<HTMLElement>("transcript");
  const axis = element<HTMLElement>("transcriptAxis");
  if (!transcript || !axis) return;

  installStyles();
  const root = document.createElement("section");
  root.id = "novaveiConversationFind";
  root.className = "novavei-conversation-find";
  root.hidden = true;
  root.setAttribute("role", "search");
  root.setAttribute("aria-controls", axis.id || "transcriptAxis");
  root.setAttribute("aria-keyshortcuts", "Control+F Meta+F Escape");

  const form = document.createElement("form");
  form.className = "novavei-conversation-find-form";
  form.noValidate = true;
  const input = document.createElement("input");
  input.type = "search";
  input.id = "novaveiConversationFindInput";
  input.autocomplete = "off";
  input.spellcheck = false;
  input.setAttribute("aria-controls", axis.id || "transcriptAxis");
  input.setAttribute("aria-keyshortcuts", "Enter Shift+Enter Escape");
  const actions = document.createElement("div");
  actions.className = "novavei-conversation-find-actions";
  const previous = document.createElement("button");
  previous.type = "button";
  const next = document.createElement("button");
  next.type = "button";
  const earlier = document.createElement("button");
  earlier.type = "button";
  const close = document.createElement("button");
  close.type = "button";
  actions.append(previous, next, earlier, close);
  form.append(input, actions);
  const status = document.createElement("p");
  status.id = "novaveiConversationFindStatus";
  status.className = "novavei-conversation-find-status";
  status.setAttribute("role", "status");
  status.setAttribute("aria-live", "polite");
  status.setAttribute("aria-atomic", "true");
  input.setAttribute("aria-describedby", status.id);
  root.append(form, status);
  transcript.insertBefore(root, axis);

  let state: FindState = {
    query: "",
    matches: [],
    currentIndex: -1,
    phase: "idle",
    loadedMessages: 0,
    hasMoreBefore: false,
  };
  let searchSerial = 0;
  let navigationSerial = 0;
  let timer: number | undefined;
  let returnFocus: HTMLElement | undefined;
  let cachedConversation: ConversationCache | undefined;
  const renderedMessageIds = () =>
    [...axis.querySelectorAll<HTMLElement>("[data-message-id]")]
      .map((node) => node.dataset.messageId)
      .filter((id): id is string => Boolean(id));
  let observedMessageIds = renderedMessageIds();

  const copy = () => COPY[currentLanguage()];
  const activeSessionId = () => window.__novaveiHost?.getSessionId?.()?.trim();
  const activeConversationKey = () =>
    activeSessionId() || (invokeApi() ? undefined : "browser-preview");
  const isCurrent = (sessionId: string | undefined, serial: number) =>
    Boolean(
      sessionId &&
        sessionId === activeConversationKey() &&
        serial === searchSerial,
    );

  const render = () => {
    const text = copy();
    root.setAttribute("aria-label", text.label);
    input.placeholder = text.placeholder;
    input.setAttribute("aria-label", text.label);
    previous.textContent = text.previous;
    previous.title = text.previous;
    next.textContent = text.next;
    next.title = text.next;
    earlier.textContent = text.earlier;
    earlier.title = text.earlier;
    close.textContent = text.close;
    close.title = text.close;
    status.setAttribute("role", state.phase === "error" ? "alert" : "status");
    status.setAttribute(
      "aria-live",
      state.phase === "error" ? "assertive" : "polite",
    );
    const hasMatches = state.matches.length > 0;
    const busy = state.phase === "searching" || state.phase === "navigating";
    previous.disabled = !hasMatches || busy;
    next.disabled = !hasMatches || busy;
    earlier.hidden = !state.query || !state.hasMoreBefore;
    earlier.disabled = busy || !state.hasMoreBefore;
    if (!state.query) status.textContent = text.empty;
    else if (state.phase === "searching")
      status.textContent = text.searching(state.loadedMessages);
    else if (state.phase === "error")
      status.textContent = state.error || text.unavailable;
    else if (!hasMatches)
      status.textContent = state.hasMoreBefore
        ? text.partialNoResults(state.loadedMessages)
        : text.noResults;
    else if (state.phase === "navigating")
      status.textContent = text.locating(
        state.currentIndex + 1,
        state.matches.length,
        Math.max(1, state.loadedMessages),
      );
    else if (state.hasMoreBefore)
      status.textContent = text.partialCount(
        state.currentIndex + 1,
        state.matches.length,
        state.loadedMessages,
      );
    else
      status.textContent = text.count(
        state.currentIndex + 1,
        state.matches.length,
      );
  };

  const clearSearch = () => {
    if (timer !== undefined) {
      window.clearTimeout(timer);
      timer = undefined;
    }
    searchSerial += 1;
    navigationSerial += 1;
    state = {
      query: "",
      matches: [],
      currentIndex: -1,
      phase: "idle",
      loadedMessages: 0,
      hasMoreBefore: false,
    };
    cachedConversation = undefined;
    removeCurrentMark();
    render();
  };

  const closeFind = (restoreFocus = true) => {
    if (root.hidden) return;
    root.hidden = true;
    input.value = "";
    clearSearch();
    if (!restoreFocus) return;
    const target = returnFocus?.isConnected
      ? returnFocus
      : element<HTMLTextAreaElement>("composerInput");
    target?.focus({ preventScroll: true });
  };

  const openFind = () => {
    if (root.hidden) {
      const active = document.activeElement;
      returnFocus = active instanceof HTMLElement ? active : undefined;
      root.hidden = false;
    }
    render();
    window.requestAnimationFrame(() => {
      input.focus({ preventScroll: true });
      input.select();
    });
  };

  const requestConversationPage = async (
    sessionId: string,
    cursor?: ConversationCursor,
  ) => {
    const invoke = invokeApi();
    if (!invoke) throw new Error(copy().unavailable);
    const args: Record<string, unknown> = {
      sessionId,
      session_id: sessionId,
      limit: MESSAGE_PAGE_SIZE,
    };
    if (cursor) {
      args.beforeCreatedAt = cursor.createdAt;
      args.before_created_at = cursor.createdAt;
      args.beforeId = cursor.id;
      args.before_id = cursor.id;
    }
    const page = pageFromValue(await invoke<unknown>("sessions_get", args));
    if (!page) throw new Error(copy().unavailable);
    return page;
  };

  const loadConversation = async (sessionId: string, serial: number) => {
    if (cachedConversation?.sessionId === sessionId) return cachedConversation;
    const invoke = invokeApi();
    if (!invoke) {
      const visible = [
        ...axis.querySelectorAll<HTMLElement>(".msg-user, .msg-assistant"),
      ]
        .map((node, index) => {
          const role = node.classList.contains("msg-user")
            ? "user"
            : "assistant";
          const id = node.dataset.messageId || `preview-${role}-${index}`;
          // Static preview assistant cards predate durable message ids. Give
          // each a renderer-only id so its find result can still be focused.
          if (!node.dataset.messageId) node.dataset.messageId = id;
          const text = cleanText(node.textContent);
          return text
            ? ({ id, role, text } satisfies ConversationMessage)
            : undefined;
        })
        .filter((message): message is ConversationMessage => Boolean(message));
      cachedConversation = {
        sessionId,
        messages: visible,
        cursor: pageCursor(visible[0]),
        hasMoreBefore: false,
      };
      return cachedConversation;
    }

    if (!isCurrent(sessionId, serial)) return undefined;
    const page = await requestConversationPage(sessionId);
    if (!isCurrent(sessionId, serial)) return undefined;
    const cursor = pageCursor(page.messages[0]);
    if (page.hasMoreBefore && !cursor) throw new Error(copy().unavailable);
    cachedConversation = {
      sessionId,
      messages: page.messages,
      cursor,
      hasMoreBefore: page.hasMoreBefore,
    };
    return cachedConversation;
  };

  const loadEarlierConversation = async (sessionId: string, serial: number) => {
    const current = await loadConversation(sessionId, serial);
    if (!current || !isCurrent(sessionId, serial)) return undefined;
    if (!current.hasMoreBefore) return current;
    const cursor = current.cursor;
    if (!cursor) throw new Error(copy().unavailable);
    const page = await requestConversationPage(sessionId, cursor);
    if (!isCurrent(sessionId, serial) || cachedConversation !== current)
      return undefined;
    const nextCursor = pageCursor(page.messages[0]);
    const madeCursorProgress =
      nextCursor &&
      (nextCursor.createdAt !== cursor.createdAt ||
        nextCursor.id !== cursor.id);
    const messages = mergeMessages(page.messages, current.messages);
    if (
      page.hasMoreBefore &&
      (!madeCursorProgress || messages.length === current.messages.length)
    )
      throw new Error(copy().unavailable);
    cachedConversation = {
      sessionId,
      messages,
      cursor: pageCursor(messages[0]),
      hasMoreBefore: page.hasMoreBefore,
    };
    return cachedConversation;
  };

  const search = async (
    options: { preferredMatchId?: string; revealFirst?: boolean } = {},
  ) => {
    if (timer !== undefined) {
      window.clearTimeout(timer);
      timer = undefined;
    }
    const query = cleanText(input.value);
    const serial = ++searchSerial;
    navigationSerial += 1;
    const sessionId = activeConversationKey();
    if (!query) {
      state = {
        query: "",
        matches: [],
        currentIndex: -1,
        phase: "idle",
        loadedMessages: 0,
        hasMoreBefore: false,
      };
      removeCurrentMark();
      render();
      return;
    }
    if (!sessionId) {
      state = {
        query,
        matches: [],
        currentIndex: -1,
        phase: "error",
        loadedMessages: 0,
        hasMoreBefore: false,
        error: copy().unavailable,
      };
      render();
      return;
    }
    state = {
      sessionId,
      query,
      matches: [],
      currentIndex: -1,
      phase: "searching",
      loadedMessages:
        cachedConversation?.sessionId === sessionId
          ? cachedConversation.messages.length
          : 0,
      hasMoreBefore:
        cachedConversation?.sessionId === sessionId
          ? cachedConversation.hasMoreBefore
          : false,
    };
    if (options.revealFirst !== false) removeCurrentMark();
    render();
    try {
      const conversation = await loadConversation(sessionId, serial);
      if (!conversation || !isCurrent(sessionId, serial)) return;
      const needle = normalize(query);
      const matches = conversation.messages.filter((message) =>
        normalize(message.text).includes(needle),
      );
      const preferredIndex = options.preferredMatchId
        ? matches.findIndex((match) => match.id === options.preferredMatchId)
        : -1;
      if (options.revealFirst === false && preferredIndex < 0)
        removeCurrentMark();
      state = {
        sessionId,
        query,
        matches,
        currentIndex:
          preferredIndex >= 0 ? preferredIndex : matches.length > 0 ? 0 : -1,
        phase: "ready",
        loadedMessages: conversation.messages.length,
        hasMoreBefore: conversation.hasMoreBefore,
      };
      render();
      if (matches.length > 0 && options.revealFirst !== false)
        await navigate(0, serial);
    } catch (error) {
      if (!isCurrent(sessionId, serial)) return;
      state = {
        sessionId,
        query,
        matches: [],
        currentIndex: -1,
        phase: "error",
        loadedMessages: state.loadedMessages,
        hasMoreBefore: state.hasMoreBefore,
        error: error instanceof Error ? error.message : copy().unavailable,
      };
      render();
    }
  };

  const revealMatch = async (
    match: FindMatch,
    sessionId: string,
    serial: number,
  ) => {
    const attemptedWindows = new Set<string>();
    while (true) {
      if (!isCurrent(sessionId, serial)) return undefined;
      const direct = renderedMessage(match.id);
      if (direct) return direct;
      const currentAxis = element<HTMLElement>("transcriptAxis") ?? axis;
      const windowKey = [
        ...currentAxis.querySelectorAll<HTMLElement>("[data-message-id]"),
      ]
        .map((node) => node.dataset.messageId)
        .filter((id): id is string => Boolean(id))
        .join("\u001F");
      // A repeated window means that paging has stopped making progress. Do
      // not report a false location or spin forever if a host integration is
      // unavailable.
      if (!windowKey || attemptedWindows.has(windowKey)) return undefined;
      attemptedWindows.add(windowKey);
      const control = document.querySelector<HTMLButtonElement>(
        "[data-novavei-load-earlier]",
      );
      if (!control || control.disabled) return undefined;
      state.loadedMessages = attemptedWindows.size;
      render();
      const changed = waitForAxisMutation(currentAxis);
      control.click();
      if (!(await changed)) return undefined;
      const target = renderedMessage(match.id);
      if (target) return target;
      const nextWindowKey = [
        ...currentAxis.querySelectorAll<HTMLElement>("[data-message-id]"),
      ]
        .map((node) => node.dataset.messageId)
        .filter((id): id is string => Boolean(id))
        .join("\u001F");
      if (nextWindowKey === windowKey) return undefined;
    }
  };

  const navigate = async (
    direction: number,
    expectedSearchSerial = searchSerial,
  ) => {
    if (!state.matches.length || !state.sessionId) return;
    const sessionId = state.sessionId;
    const navigation = ++navigationSerial;
    if (direction !== 0) {
      state.currentIndex =
        (state.currentIndex + direction + state.matches.length) %
        state.matches.length;
    }
    state.phase = "navigating";
    state.loadedMessages = 0;
    render();
    const match = state.matches[state.currentIndex];
    const target = await revealMatch(match, sessionId, expectedSearchSerial);
    if (
      navigation !== navigationSerial ||
      expectedSearchSerial !== searchSerial ||
      sessionId !== activeConversationKey()
    )
      return;
    state.phase = "ready";
    state.loadedMessages =
      cachedConversation?.sessionId === sessionId
        ? cachedConversation.messages.length
        : 0;
    if (target) focusMatch(target);
    else {
      state.phase = "error";
      state.error = copy().revealFailed;
    }
    render();
  };

  const searchEarlier = async () => {
    const sessionId = state.sessionId;
    const query = state.query;
    if (
      !sessionId ||
      !query ||
      !state.hasMoreBefore ||
      state.phase === "searching" ||
      state.phase === "navigating"
    )
      return;
    if (cleanText(input.value) !== query) {
      await search();
      return;
    }
    if (timer !== undefined) {
      window.clearTimeout(timer);
      timer = undefined;
    }
    const serial = ++searchSerial;
    navigationSerial += 1;
    const previousMatchId = state.matches[state.currentIndex]?.id;
    const hadMatches = state.matches.length > 0;
    state.phase = "searching";
    state.error = undefined;
    render();
    try {
      const conversation = await loadEarlierConversation(sessionId, serial);
      if (
        !conversation ||
        !isCurrent(sessionId, serial) ||
        cleanText(input.value) !== query
      )
        return;
      const needle = normalize(query);
      const matches = conversation.messages.filter((message) =>
        normalize(message.text).includes(needle),
      );
      const previousIndex = previousMatchId
        ? matches.findIndex((match) => match.id === previousMatchId)
        : -1;
      state = {
        sessionId,
        query,
        matches,
        currentIndex:
          previousIndex >= 0 ? previousIndex : matches.length > 0 ? 0 : -1,
        phase: "ready",
        loadedMessages: conversation.messages.length,
        hasMoreBefore: conversation.hasMoreBefore,
      };
      render();
      if (!hadMatches && matches.length > 0) await navigate(0, serial);
    } catch (error) {
      if (!isCurrent(sessionId, serial)) return;
      state.phase = "error";
      state.error = error instanceof Error ? error.message : copy().unavailable;
      render();
    }
  };

  const scheduleSearch = (
    options: { preferredMatchId?: string; revealFirst?: boolean } = {},
  ) => {
    if (timer !== undefined) window.clearTimeout(timer);
    timer = window.setTimeout(() => {
      timer = undefined;
      void search(options);
    }, SEARCH_DEBOUNCE_MS);
  };

  const searchThenNavigate = (direction: -1 | 1) => {
    const hadPendingSearch = timer !== undefined;
    if (timer !== undefined) {
      window.clearTimeout(timer);
      timer = undefined;
    }
    const query = cleanText(input.value);
    const continuingQuery = query === state.query;
    const preferredMatchId = continuingQuery
      ? state.matches[state.currentIndex]?.id
      : undefined;
    if (
      hadPendingSearch ||
      !continuingQuery ||
      state.phase === "idle" ||
      state.phase === "error" ||
      state.phase === "searching" ||
      cachedConversation?.sessionId !== state.sessionId
    ) {
      const pendingSearch = search({ preferredMatchId, revealFirst: false });
      const expectedSearchSerial = searchSerial;
      void pendingSearch.then(() => {
        if (
          expectedSearchSerial !== searchSerial ||
          cleanText(input.value) !== query ||
          state.phase !== "ready" ||
          !state.matches.length
        )
          return;
        if (
          preferredMatchId &&
          state.matches[state.currentIndex]?.id === preferredMatchId
        ) {
          void navigate(direction, expectedSearchSerial);
          return;
        }
        state.currentIndex = direction < 0 ? state.matches.length - 1 : 0;
        void navigate(0, expectedSearchSerial);
      });
      return;
    }
    void navigate(direction);
  };

  previous.addEventListener("click", () => searchThenNavigate(-1));
  next.addEventListener("click", () => searchThenNavigate(1));
  earlier.addEventListener("click", () => void searchEarlier());
  close.addEventListener("click", () => closeFind());
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    searchThenNavigate(1);
  });
  input.addEventListener("input", () => scheduleSearch());
  input.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closeFind();
      return;
    }
    if (event.key === "Enter" && !event.isComposing) {
      event.preventDefault();
      searchThenNavigate(event.shiftKey ? -1 : 1);
    }
  });
  document.addEventListener(
    "keydown",
    (event) => {
      if (event.key === "Escape" && !root.hidden) {
        event.preventDefault();
        event.stopImmediatePropagation();
        closeFind();
        return;
      }
      if (
        event.defaultPrevented ||
        event.isComposing ||
        event.altKey ||
        event.shiftKey ||
        !(event.ctrlKey || event.metaKey) ||
        event.key.toLowerCase() !== "f"
      )
        return;
      // A current-session search must remain available from the composer and
      // other editable controls too; it never mutates their draft text.
      event.preventDefault();
      event.stopImmediatePropagation();
      openFind();
    },
    true,
  );
  window.addEventListener("novavei:session-changed", () => {
    if (timer !== undefined) {
      window.clearTimeout(timer);
      timer = undefined;
    }
    cachedConversation = undefined;
    observedMessageIds = renderedMessageIds();
    if (root.hidden) {
      return;
    }
    const query = cleanText(input.value);
    searchSerial += 1;
    navigationSerial += 1;
    removeCurrentMark();
    if (!query) {
      state = {
        query: "",
        matches: [],
        currentIndex: -1,
        phase: "idle",
        loadedMessages: 0,
        hasMoreBefore: false,
      };
      render();
      return;
    }
    state = {
      sessionId: activeConversationKey(),
      query,
      matches: [],
      currentIndex: -1,
      phase: "error",
      loadedMessages: 0,
      hasMoreBefore: false,
      error: copy().interrupted,
    };
    render();
  });
  const transcriptObserver = new MutationObserver(() => {
    const nextMessageIds = renderedMessageIds();
    const previousMessageIds = observedMessageIds;
    observedMessageIds = nextMessageIds;
    const sessionId = activeConversationKey();
    if (!sessionId) return;
    const previousIds = new Set(previousMessageIds);
    let appendedIds: string[];
    if (previousMessageIds.length === 0) {
      appendedIds = nextMessageIds;
    } else {
      const previousTail = previousMessageIds[previousMessageIds.length - 1];
      const tailIndex = nextMessageIds.indexOf(previousTail);
      if (tailIndex < 0) return;
      appendedIds = nextMessageIds.slice(tailIndex + 1);
    }
    if (!appendedIds.some((id) => !previousIds.has(id))) return;
    const currentCache =
      cachedConversation?.sessionId === sessionId
        ? cachedConversation
        : undefined;
    if (currentCache) {
      const cachedIds = new Set(
        currentCache.messages.map((message) => message.id),
      );
      if (appendedIds.every((id) => cachedIds.has(id))) return;
      cachedConversation = undefined;
    }
    const query = cleanText(input.value);
    if (
      root.hidden ||
      !query ||
      state.phase === "navigating" ||
      query !== state.query
    )
      return;
    scheduleSearch({
      preferredMatchId: state.matches[state.currentIndex]?.id,
      revealFirst: false,
    });
  });
  transcriptObserver.observe(axis, { childList: true });
  window.addEventListener("novavei:language-changed", render);
  render();
}
