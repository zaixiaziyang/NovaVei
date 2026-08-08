/**
 * Translation overlay: left source, right Chinese output, auto-debounce translation.
 * Uses the native translate_text / get_active_translation_model IPC commands.
 */

type Invoke = <T = unknown>(command: string, args?: Record<string, unknown>) => Promise<T>;

function invokeApi(): Invoke | undefined {
  return window.__TAURI__?.core?.invoke as Invoke | undefined;
}

function isEnglish() {
  return document.documentElement.lang.toLowerCase().startsWith("en");
}

function text(zh: string, en: string) {
  return isEnglish() ? en : zh;
}

function node<T extends HTMLElement>(id: string) {
  return document.getElementById(id) as T | null;
}

function setBusy(button: HTMLButtonElement, busy: boolean, busyText?: string) {
  button.disabled = busy;
  button.setAttribute("aria-busy", String(busy));
  if (busy && busyText) {
    button.dataset.restoreText = button.textContent || "";
    button.textContent = busyText;
  } else if (!busy && button.dataset.restoreText) {
    button.textContent = button.dataset.restoreText;
    delete button.dataset.restoreText;
  }
}

export function installTranslation() {
  const invoke = invokeApi();
  const overlay = node<HTMLElement>("overlayTranslation");
  const source = node<HTMLTextAreaElement>("translationSource");
  const output = node<HTMLElement>("translationOutput");
  const goBtn = node<HTMLButtonElement>("btnTranslationGo");
  const clearBtn = node<HTMLButtonElement>("btnTranslationClear");
  const copyBtn = node<HTMLButtonElement>("btnTranslationCopy");
  const statusEl = node<HTMLElement>("translationStatus");
  if (!overlay || !source || !output || !goBtn || !clearBtn || !copyBtn || !statusEl) return;

  // Without native invoke, show a message and disable
  if (!invoke) {
    output.textContent = text("翻译功能仅在桌面应用中可用。", "Translation is available only in the desktop app.");
    goBtn.disabled = true;
    source.disabled = true;
    return;
  }

  let debounceTimer: number | undefined;
  let isTranslating = false;

  const setStatus = (message: string, isError = false) => {
    statusEl.textContent = message;
    statusEl.style.color = isError ? "var(--danger)" : "var(--muted)";
  };

  const doTranslate = async () => {
    const textToTranslate = source.value.trim();
    if (!textToTranslate) {
      output.textContent = "中文翻译结果将在此显示";
      return;
    }

    if (isTranslating) return;
    isTranslating = true;
    setBusy(goBtn, true, text("翻译中…", "Translating…"));
    setStatus(text("翻译中…", "Translating…"));

    try {
      const sessionId = window.__novaveiHost?.getSessionId?.()?.trim() || undefined;
      const model = (await invoke<string>("get_active_translation_model", {
        sessionId: sessionId ?? null,
      }))?.trim() || undefined;

      const translated = await invoke<string>("translate_text", {
        text: textToTranslate,
        targetLang: "zh",
        sourceLang: "auto",
        model: model ?? null,
        sessionId: sessionId ?? null,
      });

      if (translated?.trim()) {
        output.textContent = translated.trim();
        setStatus(text("翻译完成", "Translation completed"));
      } else {
        output.textContent = text("翻译没有返回内容。", "Translation returned no content.");
        setStatus(text("翻译结果为空", "Empty translation result"), true);
      }
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      output.textContent = text("翻译失败：", "Translation failed: ") + msg;
      setStatus(text("翻译失败", "Translation failed"), true);
    } finally {
      isTranslating = false;
      setBusy(goBtn, false);
    }
  };

  const scheduleTranslate = () => {
    if (debounceTimer !== undefined) {
      clearTimeout(debounceTimer);
    }
    debounceTimer = window.setTimeout(() => {
      debounceTimer = undefined;
      void doTranslate();
    }, 1500);
  };

  // Manual translate button click
  goBtn.addEventListener("click", () => {
    if (debounceTimer !== undefined) {
      clearTimeout(debounceTimer);
      debounceTimer = undefined;
    }
    void doTranslate();
  });

  // Auto-debounce on input
  source.addEventListener("input", scheduleTranslate);

  // Clear button
  clearBtn.addEventListener("click", () => {
    source.value = "";
    output.textContent = "中文翻译结果将在此显示";
    setStatus("");
    if (debounceTimer !== undefined) {
      clearTimeout(debounceTimer);
      debounceTimer = undefined;
    }
    source.focus();
  });

  // Copy button
  copyBtn.addEventListener("click", async () => {
    const textToCopy = output.textContent || "";
    if (!textToCopy || textToCopy === "中文翻译结果将在此显示") {
      setStatus(text("没有可复制的内容", "Nothing to copy"), true);
      return;
    }
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(textToCopy);
      } else {
        const ta = document.createElement("textarea");
        ta.value = textToCopy;
        ta.style.position = "fixed";
        ta.style.left = "-9999px";
        document.body.appendChild(ta);
        ta.select();
        document.execCommand("copy");
        ta.remove();
      }
      setStatus(text("已复制到剪贴板", "Copied to clipboard"));
    } catch {
      setStatus(text("复制失败", "Copy failed"), true);
    }
  });

  // Reset state when overlay opens
  const observer = new MutationObserver(() => {
    if (overlay.classList.contains("show")) {
      source.focus();
    }
  });
  observer.observe(overlay, { attributes: true, attributeFilter: ["class"] });
}