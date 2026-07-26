/**
 * Human-facing path helpers for Windows extended-length / device paths.
 *
 * Native `canonicalize` may surface `\\?\E:\...` or `\\?\UNC\server\share`.
 * Those forms must not appear in UI labels, titles, or path-equality keys.
 * Prefer cleaning at the native boundary (`path_display.rs`); this module is
 * the renderer-side safety net for legacy stored values and any remaining
 * display sites.
 */

/**
 * Native workspace keys only fold case and slash variants on Windows. Keep
 * renderer grouping on the same platform rule so `/work/Foo` and `/work/foo`
 * remain distinct on case-sensitive filesystems.
 */
function usesWindowsPathSemantics(): boolean {
  if (typeof navigator === "undefined") return false;
  return (
    /^win/i.test(navigator.platform || "") ||
    /\bwindows\b/i.test(navigator.userAgent || "")
  );
}

/** Strip Windows extended-length / device prefixes for display and comparison. */
export function displayPath(value: string | undefined | null): string {
  const raw = (value ?? "").trim();
  if (!raw) return "";
  if (!usesWindowsPathSemantics()) return raw;

  if (raw.startsWith("\\\\?\\UNC\\")) {
    return `\\\\${raw.slice("\\\\?\\UNC\\".length)}`;
  }
  if (raw.startsWith("\\\\?\\")) {
    return raw.slice("\\\\?\\".length);
  }
  if (raw.startsWith("//?/UNC/")) {
    return `//${raw.slice("//?/UNC/".length)}`;
  }
  if (raw.startsWith("//?/")) {
    return raw.slice("//?/".length);
  }
  return raw;
}

/** Platform-aware comparison key that also ignores Windows extended prefixes. */
export function pathKey(value: string | undefined | null): string {
  const display = displayPath(value);
  if (!usesWindowsPathSemantics()) {
    let normalized = display;
    while (normalized.length > 1 && normalized.endsWith("/"))
      normalized = normalized.slice(0, -1);
    return normalized;
  }
  const isUnc = /^[\\/]{2}/.test(display);
  const withoutUncPrefix = isUnc ? display.replace(/^[\\/]+/, "") : display;
  let normalized = withoutUncPrefix.replace(/[\\/]+/g, "\\");
  if (isUnc) normalized = `\\\\${normalized}`;
  // A drive root is the one Windows path whose final separator is semantic.
  // Do not reduce `C:\\` to the drive-relative form `C:`; the Rust metadata
  // key applies the same rule so history grouping survives slash variants.
  if (!/^[A-Za-z]:\\$/.test(normalized))
    normalized = normalized.replace(/\\+$/, "");
  return normalized.toLowerCase();
}

/** Final path segment for project / folder labels. */
export function pathName(value: string | undefined | null): string {
  const windows = usesWindowsPathSemantics();
  const normalized = displayPath(value).replace(
    windows ? /[\\/]+$/ : /\/+$/,
    "",
  );
  const match = normalized.match(
    windows ? /(?:^|[\\/])([^\\/]+)$/ : /(?:^|\/)([^/]+)$/,
  );
  return match?.[1] || normalized || "工作区";
}
