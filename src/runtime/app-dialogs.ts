/**
 * In-app modal dialogs for confirm / prompt / error.
 *
 * Host browser prompts (window.confirm / prompt / alert) are banned for user
 * flows. Prefer these helpers so desktop and WebView share Luminous Quiet
 * chrome. Error dialogs keep the full message selectable and copyable.
 */

export type AppConfirmOptions = {
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** Defaults to true (danger styling on the accept button). */
  danger?: boolean;
};

export type AppChoiceOption<Value extends string = string> = {
  value: Value;
  label: string;
  description?: string;
  tone?: "default" | "primary" | "danger";
  disabled?: boolean;
};

export type AppChoiceOptions<Value extends string = string> = {
  title: string;
  message: string;
  choices: readonly AppChoiceOption<Value>[];
  cancelLabel?: string;
};

export type AppPromptOptions = {
  title: string;
  message?: string;
  label?: string;
  initialValue?: string;
  placeholder?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  maxLength?: number;
  multiline?: boolean;
  required?: boolean;
};

export type AppAlertOptions = {
  title?: string;
  message: string;
  detail?: string;
  closeLabel?: string;
  copyLabel?: string;
  copiedLabel?: string;
  copyFailedLabel?: string;
};

type DialogLocale = "zh" | "en";

const DEFAULT_COPY = {
  zh: {
    confirmTitle: "确认操作",
    confirmAccept: "确定",
    confirmCancel: "取消",
    promptTitle: "输入",
    promptLabel: "内容",
    alertTitle: "出错了",
    alertClose: "关闭",
    alertCopy: "复制错误信息",
    alertCopied: "已复制错误信息",
    alertCopyFailed: "复制失败，请手动选择文本",
    busy: "已有对话框打开。",
    required: "内容不能为空。",
    tooLong: (max: number) => `内容不能超过 ${max} 个字符。`,
  },
  en: {
    confirmTitle: "Confirm",
    confirmAccept: "Confirm",
    confirmCancel: "Cancel",
    promptTitle: "Input",
    promptLabel: "Value",
    alertTitle: "Something went wrong",
    alertClose: "Close",
    alertCopy: "Copy error",
    alertCopied: "Error copied",
    alertCopyFailed: "Copy failed; select the text manually",
    busy: "A dialog is already open.",
    required: "Content cannot be empty.",
    tooLong: (max: number) => `Content cannot exceed ${max} characters.`,
  },
} as const;

function dialogLocale(): DialogLocale {
  return document.documentElement.lang.toLowerCase().startsWith("en")
    ? "en"
    : "zh";
}

function copy() {
  return DEFAULT_COPY[dialogLocale()];
}

function element<T extends HTMLElement>(id: string): T | null {
  return document.getElementById(id) as T | null;
}

function toast(message: string) {
  const target = element<HTMLElement>("toast");
  if (!target) {
    console.warn("[NovaVei dialogs]", message);
    return;
  }
  target.textContent = message;
  target.classList.add("show");
  window.setTimeout(() => target.classList.remove("show"), 2200);
}

export function formatErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim())
    return error.message.trim();
  if (typeof error === "string" && error.trim()) return error.trim();
  try {
    return JSON.stringify(error, null, 2);
  } catch {
    return String(error ?? "Unknown error");
  }
}

async function copyText(value: string) {
  const text = value.trim();
  if (!text) throw new Error(copy().required);
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const fallback = document.createElement("textarea");
  fallback.value = text;
  fallback.setAttribute("readonly", "true");
  fallback.style.position = "fixed";
  fallback.style.left = "-9999px";
  document.body.appendChild(fallback);
  fallback.select();
  try {
    if (!document.execCommand("copy")) throw new Error("copy failed");
  } finally {
    fallback.remove();
  }
}

function anyDialogOpen(): boolean {
  return Boolean(document.querySelector("dialog[open]"));
}

function focusFirst(control: HTMLElement | null) {
  window.requestAnimationFrame(() => control?.focus());
}

/**
 * Confirm destructive or high-impact actions with the app chrome.
 * Returns true only when the user explicitly accepts.
 */
export async function requestAppConfirm(
  options: AppConfirmOptions,
): Promise<boolean> {
  const dialog = element<HTMLDialogElement>("appConfirmDialog");
  const form = element<HTMLFormElement>("appConfirmForm");
  const title = element<HTMLElement>("appConfirmTitle");
  const description = element<HTMLElement>("appConfirmDescription");
  const cancel = element<HTMLButtonElement>("appConfirmCancel");
  const accept = element<HTMLButtonElement>("appConfirmAccept");
  const defaults = copy();
  if (
    !dialog ||
    !form ||
    !title ||
    !description ||
    !cancel ||
    !accept ||
    typeof dialog.showModal !== "function"
  ) {
    console.warn("[NovaVei dialogs] confirm shell missing; denying action");
    return false;
  }
  if (dialog.open || anyDialogOpen()) {
    toast(defaults.busy);
    return false;
  }
  return await new Promise<boolean>((resolve) => {
    let finished = false;
    const finish = (confirmed: boolean) => {
      if (finished) return;
      finished = true;
      form.removeEventListener("submit", onSubmit);
      cancel.removeEventListener("click", onCancelClick);
      dialog.removeEventListener("cancel", onDialogCancel);
      dialog.removeEventListener("close", onDialogClose);
      if (dialog.open) dialog.close();
      resolve(confirmed);
    };
    const onSubmit = (event: Event) => {
      event.preventDefault();
      finish(true);
    };
    const onCancelClick = () => finish(false);
    const onDialogCancel = (event: Event) => {
      event.preventDefault();
      finish(false);
    };
    const onDialogClose = () => finish(false);

    title.textContent = options.title || defaults.confirmTitle;
    description.textContent = options.message;
    cancel.textContent = options.cancelLabel || defaults.confirmCancel;
    accept.textContent = options.confirmLabel || defaults.confirmAccept;
    const danger = options.danger !== false;
    accept.classList.toggle("danger", danger);
    accept.classList.toggle("primary", !danger);
    form.addEventListener("submit", onSubmit);
    cancel.addEventListener("click", onCancelClick);
    dialog.addEventListener("cancel", onDialogCancel);
    dialog.addEventListener("close", onDialogClose);
    dialog.showModal();
    focusFirst(cancel);
  });
}

/**
 * Present one or more explicit outcomes in one accessible app-owned dialog.
 * Returning null is always the cancel/Escape/close path; callers never infer a
 * destructive choice from dismissal.
 */
export async function requestAppChoice<Value extends string>(
  options: AppChoiceOptions<Value>,
): Promise<Value | null> {
  const dialog = element<HTMLDialogElement>("appChoiceDialog");
  const form = element<HTMLFormElement>("appChoiceForm");
  const title = element<HTMLElement>("appChoiceTitle");
  const description = element<HTMLElement>("appChoiceDescription");
  const choices = element<HTMLElement>("appChoiceOptions");
  const cancel = element<HTMLButtonElement>("appChoiceCancel");
  const defaults = copy();
  if (
    !dialog ||
    !form ||
    !title ||
    !description ||
    !choices ||
    !cancel ||
    typeof dialog.showModal !== "function"
  ) {
    console.warn("[NovaVei dialogs] choice shell missing; cancelling choice");
    return null;
  }
  if (dialog.open || anyDialogOpen()) {
    toast(defaults.busy);
    return null;
  }
  const enabledChoices = options.choices.filter((choice) => !choice.disabled);
  if (enabledChoices.length < 1) {
    console.warn("[NovaVei dialogs] choice dialog has no enabled outcome");
    return null;
  }
  const previouslyFocused =
    document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null;

  return await new Promise<Value | null>((resolve) => {
    let finished = false;
    const finish = (value: Value | null) => {
      if (finished) return;
      finished = true;
      cancel.removeEventListener("click", onCancelClick);
      dialog.removeEventListener("cancel", onDialogCancel);
      dialog.removeEventListener("close", onDialogClose);
      choices.replaceChildren();
      if (dialog.open) dialog.close();
      focusFirst(previouslyFocused?.isConnected ? previouslyFocused : null);
      resolve(value);
    };
    const onCancelClick = () => finish(null);
    const onDialogCancel = (event: Event) => {
      event.preventDefault();
      finish(null);
    };
    const onDialogClose = () => finish(null);

    title.textContent = options.title;
    description.textContent = options.message;
    cancel.textContent = options.cancelLabel || defaults.confirmCancel;
    choices.replaceChildren();
    for (const choice of options.choices) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "btn app-choice-option";
      button.disabled = choice.disabled === true;
      button.classList.toggle("primary", choice.tone === "primary");
      button.classList.toggle("danger", choice.tone === "danger");
      const label = document.createElement("strong");
      label.textContent = choice.label;
      button.appendChild(label);
      if (choice.description?.trim()) {
        const detail = document.createElement("small");
        detail.textContent = choice.description.trim();
        button.appendChild(detail);
      }
      button.addEventListener("click", () => finish(choice.value), {
        once: true,
      });
      choices.appendChild(button);
    }
    cancel.addEventListener("click", onCancelClick);
    dialog.addEventListener("cancel", onDialogCancel);
    dialog.addEventListener("close", onDialogClose);
    dialog.showModal();
    focusFirst(cancel);
  });
}

/**
 * Collect a single text value with the app chrome.
 * Returns null when cancelled; otherwise the trimmed string (may be empty if
 * required is false).
 */
export async function requestAppPrompt(
  options: AppPromptOptions,
): Promise<string | null> {
  const dialog = element<HTMLDialogElement>("appPromptDialog");
  const form = element<HTMLFormElement>("appPromptForm");
  const title = element<HTMLElement>("appPromptTitle");
  const description = element<HTMLElement>("appPromptDescription");
  const label = element<HTMLElement>("appPromptLabel");
  const input = element<HTMLInputElement | HTMLTextAreaElement>(
    "appPromptInput",
  );
  const error = element<HTMLElement>("appPromptError");
  const cancel = element<HTMLButtonElement>("appPromptCancel");
  const accept = element<HTMLButtonElement>("appPromptAccept");
  const defaults = copy();
  if (
    !dialog ||
    !form ||
    !title ||
    !description ||
    !label ||
    !input ||
    !error ||
    !cancel ||
    !accept ||
    typeof dialog.showModal !== "function"
  ) {
    console.warn("[NovaVei dialogs] prompt shell missing; cancelling input");
    return null;
  }
  if (dialog.open || anyDialogOpen()) {
    toast(defaults.busy);
    return null;
  }

  const multiline = options.multiline === true;
  input.hidden = false;
  if (input instanceof HTMLTextAreaElement) {
    input.rows = multiline ? 5 : 1;
  }
  if (options.maxLength && options.maxLength > 0) {
    input.maxLength = options.maxLength;
  } else {
    input.removeAttribute("maxlength");
  }

  return await new Promise<string | null>((resolve) => {
    let finished = false;
    const clearError = () => {
      error.hidden = true;
      error.textContent = "";
      input.removeAttribute("aria-invalid");
    };
    const showError = (message: string) => {
      error.textContent = message;
      error.hidden = false;
      input.setAttribute("aria-invalid", "true");
    };
    const finish = (value: string | null) => {
      if (finished) return;
      finished = true;
      form.removeEventListener("submit", onSubmit);
      cancel.removeEventListener("click", onCancelClick);
      dialog.removeEventListener("cancel", onDialogCancel);
      dialog.removeEventListener("close", onDialogClose);
      if (dialog.open) dialog.close();
      resolve(value);
    };
    const onSubmit = (event: Event) => {
      event.preventDefault();
      const value = input.value.trim();
      if (options.required !== false && !value) {
        showError(defaults.required);
        input.focus();
        return;
      }
      if (options.maxLength && Array.from(value).length > options.maxLength) {
        showError(defaults.tooLong(options.maxLength));
        input.focus();
        return;
      }
      clearError();
      finish(value);
    };
    const onCancelClick = () => finish(null);
    const onDialogCancel = (event: Event) => {
      event.preventDefault();
      finish(null);
    };
    const onDialogClose = () => finish(null);

    title.textContent = options.title || defaults.promptTitle;
    if (options.message?.trim()) {
      description.hidden = false;
      description.textContent = options.message.trim();
    } else {
      description.hidden = true;
      description.textContent = "";
    }
    label.textContent = options.label || defaults.promptLabel;
    input.value = options.initialValue ?? "";
    if ("placeholder" in input) input.placeholder = options.placeholder ?? "";
    cancel.textContent = options.cancelLabel || defaults.confirmCancel;
    accept.textContent = options.confirmLabel || defaults.confirmAccept;
    clearError();
    form.addEventListener("submit", onSubmit);
    cancel.addEventListener("click", onCancelClick);
    dialog.addEventListener("cancel", onDialogCancel);
    dialog.addEventListener("close", onDialogClose);
    dialog.showModal();
    focusFirst(input);
    if (
      input instanceof HTMLInputElement ||
      input instanceof HTMLTextAreaElement
    ) {
      input.select();
    }
  });
}

/**
 * Show an error (or plain alert) with the full message and a Copy action.
 */
export async function requestAppAlert(
  options: AppAlertOptions | string,
): Promise<void> {
  const normalized: AppAlertOptions =
    typeof options === "string" ? { message: options } : options;
  const dialog = element<HTMLDialogElement>("appAlertDialog");
  const form = element<HTMLFormElement>("appAlertForm");
  const title = element<HTMLElement>("appAlertTitle");
  const description = element<HTMLElement>("appAlertDescription");
  const detail = element<HTMLElement>("appAlertDetail");
  const copyButton = element<HTMLButtonElement>("appAlertCopy");
  const closeButton = element<HTMLButtonElement>("appAlertClose");
  const defaults = copy();
  if (
    !dialog ||
    !form ||
    !title ||
    !description ||
    !detail ||
    !copyButton ||
    !closeButton ||
    typeof dialog.showModal !== "function"
  ) {
    console.error(
      "[NovaVei dialogs]",
      normalized.title || defaults.alertTitle,
      normalized.message,
      normalized.detail,
    );
    toast(normalized.message);
    return;
  }
  if (dialog.open) {
    // Replace content of an already-open alert rather than stacking hosts.
    title.textContent = normalized.title || defaults.alertTitle;
    description.textContent = normalized.message;
    if (normalized.detail?.trim()) {
      detail.hidden = false;
      detail.textContent = normalized.detail.trim();
    }
    return;
  }
  if (anyDialogOpen()) {
    // Prefer not to block confirm/prompt; still surface via toast + console.
    console.error("[NovaVei dialogs]", normalized.message, normalized.detail);
    toast(normalized.message);
    return;
  }

  await new Promise<void>((resolve) => {
    let finished = false;
    const finish = () => {
      if (finished) return;
      finished = true;
      form.removeEventListener("submit", onSubmit);
      closeButton.removeEventListener("click", onClose);
      copyButton.removeEventListener("click", onCopy);
      dialog.removeEventListener("cancel", onDialogCancel);
      dialog.removeEventListener("close", onDialogClose);
      if (dialog.open) dialog.close();
      resolve();
    };
    const onSubmit = (event: Event) => {
      event.preventDefault();
      finish();
    };
    const onClose = () => finish();
    const onDialogCancel = (event: Event) => {
      event.preventDefault();
      finish();
    };
    const onDialogClose = () => finish();
    const onCopy = () => {
      const payload = [normalized.message, normalized.detail?.trim()]
        .filter(Boolean)
        .join("\n\n");
      void copyText(payload).then(
        () => toast(normalized.copiedLabel || defaults.alertCopied),
        () => toast(normalized.copyFailedLabel || defaults.alertCopyFailed),
      );
    };

    title.textContent = normalized.title || defaults.alertTitle;
    description.textContent = normalized.message;
    if (normalized.detail?.trim()) {
      detail.hidden = false;
      detail.textContent = normalized.detail.trim();
    } else {
      detail.hidden = true;
      detail.textContent = "";
    }
    copyButton.textContent = normalized.copyLabel || defaults.alertCopy;
    closeButton.textContent = normalized.closeLabel || defaults.alertClose;
    form.addEventListener("submit", onSubmit);
    closeButton.addEventListener("click", onClose);
    copyButton.addEventListener("click", onCopy);
    dialog.addEventListener("cancel", onDialogCancel);
    dialog.addEventListener("close", onDialogClose);
    dialog.showModal();
    focusFirst(copyButton);
  });
}

/** Convenience: format unknown errors and open the copyable alert. */
export async function showAppError(
  error: unknown,
  title?: string,
): Promise<void> {
  const defaults = copy();
  await requestAppAlert({
    title: title || defaults.alertTitle,
    message: formatErrorMessage(error),
  });
}

/** Expose helpers for the large index.html surface (no module imports there). */
export function installAppDialogs() {
  window.__novaveiDialogs = {
    confirm: requestAppConfirm,
    choice: requestAppChoice,
    prompt: requestAppPrompt,
    alert: requestAppAlert,
    error: showAppError,
    formatError: formatErrorMessage,
  };
}

declare global {
  interface Window {
    __novaveiDialogs?: {
      confirm: typeof requestAppConfirm;
      choice: typeof requestAppChoice;
      prompt: typeof requestAppPrompt;
      alert: typeof requestAppAlert;
      error: typeof showAppError;
      formatError: typeof formatErrorMessage;
    };
  }
}
