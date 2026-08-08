import { PiRuntimeController } from "./controller";
import { installOverlayAccessibility } from "./accessibility";
import { installAppDialogs } from "./app-dialogs";
import { installAppSecuritySettings } from "./app-security";
import { installComposerAttachments } from "./attachments";
import { installBrowser } from "./browser";
import { installTranscriptNavigation } from "./chat-navigation";
import { installComposerCommands } from "./composer-commands";
import { createDomPiRuntime } from "./dom";
import { installGoals } from "./goals";
import { installHistorySearch } from "./history-search";
import { installNativeShell } from "./host";
import { installPermissionPicker } from "./permission-picker";
import { installStorageModeSettings } from "./portable-mode";
import { installPortableStorageGate } from "./portable-storage";
import {
  getComposerModelLabel,
  getComposerPermissionLabel,
} from "./shell-chrome";
import type { PiReasoningLevel, PiRuntimePublicApi } from "./types";
import { installWorkbench } from "./workbench";

// Ownership map (TypeScript entry owns runtime wiring; HTML owns visual chrome):
// - installAppDialogs / installPermissionPicker / installNativeShell /
//   installComposerSubmitBridge: this file
// - permission picker DOM markup: index.html; wiring: permission-picker.ts
// - durable history + DPAPI transcripts: native backend (history_store / secret_store)

function toast(message: string) {
  const target = document.getElementById("toast");
  if (!target) {
    console.warn("[NovaVei Pi]", message);
    return;
  }
  target.textContent = message;
  target.classList.add("show");
  window.setTimeout(() => target.classList.remove("show"), 2200);
}

/**
 * The embedded Agent and the optional Workflow/Local Services surfaces are
 * large modules. Defer them until the first paint so the initial chat shell
 * and transcript can become interactive sooner. Secondary-feature failures
 * must never prevent the primary Agent transport from being attached.
 */
function installDeferredRuntimeAfterFirstPaint(
  controller: PiRuntimeController,
) {
  let started = false;
  const load = async () => {
    if (started) return;
    started = true;
    void import("./embedded")
      .then(async ({ createEmbeddedPiTransport }) => {
        const embedded = createEmbeddedPiTransport();
        window.__novaveiPiEmbedded = embedded;
        await controller.attachTransport(embedded);
      })
      .catch((error) => {
        controller.failDeferredTransport(error);
        console.warn("[NovaVei] embedded runtime initialization failed", error);
      });
    void Promise.all([import("./workflows"), import("./local-services")])
      .then(([workflowsModule, localServicesModule]) => {
        workflowsModule.installPiWorkflows();
        localServicesModule.installLocalServices();
      })
      .catch((error) =>
        console.warn("[NovaVei] deferred feature initialization failed", error),
      );
    void import("./translation")
      .then(({ installTranslation }) => {
        installTranslation();
      })
      .catch((error) =>
        console.warn("[NovaVei] translation initialization failed", error),
      );
  };
  window.requestAnimationFrame(() => {
    window.setTimeout(() => void load(), 0);
  });
}

/**
 * The visual shell is still an HTML-first design surface.  Keep its submit
 * interception in the TypeScript entry rather than in an injected inline
 * Tauri bridge, so design-sync tooling can retain one stable module entry.
 */
function installComposerSubmitBridge(runtime: PiRuntimePublicApi) {
  const form = document.getElementById("composerForm");
  const input = document.getElementById(
    "composerInput",
  ) as HTMLTextAreaElement | null;
  if (!(form instanceof HTMLFormElement) || !input) return;
  if (form.dataset.novaveiPiSubmitBound === "true") return;
  form.dataset.novaveiPiSubmitBound = "true";
  const commands = installComposerCommands({ form, input });
  const commandPreparations = new Map<string, Promise<unknown>>();
  const reasoningLevels: readonly PiReasoningLevel[] = [
    "off",
    "minimal",
    "low",
    "medium",
    "high",
    "xhigh",
    "max",
  ];
  const composerSubmitContext = () => {
    const host = window.__novaveiHost;
    const sessionId = host?.getSessionId?.()?.trim() || undefined;
    const cwd = host?.getWorkdir?.()?.trim() || undefined;
    const selectedModel =
      document.querySelector<HTMLElement>(".model-option.on") ??
      document.querySelector<HTMLElement>(".model-option");
    const picker = document.getElementById("modelPickerName");
    const providerId =
      selectedModel?.dataset.providerId || picker?.dataset.providerId;
    const model =
      selectedModel?.dataset.modelId ||
      picker?.dataset.modelId ||
      getComposerModelLabel();
    const permissionValue = window.__novaveiPermission?.get?.();
    const permission =
      permissionValue === "readonly" ||
      permissionValue === "ask" ||
      permissionValue === "full"
        ? permissionValue
        : getComposerPermissionLabel();
    const reasoningIndex = Number(
      (document.getElementById("reasoningSlider") as HTMLInputElement | null)
        ?.value ??
        document.querySelector<HTMLElement>(
          ".reasoning-step.on[data-reasoning]",
        )?.dataset.reasoning,
    );
    const reasoning = Number.isFinite(reasoningIndex)
      ? reasoningLevels[
          Math.max(0, Math.min(reasoningLevels.length - 1, reasoningIndex))
        ]
      : undefined;
    return {
      sessionId,
      cwd,
      providerId,
      model,
      permission,
      reasoning,
      key: sessionId ? `session:${sessionId}` : cwd ? `cwd:${cwd}` : "preview",
    };
  };

  form.addEventListener(
    "submit",
    (event) => {
      // The design HTML also provides a browser-preview submit handler.  A
      // capture listener makes the real desktop path win without changing the
      // preview's standalone behavior.
      event.preventDefault();
      event.stopImmediatePropagation();
      const state = runtime.getState();
      if (
        [
          "starting",
          "running",
          "waiting_permission",
          "cancelling",
          "cancel_failed",
        ].includes(state.status)
      ) {
        // Ctrl/Cmd+Enter with an empty composer is the direct-stop path and
        // pauses any queued prompts. A non-empty draft continues below so the
        // Composer runtime can enqueue it behind this exact active turn.
        if (!input.value.trim() && state.status !== "cancelling") {
          void runtime
            .cancel()
            .catch((error) =>
              toast(error instanceof Error ? error.message : String(error)),
            );
        }
        if (!input.value.trim()) return;
      }
      const providerState = form.dataset.novaveiProviderState;
      if (
        providerState &&
        providerState !== "ready" &&
        providerState !== "preview"
      ) {
        if (providerState === "loading") {
          // A disabled button does not dispatch a synthetic click, so Enter
          // needs its own feedback while the provider catalog is loading.
          toast(
            document.documentElement.lang.toLowerCase().startsWith("en")
              ? "Loading provider settings. Please wait."
              : "正在读取供应商设置，请稍候",
          );
          return;
        }
        // Delegate to the Composer's provider-aware click handler so the
        // keyboard path opens the same Settings → Providers recovery route.
        (
          document.getElementById("btnSend") as HTMLButtonElement | null
        )?.click();
        return;
      }
      const text = input.value.trim();
      const hasAttachments =
        window.__novaveiComposerAttachments?.has?.() === true;
      if (!text && !hasAttachments) {
        toast("先输入内容或添加附件");
        return;
      }
      const submitContext = composerSubmitContext();
      if (commandPreparations.has(submitContext.key)) {
        toast("上一条消息仍在准备，请稍候");
        return;
      }
      const preparation = commands.prepare(text || "请分析所附文件。");
      commandPreparations.set(submitContext.key, preparation);
      void (async () => {
        let command: Awaited<ReturnType<typeof commands.prepare>>;
        try {
          command = await preparation;
        } catch (error) {
          toast(error instanceof Error ? error.message : String(error));
          return;
        } finally {
          if (commandPreparations.get(submitContext.key) === preparation)
            commandPreparations.delete(submitContext.key);
        }
        if (!command) return;
        if ("handled" in command) {
          if (input.value.trim() === text) {
            input.value = "";
            // Keep the slash-menu projection and its ARIA state in sync after
            // a local-only command completes without starting a Pi turn.
            input.dispatchEvent(new Event("input", { bubbles: true }));
          }
          toast(command.message);
          return;
        }
        void runtime
          .submit({
            ...command,
            sessionId: submitContext.sessionId,
            cwd: submitContext.cwd,
            providerId: submitContext.providerId,
            model: submitContext.model,
            permission: submitContext.permission,
            reasoning: submitContext.reasoning,
          })
          .catch((error) =>
            toast(error instanceof Error ? error.message : String(error)),
          );
      })();
    },
    true,
  );

  // Ctrl/Cmd+Enter submits (or stops) from the composer textarea. Plain Enter
  // remains a newline so multi-line prompts stay easy to write.
  input.addEventListener(
    "keydown",
    (event) => {
      if (
        event.key !== "Enter" ||
        !(event.ctrlKey || event.metaKey) ||
        event.isComposing
      )
        return;
      if (event.altKey || event.shiftKey) return;
      event.preventDefault();
      event.stopPropagation();
      if (typeof form.requestSubmit === "function") {
        form.requestSubmit();
        return;
      }
      form.dispatchEvent(
        new Event("submit", { bubbles: true, cancelable: true }),
      );
    },
    true,
  );
}

async function install() {
  // The gate intentionally precedes every stateful runtime bridge. A locked
  // portable drive must not hydrate sessions/settings or start an agent until
  // the password-derived key is available in native memory.
  if (!(await installPortableStorageGate())) return;
  // Shared confirm / prompt / error dialogs must bind before any feature that
  // might open them. Also exposes window.__novaveiDialogs for the large HTML
  // surface that cannot import modules directly.
  installAppDialogs();
  installStorageModeSettings();
  installAppSecuritySettings();
  // The permission picker owns its own popover interactions; Full
  // confirmation itself is deliberately performed by the native host
  // immediately before run.
  installPermissionPicker();
  // Let the lightweight shell render before parsing the embedded Agent and
  // workflow modules. The controller gains that transport immediately after
  // first paint; Tauri remains its capability boundary and no sidecar starts.
  const controller = new PiRuntimeController(null);
  controller.deferTransport();
  installOverlayAccessibility();
  const runtime = createDomPiRuntime(controller);
  window.__novaveiPiRuntime = runtime as PiRuntimePublicApi;
  (
    window as Window & { __novaveiPiController?: PiRuntimeController }
  ).__novaveiPiController = controller;
  installNativeShell();
  installTranscriptNavigation();
  installHistorySearch();
  installGoals();
  installWorkbench();
  installBrowser();
  installComposerAttachments();
  installDeferredRuntimeAfterFirstPaint(controller);
  installComposerSubmitBridge(runtime);
  void controller.ready.catch((error) => {
    console.warn("[NovaVei Pi] transport initialization failed", error);
  });
}

if (document.readyState === "loading") {
  document.addEventListener(
    "DOMContentLoaded",
    () => {
      void install();
    },
    { once: true },
  );
} else {
  void install();
}
