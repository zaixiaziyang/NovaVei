/**
 * Composer permission picker: safe default preferences plus a root-scoped
 * full-access intent. DOM chrome stays in index.html; this module owns the
 * interactive wiring and window.__novaveiPermission surface. A Full choice
 * is never itself a capability: native mints a one-use grant for the exact
 * run immediately before agent_run.
 */

import type { FullPermissionGrantRequest } from "./types";

type PermissionKey = "readonly" | "ask" | "auto-approve" | "full";

type FullPermissionRunGrant = {
  grantToken?: unknown;
  workdir?: unknown;
  expiresAt?: unknown;
  expiresAtMs?: unknown;
};

type UnknownRecord = Record<string, unknown>;

type Invoke = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

const PERMISSION_COPY: Record<PermissionKey, { label: string; toast: string }> =
  {
    readonly: { label: "只读", toast: "权限：只读" },
    ask: { label: "请求批准", toast: "权限：请求批准" },
    "auto-approve": { label: "替我审批", toast: "权限：替我审批" },
    full: {
      label: "完全访问权限",
      toast: "权限：完全访问（仅当前运行）",
    },
  };

function toast(message: string) {
  const target = document.getElementById("toast");
  if (!target) {
    console.warn("[NovaVei permission]", message);
    return;
  }
  target.textContent = message;
  target.classList.add("show");
  window.setTimeout(() => target.classList.remove("show"), 2200);
}

function isEnglish() {
  return document.documentElement.lang.toLowerCase().startsWith("en");
}

function getInvoke(): Invoke | undefined {
  return window.__TAURI__?.core?.invoke as Invoke | undefined;
}

function normalizePermissionKey(value: unknown): PermissionKey {
  if (value === "readonly" || value === "read_only") return "readonly";
  if (value === "auto-approve" || value === "auto") return "auto-approve";
  if (value === "full") return "full";
  return "ask";
}

/** Only the three non-Full tiers are valid durable project preferences. */
function projectPermissionKey(
  value: unknown,
): Exclude<PermissionKey, "full"> | undefined {
  if (typeof value !== "string") return undefined;
  const normalized = value.trim().toLowerCase();
  if (normalized === "readonly" || normalized === "read_only")
    return "readonly";
  if (normalized === "ask") return "ask";
  if (normalized === "auto-approve" || normalized === "auto")
    return "auto-approve";
  return undefined;
}

function normalizePermissionRoot(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function permissionRootKey(root: string): string {
  return normalizePermissionRoot(root)
    .replace(/[\\/]+$/, "")
    .replace(/\\/g, "/")
    .toLocaleLowerCase();
}

function currentPermissionRoot(): string {
  const nativeRoot = normalizePermissionRoot(
    window.__novaveiHost?.getWorkdir?.(),
  );
  // A desktop authorization must use the host's canonical current workspace
  // only. The visual shell can retain a DOM fallback for browser-preview
  // interaction, but that text must never become a Tauri permission scope
  // while hydration is incomplete.
  if (window.__TAURI__?.core?.invoke) return nativeRoot;
  const previewRoot = document.querySelector<HTMLElement>(
    '.project-row[aria-current="page"][data-workdir]',
  )?.dataset.workdir;
  return nativeRoot || normalizePermissionRoot(previewRoot);
}

function fullPermissionGrantToken(
  value: unknown,
  expectedRoot: string,
): string | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) return;
  const grant = value as FullPermissionRunGrant;
  const token = normalizePermissionRoot(grant.grantToken);
  const nativeRoot = normalizePermissionRoot(grant.workdir);
  const expiresAt =
    typeof grant.expiresAtMs === "number"
      ? grant.expiresAtMs
      : typeof grant.expiresAt === "number"
        ? grant.expiresAt
        : typeof grant.expiresAt === "string"
          ? Date.parse(grant.expiresAt)
          : Number.NaN;
  if (
    !token ||
    !nativeRoot ||
    permissionRootKey(nativeRoot) !== permissionRootKey(expectedRoot) ||
    !Number.isFinite(expiresAt) ||
    expiresAt <= Date.now()
  ) {
    return;
  }
  return token;
}

export function installPermissionPicker() {
  const permissionBtn = document.getElementById(
    "btnPermission",
  ) as HTMLButtonElement | null;
  const permissionPopover = document.getElementById(
    "permissionPopover",
  ) as HTMLElement | null;
  if (!permissionBtn || !permissionPopover) return;

  let permissionKey: PermissionKey = "ask";
  let defaultPermissionKey: PermissionKey = "ask";
  // These roots only retain the user's visible intent for this renderer
  // lifetime. They never authorize a run and are deliberately never saved.
  const fullPermissionIntentRoots = new Set<string>();
  let permissionSettingsRevision = 0;
  let permissionProjectSyncTimer: number | undefined;
  let permissionOptionsBusy = false;

  const hasFullPermissionIntent = (root: string) => {
    const key = permissionRootKey(root);
    return Boolean(key && fullPermissionIntentRoots.has(key));
  };

  const permissionForCurrentRoot = (
    root: string = currentPermissionRoot(),
  ): PermissionKey => {
    if (hasFullPermissionIntent(root)) return "full";
    const projectPermission = projectPermissionKey(
      window.__novaveiHost?.getCurrentProjectPreferences?.()?.permission,
    );
    if (projectPermission) return projectPermission;
    // Older builds persisted full access as a global default. Never carry
    // that privilege into a project without a native, one-use run grant. The
    // system value remains only as a backward-compatible/default fallback for
    // a project that has not selected its own tier yet.
    return defaultPermissionKey === "full" ? "ask" : defaultPermissionKey;
  };

  const syncPermissionPicker = () => {
    const label = document.getElementById("permissionLabel");
    if (label) label.textContent = PERMISSION_COPY[permissionKey].label;
    document
      .querySelectorAll<HTMLElement>(".permission-option")
      .forEach((option) => {
        const selected = option.dataset.permission === permissionKey;
        option.classList.toggle("on", selected);
        option.setAttribute("aria-pressed", String(selected));
      });
  };

  const syncPermissionScope = (root: string = currentPermissionRoot()) => {
    const rootLabel = document.getElementById("permissionProjectRootLabel");
    const rootValue = document.getElementById("permissionProjectRoot");
    const alwaysScope = document.getElementById("permissionAlwaysScope");
    const hasRoot = Boolean(root);
    if (rootLabel) {
      rootLabel.textContent = isEnglish()
        ? "Current project root"
        : "当前项目根";
    }
    if (rootValue) {
      rootValue.textContent =
        root || (isEnglish() ? "No active project" : "未打开项目");
      rootValue.title = root || "";
    }
    if (alwaysScope) {
      alwaysScope.textContent = isEnglish()
        ? "Read-only, Ask, and Auto approve are saved for this project. Full access applies only to the current run."
        : "只读、请求批准和替我审批会分别保存到当前项目；完全访问仅作用于当前运行。";
    }
    const fullOption = document.querySelector<HTMLButtonElement>(
      '.permission-option[data-permission="full"]',
    );
    if (fullOption) fullOption.disabled = !hasRoot || permissionOptionsBusy;
    permissionPopover.setAttribute(
      "aria-label",
      hasRoot
        ? isEnglish()
          ? `Permission options for ${root}`
          : `权限选项，项目根：${root}`
        : isEnglish()
          ? "Permission options without an active project"
          : "权限选项，尚未打开项目",
    );
  };

  const syncPermissionForCurrentProject = () => {
    const root = currentPermissionRoot();
    permissionKey = permissionForCurrentRoot(root);
    syncPermissionPicker();
    syncPermissionScope(root);
  };

  const schedulePermissionScopeSync = () => {
    if (permissionProjectSyncTimer !== undefined) {
      window.clearTimeout(permissionProjectSyncTimer);
    }
    permissionProjectSyncTimer = window.setTimeout(
      syncPermissionForCurrentProject,
      0,
    );
  };

  const applySavedPermissionSettings = (system: unknown) => {
    const nextSystem =
      system && typeof system === "object" && !Array.isArray(system)
        ? (system as UnknownRecord)
        : {};
    const configured = normalizePermissionKey(
      nextSystem.defaultPermissionTier ?? nextSystem.default_permission_tier,
    );
    defaultPermissionKey = configured === "full" ? "ask" : configured;
    syncPermissionForCurrentProject();
  };

  const persistPermissionKey = async (next: PermissionKey, root: string) => {
    const invoke = getInvoke();
    const revision = ++permissionSettingsRevision;
    // Full is an ephemeral run intent, never a persisted default or a saved
    // root authorization. For a registered project, every other choice is
    // saved alongside its model/reasoning preferences so switching folders
    // restores that folder's policy rather than changing every project.
    if (next !== "full") {
      const host = window.__novaveiHost;
      const hostRoot = normalizePermissionRoot(host?.getWorkdir?.());
      if (
        host &&
        root &&
        permissionRootKey(root) === permissionRootKey(hostRoot)
      ) {
        const saved = await host.saveCurrentProjectPreferences({
          permission: next,
        });
        if (!saved) {
          throw new Error(
            isEnglish()
              ? "Open a registered project before changing its permission setting."
              : "请先打开已登记的项目，再修改该项目的权限设置。",
          );
        }
        return;
      }
    }

    // Before a project is available (including browser preview), retain the
    // former system-level behavior as a harmless fallback for new projects.
    // Full deliberately does not alter even that fallback.
    if (next === "full") return;
    const system: UnknownRecord = {
      defaultPermissionTier: next,
      workdirPolicy: "project",
    };
    if (invoke) await invoke("settings_save_system", { payload: system });
    if (revision !== permissionSettingsRevision) return;
    defaultPermissionKey = normalizePermissionKey(system.defaultPermissionTier);
    window.dispatchEvent(
      new CustomEvent("novavei:system-settings-merged", { detail: system }),
    );
  };

  const requestFullPermissionGrant = async (
    request: FullPermissionGrantRequest,
  ): Promise<string | undefined> => {
    const activeRoot = currentPermissionRoot();
    if (permissionForCurrentRoot(activeRoot) !== "full") return undefined;

    const requestId = normalizePermissionRoot(request?.requestId);
    const sessionId = normalizePermissionRoot(request?.sessionId);
    const conversationId = normalizePermissionRoot(request?.conversationId);
    const workdir = normalizePermissionRoot(request?.workdir);
    const providerId = normalizePermissionRoot(request?.providerId);
    const model = normalizePermissionRoot(request?.model);
    const reasoning = normalizePermissionRoot(request?.reasoning);
    // Preserve the exact runtime text for native binding; trimming is only a
    // validation check so whitespace is never silently rewritten.
    const text = typeof request?.text === "string" ? request.text : "";
    if (
      !requestId ||
      !sessionId ||
      !conversationId ||
      !workdir ||
      !providerId ||
      !model ||
      !text.trim() ||
      !activeRoot ||
      permissionRootKey(workdir) !== permissionRootKey(activeRoot)
    ) {
      throw new Error(
        isEnglish()
          ? "Full access is available only for the active run and project."
          : "完全访问仅可用于当前运行和项目。",
      );
    }

    const invoke = getInvoke();
    if (!invoke) {
      throw new Error(
        isEnglish()
          ? "Full access is available only in the NovaVei desktop app."
          : "完全访问仅在 NovaVei 桌面应用中可用。",
      );
    }

    const response = await invoke<FullPermissionRunGrant>(
      "full_permission_confirm",
      {
        requestId,
        sessionId,
        conversationId,
        workdir,
        text,
        providerId,
        model,
        reasoning: reasoning || undefined,
      },
    );
    const token = fullPermissionGrantToken(response, activeRoot);
    if (!token) {
      throw new Error(
        isEnglish()
          ? "Full access did not produce a valid run grant."
          : "完全访问未生成有效的当前运行授权。",
      );
    }
    // The opaque token intentionally has no renderer-side cache. It is
    // returned directly to the immediately following agent_run call.
    return token;
  };

  const closePermissionPopover = (restoreFocus = false) => {
    permissionPopover.classList.remove("show");
    permissionBtn.setAttribute("aria-expanded", "false");
    if (restoreFocus) permissionBtn.focus();
  };

  const permissionControl =
    permissionBtn.closest<HTMLElement>(".permission-control") ??
    permissionPopover.parentElement;
  const onPermissionOutsidePointer = (event: PointerEvent) => {
    if (!permissionPopover.classList.contains("show")) return;
    const target = event.target;
    if (target instanceof Node && permissionControl?.contains(target)) return;
    // Do not restore focus on an outside press: that press may be moving
    // focus to another real control.
    closePermissionPopover();
  };
  const onPermissionEscape = (event: KeyboardEvent) => {
    if (
      event.key !== "Escape" ||
      event.isComposing ||
      !permissionPopover.classList.contains("show") ||
      document.querySelector("dialog[open]")
    ) {
      return;
    }
    // This is a capture listener so the legacy shell's Escape chain cannot
    // continue on to close search or another overlay after dismissing us.
    event.preventDefault();
    event.stopImmediatePropagation();
    closePermissionPopover(true);
  };
  document.addEventListener("pointerdown", onPermissionOutsidePointer, true);
  document.addEventListener("keydown", onPermissionEscape, true);

  permissionBtn.addEventListener("click", () => {
    const willOpen = !permissionPopover.classList.contains("show");
    if (!willOpen) {
      closePermissionPopover();
      return;
    }
    syncPermissionScope();
    permissionPopover.classList.add("show");
    permissionBtn.setAttribute("aria-expanded", "true");
    requestAnimationFrame(() => {
      (
        permissionPopover.querySelector(
          ".permission-option.on, .permission-option",
        ) as HTMLElement | null
      )?.focus();
    });
  });

  document
    .querySelectorAll<HTMLButtonElement>(".permission-option")
    .forEach((option) => {
      option.addEventListener("click", async () => {
        const nextPermission = normalizePermissionKey(
          option.dataset.permission,
        );
        const root = currentPermissionRoot();
        if (nextPermission === "full") {
          if (!root) {
            toast(
              isEnglish()
                ? "Open a project before enabling Full access."
                : "请先打开项目，再启用完全访问权限。",
            );
            return;
          }
        }
        const rootKey = permissionRootKey(root);
        const hadFullPermissionIntent = Boolean(
          rootKey && fullPermissionIntentRoots.has(rootKey),
        );
        if (rootKey) {
          if (nextPermission === "full") fullPermissionIntentRoots.add(rootKey);
          else fullPermissionIntentRoots.delete(rootKey);
        }
        permissionOptionsBusy = true;
        document
          .querySelectorAll<HTMLButtonElement>(".permission-option")
          .forEach((control) => {
            control.disabled = true;
          });
        try {
          await persistPermissionKey(nextPermission, root);
          const activeRoot = currentPermissionRoot();
          if (
            nextPermission === "full" &&
            permissionRootKey(activeRoot) !== permissionRootKey(root)
          ) {
            syncPermissionForCurrentProject();
            toast(
              isEnglish()
                ? "The project changed; Full access remains only an intent for the project you selected."
                : "项目已切换；完全访问意图仅保留给刚选择的项目。",
            );
          } else {
            permissionKey = permissionForCurrentRoot(activeRoot);
            syncPermissionPicker();
            syncPermissionScope(activeRoot);
            toast(PERMISSION_COPY[permissionKey].toast);
          }
          closePermissionPopover(true);
        } catch {
          if (rootKey) {
            if (hadFullPermissionIntent) fullPermissionIntentRoots.add(rootKey);
            else fullPermissionIntentRoots.delete(rootKey);
          }
          permissionKey = permissionForCurrentRoot(currentPermissionRoot());
          syncPermissionPicker();
          toast(
            isEnglish()
              ? "Could not save the permission setting."
              : "权限设置保存失败，已恢复原值。",
          );
        } finally {
          permissionOptionsBusy = false;
          document
            .querySelectorAll<HTMLButtonElement>(".permission-option")
            .forEach((control) => {
              control.disabled = false;
            });
          syncPermissionScope();
        }
      });
    });

  syncPermissionForCurrentProject();
  window.__novaveiPermission = {
    get: () => {
      permissionKey = permissionForCurrentRoot();
      syncPermissionPicker();
      syncPermissionScope();
      return permissionKey;
    },
    set: (value: string) => {
      defaultPermissionKey = normalizePermissionKey(value);
      if (defaultPermissionKey === "full") defaultPermissionKey = "ask";
      const rootKey = permissionRootKey(currentPermissionRoot());
      if (rootKey && value !== "full")
        fullPermissionIntentRoots.delete(rootKey);
      syncPermissionForCurrentProject();
    },
    requestFullPermissionGrant,
  };
  window.addEventListener(
    "novavei:session-changed",
    schedulePermissionScopeSync,
  );
  window.addEventListener(
    "novavei:current-project-changed",
    schedulePermissionScopeSync,
  );
  window.addEventListener(
    "novavei:project-preferences-changed",
    schedulePermissionScopeSync,
  );
  window.addEventListener("novavei:language-changed", () => {
    syncPermissionScope();
  });
  window.addEventListener(
    "novavei:workdir-changed",
    schedulePermissionScopeSync,
  );

  const invoke = getInvoke();
  if (invoke) {
    const revision = ++permissionSettingsRevision;
    void invoke<UnknownRecord>("settings_load_all")
      .then((settings) => {
        if (revision !== permissionSettingsRevision) return;
        applySavedPermissionSettings(settings?.system);
      })
      .catch(() => {
        toast(
          isEnglish()
            ? "Could not load the saved permission setting."
            : "无法读取已保存的权限设置。",
        );
      });
  }
}
