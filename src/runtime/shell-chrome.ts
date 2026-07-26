/**
 * Ownership: shell chrome labels that the HTML design surface mutates
 * (permission picker markup, model picker) and that TypeScript composer bridges read.
 *
 * Permission picker interaction and settings persistence live in
 * `permission-picker.ts`. This module is the stable TypeScript boundary for
 * reading those labels without duplicating DOM ids.
 */

const DEFAULT_PERMISSION_LABEL = "请求批准";
const DEFAULT_MODEL_LABEL = "未选择模型";

/** Current composer permission label from the shell chrome. */
export function getComposerPermissionLabel(): string {
  return (
    document.getElementById("permissionLabel")?.textContent?.trim() ||
    DEFAULT_PERMISSION_LABEL
  );
}

/** Current model picker display name from the shell chrome. */
export function getComposerModelLabel(): string {
  return (
    document.getElementById("modelPickerName")?.textContent?.trim() ||
    DEFAULT_MODEL_LABEL
  );
}
