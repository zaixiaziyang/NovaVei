/**
 * User-message editable bubble + row actions.
 *
 * Each durable user bubble gets a hover / keyboard-focus toolstrip: Copy and
 * Edit. Editing swaps a textarea into the bubble in place (Esc cancels and
 * restores the original body and scroll position; Ctrl/⌘+Enter saves). Save
 * performs a "resend from this floor": truncate the session at this message,
 * then resubmit the edited text as a fresh turn.
 *
 * All editable actions require a stable message identity. Historic rows
 * without a durable id disable the Edit button and explain why.
 */

import type { PiRuntimePublicApi } from "./types";

type InvokeFn = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

type NativeShellShape = {
  getSessionId: () => string | undefined;
  refreshSessions: (options?: { loadActive?: boolean }) => Promise<void>;
};

/** Marker while a bubble is being edited; blocks duplicate enters. */
const EDIT_GUARD = "novaveiEditing";
/** Prior body node (text bubble or `.novavei-user-message-copy`) stored so a
 * cancel can restore the exact element, listeners included. */
const originalBody = new WeakMap<HTMLElement, Node | null>();

function node<T extends HTMLElement>(id: string) {
  return document.getElementById(id) as T | null;
}

function isEnglish() {
  return document.documentElement.lang.toLowerCase().startsWith("en");
}

function text(zh: string, en: string) {
  return isEnglish() ? en : zh;
}

function toast(message: string) {
  const target = node<HTMLElement>("toast");
  if (!target) {
    console.warn("[NovaVei] message edit", message);
    return;
  }
  target.textContent = message;
  target.classList.add("show");
  window.setTimeout(() => target.classList.remove("show"), 2200);
}

function runtimeApi(): PiRuntimePublicApi | undefined {
  return window.__novaveiPiRuntime;
}

function hostApi(): NativeShellShape | undefined {
  const host = window.__novaveiHost as NativeShellShape | undefined;
  if (!host?.getSessionId || !host?.refreshSessions) return undefined;
  return host;
}

function invokeApi(): InvokeFn | undefined {
  return window.__TAURI__?.core?.invoke as InvokeFn | undefined;
}

/**
 * The anchor used for truncation. A durable `data-history-message-id` is the
 * only trustworthy anchor; a live id is accepted only when it is a real
 * streamed turn id (never a `legacy:` synthetic key). Absent → not editable.
 */
function durableMessageId(bubble: HTMLElement): string | undefined {
  const durable = bubble.dataset.historyMessageId?.trim();
  if (durable) return durable;
  const liveId = bubble.dataset.liveMessageId?.trim();
  if (liveId && !liveId.startsWith("legacy:")) return liveId;
  return undefined;
}

/** The plain text source of the bubble, independent of rendered media. */
function bubbleSourceText(bubble: HTMLElement): string {
  const stored = bubble.dataset.historyContent?.trim();
  if (stored) return stored;
  const copySpan = bubble.querySelector<HTMLElement>(
    ".novavei-user-message-copy",
  );
  if (copySpan?.textContent?.trim()) return copySpan.textContent.trim();
  return bubble.textContent?.trim() ?? "";
}

function findBodyNode(bubble: HTMLElement): Node | null {
  // Prefer the copy span (used when media are present); otherwise capture the
  // first child node so a pure-text bubble can restore the exact text node.
  const copySpan = bubble.querySelector<HTMLElement>(
    ".novavei-user-message-copy",
  );
  if (copySpan) return copySpan;
  const tray = bubble.querySelector(".novavei-message-attachments");
  for (const child of bubble.childNodes) {
    if (child === tray) continue;
    if (child.nodeType === Node.ELEMENT_NODE) return child;
    if (child.nodeType === Node.TEXT_NODE && child.textContent?.trim())
      return child;
  }
  return null;
}

function buildActionStrip(bubble: HTMLElement, _editable: boolean) {
  const existing = bubble.querySelector<HTMLElement>("[data-message-actions]");
  if (existing) return existing;

  const strip = document.createElement("div");
  strip.className = "msg-actions msg-user-actions";
  strip.dataset.messageActions = "true";
  strip.setAttribute("aria-hidden", "false");

  const copy = document.createElement("button");
  copy.type = "button";
  copy.dataset.messageEditAction = "copy";
  copy.textContent = text("复制", "Copy");
  copy.setAttribute("aria-label", text("复制该条消息", "Copy this message"));
  copy.addEventListener("click", (event) => {
    event.stopPropagation();
    void copyBubble(bubble, copy);
  });
  strip.appendChild(copy);

  const edit = document.createElement("button");
  edit.type = "button";
  edit.dataset.messageEditAction = "edit";
  edit.textContent = text("编辑", "Edit");
  edit.setAttribute("aria-label", text("编辑该条消息", "Edit this message"));
  if (!isEditable(bubble)) {
    edit.disabled = true;
    edit.setAttribute("aria-disabled", "true");
    edit.title = text(
      "旧消息未保存消息标识，无法安全编辑",
      "This message has no saved identity and cannot be edited",
    );
  }
  edit.addEventListener("click", (event) => {
    event.stopPropagation();
    void beginEdit(bubble).catch((error) =>
      toast(error instanceof Error ? error.message : String(error)),
    );
  });
  strip.appendChild(edit);

  bubble.appendChild(strip);
  return strip;
}

function isEditable(bubble: HTMLElement) {
  return (
    Boolean(durableMessageId(bubble)) &&
    bubble.dataset.novaveiHistory === "true"
  );
}

async function copyBubble(bubble: HTMLElement, button: HTMLButtonElement) {
  const value = bubbleSourceText(bubble);
  if (!value) {
    toast(text("没有可复制内容", "Nothing to copy"));
    return;
  }
  const previous = button.textContent;
  button.disabled = true;
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(value);
    } else {
      const hidden = document.createElement("textarea");
      hidden.value = value;
      hidden.style.position = "fixed";
      hidden.style.left = "-9999px";
      document.body.appendChild(hidden);
      hidden.select();
      if (!document.execCommand("copy"))
        throw new Error(text("系统剪贴板不可用", "Clipboard unavailable"));
      hidden.remove();
    }
    toast(text("已复制", "Copied"));
  } finally {
    button.disabled = false;
    button.textContent = previous;
  }
}

function enterEdit(bubble: HTMLElement): HTMLTextAreaElement {
  const source = bubbleSourceText(bubble);
  const original = findBodyNode(bubble);
  originalBody.set(bubble, original);
  bubble.dataset[EDIT_GUARD] = "true";

  const textarea = document.createElement("textarea");
  textarea.className = "msg-edit-textarea";
  textarea.dataset.editableField = "true";
  textarea.setAttribute("aria-label", text("编辑消息", "Edit message"));
  textarea.value = source;
  textarea.rows = Math.max(1, Math.min(8, source.split("\n").length));
  textarea.placeholder = text(
    "编辑后从此处重新发送",
    "Edit, then resend from here",
  );

  if (original && original.parentNode === bubble) {
    (original as ChildNode).replaceWith(textarea);
  } else {
    const media = bubble.querySelector(".novavei-message-attachments");
    if (media) bubble.insertBefore(textarea, media);
    else bubble.appendChild(textarea);
  }

  textarea.focus({ preventScroll: true });
  const len = textarea.value.length;
  textarea.setSelectionRange(len, len);
  return textarea;
}

/** Restore the pre-edit body and stored scroll position. */
function exitEdit(
  bubble: HTMLElement,
  options: { restoreScroll?: boolean } = {},
) {
  if (bubble.dataset[EDIT_GUARD] !== "true") return;
  delete bubble.dataset[EDIT_GUARD];
  const textarea = bubble.querySelector<HTMLTextAreaElement>(
    "[data-editable-field]",
  );
  const original = originalBody.get(bubble) ?? null;
  originalBody.delete(bubble);
  if (textarea) {
    if (original) {
      // The original node was moved out of the bubble when the textarea was
      // inserted, so it has no live parent; re-insert it in place of the box.
      (textarea as ChildNode).replaceWith(original);
    } else {
      textarea.remove();
    }
  }
  if (options.restoreScroll) {
    const transcript = node<HTMLElement>("transcript");
    const saved = bubble.dataset.novaveiScroll;
    if (transcript && saved !== undefined) {
      transcript.scrollTop = Number(saved) || 0;
    }
    delete bubble.dataset.novaveiScroll;
  }
}

async function saveEdit(bubble: HTMLElement, textarea: HTMLTextAreaElement) {
  const next = textarea.value.trim();
  if (!next) {
    toast(text("消息不能为空", "Message cannot be empty"));
    return;
  }
  const anchor = durableMessageId(bubble);
  const host = hostApi();
  const invoke = invokeApi();
  const sessionId = host?.getSessionId();
  if (!anchor || !host || !invoke || !sessionId) {
    toast(
      text("当前会话不支持就地编辑", "In-place editing is not available here"),
    );
    return;
  }

  textarea.disabled = true;
  try {
    // Resend from this round: truncate durable history at this message, then
    // reload the visible transcript so every later message is dropped.
    await invoke("chat_history_truncate", {
      id: sessionId,
      messageId: anchor,
    });
    await host.refreshSessions({ loadActive: true });

    const runtime = runtimeApi();
    if (runtime) {
      await runtime.submit({ text: next, sessionId });
    } else {
      toast(text("已就地更新，可继续发送新消息", "Updated — send to confirm"));
    }
  } catch (error) {
    toast(error instanceof Error ? error.message : String(error));
  } finally {
    exitEdit(bubble);
  }
}

async function beginEdit(bubble: HTMLElement) {
  if (bubble.dataset[EDIT_GUARD] === "true") return;
  if (!isEditable(bubble)) {
    toast(
      text(
        "旧消息未保存消息标识，无法安全编辑",
        "This message has no identity, so it cannot be edited",
      ),
    );
    return;
  }
  const transcript = node<HTMLElement>("transcript");
  if (transcript) bubble.dataset.novaveiScroll = String(transcript.scrollTop);
  const textarea = enterEdit(bubble);
  if (!textarea) return;

  textarea.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      exitEdit(bubble, { restoreScroll: true });
      return;
    }
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      event.stopPropagation();
      void saveEdit(bubble, textarea);
    }
  });

  const onOutsidePointer = (event: PointerEvent) => {
    if (event.target instanceof Node) {
      if (bubble.contains(event.target)) return;
      document.removeEventListener("pointerdown", onOutsidePointer, true);
      exitEdit(bubble, { restoreScroll: true });
    }
  };
  document.addEventListener("pointerdown", onOutsidePointer, true);
}

/**
 * Install the row-affordances for a durable user bubble. Called from the
 * history renderer for every history user row; returns the editable flag.
 */
export function decorateUserMessage(bubble: HTMLElement): {
  editable: boolean;
} {
  buildActionStrip(bubble, false);
  return { editable: isEditable(bubble) };
}

/** Exposed for the workbench to query whether a user row is editable. */
export function userMessageEditable(bubble: HTMLElement): boolean {
  return isEditable(bubble);
}
