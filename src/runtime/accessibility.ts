/**
 * Keyboard and focus handling for the HTML-first shell overlays.
 *
 * The static design surface owns presentation and opening actions.  This
 * module only adds the desktop-quality dialog contract around those surfaces:
 * focus stays inside an open overlay, Escape has a predictable exit route,
 * and closing returns the user to the control that opened it.
 */

const SURFACE_IDS = [
  "overlaySkills",
  "overlayMcp",
  "overlaySettings",
  "overlayCouncil",
  "searchPalette",
] as const;

const FOCUSABLE = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled]):not([type=hidden])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

type Surface = HTMLElement;

function isSearchPalette(surface: Surface) {
  return surface.id === "searchPalette";
}

function isOpen(surface: Surface) {
  return surface.classList.contains("show") && !surface.hidden;
}

function isFocusable(target: Element | null): target is HTMLElement {
  if (!(target instanceof HTMLElement)) return false;
  if (target.matches("[disabled], [aria-hidden='true']")) return false;
  return !target.closest("[hidden], [aria-hidden='true']");
}

function focusableChildren(surface: Surface) {
  return [...surface.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
    isFocusable,
  );
}

function fallbackFocus() {
  const composer = document.getElementById("composerInput");
  if (isFocusable(composer)) composer.focus();
}

function firstFocusable(surface: Surface) {
  const [first] = focusableChildren(surface);
  if (first) first.focus();
  else {
    surface.tabIndex = -1;
    surface.focus();
  }
}

function visibleSurface(surfaces: readonly Surface[]) {
  // `searchPalette` can be layered over an already-open overlay.  Resolve
  // the last surface in DOM/stack order so Escape and the focus trap operate
  // on what the user can actually see, rather than the first overlay listed
  // in the source.
  return [...surfaces].reverse().find(isOpen);
}

function nativeDialogOpen() {
  return document.querySelector("dialog[open]") !== null;
}

/** Install once. It is safe in the browser preview as well as Tauri. */
export function installOverlayAccessibility() {
  const surfaces = SURFACE_IDS.map((id) => document.getElementById(id)).filter(
    (surface): surface is Surface => surface instanceof HTMLElement,
  );
  if (!surfaces.length) return;

  const restoreTargets = new WeakMap<Surface, HTMLElement>();
  let lastOpenSurface: Surface | undefined;

  const rememberTrigger = (surface: Surface) => {
    const active = document.activeElement;
    if (isFocusable(active) && !surface.contains(active))
      restoreTargets.set(surface, active);
  };

  const restoreFocus = (surface: Surface) => {
    const target = restoreTargets.get(surface);
    if (target?.isConnected && isFocusable(target)) {
      target.focus();
      return;
    }
    fallbackFocus();
  };

  const closeSurface = (surface: Surface) => {
    surface.classList.remove("show");
    if (isSearchPalette(surface)) surface.hidden = true;
    surface.setAttribute("aria-hidden", "true");
    if (lastOpenSurface === surface) lastOpenSurface = undefined;
    window.requestAnimationFrame(() => restoreFocus(surface));
  };

  const closeOpenOverlays = (primary: Surface) => {
    // The shell's `closeOverlays()` closes every app overlay at once. Mirror
    // that contract for Escape so residual multi-open states cannot leave a
    // second modal active after the top one is dismissed. Search palette is
    // independent and closes alone.
    if (isSearchPalette(primary)) {
      closeSurface(primary);
      return;
    }
    for (const surface of surfaces) {
      if (!isOpen(surface) || isSearchPalette(surface)) continue;
      surface.classList.remove("show");
      surface.setAttribute("aria-hidden", "true");
    }
    if (lastOpenSurface && !isSearchPalette(lastOpenSurface))
      lastOpenSurface = undefined;
    window.requestAnimationFrame(() => restoreFocus(primary));
  };

  for (const surface of surfaces) {
    if (!surface.hasAttribute("role")) surface.setAttribute("role", "dialog");
    surface.setAttribute("aria-modal", "true");
    surface.setAttribute("aria-hidden", String(!isOpen(surface)));
  }

  const observer = new MutationObserver(() => {
    const open = visibleSurface(surfaces);
    if (open) {
      for (const surface of surfaces)
        surface.setAttribute("aria-hidden", String(surface !== open));
      if (open !== lastOpenSurface) {
        rememberTrigger(open);
        lastOpenSurface = open;
      }
      window.requestAnimationFrame(() => {
        if (isOpen(open) && !open.contains(document.activeElement))
          firstFocusable(open);
      });
      return;
    }

    if (!lastOpenSurface) return;
    const closed = lastOpenSurface;
    lastOpenSurface = undefined;
    closed.setAttribute("aria-hidden", "true");
    window.requestAnimationFrame(() => restoreFocus(closed));
  });

  for (const surface of surfaces) {
    observer.observe(surface, {
      attributes: true,
      attributeFilter: ["class", "hidden"],
    });
  }

  document.addEventListener(
    "keydown",
    (event) => {
      if (nativeDialogOpen()) return;
      const surface = visibleSurface(surfaces);
      if (!surface) return;

      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        closeOpenOverlays(surface);
        return;
      }

      if (event.key !== "Tab") return;
      const controls = focusableChildren(surface);
      if (!controls.length) {
        event.preventDefault();
        surface.focus();
        return;
      }
      const first = controls[0];
      const last = controls.at(-1);
      if (!last) return;
      const active = document.activeElement;
      if (event.shiftKey && (active === first || !surface.contains(active))) {
        event.preventDefault();
        last.focus();
      } else if (
        !event.shiftKey &&
        (active === last || !surface.contains(active))
      ) {
        event.preventDefault();
        first.focus();
      }
    },
    true,
  );
}
