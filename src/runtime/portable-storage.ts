type PortableStorageStatus = {
  portable: boolean;
  initialized: boolean;
  unlocked: boolean;
  recoveryConfigured: boolean;
  recoveryQuestions: string[];
};

type PortableRecoverySetup = {
  questions: string[];
  answers: string[];
};

type PortableFlow = "unlock" | "setup" | "recover";

type Invoke = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

function getInvoke(): Invoke | undefined {
  return window.__TAURI__?.core?.invoke as Invoke | undefined;
}

const PORTABLE_STORAGE_ERROR_COPY: Readonly<Record<string, string>> = {
  "portable storage password must contain at least 12 characters":
    "便携库密码至少需要 12 个字符。",
  "portable storage password cannot contain control characters":
    "便携库密码不能包含控制字符。",
  "portable storage password is incorrect": "便携库密码不正确，请重试。",
  "portable storage password is incorrect or data is damaged":
    "便携库密码不正确，或便携数据已损坏。",
  "portable recovery answers are incorrect or data is damaged":
    "三道安全问题的答案不正确，或便携数据已损坏。",
  "portable recovery requires exactly three questions":
    "请完整设置三道安全问题。",
  "portable recovery requires exactly three answers":
    "请完整回答三道安全问题。",
  "portable recovery questions are required when creating portable data":
    "创建便携数据时必须设置三道安全问题。",
  "portable recovery is already configured":
    "此便携库已设置安全问题，请使用密码解锁。",
  "portable recovery is not configured; unlock with the current password and set three recovery questions first":
    "此便携库尚未设置安全问题。请先用当前密码解锁并完成设置。",
  "each portable recovery question must contain at least 6 characters":
    "每道安全问题至少需要 6 个字符。",
  "each portable recovery answer must contain at least 4 characters":
    "每个安全问题答案至少需要 4 个字符。",
  "portable recovery questions must be different": "三道安全问题不能重复。",
  "portable storage recovery is required before it can be unlocked":
    "便携数据需要先恢复后才能解锁。",
  "portable storage is not active for this application":
    "当前应用未启用便携数据存储。",
  "portable distribution marker is invalid; repair the portable package before unlocking data":
    "便携版标识无效。请修复便携版文件后重试。",
  "portable storage configuration is invalid": "便携数据配置无效。",
  "portable storage configuration version is unsupported":
    "当前版本不支持此便携数据配置。",
  "portable storage configuration is unavailable for recovery":
    "无法读取便携库恢复配置。",
};

function isEnglishInterface(): boolean {
  return document.documentElement.lang.toLowerCase().startsWith("en");
}

function message(error: unknown): string {
  const source =
    error instanceof Error && error.message.trim()
      ? error.message
      : typeof error === "string" && error.trim()
        ? error
        : undefined;
  if (isEnglishInterface()) {
    return (
      source ||
      "Unable to unlock portable data. Check the password or recovery answers and try again."
    );
  }
  return (
    (source ? PORTABLE_STORAGE_ERROR_COPY[source] : undefined) ||
    "无法解锁便携数据。请检查密码、安全问题答案或数据状态后重试。"
  );
}

function appendInput(
  container: HTMLElement,
  options: {
    id: string;
    label: string;
    type: "text" | "password";
    autocomplete: HTMLInputElement["autocomplete"];
    minLength: number;
    maxLength: number;
    recoveryAnswer?: boolean;
  },
): HTMLInputElement {
  const label = document.createElement("label");
  label.className = "portable-storage-recovery-field";
  label.htmlFor = options.id;
  const text = document.createElement("span");
  text.textContent = options.label;
  const input = document.createElement("input");
  input.id = options.id;
  input.type = options.type;
  input.autocomplete = options.autocomplete;
  input.minLength = options.minLength;
  input.maxLength = options.maxLength;
  input.required = true;
  if (options.recoveryAnswer) input.dataset.portableRecoveryAnswer = "true";
  input.setAttribute("aria-describedby", "portableStorageError");
  label.append(text, input);
  container.appendChild(label);
  return input;
}

function appendText(container: HTMLElement, value: string, className?: string) {
  const element = document.createElement("p");
  if (className) element.className = className;
  element.textContent = value;
  container.appendChild(element);
}

function appendRecoveryAnswerVisibilityToggle(container: HTMLElement) {
  const toggle = document.createElement("button");
  toggle.type = "button";
  toggle.className = "portable-storage-recovery-answer-visibility";
  toggle.textContent = "显示安全问题答案";
  toggle.setAttribute("aria-pressed", "false");
  toggle.addEventListener("click", () => {
    const visible = toggle.getAttribute("aria-pressed") === "true";
    container
      .querySelectorAll<HTMLInputElement>("[data-portable-recovery-answer]")
      .forEach((input) => {
        input.type = visible ? "password" : "text";
      });
    toggle.textContent = visible ? "显示安全问题答案" : "隐藏安全问题答案";
    toggle.setAttribute("aria-pressed", String(!visible));
  });
  container.appendChild(toggle);
}

/**
 * Blocks all runtime initialization until the explicit portable key has been
 * accepted. Installed builds and browser previews pass through immediately.
 */
export async function installPortableStorageGate(): Promise<boolean> {
  const gate = document.getElementById("portableStorageGate");
  const title = document.getElementById("portableStorageTitle");
  const description = document.getElementById("portableStorageDescription");
  const form = document.getElementById(
    "portableStorageForm",
  ) as HTMLFormElement | null;
  const passwordLabel = document.getElementById("portableStoragePasswordLabel");
  const password = document.getElementById(
    "portableStoragePassword",
  ) as HTMLInputElement | null;
  const passwordHint = document.getElementById("portableStoragePasswordHint");
  const visibility = document.getElementById(
    "portableStoragePasswordVisibility",
  ) as HTMLButtonElement | null;
  const recoveryFields = document.getElementById(
    "portableStorageRecoveryFields",
  );
  const forgotPassword = document.getElementById(
    "portableStorageForgotPassword",
  ) as HTMLButtonElement | null;
  const recoveryBack = document.getElementById(
    "portableStorageRecoveryBack",
  ) as HTMLButtonElement | null;
  const submit = document.getElementById(
    "portableStorageSubmit",
  ) as HTMLButtonElement | null;
  const error = document.getElementById("portableStorageError");
  const invoke = getInvoke();

  if (
    !gate ||
    !title ||
    !description ||
    !form ||
    !passwordLabel ||
    !password ||
    !passwordHint ||
    !recoveryFields ||
    !forgotPassword ||
    !recoveryBack ||
    !submit ||
    !error
  ) {
    return false;
  }
  if (!invoke) {
    gate.hidden = true;
    return true;
  }

  let flow: PortableFlow = "unlock";
  let status: PortableStorageStatus;
  let submitLabel = "继续";
  const showError = (value: string) => {
    error.textContent = value;
    error.hidden = false;
  };
  const clearError = () => {
    error.textContent = "";
    error.hidden = true;
  };
  const setBusy = (busy: boolean) => {
    submit.disabled = busy;
    gate.setAttribute("aria-busy", String(busy));
    submit.textContent = busy ? "正在解锁…" : submitLabel;
  };
  const clearSensitiveFields = () => {
    password.value = "";
    recoveryFields
      .querySelectorAll<HTMLInputElement>("input")
      .forEach((input) => {
        input.value = "";
      });
  };
  const setupFields = () => {
    recoveryFields.replaceChildren();
    recoveryFields.hidden = false;
    appendText(
      recoveryFields,
      "请设置三道仅你知道答案的问题。答案不会以明文保存；避免公开或容易猜到的内容。忘记密码时，必须答对三题才能恢复同一份便携数据。",
    );
    appendInput(recoveryFields, {
      id: "portableStoragePasswordConfirm",
      label: "确认便携库密码",
      type: "password",
      autocomplete: "new-password",
      minLength: 12,
      maxLength: 1024,
    });
    for (let index = 1; index <= 3; index += 1) {
      appendInput(recoveryFields, {
        id: `portableStorageRecoveryQuestion${index}`,
        label: `安全问题 ${index}`,
        type: "text",
        autocomplete: "off",
        minLength: 6,
        maxLength: 240,
      });
      appendInput(recoveryFields, {
        id: `portableStorageRecoveryAnswer${index}`,
        label: `安全问题 ${index} 的答案`,
        type: "password",
        autocomplete: "off",
        minLength: 4,
        maxLength: 1024,
        recoveryAnswer: true,
      });
    }
    appendRecoveryAnswerVisibilityToggle(recoveryFields);
  };
  const recoveryFieldsForConfiguredQuestions = () => {
    recoveryFields.replaceChildren();
    recoveryFields.hidden = false;
    appendText(
      recoveryFields,
      "答对全部三题后可设置新密码；原有对话、设置和数据密钥不会被替换。",
    );
    appendInput(recoveryFields, {
      id: "portableStoragePasswordConfirm",
      label: "确认新便携库密码",
      type: "password",
      autocomplete: "new-password",
      minLength: 12,
      maxLength: 1024,
    });
    status.recoveryQuestions.forEach((question, index) => {
      appendInput(recoveryFields, {
        id: `portableStorageRecoveryAnswer${index + 1}`,
        label: `问题 ${index + 1}：${question}`,
        type: "password",
        autocomplete: "off",
        minLength: 4,
        maxLength: 1024,
        recoveryAnswer: true,
      });
    });
    appendRecoveryAnswerVisibilityToggle(recoveryFields);
  };
  const inputValue = (id: string) => {
    const input = document.getElementById(id) as HTMLInputElement | null;
    return input?.value ?? "";
  };
  const collectSetup = (): PortableRecoverySetup => ({
    questions: [1, 2, 3].map((index) =>
      inputValue(`portableStorageRecoveryQuestion${index}`),
    ),
    answers: [1, 2, 3].map((index) =>
      inputValue(`portableStorageRecoveryAnswer${index}`),
    ),
  });
  const collectAnswers = () =>
    [1, 2, 3].map((index) =>
      inputValue(`portableStorageRecoveryAnswer${index}`),
    );
  const passwordConfirmationMatches = () => {
    const confirmation = inputValue("portableStoragePasswordConfirm");
    if (password.value === confirmation) return true;
    showError("两次输入的便携库密码不一致，请重新确认。");
    const input = document.getElementById(
      "portableStoragePasswordConfirm",
    ) as HTMLInputElement | null;
    input?.focus();
    return false;
  };
  const showFlow = (next: PortableFlow) => {
    flow = next;
    clearSensitiveFields();
    clearError();
    password.type = "password";
    visibility?.setAttribute("aria-pressed", "false");
    if (visibility) visibility.textContent = "显示";
    forgotPassword.hidden = true;
    recoveryBack.hidden = true;
    recoveryFields.hidden = true;
    recoveryFields.replaceChildren();

    if (next === "unlock") {
      title.textContent = "解锁便携数据";
      description.textContent =
        "此便携版的数据位于 EXE 同级 novavei 文件夹。输入密码后才会加载对话和设置。";
      passwordLabel.textContent = "便携库密码";
      password.autocomplete = "current-password";
      passwordHint.textContent = "密码至少 12 个字符；它不会写入便携盘。";
      submitLabel = "解锁并继续";
      forgotPassword.hidden = !status.recoveryConfigured;
    } else if (next === "setup") {
      const isNewPortableStore = !status.initialized;
      title.textContent = isNewPortableStore
        ? "创建便携数据密码"
        : "设置便携数据恢复方式";
      description.textContent = isNewPortableStore
        ? "此便携版会把数据保存在 EXE 同级 novavei 文件夹。请创建密码，并设置三道自定义安全问题。"
        : "输入当前密码，并补充三道自定义安全问题。之后忘记密码时可恢复原有数据。";
      passwordLabel.textContent = isNewPortableStore
        ? "便携库密码"
        : "当前便携库密码";
      password.autocomplete = isNewPortableStore
        ? "new-password"
        : "current-password";
      passwordHint.textContent =
        "密码至少 12 个字符；它不会写入便携盘。请妥善保存三题答案。";
      submitLabel = isNewPortableStore ? "创建并继续" : "设置恢复方式并继续";
      setupFields();
    } else {
      title.textContent = "通过安全问题恢复密码";
      description.textContent =
        "设置新密码后即可解锁原有便携数据；不会新建或覆盖数据密钥。";
      passwordLabel.textContent = "新便携库密码";
      password.autocomplete = "new-password";
      passwordHint.textContent = "新密码至少 12 个字符；它不会写入便携盘。";
      submitLabel = "恢复并解锁";
      recoveryBack.hidden = false;
      recoveryFieldsForConfiguredQuestions();
    }
    submit.textContent = submitLabel;
    form.hidden = false;
    gate.removeAttribute("aria-busy");
    window.requestAnimationFrame(() => password.focus());
  };

  try {
    status = await invoke<PortableStorageStatus>("portable_storage_status");
  } catch (invokeError) {
    title.textContent = "无法确认便携数据状态";
    description.textContent = "请关闭应用，检查本机存储权限后重试。";
    showError(message(invokeError));
    gate.removeAttribute("aria-busy");
    return false;
  }
  if (!status.portable || status.unlocked) {
    gate.hidden = true;
    return true;
  }

  showFlow(
    !status.initialized || !status.recoveryConfigured ? "setup" : "unlock",
  );
  visibility?.addEventListener("click", () => {
    const visible = password.type === "text";
    password.type = visible ? "password" : "text";
    if (visibility) {
      visibility.textContent = visible ? "显示" : "隐藏";
      visibility.setAttribute("aria-pressed", String(!visible));
    }
    password.focus();
  });
  forgotPassword.addEventListener("click", () => {
    if (!status.recoveryConfigured || status.recoveryQuestions.length !== 3) {
      showError("此便携库尚未完成三道安全问题设置，无法恢复密码。");
      return;
    }
    showFlow("recover");
  });
  recoveryBack.addEventListener("click", () => showFlow("unlock"));

  return new Promise<boolean>((resolve) => {
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      if (submit.disabled) return;
      if (!form.checkValidity()) {
        form.reportValidity();
        return;
      }
      if (
        (flow === "setup" || flow === "recover") &&
        !passwordConfirmationMatches()
      ) {
        return;
      }
      const suppliedPassword = password.value;
      const recovery = flow === "setup" ? collectSetup() : undefined;
      const answers = flow === "recover" ? collectAnswers() : undefined;
      // Do not retain a password or answer in the DOM after it crosses the IPC
      // boundary. These short-lived values are never logged or persisted by
      // the renderer; native code keeps only encrypted key wrappers.
      clearSensitiveFields();
      clearError();
      setBusy(true);
      const request =
        flow === "recover"
          ? invoke<PortableStorageStatus>("portable_storage_recover", {
              answers,
              newPassword: suppliedPassword,
            })
          : invoke<PortableStorageStatus>("portable_storage_unlock", {
              password: suppliedPassword,
              recovery,
            });
      void request
        .then((next) => {
          if (!next.portable || !next.unlocked) {
            throw new Error("便携数据尚未解锁，请重试。");
          }
          gate.hidden = true;
          resolve(true);
        })
        .catch((unlockError) => {
          showError(message(unlockError));
          setBusy(false);
          password.focus();
        });
    });
  });
}
