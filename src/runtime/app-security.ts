import { formatErrorMessage, requestAppConfirm } from "./app-dialogs";

type AppSecurityStatus = {
  portable: boolean;
  passwordRequired: boolean;
  passwordConfigured: boolean;
  unlocked: boolean;
  portableInitialized: boolean;
  portableRecoveryConfigured: boolean;
  portableRecoveryQuestions: string[];
};

type PortableRecoverySetup = {
  questions: string[];
  answers: string[];
};

type Invoke = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

const MIN_PASSWORD_LENGTH = 12;

const ERROR_COPY: Readonly<Record<string, string>> = {
  "portable storage password must contain at least 12 characters":
    "密码至少需要 12 个字符。",
  "portable storage password cannot contain control characters":
    "密码不能包含控制字符。",
  "portable storage password is incorrect": "当前密码不正确，请重试。",
  "portable storage password is incorrect or data is damaged":
    "当前密码不正确，或便携数据已损坏。",
  "current portable storage password is required before changing it":
    "更改便携版启动密码前，请输入当前密码。",
  "new portable storage password is required": "请设置新的启动密码。",
  "application password is incorrect": "当前密码不正确，请重试。",
  "application password is required": "请输入启动密码。",
  "application password is not configured": "启动密码尚未设置。",
  "current application password is required before changing it":
    "更改启动密码前，请输入当前密码。",
  "current application password is required before disabling it":
    "关闭启动密码前，请输入当前密码。",
  "application security configuration is invalid": "启动密码配置无效。",
  "application security configuration version is unsupported":
    "当前版本不支持此启动密码配置。",
  "portable recovery questions are required before enabling a portable password":
    "便携版启用启动密码前，需要先设置三组安全问题。",
};

function invokeApi(): Invoke | undefined {
  return window.__TAURI__?.core?.invoke as Invoke | undefined;
}

function element<T extends HTMLElement>(id: string): T | null {
  return document.getElementById(id) as T | null;
}

function isEnglish(): boolean {
  return document.documentElement.lang.toLowerCase().startsWith("en");
}

function text(chinese: string, english: string): string {
  return isEnglish() ? english : chinese;
}

function toast(message: string) {
  const target = element<HTMLElement>("toast");
  if (!target) {
    console.warn("[NovaVei security]", message);
    return;
  }
  target.textContent = message;
  target.classList.add("show");
  window.setTimeout(() => target.classList.remove("show"), 2200);
}

function friendlyError(error: unknown): string {
  const message = formatErrorMessage(error);
  return isEnglish() ? message : ERROR_COPY[message] || message;
}

function passwordValue(input: HTMLInputElement): string | undefined {
  return input.value.length > 0 ? input.value : undefined;
}

function setInlineStatus(
  target: HTMLElement,
  message: string,
  state: "info" | "success" | "error" = "info",
) {
  target.textContent = message;
  target.dataset.state = state;
}

export function installAppSecuritySettings() {
  const panel = element<HTMLElement>("appSecuritySettings");
  const stateText = element<HTMLElement>("appSecurityState");
  const modeText = element<HTMLElement>("appSecurityMode");
  const form = element<HTMLFormElement>("appSecurityForm");
  const currentField = element<HTMLElement>("appSecurityCurrentPasswordField");
  const currentPassword = element<HTMLInputElement>(
    "appSecurityCurrentPassword",
  );
  const newField = element<HTMLElement>("appSecurityNewPasswordField");
  const newPassword = element<HTMLInputElement>("appSecurityNewPassword");
  const confirmField = element<HTMLElement>("appSecurityConfirmPasswordField");
  const confirmPassword = element<HTMLInputElement>(
    "appSecurityConfirmPassword",
  );
  const recoveryPanel = element<HTMLElement>("appSecurityPortableRecovery");
  const recoveryQuestionInputs = [1, 2, 3]
    .map((index) =>
      element<HTMLInputElement>(`appSecurityRecoveryQuestion${index}`),
    )
    .filter((input): input is HTMLInputElement => input !== null);
  const recoveryAnswerInputs = [1, 2, 3]
    .map((index) =>
      element<HTMLInputElement>(`appSecurityRecoveryAnswer${index}`),
    )
    .filter((input): input is HTMLInputElement => input !== null);
  const save = element<HTMLButtonElement>("appSecuritySave");
  const refresh = element<HTMLButtonElement>("appSecurityRefresh");
  const notice = element<HTMLElement>("appSecurityNotice");
  const choices = Array.from(
    document.querySelectorAll<HTMLButtonElement>(
      "[data-app-security-required]",
    ),
  );
  const invoke = invokeApi();

  if (
    !panel ||
    !stateText ||
    !modeText ||
    !form ||
    !currentField ||
    !currentPassword ||
    !newField ||
    !newPassword ||
    !confirmField ||
    !confirmPassword ||
    !recoveryPanel ||
    recoveryQuestionInputs.length !== 3 ||
    recoveryAnswerInputs.length !== 3 ||
    !save ||
    !refresh ||
    !notice ||
    !choices.length
  ) {
    return;
  }

  let status: AppSecurityStatus | undefined;
  let selectedRequired = false;
  let busy = false;

  const clearSecrets = () => {
    currentPassword.value = "";
    newPassword.value = "";
    confirmPassword.value = "";
    recoveryQuestionInputs.forEach((input) => {
      input.value = "";
    });
    recoveryAnswerInputs.forEach((input) => {
      input.value = "";
    });
  };

  const requiresCurrentPassword = () =>
    Boolean(status?.passwordRequired && status.passwordConfigured);

  const newPasswordTouched = () =>
    newPassword.value.length > 0 || confirmPassword.value.length > 0;

  const portableRecoveryRequired = () =>
    Boolean(
      status?.portable &&
        selectedRequired &&
        !status.portableRecoveryConfigured,
    );

  const recoverySetup = (): PortableRecoverySetup | undefined => {
    if (!portableRecoveryRequired()) return undefined;
    return {
      questions: recoveryQuestionInputs.map((input) => input.value.trim()),
      answers: recoveryAnswerInputs.map((input) => input.value),
    };
  };

  const recoveryTouched = () =>
    recoveryQuestionInputs.some((input) => input.value.length > 0) ||
    recoveryAnswerInputs.some((input) => input.value.length > 0);

  const render = () => {
    const available = Boolean(invoke && status);
    const currentRequired = requiresCurrentPassword();
    const canChangeExisting =
      Boolean(status?.passwordRequired) && selectedRequired;
    const statusChanged =
      Boolean(status) && selectedRequired !== status?.passwordRequired;
    const saveNeeded =
      statusChanged ||
      (canChangeExisting && (newPasswordTouched() || recoveryTouched()));

    choices.forEach((button) => {
      const required = button.dataset.appSecurityRequired === "true";
      const selected = required === selectedRequired;
      button.classList.toggle("on", selected);
      button.setAttribute("aria-checked", String(selected));
      button.tabIndex = selected ? 0 : -1;
      button.disabled = busy || !available;
    });

    if (!invoke) {
      stateText.textContent = text(
        "仅 NovaVei 桌面端可设置启动密码。",
        "Startup password is available in NovaVei Desktop only.",
      );
      modeText.textContent = "";
      save.disabled = true;
      refresh.disabled = true;
      setInlineStatus(
        notice,
        text(
          "浏览器预览不会更改本机或便携版安全设置。",
          "Browser preview does not change installed or portable security settings.",
        ),
      );
      return;
    }

    refresh.disabled = busy;
    if (!status) {
      stateText.textContent = text(
        "正在读取启动密码状态…",
        "Reading password status…",
      );
      modeText.textContent = "";
      save.disabled = true;
      return;
    }

    stateText.textContent = selectedRequired
      ? text("启动密码：已开启", "Startup password: On")
      : text("启动密码：已关闭", "Startup password: Off");
    modeText.textContent = status.portable
      ? text("当前运行：便携版", "Running now: Portable")
      : text("当前运行：本机版", "Running now: Installed");

    currentField.hidden = !currentRequired;
    currentPassword.disabled = busy || !currentRequired;
    currentPassword.required = currentRequired;

    newField.hidden = !selectedRequired;
    confirmField.hidden = !selectedRequired;
    newPassword.disabled = busy || !selectedRequired;
    confirmPassword.disabled = busy || !selectedRequired;
    newPassword.required = selectedRequired;
    confirmPassword.required = selectedRequired;

    const recoveryRequired = portableRecoveryRequired();
    recoveryPanel.hidden = !recoveryRequired;
    recoveryQuestionInputs.forEach((input) => {
      input.disabled = busy || !recoveryRequired;
      input.required = recoveryRequired;
    });
    recoveryAnswerInputs.forEach((input) => {
      input.disabled = busy || !recoveryRequired;
      input.required = recoveryRequired;
    });

    save.textContent = busy
      ? text("正在保存…", "Saving…")
      : selectedRequired
        ? status.passwordRequired
          ? text("更新启动密码", "Update password")
          : text("开启启动密码", "Enable password")
        : text("关闭启动密码", "Disable password");
    save.disabled = busy || !saveNeeded;
  };

  const refreshStatus = async () => {
    if (!invoke || busy) return;
    status = undefined;
    render();
    try {
      status = await invoke<AppSecurityStatus>("app_security_status");
      selectedRequired = status.passwordRequired;
      clearSecrets();
      setInlineStatus(
        notice,
        status.passwordRequired
          ? text(
              "下次启动会先要求输入密码。",
              "The next launch will require the password first.",
            )
          : text(
              "下次启动会直接打开应用。",
              "The next launch will open the app directly.",
            ),
      );
    } catch (error) {
      setInlineStatus(notice, friendlyError(error), "error");
    } finally {
      render();
    }
  };

  const validatePasswordForm = () => {
    if (!selectedRequired) return true;
    if (newPassword.value.length < MIN_PASSWORD_LENGTH) {
      setInlineStatus(
        notice,
        text(
          `新密码至少需要 ${MIN_PASSWORD_LENGTH} 个字符。`,
          `New password must contain at least ${MIN_PASSWORD_LENGTH} characters.`,
        ),
        "error",
      );
      newPassword.focus();
      return false;
    }
    if (newPassword.value !== confirmPassword.value) {
      setInlineStatus(
        notice,
        text("两次输入的新密码不一致。", "The new passwords do not match."),
        "error",
      );
      confirmPassword.focus();
      return false;
    }
    if (portableRecoveryRequired()) {
      const questions = recoveryQuestionInputs.map((input) =>
        input.value.trim(),
      );
      const answers = recoveryAnswerInputs.map((input) => input.value);
      if (questions.some((question) => question.length < 6)) {
        setInlineStatus(
          notice,
          text(
            "便携版安全问题至少需要 6 个字符。",
            "Portable security questions must contain at least 6 characters.",
          ),
          "error",
        );
        recoveryQuestionInputs
          .find((input) => input.value.trim().length < 6)
          ?.focus();
        return false;
      }
      if (
        new Set(questions.map((question) => question.toLowerCase())).size < 3
      ) {
        setInlineStatus(
          notice,
          text(
            "三组安全问题不能重复。",
            "The three security questions must be different.",
          ),
          "error",
        );
        recoveryQuestionInputs[0]?.focus();
        return false;
      }
      if (answers.some((answer) => answer.length < 4)) {
        setInlineStatus(
          notice,
          text(
            "便携版安全问题的答案至少需要 4 个字符。",
            "Portable security answers must contain at least 4 characters.",
          ),
          "error",
        );
        recoveryAnswerInputs.find((input) => input.value.length < 4)?.focus();
        return false;
      }
      if (!questions.every((question) => question.trim().length > 0)) {
        setInlineStatus(
          notice,
          text(
            "请填写三组安全问题。",
            "Please fill in all three security questions.",
          ),
          "error",
        );
        recoveryQuestionInputs.find((input) => !input.value.trim())?.focus();
        return false;
      }
    }
    return true;
  };

  const savePasswordSettings = async () => {
    if (!invoke || !status || busy) return;
    if (!form.checkValidity()) {
      form.reportValidity();
      return;
    }
    if (!validatePasswordForm()) return;
    if (!selectedRequired) {
      const confirmed = await requestAppConfirm({
        title: text("关闭启动密码", "Disable startup password"),
        message: status.portable
          ? text(
              "便携版下次启动将直接打开当前便携数据。",
              "Portable mode will open the current portable data directly on the next launch.",
            )
          : text(
              "本机版下次启动将直接打开当前用户的数据。",
              "Installed mode will open this user's data directly on the next launch.",
            ),
        confirmLabel: text("关闭启动密码", "Disable password"),
        cancelLabel: text("取消", "Cancel"),
        danger: true,
      });
      if (!confirmed) return;
    }

    const currentPasswordValue = passwordValue(currentPassword);
    const newPasswordValue = newPassword.value;
    const recovery = recoverySetup();
    if (
      selectedRequired &&
      status.portable &&
      !recovery &&
      !status.portableRecoveryConfigured
    ) {
      setInlineStatus(
        notice,
        text(
          "便携版启用启动密码前，需要先设置三组安全问题。",
          "Portable mode needs three recovery questions before enabling a startup password.",
        ),
        "error",
      );
      return;
    }
    busy = true;
    render();
    try {
      const args: Record<string, unknown> = {
        currentPassword: currentPasswordValue,
        newPassword: newPasswordValue,
      };
      if (recovery) args.recoverySetup = recovery;
      status = selectedRequired
        ? await invoke<AppSecurityStatus>("app_security_set_password", args)
        : await invoke<AppSecurityStatus>("app_security_disable_password", {
            currentPassword: currentPasswordValue,
          });
      selectedRequired = status.passwordRequired;
      clearSecrets();
      setInlineStatus(
        notice,
        status.passwordRequired
          ? text("启动密码已开启。", "Startup password enabled.")
          : text("启动密码已关闭。", "Startup password disabled."),
        "success",
      );
      toast(
        status.passwordRequired
          ? text("启动密码已开启", "Startup password enabled")
          : text("启动密码已关闭", "Startup password disabled"),
      );
      window.dispatchEvent(
        new CustomEvent("novavei:app-security-changed", { detail: status }),
      );
    } catch (error) {
      clearSecrets();
      setInlineStatus(notice, friendlyError(error), "error");
    } finally {
      busy = false;
      render();
    }
  };

  choices.forEach((button) => {
    button.addEventListener("click", () => {
      if (button.disabled) return;
      selectedRequired = button.dataset.appSecurityRequired === "true";
      render();
      if (selectedRequired) newPassword.focus();
      else if (requiresCurrentPassword()) currentPassword.focus();
    });
  });

  form.addEventListener("input", render);
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    void savePasswordSettings();
  });
  refresh.addEventListener("click", () => {
    void refreshStatus();
  });
  window.addEventListener("novavei:language-changed", render);

  render();
  void refreshStatus();
}
