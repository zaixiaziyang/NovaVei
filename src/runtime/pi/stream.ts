import type { AnthropicOptions } from "@earendil-works/pi-ai/api/anthropic-messages";
import type { GoogleOptions } from "@earendil-works/pi-ai/api/google-generative-ai";
import type { GoogleThinkingLevel } from "@earendil-works/pi-ai/api/google-shared";
import type { OpenAICompletionsOptions } from "@earendil-works/pi-ai/api/openai-completions";
import type { OpenAIResponsesOptions } from "@earendil-works/pi-ai/api/openai-responses";
import {
  createAssistantMessageEventStream,
  type Api,
  type AssistantMessage,
  type AssistantMessageEvent,
  type AssistantMessageEventStream,
  type Context,
  type Model,
} from "@earendil-works/pi-ai";
import type { PiProviderApi, PiStreamOptions } from "./types";

function thinkingBudget(
  level: PiStreamOptions["reasoning"],
): number | undefined {
  switch (level) {
    case "minimal":
      return 1024;
    case "low":
      return 2048;
    case "medium":
      return 4096;
    case "high":
      return 8192;
    case "xhigh":
      return 12288;
    case "max":
      return 16384;
    default:
      return undefined;
  }
}

type ActiveReasoning = Exclude<PiStreamOptions["reasoning"], "off" | undefined>;

function reasoningLevel(
  level: PiStreamOptions["reasoning"],
): ActiveReasoning | undefined {
  return level === "off" ? undefined : level;
}

function anthropicEffort(
  level: ActiveReasoning | undefined,
): AnthropicOptions["effort"] {
  if (!level) return undefined;
  return level === "minimal" ? "low" : level;
}

function googleThinkingLevel(
  level: PiStreamOptions["reasoning"],
): GoogleThinkingLevel | undefined {
  switch (level) {
    case "minimal":
      return "MINIMAL";
    case "low":
      return "LOW";
    case "medium":
      return "MEDIUM";
    case "high":
    case "xhigh":
    case "max":
      return "HIGH";
    default:
      return undefined;
  }
}

function baseOptions(model: Model<Api>, options: PiStreamOptions) {
  return {
    temperature: options.temperature,
    maxTokens: options.maxTokens ?? model.maxTokens,
    signal: options.signal,
    apiKey: options.apiKey,
    cacheRetention: options.cacheRetention,
    sessionId: options.sessionId,
    headers: options.headers,
    onPayload: options.onPayload,
    onResponse: options.onResponse,
    maxRetryDelayMs: options.maxRetryDelayMs,
    metadata: options.metadata,
  };
}

function openAiToolChoice(
  choice: PiStreamOptions["toolChoice"],
): OpenAICompletionsOptions["toolChoice"] | undefined {
  if (!choice) return undefined;
  if (choice === "any") return "required";
  if (choice === "auto" || choice === "none") return choice;
  return { type: "function", function: { name: choice.name } };
}

function anthropicToolChoice(
  choice: PiStreamOptions["toolChoice"],
): AnthropicOptions["toolChoice"] | undefined {
  if (!choice) return undefined;
  if (choice === "any") return "any";
  if (choice === "auto" || choice === "none") return choice;
  return { type: "tool", name: choice.name };
}

function googleToolChoice(
  choice: PiStreamOptions["toolChoice"],
): GoogleOptions["toolChoice"] | undefined {
  if (!choice) return undefined;
  if (choice === "any" || choice === "auto" || choice === "none") return choice;
  return "auto";
}

function formatStreamError(error: unknown): string {
  if (!(error instanceof Error)) return String(error);
  const parts = [
    error.message.trim() || error.name || "Provider request failed",
  ];
  // OpenAI APIConnectionError keeps the underlying fetch failure on `cause`
  // (often "Failed to fetch" / CORS). Surface it so the UI is not only
  // "Connection error."
  let nested: unknown = (error as Error & { cause?: unknown }).cause;
  for (let depth = 0; nested && depth < 3; depth += 1) {
    if (nested instanceof Error) {
      const text = nested.message.trim();
      if (text && !parts.some((part) => part.includes(text))) parts.push(text);
      nested = (nested as Error & { cause?: unknown }).cause;
      continue;
    }
    const text = String(nested).trim();
    if (
      text &&
      text !== "undefined" &&
      !parts.some((part) => part.includes(text))
    )
      parts.push(text);
    break;
  }
  return parts.join(" · ");
}

function errorMessage(
  model: Model<Api>,
  error: unknown,
  aborted = false,
): AssistantMessage {
  const message = formatStreamError(error);
  return {
    role: "assistant",
    content: [],
    api: model.api,
    provider: model.provider,
    model: model.id,
    usage: {
      input: 0,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      totalTokens: 0,
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
    },
    stopReason: aborted ? "aborted" : "error",
    errorMessage:
      message || (aborted ? "Cancelled" : "Provider request failed"),
    timestamp: Date.now(),
  };
}

function isVisibleEvent(event: AssistantMessageEvent): boolean {
  return (
    event.type === "text_delta" ||
    event.type === "text_end" ||
    event.type === "thinking_delta" ||
    event.type === "thinking_end" ||
    event.type === "toolcall_start" ||
    event.type === "toolcall_delta" ||
    event.type === "toolcall_end"
  );
}

function waitForRetryBackoff(
  milliseconds: number,
  signal: AbortSignal | undefined,
): Promise<void> {
  if (!signal)
    return new Promise((resolve) => setTimeout(resolve, milliseconds));
  if (signal.aborted) return Promise.resolve();

  return new Promise((resolve) => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const finish = () => {
      if (timer !== undefined) clearTimeout(timer);
      signal.removeEventListener("abort", finish);
      resolve();
    };
    timer = setTimeout(finish, milliseconds);
    signal.addEventListener("abort", finish, { once: true });
  });
}

/**
 * Retry only before any visible output is committed. This matches the product
 * stream retry boundary and keeps a failed partial tool call from being replayed.
 */
function withStreamRetry(
  model: Model<Api>,
  factory: () =>
    | AssistantMessageEventStream
    | Promise<AssistantMessageEventStream>,
  signal: AbortSignal | undefined,
  retryCount: number,
): AssistantMessageEventStream {
  const output = createAssistantMessageEventStream();
  void (async () => {
    let attempt = 0;
    while (true) {
      if (signal?.aborted) {
        output.push({
          type: "error",
          reason: "aborted",
          error: errorMessage(model, new Error("Cancelled"), true),
        });
        return;
      }
      const buffered: AssistantMessageEvent[] = [];
      let visible = false;
      let retry = false;
      try {
        const source = await factory();
        for await (const event of source) {
          if (
            event.type === "error" &&
            !visible &&
            attempt < retryCount &&
            !signal?.aborted
          ) {
            retry = true;
            break;
          }
          buffered.push(event);
          if (isVisibleEvent(event)) visible = true;
          if (event.type === "done" || event.type === "error") {
            for (const item of buffered) output.push(item);
            return;
          }
          if (visible) {
            while (buffered.length > 0)
              output.push(buffered.shift() as AssistantMessageEvent);
          }
        }
        if (!retry) {
          if (signal?.aborted) {
            output.push({
              type: "error",
              reason: "aborted",
              error: errorMessage(model, new Error("Cancelled"), true),
            });
          } else {
            output.push({
              type: "error",
              reason: "error",
              error: errorMessage(
                model,
                new Error("Provider stream ended unexpectedly"),
              ),
            });
          }
          return;
        }
      } catch (error) {
        if (signal?.aborted) {
          output.push({
            type: "error",
            reason: "aborted",
            error: errorMessage(model, error, true),
          });
          return;
        }
        if (visible || attempt >= retryCount) {
          output.push({
            type: "error",
            reason: "error",
            error: errorMessage(model, error),
          });
          return;
        }
        retry = true;
      }
      if (!retry) return;
      attempt += 1;
      await waitForRetryBackoff(Math.min(250 * attempt, 1000), signal);
    }
  })();
  return output;
}

async function createSource(
  api: PiProviderApi,
  model: Model<Api>,
  context: Context,
  options: PiStreamOptions,
): Promise<AssistantMessageEventStream> {
  const base = baseOptions(model, options);
  const level = reasoningLevel(options.reasoning);
  const hasTools = Boolean(context.tools?.length);
  switch (api) {
    case "anthropic-messages": {
      const { stream } = await import(
        "@earendil-works/pi-ai/api/anthropic-messages"
      );
      if (options.signal?.aborted) throw new Error("Cancelled");
      return stream(model as never, context, {
        ...base,
        thinkingEnabled: Boolean(level),
        thinkingBudgetTokens: thinkingBudget(level),
        effort: anthropicEffort(level),
        toolChoice: anthropicToolChoice(
          options.toolChoice ?? (hasTools ? "auto" : "none"),
        ),
      });
    }
    case "openai-completions": {
      const { stream } = await import(
        "@earendil-works/pi-ai/api/openai-completions"
      );
      if (options.signal?.aborted) throw new Error("Cancelled");
      return stream(model as never, context, {
        ...base,
        reasoningEffort: level,
        toolChoice: hasTools
          ? openAiToolChoice(options.toolChoice ?? "auto")
          : undefined,
      });
    }
    case "openai-responses": {
      const { stream } = await import(
        "@earendil-works/pi-ai/api/openai-responses"
      );
      if (options.signal?.aborted) throw new Error("Cancelled");
      return stream(model as never, context, {
        ...base,
        reasoningEffort: level,
      } satisfies OpenAIResponsesOptions);
    }
    case "google-generative-ai": {
      const { stream } = await import(
        "@earendil-works/pi-ai/api/google-generative-ai"
      );
      if (options.signal?.aborted) throw new Error("Cancelled");
      return stream(model as never, context, {
        ...base,
        thinking: {
          enabled: Boolean(level),
          level: googleThinkingLevel(level),
        },
        toolChoice: googleToolChoice(
          options.toolChoice ?? (hasTools ? "auto" : "none"),
        ),
      } as GoogleOptions);
    }
    default:
      throw new Error(`Unsupported Pi provider API: ${api}`);
  }
}

export function streamByApi(
  api: PiProviderApi,
  model: Model<Api>,
  context: Context,
  options: PiStreamOptions,
): AssistantMessageEventStream {
  return withStreamRetry(
    model,
    () => createSource(api, model, context, options),
    options.signal,
    options.retryCount ?? 2,
  );
}
