import { requestAppConfirm, showAppError } from "./app-dialogs";

type StorageMode = "installed" | "portable";

type StorageModeStatus = {
  currentMode: StorageMode;
  nextLaunchMode: StorageMode;
  restartRequired: boolean;
};

type Invoke = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

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

function modeLabel(mode: StorageMode): string {
  return mode === "portable"
    ? text("便携版", "Portable")
    : text("本机版", "Installed");
}

function statusFrom(value: unknown): StorageModeStatus | undefined {
  if (!value || typeof value !== "object") return undefined;
  const record = value as Record<string, unknown>;
  const currentMode = record.currentMode;
  const nextLaunchMode = record.nextLaunchMode;
  if (
    (currentMode !== "installed" && currentMode !== "portable") ||
    (nextLaunchMode !== "installed" && nextLaunchMode !== "portable")
  ) {
    return undefined;
  }
  return {
    currentMode,
    nextLaunchMode,
    restartRequired: record.restartRequired === true,
  };
}

/**
 * Binds System → Portable mode to the native startup marker. The current
 * process never re-roots live storage; a switch applies only after a full
 * restart, keeping database and WebView state in one authoritative root.
 */
export function installStorageModeSettings() {
  const panel = element<HTMLElement>("portableModeSettings");
  const current = element<HTMLElement>("portableModeCurrent");
  const next = element<HTMLElement>("portableModeNext");
  const notice = element<HTMLElement>("portableModeNotice");
  const apply = element<HTMLButtonElement>("portableModeApply");
  const buttons = Array.from(
    document.querySelectorAll<HTMLButtonElement>("[data-storage-mode]"),
  );
  const invoke = invokeApi();
  if (!panel || !current || !next || !notice || !apply || !buttons.length)
    return;

  let status: StorageModeStatus | undefined;
  let selected: StorageMode = "installed";
  let busy = false;

  const render = () => {
    const available = Boolean(invoke && status);
    buttons.forEach((button) => {
      const mode = button.dataset.storageMode as StorageMode | undefined;
      const active = mode === selected;
      button.classList.toggle("on", active);
      button.setAttribute("aria-checked", String(active));
      button.disabled = !available || busy;
    });
    if (!invoke) {
      current.textContent = text(
        "仅 NovaVei 桌面端可读取运行模式。",
        "Run mode is available in NovaVei Desktop only.",
      );
      next.hidden = true;
      notice.textContent = text(
        "浏览器预览不会更改应用文件或数据位置。",
        "Browser preview never changes app files or data locations.",
      );
      apply.disabled = true;
      return;
    }
    next.hidden = false;
    if (!status) {
      current.textContent = text("正在读取运行模式…", "Reading run mode…");
      next.textContent = "";
      notice.textContent = "";
      apply.disabled = true;
      return;
    }
    current.textContent = text(
      `当前运行：${modeLabel(status.currentMode)}`,
      `Running now: ${modeLabel(status.currentMode)}`,
    );
    next.textContent = text(
      `下次启动：${modeLabel(status.nextLaunchMode)}`,
      `Next launch: ${modeLabel(status.nextLaunchMode)}`,
    );
    notice.textContent = status.restartRequired
      ? text(
          "已保存模式切换。请完全退出并重新打开 NovaVei 后生效。",
          "The mode change is saved. Fully quit and reopen NovaVei to apply it.",
        )
      : text(
          "当前模式已生效。切换会在下次完整重启后应用。",
          "The current mode is active. A change applies after the next full restart.",
        );
    apply.textContent = busy
      ? text("正在保存…", "Saving…")
      : text(
          `切换到${modeLabel(selected)}`,
          `Switch to ${modeLabel(selected)}`,
        );
    apply.disabled = busy || selected === status.nextLaunchMode;
  };

  const refresh = async () => {
    if (!invoke) return;
    status = undefined;
    render();
    const response = await invoke<unknown>("storage_mode_status");
    const parsed = statusFrom(response);
    if (!parsed) throw new Error("storage mode status is invalid");
    status = parsed;
    selected = parsed.nextLaunchMode;
    render();
  };

  buttons.forEach((button) => {
    button.addEventListener("click", () => {
      const mode = button.dataset.storageMode;
      if (mode !== "installed" && mode !== "portable") return;
      selected = mode;
      render();
    });
  });

  apply.addEventListener("click", () => {
    if (!invoke || !status || busy || selected === status.nextLaunchMode)
      return;
    const target = selected;
    const currentMode = status.currentMode;
    void requestAppConfirm({
      title: text("切换运行模式", "Switch run mode"),
      message: text(
        `将在下次启动时使用${modeLabel(target)}。现有${modeLabel(currentMode)}数据会保留且不会自动复制；确认后请完全退出并重新打开应用。`,
        `The next launch will use ${modeLabel(target)}. Existing ${modeLabel(currentMode)} data stays in place and is not copied automatically; fully quit and reopen the app after confirming.`,
      ),
      confirmLabel: text("保存并下次启动生效", "Save for next launch"),
      cancelLabel: text("取消", "Cancel"),
      danger: false,
    }).then((confirmed) => {
      if (!confirmed) return;
      busy = true;
      render();
      return invoke<unknown>("storage_mode_set", { mode: target })
        .then((response) => {
          const parsed = statusFrom(response);
          if (!parsed) throw new Error("storage mode update is invalid");
          status = parsed;
          selected = parsed.nextLaunchMode;
        })
        .catch((error) =>
          showAppError(
            error,
            text("无法切换运行模式", "Unable to switch run mode"),
          ),
        )
        .finally(() => {
          busy = false;
          render();
        });
    });
  });

  window.addEventListener("novavei:language-changed", render);
  render();
  void refresh().catch((error) => {
    current.textContent = text(
      "无法读取运行模式。",
      "Unable to read the run mode.",
    );
    notice.textContent = text(
      "请检查应用目录权限后重试。",
      "Check the application-folder permissions and try again.",
    );
    void showAppError(
      error,
      text("无法读取运行模式", "Unable to read run mode"),
    );
  });
}
