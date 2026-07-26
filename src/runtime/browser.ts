/**
 * Side-browser renderer bridge.
 *
 * The viewport is a native child WebView; this module owns only the existing
 * Luminous Quiet dock controls and mirrors their geometry to the host. Plain
 * browser previews retain a disabled, truthful surface and never fake a page.
 */

type Invoke = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

type BrowserState = {
  available?: boolean;
  url?: string;
};

type BrowserCopy = {
  ready: string;
  opening: string;
  loading: string;
  opened: (url: string) => string;
  unavailable: string;
  invalidAddress: string;
  back: string;
  reload: string;
};

const COPY: Record<"zh" | "en", BrowserCopy> = {
  zh: {
    ready: "在地址栏输入公开网页地址；登录和密码输入仅由你本人完成。",
    opening: "正在准备侧栏浏览器…",
    loading: "正在加载网页…",
    opened: (url) => `已打开 ${url}`,
    unavailable: "侧栏浏览器当前不可用。",
    invalidAddress: "请输入完整的 http:// 或 https:// 地址。",
    back: "已请求后退。",
    reload: "正在刷新网页。",
  },
  en: {
    ready:
      "Enter a public webpage address; you complete sign-in and password entry yourself.",
    opening: "Preparing the sidebar browser…",
    loading: "Loading webpage…",
    opened: (url) => `Opened ${url}`,
    unavailable: "The sidebar browser is unavailable.",
    invalidAddress: "Enter a complete http:// or https:// address.",
    back: "Requested browser back navigation.",
    reload: "Reloading webpage.",
  },
};

function currentCopy() {
  return COPY[
    document.documentElement.lang.toLowerCase().startsWith("en") ? "en" : "zh"
  ];
}

function invokeApi(): Invoke | undefined {
  return window.__TAURI__?.core?.invoke as Invoke | undefined;
}

function isHttpUrl(value: string) {
  try {
    const parsed = new URL(value.trim());
    return (
      (parsed.protocol === "https:" || parsed.protocol === "http:") &&
      Boolean(parsed.hostname) &&
      !parsed.username &&
      !parsed.password
    );
  } catch {
    return false;
  }
}

/** Install once after the static dock controller has registered its tabs. */
export function installBrowser() {
  const invoke = invokeApi();
  const choice = document.querySelector<HTMLButtonElement>(
    '[data-dock-tool="browser"]',
  );
  const panel = document.getElementById("dock-pane-browser");
  const workbench = document.getElementById("workbench");
  const viewport = document.getElementById("browserViewport");
  const form = document.getElementById("browserNavigation");
  const address = document.getElementById("browserAddress");
  const open = document.getElementById("browserOpen");
  const back = document.getElementById("browserBack");
  const reload = document.getElementById("browserReload");
  const status = document.getElementById("browserStatus");
  if (
    !choice ||
    !panel ||
    !workbench ||
    !viewport ||
    !(form instanceof HTMLFormElement) ||
    !(address instanceof HTMLInputElement) ||
    !(open instanceof HTMLButtonElement) ||
    !(back instanceof HTMLButtonElement) ||
    !(reload instanceof HTMLButtonElement) ||
    !(status instanceof HTMLElement)
  ) {
    return;
  }

  // Browser preview remains deliberately inert. It must not imply that a
  // normal browser tab is connected to native WebView2 controls.
  if (!invoke) return;

  choice.disabled = false;
  choice.setAttribute("aria-disabled", "false");
  for (const control of [address, open, back, reload]) control.disabled = false;
  panel.removeAttribute("data-feature-unavailable");

  let opening = false;
  let layoutQueued = false;
  let layoutInFlight = false;
  let layoutAgain = false;
  let lastViewport:
    | { x: number; y: number; width: number; height: number }
    | undefined;

  const setStatus = (message: string, busy = false) => {
    status.textContent = message;
    status.setAttribute("aria-busy", String(busy));
  };

  const setOpening = (busy: boolean) => {
    opening = busy;
    open.disabled = busy;
    address.disabled = busy;
    back.disabled = busy;
    reload.disabled = busy;
  };

  const browserIsVisible = () =>
    panel.classList.contains("on") &&
    !panel.hidden &&
    !workbench.classList.contains("dock-closed");

  const updateState = (state: BrowserState | undefined) => {
    const url = typeof state?.url === "string" ? state.url : "";
    if (url) {
      address.value = url;
      viewport.dataset.browserActive = "true";
    } else {
      delete viewport.dataset.browserActive;
    }
  };

  const syncViewport = async () => {
    if (layoutInFlight) {
      layoutAgain = true;
      return;
    }
    const rect = viewport.getBoundingClientRect();
    if (browserIsVisible() && rect.width >= 1 && rect.height >= 1) {
      lastViewport = {
        x: Math.max(0, rect.left),
        y: Math.max(0, rect.top),
        width: rect.width,
        height: rect.height,
      };
    }
    if (!lastViewport) return;
    layoutInFlight = true;
    try {
      const state = await invoke<BrowserState>("browser_layout", {
        ...lastViewport,
        visible: browserIsVisible(),
      });
      updateState(state);
    } catch {
      // Geometry sync can race app shutdown. Keep the page state rather than
      // replacing it with a misleading renderer-only error.
    } finally {
      layoutInFlight = false;
      if (layoutAgain) {
        layoutAgain = false;
        queueViewportSync();
      }
    }
  };

  const queueViewportSync = () => {
    if (layoutQueued) return;
    layoutQueued = true;
    requestAnimationFrame(() => {
      layoutQueued = false;
      void syncViewport();
    });
  };

  const refreshStatus = async () => {
    try {
      const state = await invoke<BrowserState>("browser_status");
      updateState(state);
      if (state.available && state.url)
        setStatus(currentCopy().opened(state.url));
    } catch {
      // The host can be closing or the browser was never created. Either is a
      // normal state for an unopened dock.
    }
  };

  const openUrl = async () => {
    const url = address.value.trim();
    if (!isHttpUrl(url)) {
      setStatus(currentCopy().invalidAddress);
      address.focus();
      return;
    }
    setOpening(true);
    setStatus(currentCopy().opening, true);
    try {
      const state = await invoke<BrowserState>("browser_open", { url });
      updateState(state);
      setStatus(currentCopy().loading, true);
      await syncViewport();
      setStatus(currentCopy().opened(state.url || url));
    } catch (error) {
      setStatus(
        error instanceof Error ? error.message : currentCopy().unavailable,
      );
    } finally {
      setOpening(false);
    }
  };

  form.addEventListener("submit", (event) => {
    event.preventDefault();
    void openUrl();
  });
  back.addEventListener("click", () => {
    setStatus(currentCopy().back, true);
    void invoke<BrowserState>("browser_back")
      .then((state) => {
        updateState(state);
        window.setTimeout(() => void refreshStatus(), 180);
      })
      .catch(() => setStatus(currentCopy().unavailable))
      .finally(() => status.setAttribute("aria-busy", "false"));
  });
  reload.addEventListener("click", () => {
    setStatus(currentCopy().reload, true);
    void invoke<BrowserState>("browser_reload")
      .then((state) => updateState(state))
      .catch(() => setStatus(currentCopy().unavailable))
      .finally(() => status.setAttribute("aria-busy", "false"));
  });
  choice.addEventListener("click", () => {
    window.setTimeout(() => {
      queueViewportSync();
      void refreshStatus();
    }, 0);
  });
  // Agent navigation must not happen in an invisible child WebView. Reuse
  // the existing dock activation path so the page becomes inspectable before
  // later snapshot, click, or ordinary-text operations continue.
  window.addEventListener("novavei:browser-agent-navigated", () => {
    if (!choice.disabled) choice.click();
  });
  window.addEventListener("novavei:dock-pane-activated", () => {
    queueViewportSync();
    if (browserIsVisible()) void refreshStatus();
  });
  window.addEventListener("resize", queueViewportSync, { passive: true });
  window.addEventListener("novavei:language-changed", () => {
    if (!opening && !status.textContent?.trim()) setStatus(currentCopy().ready);
  });
  new ResizeObserver(queueViewportSync).observe(viewport);
  new MutationObserver(queueViewportSync).observe(panel, {
    attributes: true,
    attributeFilter: ["class", "hidden"],
  });
  new MutationObserver(queueViewportSync).observe(workbench, {
    attributes: true,
    attributeFilter: ["class"],
  });
  document
    .getElementById("btnCloseDock")
    ?.addEventListener("click", queueViewportSync);
  document
    .getElementById("btnRemoveDockTool")
    ?.addEventListener("click", queueViewportSync);

  setStatus(currentCopy().ready);
  queueViewportSync();
}
