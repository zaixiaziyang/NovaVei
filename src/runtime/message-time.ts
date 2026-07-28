const FULL_MESSAGE_TIMESTAMP_STORAGE_KEY = "novavei.showFullMessageTimestamp";

export function normalizeFullMessageTimestampPreference(value: unknown) {
  if (typeof value === "boolean") return value;
  if (typeof value !== "string") return false;
  const normalized = value.trim().toLowerCase();
  return ["1", "true", "full", "date", "datetime"].includes(normalized);
}

export function currentFullMessageTimestampPreference() {
  const mode = document.documentElement.dataset.messageTimestampMode;
  if (mode === "full") return true;
  if (mode === "time") return false;
  try {
    return normalizeFullMessageTimestampPreference(
      window.localStorage?.getItem(FULL_MESSAGE_TIMESTAMP_STORAGE_KEY),
    );
  } catch {
    return false;
  }
}

export function applyFullMessageTimestampPreference(showFull: boolean) {
  const enabled = Boolean(showFull);
  document.documentElement.dataset.messageTimestampMode = enabled
    ? "full"
    : "time";
  try {
    window.localStorage?.setItem(
      FULL_MESSAGE_TIMESTAMP_STORAGE_KEY,
      String(enabled),
    );
  } catch {
    // localStorage can be unavailable in restricted WebViews.
  }
  refreshMessageTimestampLabels();
}

export function formatMessageTimestamp(
  value: Date,
  showFull = currentFullMessageTimestampPreference(),
) {
  if (showFull) {
    const minute = String(value.getMinutes()).padStart(2, "0");
    return `${value.getFullYear()}.${value.getMonth() + 1}.${value.getDate()} ${value.getHours()}.${minute}`;
  }
  return value.toLocaleTimeString([], { hour12: false });
}

export function refreshMessageTimestampLabels(root: ParentNode = document) {
  const showFull = currentFullMessageTimestampPreference();
  for (const node of root.querySelectorAll<HTMLTimeElement>("time.msg-ended")) {
    if (!node.dateTime) continue;
    const value = new Date(node.dateTime);
    if (!Number.isFinite(value.getTime())) continue;
    node.textContent = formatMessageTimestamp(value, showFull);
  }
}
