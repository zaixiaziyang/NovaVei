/**
 * Context Checkpoint card.
 *
 * When a run performs context compaction, the conversation silently replaces
 * earlier turns with a summary. Instead of truncating without a trace, this
 * inserts a small collapsible card into the assistant message showing what
 * was covered (message count) and which model produced the compaction, so the
 * reader can tell that older context was folded rather than lost.
 */

const CARD_DATA = "context-checkpoint";

function isEnglish() {
  return document.documentElement.lang.toLowerCase().startsWith("en");
}

function text(zh: string, en: string) {
  return isEnglish() ? en : zh;
}

type CheckpointInfo = {
  coveredMessages: number;
  model?: string;
  generatedAt?: number;
};

/**
 * Normalize a `contextTrim` projection into the small set of fields the card
 * needs. Missing or malformed values fall back to conservative labels rather
 * than inventing numbers.
 */
export function contextualCheckpointFrom(
  compaction: unknown,
  fallbackModel?: string,
  fallbackCovered = 0,
): CheckpointInfo | undefined {
  const record =
    compaction !== null &&
    typeof compaction === "object" &&
    !Array.isArray(compaction)
      ? (compaction as Record<string, unknown>)
      : undefined;
  if (!record) return undefined;
  const start = finiteNumber(record.sourceMessageStart);
  const end = finiteNumber(record.sourceMessageEnd);
  const covered =
    start !== undefined && end !== undefined && end >= start
      ? end - start + 1
      : fallbackCovered;
  const generatedAt = finiteNumber(record.generatedAt);
  const model =
    typeof record.model === "string" && record.model.trim()
      ? record.model.trim()
      : typeof fallbackModel === "string" && fallbackModel.trim()
        ? fallbackModel.trim()
        : undefined;
  if (covered <= 0 && !model) return undefined;
  return {
    coveredMessages: Math.max(0, covered),
    ...(model ? { model } : {}),
    ...(generatedAt && generatedAt > 0 ? { generatedAt } : {}),
  };
}

function finiteNumber(value: unknown): number | undefined {
  const numeric = typeof value === "number" ? value : Number(value);
  return Number.isFinite(numeric) ? numeric : undefined;
}

/**
 * Render (or refresh) a checkpoint card inside an assistant message. The card
 * is placed before the Markdown body (`[data-pi-text]` or `[data-history-content]`).
 * Idempotent per summary: a second call with the same covered count +
 * model does not duplicate the card.
 */
export function renderContextCheckpoint(
  article: HTMLElement,
  info: CheckpointInfo | undefined,
): void {
  const existing = article.querySelector<HTMLElement>(`[data-${CARD_DATA}]`);
  const body =
    article.querySelector<HTMLElement>("[data-pi-text]") ??
    article.querySelector<HTMLElement>("[data-history-content]");
  if (!info) {
    existing?.remove();
    return;
  }
  if (existing) {
    const current = existing.dataset.checkpointFingerprint;
    const nextFingerprint = fingerprint(info);
    if (current === nextFingerprint) return;
    existing.remove();
  }
  if (!body) return;

  const card = document.createElement("details");
  card.className = "context-checkpoint";
  card.dataset[CARD_DATA] = "true";
  card.dataset.checkpointFingerprint = fingerprint(info);

  const summary = document.createElement("summary");
  summary.className = "context-checkpoint-summary";
  const label = document.createElement("span");
  label.textContent = checkpointTitle(info);
  summary.appendChild(label);
  card.appendChild(summary);

  const bodyText = document.createElement("div");
  bodyText.className = "context-checkpoint-body";
  const line = document.createElement("p");
  line.textContent = checkpointDescription(info);
  bodyText.appendChild(line);
  card.appendChild(bodyText);

  body.parentNode?.insertBefore(card, body);
}

function checkpointTitle(info: CheckpointInfo) {
  const covered = info.coveredMessages > 0 ? `${info.coveredMessages} 条` : "";
  const model = info.model ? ` · ${info.model}` : "";
  if (covered || model) return `上下文检查点${covered}${model}`;
  return text("上下文检查点", "Context checkpoint");
}

function checkpointDescription(info: CheckpointInfo) {
  const parts = [`覆盖 ${info.coveredMessages} 条消息`];
  if (info.model) parts.push(`生成模型 ${info.model}`);
  if (info.generatedAt) {
    try {
      parts.push(new Date(info.generatedAt).toLocaleString());
    } catch {
      /* ignore timestamp */
    }
  }
  return text(
    `上下文在此已被压缩为摘要，未静默丢弃。${parts.join("，")}。`,
    `Context was compacted here; older turns were folded, not silently dropped. ${parts.join(", ")}.`,
  );
}

function fingerprint(info: CheckpointInfo) {
  return `${info.coveredMessages}:${info.model ?? ""}:${info.generatedAt ?? 0}`;
}
