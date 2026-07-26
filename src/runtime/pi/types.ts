import type {
  Api,
  AssistantMessageEventStream,
  Context,
  Model,
  SimpleStreamOptions,
  ThinkingLevel,
} from "@earendil-works/pi-ai";
import type { PiRunHandle, PiRunInput } from "../types";

export type PiProviderType = "codex" | "claude_code" | "gemini";
export type PiProviderApi =
  | "openai-responses"
  | "openai-completions"
  | "anthropic-messages"
  | "google-generative-ai";

export type PiCustomHeader = { key: string; value: string };

export type PiProviderModel = {
  id: string;
  name?: string;
  label?: string;
  enabled?: boolean;
  contextWindow?: number;
  maxOutputToken?: number;
  cost?: unknown;
};

export type PiProviderRecord = {
  id: string;
  name?: string;
  enabled?: boolean;
  isDefault?: boolean;
  type?: PiProviderType | string;
  baseUrl?: string;
  apiKey?: string;
  customHeaders?: PiCustomHeader[];
  models?: PiProviderModel[];
  activeModels?: string[];
  defaultModel?: string;
  requestFormat?: "openai-responses" | "openai-completions";
  reasoning?: ThinkingLevel | "off";
  promptCachingEnabled?: boolean;
  useSystemProxy?: boolean;
};

export type PiProviderConfig = {
  id: string;
  type: PiProviderType;
  api: PiProviderApi;
  modelId: string;
  baseUrl: string;
  apiKey: string;
  customHeaders: PiCustomHeader[];
  reasoning: ThinkingLevel | "off";
  promptCachingEnabled: boolean;
  useSystemProxy: boolean;
  modelConfig?: PiProviderModel;
};

export type PiProxyRequest = {
  baseUrl: string;
  upstreamBaseUrl: string;
  headers: Record<string, string>;
};

export type PiStreamOptions = Omit<SimpleStreamOptions, "reasoning"> & {
  reasoning?: ThinkingLevel | "off";
  toolChoice?: "auto" | "any" | "none" | { type: "tool"; name: string };
  retryCount?: number;
};

export type PiStreamFactory = (
  model: Model<Api>,
  context: Context,
  options: PiStreamOptions,
) => AssistantMessageEventStream;

export type PiContextLoader = (
  input: PiRunInput,
  provider: PiProviderConfig,
) => Promise<Context | undefined> | Context | undefined;

export type PiNativeCancel = (handle: PiRunHandle) => Promise<void> | void;
