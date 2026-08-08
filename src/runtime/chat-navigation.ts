import {
  createScrollFollowState,
  scrollFollow,
  type FollowState,
} from "./scroll-follow-core";

const BOTTOM_THRESHOLD_PX = 72;
const CONTROL_ID = "novaveiReturnToLatest";
const STYLE_ID = "novaveiReturnToLatestStyles";
const TRANSCRIPT_CONTENT_CHANGED_EVENT = "novavei:transcript-content-changed";

type Copy = {
  label: string;
  ariaLabel: string;
  title: string;
};

function transcriptElement() {
  return document.getElementById("transcript") as HTMLElement | null;
}

function copy(): Copy {
  if (document.documentElement.lang.toLowerCase().startsWith("en")) {
    return {
      label: "Back to latest",
      ariaLabel: "Return to the latest message",
      title: "Return to the latest message",
    };
  }
  return {
    label: "回到底部",
    ariaLabel: "回到最新消息",
    title: "回到最新消息",
  };
}

function isNearBottom(transcript: HTMLElement) {
  return (
    transcript.scrollHeight - transcript.scrollTop - transcript.clientHeight <=
    BOTTOM_THRESHOLD_PX
  );
}

function appendDownIcon(button: HTMLButtonElement) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  svg.classList.add("ico");
  const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
  path.setAttribute("d", "M12 4v14m0 0 5-5m-5 5-5-5");
  svg.appendChild(path);
  button.appendChild(svg);
}

function installStyles() {
  if (document.getElementById(STYLE_ID)) return;
  const style = document.createElement("style");
  style.id = STYLE_ID;
  style.textContent = `
    .chat-return-latest {
      position: absolute;
      right: 24px;
      bottom: 16px;
      z-index: 13;
      min-height: 44px;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      gap: 8px;
      padding: 0 14px;
      border: 1px solid var(--line-strong);
      border-radius: var(--r-pill);
      background: var(--glass-strong);
      color: var(--text);
      box-shadow: var(--shadow-md), 0 1px 0 var(--inset) inset;
      backdrop-filter: blur(20px) saturate(130%);
      font: inherit;
      font-size: var(--text-base);
      font-weight: 650;
      cursor: pointer;
      touch-action: manipulation;
      transition: background 160ms var(--ease-out), border-color 160ms var(--ease-out), color 160ms var(--ease-out);
    }
    .chat-return-latest[hidden] { display: none; }
    .chat-return-latest:hover {
      border-color: var(--blue-line);
      background: var(--blue-soft);
      color: var(--blue-strong);
    }
    .chat-return-latest:focus-visible {
      outline: 2px solid var(--blue);
      outline-offset: 2px;
    }
    .chat-return-latest .ico { width: 16px; height: 16px; }
    @media (max-width: 560px) {
      .chat-return-latest { right: 12px; bottom: 12px; }
    }
    @media (prefers-reduced-motion: reduce) {
      .chat-return-latest { transition: none; }
    }
  `;
  document.head.appendChild(style);
}

/**
 * Signals a deliberate runtime mutation of the transcript. The observer is a
 * safety net for all other DOM changes; this explicit signal keeps the live
 * message path on the same following/latest contract without touching scrollTop.
 */
export function notifyTranscriptContentChanged() {
  window.dispatchEvent(new Event(TRANSCRIPT_CONTENT_CHANGED_EVENT));
}

/**
 * Keeps a transcript pinned only while the reader is already at the latest
 * content. Once they scroll into history, streamed DOM updates never pull the
 * viewport away; a native keyboard-focusable control gives them a direct path
 * back to the newest message.
 */
export function installTranscriptNavigation() {
  if (document.getElementById(CONTROL_ID)) return;
  const transcript = transcriptElement();
  const stage = transcript?.closest<HTMLElement>(".transcript-stage");
  if (!transcript || !stage) return;

  installStyles();
  const button = document.createElement("button");
  button.id = CONTROL_ID;
  button.type = "button";
  button.className = "chat-return-latest";
  button.setAttribute("aria-controls", transcript.id);
  appendDownIcon(button);
  const label = document.createElement("span");
  button.appendChild(label);
  stage.appendChild(button);

  let followingState: FollowState = createScrollFollowState();
  let scrollFrame: number | undefined;
  let contentFrame: number | undefined;
  let lastScrollTop = transcript.scrollTop;

  const trailing = () => followingState.trailing;

  const updateCopy = () => {
    const text = copy();
    label.textContent = text.label;
    button.setAttribute("aria-label", text.ariaLabel);
    button.title = text.title;
  };

  const updateVisibility = () => {
    button.hidden = isNearBottom(transcript);
  };

  const scrollToLatest = () => {
    followingState = scrollFollow(followingState, {
      type: "history-key",
      active: true,
    }).state;
    transcript.scrollTop = transcript.scrollHeight;
    updateVisibility();
  };

  const queueViewportSync = () => {
    if (scrollFrame !== undefined) return;
    scrollFrame = window.requestAnimationFrame(() => {
      scrollFrame = undefined;
      updateVisibility();
    });
  };

  const queueContentReconciliation = () => {
    if (contentFrame !== undefined) return;
    contentFrame = window.requestAnimationFrame(() => {
      contentFrame = undefined;
      // A session load can replace the transcript and synchronously set its
      // scrollTop before observers run. Trust the actual viewport first.
      if (isNearBottom(transcript)) {
        followingState = createScrollFollowState();
        updateVisibility();
        return;
      }
      if (trailing()) {
        scrollToLatest();
        return;
      }
      updateVisibility();
    });
  };

  const observer = new MutationObserver(queueContentReconciliation);
  observer.observe(transcript, {
    childList: true,
    characterData: true,
    subtree: true,
  });
  const resizeObserver =
    typeof ResizeObserver === "undefined"
      ? undefined
      : new ResizeObserver(queueContentReconciliation);
  resizeObserver?.observe(
    document.getElementById("transcriptAxis") ?? transcript,
  );

  const onScroll = () => {
    // Feed the pure state machine. Update synchronously so a token-render
    // MutationObserver queued in the same frame cannot yank a reader back
    // down after they scroll upward.
    const deltaY = transcript.scrollTop - lastScrollTop;
    lastScrollTop = transcript.scrollTop;
    const out = scrollFollow(followingState, {
      type: "scroll",
      deltaY,
      scrollTop: transcript.scrollTop,
      clientHeight: transcript.clientHeight,
      scrollHeight: transcript.scrollHeight,
    });
    followingState = out.state;
    // Note: mutation-observers reuse `trailing()` to decide whether to keep
    // pulling latest; a reattach scroll is applied here via the decision.
    if (out.decision.reason === "reattach") {
      transcript.scrollTop = transcript.scrollHeight;
    }
    queueViewportSync();
  };
  const onTranscriptContentChanged = () => queueContentReconciliation();
  const onLanguageChange = () => updateCopy();
  transcript.addEventListener("scroll", onScroll, { passive: true });
  transcript.addEventListener("pointerdown", () => {
    followingState = scrollFollow(followingState, {
      type: "pointer",
      phase: "down",
    }).state;
  });
  transcript.addEventListener("pointerup", () => {
    followingState = scrollFollow(followingState, {
      type: "pointer",
      phase: "up",
    }).state;
  });
  window.addEventListener(
    TRANSCRIPT_CONTENT_CHANGED_EVENT,
    onTranscriptContentChanged,
  );
  window.addEventListener("novavei:language-changed", onLanguageChange);
  window.addEventListener("novavei:service-language-changed", onLanguageChange);
  button.addEventListener("click", scrollToLatest);
  updateCopy();
  updateVisibility();
}
