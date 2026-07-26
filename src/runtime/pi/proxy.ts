import type { PiProxyRequest } from "./types";

export type PiInvoke = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

const PROXY_TOKEN_HEADER = "x-novavei-proxy-token";
const PROXY_REQUEST_ID_HEADER = "x-novavei-proxy-request-id";
const UPSTREAM_ORIGIN_HEADER = "x-novavei-upstream-origin";
const USE_SYSTEM_PROXY_HEADER = "x-novavei-use-system-proxy";
const UPSTREAM_USER_AGENT_HEADER = "x-novavei-upstream-user-agent";
const UPSTREAM_CONTENT_TYPE_HEADER = "x-novavei-upstream-content-type";

type ProxyServerInfo = {
  baseUrl?: string;
  base_url?: string;
  token?: string;
};

function parseHttpUrl(raw: string, label: string): URL {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    // URL parser diagnostics can echo the configured endpoint. Keep provider
    // setup errors useful without turning the UI into a configuration leak.
    throw new Error(`${label} must be an absolute URL`);
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error(`${label} must use http:// or https://`);
  }
  if (url.username || url.password) {
    throw new Error(`${label} cannot contain embedded credentials`);
  }
  if (url.search || url.hash) {
    throw new Error(`${label} cannot contain query parameters or fragments`);
  }
  return url;
}

function localProxyBaseUrl(raw: string): URL {
  const url = parseHttpUrl(raw, "Local provider proxy URL");
  const hostname = url.hostname.toLowerCase().replace(/^\[|\]$/g, "");
  // The native server deliberately binds IPv4 loopback and the CSP permits
  // only that exact host. Rejecting aliases (including localhost and ::1) as
  // well as non-loopback hosts keeps a malformed IPC result from widening the
  // capability-bound proxy token's network destination.
  if (
    hostname !== "127.0.0.1" ||
    url.protocol !== "http:" ||
    url.pathname !== "/"
  )
    throw new Error("Local provider proxy is unavailable");
  return url;
}

function readHeader(
  headers: Record<string, string>,
  name: string,
): string | undefined {
  const wanted = name.toLowerCase();
  const entry = Object.entries(headers).find(
    ([key]) => key.toLowerCase() === wanted,
  );
  return entry?.[1];
}

function upstreamHeaderOverrides(
  headers: Record<string, string>,
): Record<string, string> {
  const userAgent = readHeader(headers, "user-agent");
  const contentType = readHeader(headers, "content-type");
  return {
    ...(userAgent ? { [UPSTREAM_USER_AGENT_HEADER]: userAgent } : {}),
    ...(contentType ? { [UPSTREAM_CONTENT_TYPE_HEADER]: contentType } : {}),
  };
}

export async function preparePiProxyRequest(
  invoke: PiInvoke,
  providerId: string,
  upstreamBaseUrl: string,
  headers: Record<string, string>,
  useSystemProxy: boolean,
  requestId: string | undefined,
  capabilityToken: string | undefined,
): Promise<PiProxyRequest> {
  const upstream = parseHttpUrl(upstreamBaseUrl.trim(), "Provider base URL");
  const activeRequestId = requestId?.trim();
  const activeCapabilityToken = capabilityToken?.trim();
  if (!activeRequestId || !activeCapabilityToken)
    throw new Error("Local provider proxy is unavailable");
  let info: ProxyServerInfo;
  try {
    info = await invoke<ProxyServerInfo>("proxy_transport_info", {
      requestId: activeRequestId,
      capabilityToken: activeCapabilityToken,
    });
  } catch {
    // Tauri command errors can include platform-specific listener or client
    // diagnostics. Provider startup gets only this stable recovery message.
    throw new Error("Local provider proxy is unavailable");
  }
  const proxyBaseUrl = String(info.baseUrl ?? info.base_url ?? "")
    .trim()
    .replace(/\/+$/, "");
  const token = String(info.token ?? "").trim();
  if (!proxyBaseUrl || !token) {
    throw new Error("Local provider proxy is unavailable");
  }
  const localProxy = localProxyBaseUrl(proxyBaseUrl);

  const pathname = upstream.pathname.replace(/\/+$/, "");
  const route = providerId.trim();
  if (!route || !/^[A-Za-z0-9._-]{1,128}$/.test(route)) {
    throw new Error("Provider id is not valid for the local proxy route");
  }
  return {
    baseUrl: `${localProxy.origin}/proxy/${route}${pathname}`,
    upstreamBaseUrl: upstreamBaseUrl.trim(),
    headers: {
      ...headers,
      ...upstreamHeaderOverrides(headers),
      [UPSTREAM_ORIGIN_HEADER]: upstream.origin,
      [PROXY_TOKEN_HEADER]: token,
      // The native listener verifies this alongside the token and route, so
      // a token issued for one live run cannot be replayed as another run.
      [PROXY_REQUEST_ID_HEADER]: activeRequestId,
      ...(useSystemProxy ? { [USE_SYSTEM_PROXY_HEADER]: "1" } : {}),
    },
  };
}
