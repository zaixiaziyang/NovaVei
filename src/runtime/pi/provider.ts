import { getBuiltinModel } from "@earendil-works/pi-ai/providers/all";
import type { Api, Model } from "@earendil-works/pi-ai";
import type { PiReasoningLevel, PiRunInput } from "../types";
import type {
  PiCustomHeader,
  PiProviderApi,
  PiProviderConfig,
  PiProviderModel,
  PiProviderRecord,
  PiProviderType,
} from "./types";

type UnknownRecord = Record<string, unknown>;

const MAX_PROVIDER_MODEL_ID_BYTES = 256;

function record(value: unknown): UnknownRecord | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as UnknownRecord)
    : undefined;
}

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function hasOwn(value: UnknownRecord, key: string): boolean {
  return Object.hasOwn(value, key);
}

function boundedModelId(value: unknown): string | undefined {
  const modelId = stringValue(value);
  return modelId &&
    new TextEncoder().encode(modelId).byteLength <=
      MAX_PROVIDER_MODEL_ID_BYTES &&
    !/[\u0000-\u001F\u007F]/.test(modelId)
    ? modelId
    : undefined;
}

function numberValue(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value > 0
    ? value
    : undefined;
}

function reasoningValue(value: unknown): PiReasoningLevel | undefined {
  const reasoning = stringValue(value)?.toLowerCase();
  return reasoning &&
    ["off", "minimal", "low", "medium", "high", "xhigh", "max"].includes(
      reasoning,
    )
    ? (reasoning as PiReasoningLevel)
    : undefined;
}

function providerType(
  value: unknown,
  id: string,
  ...hints: unknown[]
): PiProviderType {
  const explicit = stringValue(value)?.toLowerCase();
  if (
    explicit === "codex" ||
    explicit === "claude_code" ||
    explicit === "gemini"
  ) {
    return explicit;
  }
  const normalized = [explicit, ...hints.map(stringValue), id]
    .filter((item): item is string => Boolean(item))
    .join(" ")
    .toLowerCase();
  if (normalized.includes("claude") || normalized.includes("anthropic"))
    return "claude_code";
  if (normalized.includes("gemini") || normalized.includes("google"))
    return "gemini";
  return "codex";
}

function requestFormat(
  value: UnknownRecord,
): PiProviderRecord["requestFormat"] {
  const configured = [
    value.requestFormat,
    value.request_format,
    value.protocol,
    value.api,
  ]
    .map(stringValue)
    .find((item): item is string => Boolean(item))
    ?.toLowerCase();
  if (
    configured === "openai-completions" ||
    configured === "openai-responses"
  ) {
    return configured;
  }
  return undefined;
}

function normalizeHeaders(value: unknown): PiCustomHeader[] {
  if (Array.isArray(value)) {
    return value.flatMap((item) => {
      const header = record(item);
      const key = stringValue(header?.key);
      if (!key) return [];
      return [
        { key, value: typeof header?.value === "string" ? header.value : "" },
      ];
    });
  }
  const objectValue = record(value);
  if (!objectValue) return [];
  return Object.entries(objectValue).flatMap(([key, raw]) => {
    if (!key.trim() || typeof raw !== "string") return [];
    return [{ key: key.trim(), value: raw }];
  });
}

function modelIdentifier(model: UnknownRecord | undefined): unknown {
  if (!model) return undefined;
  for (const key of ["id", "modelId", "model_id", "name"]) {
    // A present malformed primary key must not fall through to an alias. This
    // matches the native resolver's JSON lookup order.
    if (hasOwn(model, key)) return model[key];
  }
  return undefined;
}

function normalizeModels(value: unknown): PiProviderModel[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((item) => {
    const stringId = boundedModelId(item);
    if (typeof item === "string" && stringId) return [{ id: stringId }];
    const model = record(item);
    const id = boundedModelId(modelIdentifier(model));
    if (!id) return [];
    return [
      {
        id,
        name: stringValue(model?.name),
        label: stringValue(model?.label),
        enabled: model?.enabled === false ? false : undefined,
        contextWindow: numberValue(
          model?.contextWindow ?? model?.context_window,
        ),
        maxOutputToken: numberValue(
          model?.maxOutputToken ?? model?.max_output_token,
        ),
        cost: model?.cost,
      },
    ];
  });
}

function normalizeActiveModels(input: UnknownRecord): string[] | undefined {
  const source = hasOwn(input, "activeModels")
    ? input.activeModels
    : hasOwn(input, "active_models")
      ? input.active_models
      : undefined;
  if (
    source === undefined &&
    !hasOwn(input, "activeModels") &&
    !hasOwn(input, "active_models")
  )
    return undefined;
  if (!Array.isArray(source)) return [];
  return source.flatMap((item) => {
    const id = boundedModelId(item);
    return id ? [id] : [];
  });
}

function defaultModel(input: UnknownRecord): string | undefined {
  return [
    input.defaultModel,
    input.default_model,
    input.modelId,
    input.model_id,
    input.model,
  ]
    .map(boundedModelId)
    .find((value): value is string => Boolean(value));
}

function defaultProvider(input: UnknownRecord): boolean {
  return hasOwn(input, "default")
    ? input.default === true
    : input.isDefault === true;
}

export function normalizeProviderRecord(
  value: unknown,
  fallbackId?: string,
): PiProviderRecord | undefined {
  const input = record(value);
  if (!input) return undefined;
  const id =
    stringValue(input.id ?? input.providerId ?? input.provider_id) ??
    fallbackId;
  if (!id) return undefined;
  const protocol = [
    input.protocol,
    input.api,
    input.requestFormat,
    input.request_format,
  ]
    .map(stringValue)
    .find((item): item is string => Boolean(item));
  const type = providerType(input.type, id, protocol);
  const reasoning = reasoningValue(input.reasoning);
  const hasModels = hasOwn(input, "models");
  return {
    id,
    name: stringValue(input.name),
    enabled: input.enabled !== false,
    isDefault: defaultProvider(input),
    type,
    baseUrl: stringValue(
      input.baseUrl ?? input.base_url ?? input.endpoint ?? input.url,
    ),
    apiKey: stringValue(
      input.apiKey ?? input.api_key ?? input.key ?? input.token,
    ),
    customHeaders: normalizeHeaders(input.customHeaders ?? input.headers),
    // An existing but malformed catalogue must not become an implicit allow-all.
    // `undefined` is reserved for the legacy no-catalogue shape.
    models: hasModels ? normalizeModels(input.models) : undefined,
    activeModels: normalizeActiveModels(input),
    defaultModel: defaultModel(input),
    requestFormat: requestFormat(input),
    reasoning: reasoning ?? "medium",
    promptCachingEnabled: input.promptCachingEnabled !== false,
    useSystemProxy: input.useSystemProxy === true,
  };
}

function providerList(value: unknown): PiProviderRecord[] {
  if (Array.isArray(value)) {
    return value.flatMap((item) => {
      const normalized = normalizeProviderRecord(item);
      return normalized ? [normalized] : [];
    });
  }
  const object = record(value);
  if (!object) return [];
  const nested = object.providers ?? object.items ?? object.customProviders;
  if (nested !== undefined) return providerList(nested);
  return Object.entries(object).flatMap(([id, item]) => {
    const normalized = normalizeProviderRecord(item, id);
    return normalized ? [normalized] : [];
  });
}

function modelIsEnabled(
  recordValue: PiProviderRecord,
  modelId: string,
): boolean {
  const id = boundedModelId(modelId);
  if (!id) return false;
  if (
    recordValue.activeModels !== undefined &&
    !recordValue.activeModels.includes(id)
  )
    return false;
  if (recordValue.models !== undefined) {
    const configured = recordValue.models.find((model) => model.id === id);
    return configured?.enabled !== false && configured !== undefined;
  }
  return recordValue.defaultModel === id;
}

function modelResult(
  recordValue: PiProviderRecord,
  id: string,
): {
  id: string;
  modelConfig?: PiProviderModel;
} {
  const modelConfig = recordValue.models?.find((model) => model.id === id);
  return { id, modelConfig };
}

function resolveDefaultModel(recordValue: PiProviderRecord):
  | {
      id: string;
      modelConfig?: PiProviderModel;
    }
  | undefined {
  if (
    recordValue.defaultModel &&
    modelIsEnabled(recordValue, recordValue.defaultModel)
  ) {
    return modelResult(recordValue, recordValue.defaultModel);
  }
  const first = recordValue.models?.find((model) =>
    modelIsEnabled(recordValue, model.id),
  );
  if (first) {
    return { id: first.id, modelConfig: first };
  }
  return undefined;
}

function resolveApi(
  recordValue: PiProviderRecord,
  baseUrl: string,
): PiProviderApi {
  if (recordValue.type === "claude_code") return "anthropic-messages";
  if (recordValue.type === "gemini") return "google-generative-ai";
  const lower = baseUrl.toLowerCase().replace(/\/+$/, "");
  if (recordValue.requestFormat) return recordValue.requestFormat;
  if (lower.endsWith("/chat/completions")) return "openai-completions";
  if (lower.endsWith("/responses") || lower.endsWith("/response"))
    return "openai-responses";
  return "openai-responses";
}

function defaultBaseUrl(type: PiProviderRecord["type"]): string {
  switch (type) {
    case "claude_code":
      return "https://api.anthropic.com";
    case "gemini":
      return "https://generativelanguage.googleapis.com/v1beta";
    default:
      return "https://api.openai.com/v1";
  }
}

export function resolveProvider(
  settings: unknown,
  input: PiRunInput,
): PiProviderConfig {
  const root = record(settings);
  const providers = providerList(root?.providers ?? root?.provider ?? settings);
  const requestedId = input.providerId?.trim();
  const requestedModel = boundedModelId(input.model);
  const requestedModelWasProvided = Boolean(input.model?.trim());
  let selected: PiProviderRecord | undefined;
  if (requestedId) {
    selected = providers.find((item) => item.id === requestedId);
    if (!selected) throw new Error(`Provider ${requestedId} is not configured`);
  } else {
    selected =
      (requestedModel &&
        providers.find(
          (item) =>
            item.enabled !== false && modelIsEnabled(item, requestedModel),
        )) ||
      providers.find(
        (item) =>
          item.enabled !== false &&
          item.isDefault === true &&
          resolveDefaultModel(item) !== undefined,
      ) ||
      providers.find(
        (item) =>
          item.enabled !== false && resolveDefaultModel(item) !== undefined,
      );
  }
  if (!selected) throw new Error("No provider is configured");
  if (selected.enabled === false)
    throw new Error(`Provider ${selected.id} is disabled`);

  const type = providerType(selected.type, selected.id, selected.requestFormat);
  const baseUrl = selected.baseUrl?.trim() || defaultBaseUrl(type);
  const model = requestedModel
    ? modelIsEnabled(selected, requestedModel)
      ? modelResult(selected, requestedModel)
      : undefined
    : undefined;
  if (requestedModelWasProvided && !model) {
    throw new Error(`Provider ${selected.id} model is disabled or unavailable`);
  }
  const selectedModel = model ?? resolveDefaultModel(selected);
  if (!selectedModel)
    throw new Error(`Provider ${selected.id} has no enabled model configured`);
  return {
    id: selected.id,
    type,
    api: resolveApi(selected, baseUrl),
    modelId: selectedModel.id,
    baseUrl,
    apiKey: selected.apiKey ?? "",
    customHeaders: selected.customHeaders ?? [],
    reasoning:
      reasoningValue(input.reasoning) ?? selected.reasoning ?? "medium",
    promptCachingEnabled: selected.promptCachingEnabled !== false,
    useSystemProxy: selected.useSystemProxy === true,
    modelConfig: selectedModel.modelConfig,
  };
}

function stripEndpointSuffix(baseUrl: string): string {
  return baseUrl.replace(
    /\/(?:chat\/completions|responses?|generateContent|streamGenerateContent)$/i,
    "",
  );
}

function normalizeModelBaseUrl(baseUrl: string, api: PiProviderApi): string {
  const stripped = stripEndpointSuffix(baseUrl).replace(/\/+$/, "");
  if (api === "openai-responses" || api === "openai-completions") {
    try {
      const url = new URL(stripped);
      if (!/\/v\d+(?:beta)?$/i.test(url.pathname))
        url.pathname = `${url.pathname}/v1`;
      return url.toString().replace(/\/+$/, "");
    } catch {
      return stripped;
    }
  }
  if (api === "google-generative-ai") {
    try {
      const url = new URL(stripped);
      if (!/\/v\d+(?:beta)?$/i.test(url.pathname))
        url.pathname = `${url.pathname}/v1beta`;
      return url.toString().replace(/\/+$/, "");
    } catch {
      return stripped;
    }
  }
  return stripped;
}

function builtinModel(
  api: PiProviderApi,
  modelId: string,
  baseUrl: string,
): Model<Api> | undefined {
  const provider =
    api === "anthropic-messages"
      ? "anthropic"
      : api === "google-generative-ai"
        ? "google"
        : "openai";
  try {
    const model = getBuiltinModel(
      provider as never,
      modelId as never,
    ) as Model<Api>;
    return model?.api ? { ...model, baseUrl } : undefined;
  } catch {
    return undefined;
  }
}

export function createPiModel(
  config: PiProviderConfig,
  proxyBaseUrl: string,
  upstreamBaseUrl: string,
): Model<Api> {
  const baseUrl = normalizeModelBaseUrl(proxyBaseUrl, config.api);
  const known = builtinModel(config.api, config.modelId, baseUrl);
  const modelConfig = config.modelConfig;
  const contextWindow =
    modelConfig?.contextWindow ?? known?.contextWindow ?? 256_000;
  const maxTokens = modelConfig?.maxOutputToken ?? known?.maxTokens ?? 8192;
  const cost =
    modelConfig?.cost && typeof modelConfig.cost === "object"
      ? (modelConfig.cost as Model<Api>["cost"])
      : (known?.cost ?? { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 });

  if (known && known.api === config.api) {
    return {
      ...known,
      id: config.modelId,
      name: config.modelId,
      baseUrl,
      contextWindow,
      maxTokens,
      cost,
      ...(config.api === "openai-responses"
        ? {
            compat: {
              ...(known.compat ?? {}),
              supportsDeveloperRole: false,
              sessionAffinityFormat: "openai-nosession",
            },
          }
        : {}),
    } as Model<Api>;
  }

  const custom: Model<Api> = {
    id: config.modelId,
    name: config.modelId,
    api: config.api,
    provider: config.id,
    baseUrl,
    reasoning: true,
    input: config.api === "anthropic-messages" ? ["text"] : ["text", "image"],
    cost,
    contextWindow,
    maxTokens,
  };

  if (config.api === "openai-responses") {
    // Local loopback proxy already scopes the hop; do not emit pi-ai's default
    // session_id header (it is non-standard and trips browser CORS preflight).
    custom.compat = {
      supportsDeveloperRole: false,
      sessionAffinityFormat: "openai-nosession",
    };
  } else if (config.api === "openai-completions") {
    const upstream = upstreamBaseUrl.toLowerCase();
    custom.compat = {
      supportsStore: false,
      supportsDeveloperRole: false,
      ...(upstream.includes("z.ai")
        ? { supportsReasoningEffort: false, thinkingFormat: "zai" }
        : {}),
      ...(upstream.includes("openrouter.ai")
        ? { thinkingFormat: "openrouter" }
        : {}),
      ...(upstream.includes("chutes.ai")
        ? { maxTokensField: "max_tokens" }
        : {}),
    };
  }
  return custom;
}

export function buildProviderHeaders(
  config: PiProviderConfig,
  sessionId: string | undefined,
): Record<string, string> {
  const headers: Record<string, string> = {};
  if (config.type === "gemini") {
    if (config.apiKey) headers["x-goog-api-key"] = config.apiKey;
  } else if (config.type === "claude_code") {
    if (config.apiKey) {
      headers["x-api-key"] = config.apiKey;
      headers["anthropic-version"] = "2023-06-01";
    }
  } else if (config.apiKey) {
    headers.Authorization = `Bearer ${config.apiKey}`;
    headers["User-Agent"] = "NovaVei/0.1";
    if (sessionId) {
      headers["x-session-id"] = sessionId;
      headers["x-client-request-id"] = sessionId;
    }
  }
  for (const header of config.customHeaders) {
    const key = header.key.trim();
    if (key && header.value.trim()) headers[key] = header.value;
  }
  return headers;
}
