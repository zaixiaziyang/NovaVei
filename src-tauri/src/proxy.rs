//! Token-authenticated loopback proxy for Pi provider requests.
//!
//! The WebView only receives the proxy URL and an ephemeral token.  Provider
//! credentials can be supplied by a native resolver at request time, keeping
//! them out of the renderer once the caller wires the resolver to AppState.
//! The proxy reconstructs the target from the full native-configured provider
//! base URL. Arbitrary absolute URLs and renderer-selected gateway paths are
//! never accepted.

use crate::diagnostics;

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::{to_bytes, Body};
use axum::extract::{OriginalUri, Path, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::any;
use axum::Router;
use reqwest::Url;
use serde::Serialize;
use tokio::net::TcpListener;
use uuid::Uuid;

const ACCESS_CONTROL_REQUEST_HEADERS: &str = "access-control-request-headers";
const ACCESS_CONTROL_REQUEST_METHOD: &str = "access-control-request-method";
const ACCESS_CONTROL_PREFIX: &str = "access-control-";
const CONTENT_LENGTH: &str = "content-length";
const CONTENT_TYPE: &str = "content-type";
const CONNECTION: &str = "connection";
const HOST: &str = "host";
const KEEP_ALIVE: &str = "keep-alive";
const ORIGIN: &str = "origin";
const PROXY_AUTHENTICATE: &str = "proxy-authenticate";
const PROXY_AUTHORIZATION: &str = "proxy-authorization";
const PROXY_CONNECTION: &str = "proxy-connection";
const TE: &str = "te";
const TRAILER: &str = "trailer";
const TRANSFER_ENCODING: &str = "transfer-encoding";
const UPGRADE: &str = "upgrade";
const PROXY_PREFIX: &str = "x-novavei-";
const PROXY_TOKEN_HEADER: &str = "x-novavei-proxy-token";
const PROXY_REQUEST_ID_HEADER: &str = "x-novavei-proxy-request-id";
#[cfg(test)]
const UPSTREAM_ORIGIN_HEADER: &str = "x-novavei-upstream-origin";
const UPSTREAM_USER_AGENT_HEADER: &str = "x-novavei-upstream-user-agent";
const UPSTREAM_CONTENT_TYPE_HEADER: &str = "x-novavei-upstream-content-type";
// Fixed allowlist (not request reflection). Must cover browser SDK preflight
// headers such as OpenAI/Anthropic `X-Stainless-*`; missing entries surface in
// the WebView as opaque "Connection error." because CORS blocks the fetch.
const DEFAULT_ALLOW_HEADERS: &str = "authorization,content-type,x-api-key,x-goog-api-key,x-goog-api-client,anthropic-version,anthropic-beta,openai-beta,x-app,x-client-request-id,x-session-affinity,session_id,session-id,x-should-retry,x-stainless-retry-count,x-stainless-timeout,x-stainless-lang,x-stainless-package-version,x-stainless-os,x-stainless-arch,x-stainless-runtime,x-stainless-runtime-version,x-stainless-helper,x-stainless-helper-method,x-novavei-upstream-origin,x-novavei-upstream-user-agent,x-novavei-upstream-content-type,x-novavei-proxy-token,x-novavei-proxy-request-id,x-novavei-use-system-proxy";
const ALLOW_METHODS_VALUE: &str = "GET,POST,PUT,PATCH,DELETE,OPTIONS,HEAD";
const VARY_VALUE: &str = "Origin, Access-Control-Request-Method, Access-Control-Request-Headers";
// Tauri Windows uses HTTP unless `useHttpsScheme` is explicitly enabled.
const TAURI_WEBVIEW_ORIGIN: &str = "http://tauri.localhost";
const VITE_DEVELOPMENT_ORIGIN: &str = "http://localhost:1421";
const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10 * 60);
// A proxy token needs to survive one bounded provider request, but it must
// never become a process-lifetime bearer credential.  This leaves a small
// margin over the request timeout while keeping a leaked token short-lived.
const TRANSPORT_TOKEN_TTL: Duration = Duration::from_secs(15 * 60);
/// Stable native error for all callers that need a proxy transport but cannot
/// obtain one. It intentionally carries neither an OS/network failure nor a
/// loopback endpoint or bearer token.
pub const PROXY_UNAVAILABLE_ERROR: &str = "Local provider proxy is unavailable";
/// Stable renderer-facing error for an upstream transport failure. Reqwest's
/// error text can include a configured provider endpoint, proxy route, or
/// local network detail, so it must never be used as an HTTP response body.
const PROVIDER_REQUEST_FAILED_ERROR: &str = "Provider request failed";

#[derive(Clone, Debug, Serialize)]
pub struct ProxyServerInfo {
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    pub token: String,
}

/// One renderer-visible token is scoped to a native-authorized provider run.
/// Its cancellation flag is shared with the owning run or child grant, so a
/// terminal transition makes every previously issued token fail immediately.
#[derive(Clone)]
struct ProxyTransportToken {
    request_id: String,
    provider_id: String,
    expires_at: Instant,
    cancelled: Arc<AtomicBool>,
}

/// Native headers to inject for one provider request.
///
/// The resolver runs on the native side.  It should read the provider secret
/// from AppState/secure storage and return only the exact upstream headers
/// needed for that provider.  Header values never appear in the proxy info
/// command or in emitted events.
#[derive(Clone, Debug, Default)]
pub struct ProviderCredentials {
    pub headers: Vec<(String, String)>,
    /// Exact native-configured base URL, including a possible tenant path.
    /// Renderer headers never choose this value.
    pub upstream_base_url: String,
    /// Resolved by native settings; renderer proxy headers never select this.
    pub use_system_proxy: bool,
}

pub type CredentialResolver = Arc<dyn Fn(&str) -> Option<ProviderCredentials> + Send + Sync>;

pub struct ProxyServerState {
    base_url: String,
    direct_client: reqwest::Client,
    system_proxy_client: reqwest::Client,
    credential_resolver: Option<CredentialResolver>,
    transport_tokens: Mutex<HashMap<String, ProxyTransportToken>>,
    available: AtomicBool,
}

impl ProxyServerState {
    fn is_available(&self) -> bool {
        self.available.load(Ordering::Acquire)
    }

    fn issue_transport_info(
        &self,
        request_id: &str,
        provider_id: &str,
        authorization_expires_at: Instant,
        cancelled: Arc<AtomicBool>,
    ) -> Result<ProxyServerInfo, String> {
        let request_id = request_id.trim();
        let provider_id = provider_id.trim();
        let now = Instant::now();
        if request_id.is_empty()
            || request_id.len() > 256
            || !valid_provider_segment(provider_id)
            || authorization_expires_at <= now
            || cancelled.load(Ordering::Acquire)
        {
            return Err(PROXY_UNAVAILABLE_ERROR.to_string());
        }

        let expires_at = authorization_expires_at.min(now + TRANSPORT_TOKEN_TTL);
        let token = Uuid::new_v4().to_string();
        let mut tokens = self.transport_tokens.lock();
        // Keep the registry bounded even if a renderer repeatedly requests a
        // transport handshake, and permit at most one current token per run.
        tokens.retain(|_, grant| {
            grant.expires_at > now
                && !grant.cancelled.load(Ordering::Acquire)
                && grant.request_id != request_id
        });
        tokens.insert(
            token.clone(),
            ProxyTransportToken {
                request_id: request_id.to_string(),
                provider_id: provider_id.to_string(),
                expires_at,
                cancelled,
            },
        );
        Ok(ProxyServerInfo {
            base_url: self.base_url.clone(),
            token,
        })
    }

    fn authorizes_transport(&self, token: &str, request_id: &str, provider_id: &str) -> bool {
        let now = Instant::now();
        let mut tokens = self.transport_tokens.lock();
        let Some(grant) = tokens.get(token).cloned() else {
            return false;
        };
        if grant.expires_at <= now || grant.cancelled.load(Ordering::Acquire) {
            tokens.remove(token);
            return false;
        }
        grant.request_id == request_id
            && grant.provider_id == provider_id
            && valid_provider_segment(provider_id)
    }
}

/// The only renderer-visible representation of the provider proxy. In
/// particular, this DTO never contains the loopback URL, bearer token, an
/// operating-system error, network detail, or provider credential.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyAvailability {
    Ready,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyRuntimeStatus {
    pub status: ProxyAvailability,
    pub can_retry: bool,
}

type ProxyRuntimeStarter =
    Arc<dyn Fn(CredentialResolver) -> Result<Arc<ProxyServerState>, String> + Send + Sync>;

/// Owns the native credential resolver and the currently usable loopback
/// server. Startup and retries share one lock so a double-clicked retry cannot
/// create competing listeners or tokens.
pub struct ProxyRuntime {
    credential_resolver: CredentialResolver,
    starter: ProxyRuntimeStarter,
    server: Mutex<Option<Arc<ProxyServerState>>>,
}

impl ProxyRuntime {
    /// Return the redacted, stable state suitable for a WebView IPC response.
    pub fn status(&self) -> ProxyRuntimeStatus {
        let server = self.server.lock();
        proxy_runtime_status_for(
            server
                .as_deref()
                .is_some_and(ProxyServerState::is_available),
        )
    }

    /// Start the proxy if it is unavailable. The state lock remains held while
    /// binding and constructing the clients, making concurrent retry requests
    /// serial and idempotent.
    pub fn retry_start(&self) -> ProxyRuntimeStatus {
        self.start_if_unavailable("retry_failed")
    }

    /// Issue one short-lived, run- and provider-bound loopback token while the
    /// runtime is available. The stable error deliberately reveals neither
    /// listener details nor why a grant was rejected.
    ///
    /// The IPC wrapper is capability-gated and turns every startup failure
    /// into the stable unavailable error rather than exposing local details.
    pub fn issue_transport_info(
        &self,
        request_id: &str,
        provider_id: &str,
        authorization_expires_at: Instant,
        cancelled: Arc<AtomicBool>,
    ) -> Result<ProxyServerInfo, String> {
        self.server
            .lock()
            .as_deref()
            .filter(|server| server.is_available())
            .ok_or_else(|| PROXY_UNAVAILABLE_ERROR.to_string())?
            .issue_transport_info(request_id, provider_id, authorization_expires_at, cancelled)
    }

    fn start_if_unavailable(&self, failure_event: &'static str) -> ProxyRuntimeStatus {
        let mut server = self.server.lock();
        if server
            .as_deref()
            .is_some_and(ProxyServerState::is_available)
        {
            return proxy_runtime_status_for(true);
        }

        // Drop stale state from a listener that stopped after startup before
        // creating its replacement.
        *server = None;
        match (self.starter)(self.credential_resolver.clone()) {
            Ok(started) => {
                *server = Some(started);
                proxy_runtime_status_for(true)
            }
            Err(_) => {
                // The underlying error can include a bind path, local port, or
                // system proxy detail. It is intentionally neither retained
                // nor emitted to the renderer.
                diagnostics::record_event("proxy", failure_event, "failure", None);
                proxy_runtime_status_for(false)
            }
        }
    }
}

fn proxy_runtime_status_for(ready: bool) -> ProxyRuntimeStatus {
    if ready {
        ProxyRuntimeStatus {
            status: ProxyAvailability::Ready,
            can_retry: false,
        }
    } else {
        ProxyRuntimeStatus {
            status: ProxyAvailability::Unavailable,
            can_retry: true,
        }
    }
}

/// Create a proxy runtime without making a startup failure fatal to the
/// desktop shell. The returned runtime can later retry serially.
pub fn start_proxy_runtime(credential_resolver: CredentialResolver) -> Arc<ProxyRuntime> {
    start_proxy_runtime_with_starter(
        credential_resolver,
        Arc::new(|resolver| start_proxy_server_with_resolver(Some(resolver))),
    )
}

fn start_proxy_runtime_with_starter(
    credential_resolver: CredentialResolver,
    starter: ProxyRuntimeStarter,
) -> Arc<ProxyRuntime> {
    let runtime = Arc::new(ProxyRuntime {
        credential_resolver,
        starter,
        server: Mutex::new(None),
    });
    let _ = runtime.start_if_unavailable("startup_failed");
    runtime
}

#[derive(Clone, Debug, serde::Deserialize)]
struct ProxyRoutePath {
    provider: String,
}

/// Read the redacted availability state through IPC.
#[tauri::command]
pub fn proxy_runtime_status(state: tauri::State<'_, Arc<ProxyRuntime>>) -> ProxyRuntimeStatus {
    state.status()
}

/// Retry a previously unavailable proxy. The runtime serializes all attempts
/// and returns the same redacted DTO as proxy_runtime_status.
#[tauri::command]
pub fn proxy_runtime_retry(state: tauri::State<'_, Arc<ProxyRuntime>>) -> ProxyRuntimeStatus {
    state.retry_start()
}

/// Startup without a native resolver is intentionally rejected. Without the
/// resolver the proxy would have no authoritative provider origin or secret
/// source and could accidentally become a renderer-controlled forwarder.
#[cfg(test)]
pub fn start_proxy_server() -> Result<Arc<ProxyServerState>, String> {
    start_proxy_server_with_resolver(None)
}

pub fn start_proxy_server_with_resolver(
    credential_resolver: Option<CredentialResolver>,
) -> Result<Arc<ProxyServerState>, String> {
    if credential_resolver.is_none() {
        return Err("native provider credential resolver is required".to_string());
    }
    let std_listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| format!("bind local proxy: {error}"))?;
    std_listener
        .set_nonblocking(true)
        .map_err(|error| format!("configure local proxy: {error}"))?;
    let address = std_listener
        .local_addr()
        .map_err(|error| format!("read local proxy address: {error}"))?;

    let direct_client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("build direct proxy client: {error}"))?;
    let system_proxy_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("build system-proxy client: {error}"))?;

    let state = Arc::new(ProxyServerState {
        base_url: format!("http://{address}"),
        direct_client,
        system_proxy_client,
        credential_resolver,
        transport_tokens: Mutex::new(HashMap::new()),
        available: AtomicBool::new(true),
    });

    let app = build_proxy_router(state.clone());
    let lifecycle_state = state.clone();

    tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::from_std(std_listener) {
            Ok(listener) => listener,
            Err(_) => {
                lifecycle_state.available.store(false, Ordering::Release);
                diagnostics::record_event("proxy", "listener_conversion_failed", "failure", None);
                return;
            }
        };
        let served = axum::serve(listener, app).await;
        lifecycle_state.available.store(false, Ordering::Release);
        if served.is_err() {
            diagnostics::record_event("proxy", "server_stopped", "failure", None);
        }
    });

    Ok(state)
}

fn build_proxy_router(state: Arc<ProxyServerState>) -> Router {
    Router::new()
        .route("/proxy/{provider}", any(handle_proxy))
        .route("/proxy/{provider}/{*rest}", any(handle_proxy))
        // Apply the allowlist to every response, including framework-generated
        // 404/405 responses that never enter `handle_proxy`.
        .layer(middleware::from_fn(apply_cors_middleware))
        .with_state(state)
}

async fn apply_cors_middleware(request: Request, next: Next) -> Response {
    let request_headers = request.headers().clone();
    let mut response = next.run(request).await;
    apply_cors_headers(response.headers_mut(), &request_headers);
    response
}

async fn handle_proxy(
    State(state): State<Arc<ProxyServerState>>,
    Path(ProxyRoutePath { provider }): Path<ProxyRoutePath>,
    method: Method,
    headers: HeaderMap,
    OriginalUri(original_uri): OriginalUri,
    body: Body,
) -> Response {
    if method == Method::OPTIONS {
        return preflight_response(&headers);
    }

    if !valid_provider_segment(&provider) {
        return error_response(StatusCode::BAD_REQUEST, "Invalid provider id", &headers);
    }

    let token = match required_header(&headers, PROXY_TOKEN_HEADER) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let request_id = match required_header(&headers, PROXY_REQUEST_ID_HEADER) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    // Authorize before resolving any native provider configuration or opening
    // an upstream connection. A token is not a global bearer credential: it
    // must still name the exact native-authorized run and provider route.
    if !state.authorizes_transport(token, request_id, &provider) {
        return error_response(StatusCode::FORBIDDEN, "Invalid proxy token", &headers);
    }

    let original_path_and_query = original_uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    let credentials = state
        .credential_resolver
        .as_ref()
        .and_then(|resolver| resolver(&provider));
    let Some(credentials) = credentials else {
        return error_response(
            StatusCode::FORBIDDEN,
            "Provider is not configured",
            &headers,
        );
    };
    let target_url = match build_target_url(
        &provider,
        original_path_and_query,
        &credentials.upstream_base_url,
    ) {
        Ok(url) => url,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, &message, &headers),
    };

    let body_bytes = match to_bytes(body, MAX_REQUEST_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(error) => {
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                &format!("Proxy request body is too large or unreadable: {error}"),
                &headers,
            )
        }
    };

    // The renderer may carry this legacy header for compatibility, but it is
    // deliberately ignored. Native provider settings are authoritative.
    let use_system_proxy = credentials.use_system_proxy;
    let client = if use_system_proxy {
        &state.system_proxy_client
    } else {
        &state.direct_client
    };

    let mut upstream_headers = build_upstream_request_headers(&headers);
    inject_provider_credentials(&mut upstream_headers, credentials);

    let mut request = client.request(method, target_url).headers(upstream_headers);
    if !body_bytes.is_empty() {
        request = request.body(body_bytes);
    }

    let upstream_response = match request.send().await {
        Ok(response) => response,
        Err(_) => return provider_request_failed_response(&headers),
    };

    let status = upstream_response.status();
    if status.is_redirection() {
        return error_response(
            StatusCode::BAD_GATEWAY,
            "Provider redirects are not followed by the native proxy",
            &headers,
        );
    }
    let upstream_headers = upstream_response.headers().clone();
    // Do not buffer the provider response: SSE/chunked model streams must be
    // visible to Pi as they arrive.
    let mut response = Response::builder()
        .status(status)
        .body(Body::from_stream(upstream_response.bytes_stream()))
        .unwrap_or_else(|error| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(format!(
                    "Failed to build proxy response: {error}"
                )))
                .expect("proxy response fallback must build")
        });
    for (name, value) in &upstream_headers {
        if should_forward_response_header(name) {
            response.headers_mut().append(name, value.clone());
        }
    }
    apply_cors_headers(response.headers_mut(), &headers);
    response
}

fn valid_provider_segment(provider: &str) -> bool {
    !provider.is_empty()
        && provider.len() <= 128
        && provider
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Canonicalize an HTTP(S) origin only when the supplied value is structurally
/// an origin.  CORS and provider allowlists must never treat a path, query, or
/// embedded credential as part of an origin string.
fn normalize_origin(value: &str) -> Option<String> {
    let parsed = Url::parse(value.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.has_host()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return None;
    }
    Some(parsed.origin().ascii_serialization().to_ascii_lowercase())
}

fn build_target_url(
    provider: &str,
    original_path_and_query: &str,
    configured_base_url: &str,
) -> Result<Url, String> {
    if !valid_provider_segment(provider) {
        return Err("Invalid provider id".to_string());
    }
    let mut base = Url::parse(configured_base_url.trim())
        .map_err(|error| format!("Invalid native provider base URL: {error}"))?;
    if !matches!(base.scheme(), "http" | "https")
        || !base.has_host()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err("Native provider base URL must be an absolute http(s) URL without credentials, query, or fragment".to_string());
    }

    let base_path = base.path().trim_end_matches('/').to_string();
    let prefix = format!("/proxy/{provider}{base_path}");
    let suffix = original_path_and_query
        .strip_prefix(&prefix)
        .ok_or_else(|| "Invalid proxy path prefix".to_string())?;
    if !suffix.is_empty() && !suffix.starts_with('/') && !suffix.starts_with('?') {
        return Err(
            "Proxy request path must stay below the configured provider base path".to_string(),
        );
    }
    let (relative_path, query) = suffix
        .split_once('?')
        .map_or((suffix, None), |(path, query)| (path, Some(query)));
    if relative_path.starts_with("//") {
        return Err("Proxy request path must not begin with //".to_string());
    }
    let relative_path = relative_path.trim_start_matches('/');
    let lower_path = relative_path.to_ascii_lowercase();
    if relative_path
        .split('/')
        .any(|segment| matches!(segment, "." | ".."))
        || relative_path.contains('\\')
        || lower_path.contains("%2e")
        || lower_path.contains("%2f")
        || lower_path.contains("%5c")
    {
        return Err("Proxy request path contains an unsafe traversal segment".to_string());
    }

    if relative_path.is_empty() {
        base.set_query(query);
        return Ok(base);
    }
    let join_base_path = if base_path.is_empty() {
        "/".to_string()
    } else {
        format!("{base_path}/")
    };
    base.set_path(&join_base_path);
    let mut target = base
        .join(relative_path)
        .map_err(|error| format!("Failed to construct upstream URL: {error}"))?;
    let required_prefix = if base_path.is_empty() {
        "/".to_string()
    } else {
        format!("{base_path}/")
    };
    if !target.path().starts_with(&required_prefix) {
        return Err("Proxy request escaped the configured provider base path".to_string());
    }
    target.set_query(query);
    Ok(target)
}

fn required_header<'a>(
    headers: &'a HeaderMap,
    name: &'static str,
) -> Result<&'a str, Box<Response>> {
    let Some(value) = headers.get(name) else {
        return Err(Box::new(error_response(
            if matches!(name, PROXY_TOKEN_HEADER | PROXY_REQUEST_ID_HEADER) {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::BAD_REQUEST
            },
            &format!("Missing request header: {name}"),
            headers,
        )));
    };
    value.to_str().map_err(|_| {
        Box::new(error_response(
            if matches!(name, PROXY_TOKEN_HEADER | PROXY_REQUEST_ID_HEADER) {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::BAD_REQUEST
            },
            &format!("Request header is not valid UTF-8: {name}"),
            headers,
        ))
    })
}

fn preflight_response(request_headers: &HeaderMap) -> Response {
    let mut response = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::empty())
        .expect("proxy preflight response must build");
    apply_cors_headers(response.headers_mut(), request_headers);
    response
}

fn error_response(status: StatusCode, message: &str, request_headers: &HeaderMap) -> Response {
    let mut response = Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(Body::from(message.to_string()))
        .expect("proxy error response must build");
    apply_cors_headers(response.headers_mut(), request_headers);
    response
}

/// Keep upstream transport failures observable without turning the proxy into
/// a disclosure channel for reqwest's endpoint or network diagnostics.
fn provider_request_failed_response(request_headers: &HeaderMap) -> Response {
    diagnostics::record_event("proxy", "provider_request_failed", "failure", None);
    error_response(
        StatusCode::BAD_GATEWAY,
        PROVIDER_REQUEST_FAILED_ERROR,
        request_headers,
    )
}

fn apply_cors_headers(headers: &mut HeaderMap, request_headers: &HeaderMap) {
    headers.insert(
        HeaderName::from_static("vary"),
        HeaderValue::from_static(VARY_VALUE),
    );

    let Some(origin) = allowed_cors_origin(request_headers) else {
        // Origin-less Tauri loopback requests are normal same-origin requests;
        // omitting CORS grants avoids authorizing arbitrary web pages.
        return;
    };

    headers.insert(
        HeaderName::from_static("access-control-allow-origin"),
        HeaderValue::from_static(origin),
    );
    headers.insert(
        HeaderName::from_static("access-control-allow-methods"),
        HeaderValue::from_static(ALLOW_METHODS_VALUE),
    );
    headers.insert(
        HeaderName::from_static("access-control-allow-headers"),
        HeaderValue::from_static(DEFAULT_ALLOW_HEADERS),
    );
}

fn allowed_cors_origin(request_headers: &HeaderMap) -> Option<&'static str> {
    let request_origin = request_headers.get(ORIGIN)?.to_str().ok()?;
    let normalized_origin = normalize_origin(request_origin)?;

    match normalized_origin.as_str() {
        TAURI_WEBVIEW_ORIGIN => Some(TAURI_WEBVIEW_ORIGIN),
        // The Vite origin is useful for `tauri dev`, but release binaries must
        // not grant a development server cross-origin access to the local
        // credential proxy.
        VITE_DEVELOPMENT_ORIGIN if cfg!(debug_assertions) => Some(VITE_DEVELOPMENT_ORIGIN),
        _ => None,
    }
}

fn should_forward_request_header(name: &HeaderName) -> bool {
    let lowered = name.as_str();
    !matches!(
        lowered,
        HOST | CONTENT_LENGTH
            | CONNECTION
            | KEEP_ALIVE
            | PROXY_CONNECTION
            | PROXY_AUTHENTICATE
            | PROXY_AUTHORIZATION
            | "authorization"
            | "x-api-key"
            | "x-goog-api-key"
            | "api-key"
            | "cookie"
            | "set-cookie"
            | "forwarded"
            | "x-forwarded"
            | TE
            | TRAILER
            | TRANSFER_ENCODING
            | UPGRADE
            | ORIGIN
            | "referer"
            | ACCESS_CONTROL_REQUEST_METHOD
            | ACCESS_CONTROL_REQUEST_HEADERS
    ) && !lowered.starts_with(ACCESS_CONTROL_PREFIX)
        && !lowered.starts_with(PROXY_PREFIX)
        && !lowered.starts_with("x-forwarded-")
}

fn build_upstream_request_headers(headers: &HeaderMap) -> HeaderMap {
    let mut upstream_headers = HeaderMap::new();
    for (name, value) in headers {
        if should_forward_request_header(name) {
            upstream_headers.append(name, value.clone());
        }
    }
    if let Some(value) = headers.get(UPSTREAM_USER_AGENT_HEADER) {
        upstream_headers.insert(HeaderName::from_static("user-agent"), value.clone());
    }
    if let Some(value) = headers.get(UPSTREAM_CONTENT_TYPE_HEADER) {
        upstream_headers.insert(HeaderName::from_static(CONTENT_TYPE), value.clone());
    }
    upstream_headers
}

fn inject_provider_credentials(headers: &mut HeaderMap, credentials: ProviderCredentials) {
    for (name, value) in credentials.headers {
        let Ok(header_name) = HeaderName::from_bytes(name.trim().as_bytes()) else {
            continue;
        };
        if !should_inject_provider_header(&header_name) {
            continue;
        }
        let Ok(header_value) = HeaderValue::from_str(value.trim()) else {
            continue;
        };
        // Native credentials are authoritative.  This also prevents a stale
        // renderer value from overriding a newly rotated secret.
        headers.insert(header_name, header_value);
    }
}

fn should_inject_provider_header(name: &HeaderName) -> bool {
    let lowered = name.as_str();
    !matches!(
        lowered,
        HOST | CONTENT_LENGTH
            | CONNECTION
            | KEEP_ALIVE
            | PROXY_CONNECTION
            | PROXY_AUTHENTICATE
            | PROXY_AUTHORIZATION
            | TE
            | TRAILER
            | TRANSFER_ENCODING
            | UPGRADE
            | ORIGIN
            | "referer"
            | "forwarded"
            | "x-forwarded"
    ) && !lowered.starts_with(ACCESS_CONTROL_PREFIX)
        && !lowered.starts_with(PROXY_PREFIX)
        && !lowered.starts_with("x-forwarded-")
}

fn should_forward_response_header(name: &HeaderName) -> bool {
    let lowered = name.as_str();
    !matches!(
        lowered,
        CONTENT_LENGTH
            | CONNECTION
            | KEEP_ALIVE
            | PROXY_CONNECTION
            | PROXY_AUTHENTICATE
            | PROXY_AUTHORIZATION
            | TE
            | TRAILER
            | TRANSFER_ENCODING
            | UPGRADE
            | "set-cookie"
            | "vary"
    ) && !lowered.starts_with(ACCESS_CONTROL_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    #[test]
    fn builds_target_url_for_provider_path() {
        let target = build_target_url(
            "codex",
            "/proxy/codex/v1/responses",
            "https://api.openai.com",
        )
        .expect("target url should be built");
        assert_eq!(target.as_str(), "https://api.openai.com/v1/responses");
    }

    #[test]
    fn preserves_nested_path_and_query() {
        let target = build_target_url(
            "claude_code",
            "/proxy/claude_code/api/coding/v1/messages?stream=true",
            "https://ark.cn-beijing.volces.com",
        )
        .expect("target url should be built");
        assert_eq!(
            target.as_str(),
            "https://ark.cn-beijing.volces.com/api/coding/v1/messages?stream=true"
        );
    }

    #[test]
    fn rejects_unsafe_origins_and_paths() {
        assert!(build_target_url("hub", "/proxy/hub/x", "file:///tmp").is_err());
        assert!(build_target_url("hub", "/proxy/hub//evil", "https://example.com").is_err());
        assert!(build_target_url("../hub", "/proxy/../hub/x", "https://example.com").is_err());
    }

    #[test]
    fn preserves_the_native_gateway_path_and_rejects_sibling_tenants() {
        let target = build_target_url(
            "gateway",
            "/proxy/gateway/tenant-a/v1/responses?stream=true",
            "https://gateway.example/tenant-a/v1",
        )
        .expect("configured tenant path should be retained");
        assert_eq!(
            target.as_str(),
            "https://gateway.example/tenant-a/v1/responses?stream=true"
        );
        assert!(build_target_url(
            "gateway",
            "/proxy/gateway/tenant-b/v1/responses",
            "https://gateway.example/tenant-a/v1",
        )
        .is_err());
        assert!(build_target_url(
            "gateway",
            "/proxy/gateway/tenant-a/v1/%2e%2e/tenant-b/responses",
            "https://gateway.example/tenant-a/v1",
        )
        .is_err());
    }

    #[test]
    fn strips_renderer_credentials_proxy_and_routing_headers() {
        for name in [
            "host",
            "connection",
            PROXY_TOKEN_HEADER,
            PROXY_REQUEST_ID_HEADER,
            UPSTREAM_ORIGIN_HEADER,
            "authorization",
            "proxy-authorization",
            "x-api-key",
            "x-goog-api-key",
            "api-key",
            "cookie",
            "set-cookie",
            "forwarded",
            "x-forwarded-for",
            "x-forwarded-host",
            "x-forwarded-proto",
        ] {
            assert!(
                !should_forward_request_header(&HeaderName::from_static(name)),
                "renderer header should not reach provider: {name}"
            );
        }
        assert!(should_forward_request_header(&HeaderName::from_static(
            "x-client-request-id"
        )));
        assert!(should_forward_request_header(&HeaderName::from_static(
            "anthropic-version"
        )));
        assert!(
            !should_forward_response_header(&HeaderName::from_static("set-cookie")),
            "an upstream provider must not be able to set loopback cookies"
        );
    }

    #[test]
    fn native_credentials_are_the_only_upstream_credentials() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer stale"),
        );
        headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_static("stale-api-key"),
        );
        headers.insert(
            HeaderName::from_static("x-client-request-id"),
            HeaderValue::from_static("request-1"),
        );
        let mut upstream_headers = build_upstream_request_headers(&headers);
        assert!(upstream_headers.get("authorization").is_none());
        assert!(upstream_headers.get("x-api-key").is_none());
        assert!(upstream_headers.get("x-client-request-id").is_some());
        inject_provider_credentials(
            &mut upstream_headers,
            ProviderCredentials {
                headers: vec![
                    ("authorization".to_string(), "Bearer native".to_string()),
                    ("x-api-key".to_string(), "native-api-key".to_string()),
                ],
                upstream_base_url: "https://api.example.test/v1".to_string(),
                use_system_proxy: false,
            },
        );
        assert_eq!(
            upstream_headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer native")
        );
        assert_eq!(
            upstream_headers
                .get("x-api-key")
                .and_then(|value| value.to_str().ok()),
            Some("native-api-key")
        );
    }

    #[test]
    fn native_provider_headers_cannot_override_transport_routing() {
        let mut upstream_headers = HeaderMap::new();
        inject_provider_credentials(
            &mut upstream_headers,
            ProviderCredentials {
                headers: vec![
                    ("host".to_string(), "evil.example".to_string()),
                    ("content-length".to_string(), "1".to_string()),
                    ("connection".to_string(), "close".to_string()),
                    ("forwarded".to_string(), "host=evil.example".to_string()),
                    ("x-forwarded-host".to_string(), "evil.example".to_string()),
                    ("authorization".to_string(), "Bearer native".to_string()),
                ],
                upstream_base_url: "https://api.example.test/v1".to_string(),
                use_system_proxy: false,
            },
        );
        for name in [
            "host",
            "content-length",
            "connection",
            "forwarded",
            "x-forwarded-host",
        ] {
            assert!(
                upstream_headers.get(name).is_none(),
                "reserved header: {name}"
            );
        }
        assert_eq!(
            upstream_headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer native")
        );
    }

    #[test]
    fn refuses_to_start_without_native_resolver() {
        assert!(start_proxy_server().is_err());
        assert!(start_proxy_server_with_resolver(None).is_err());
    }

    #[test]
    fn proxy_runtime_retries_an_initial_failure_once_ready() {
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let starter_attempts = attempts.clone();
        let runtime = start_proxy_runtime_with_starter(
            Arc::new(|_| None),
            Arc::new(move |_| {
                if starter_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err("bind local proxy: test-only startup failure".to_string())
                } else {
                    Ok(test_proxy_state())
                }
            }),
        );

        assert_eq!(
            runtime.status(),
            ProxyRuntimeStatus {
                status: ProxyAvailability::Unavailable,
                can_retry: true,
            }
        );
        assert_eq!(
            runtime.retry_start(),
            ProxyRuntimeStatus {
                status: ProxyAvailability::Ready,
                can_retry: false,
            }
        );
        assert_eq!(
            runtime.retry_start(),
            ProxyRuntimeStatus {
                status: ProxyAvailability::Ready,
                can_retry: false,
            },
            "a ready runtime must not create another listener on retry"
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            2,
            "initial failure plus exactly one retry start"
        );
    }

    #[test]
    fn proxy_runtime_serializes_concurrent_retry_attempts() {
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let starter_attempts = attempts.clone();
        let runtime = start_proxy_runtime_with_starter(
            Arc::new(|_| None),
            Arc::new(move |_| {
                if starter_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err("bind local proxy: test-only startup failure".to_string())
                } else {
                    std::thread::sleep(Duration::from_millis(25));
                    Ok(test_proxy_state())
                }
            }),
        );
        let gate = Arc::new(std::sync::Barrier::new(3));

        let first_runtime = runtime.clone();
        let first_gate = gate.clone();
        let first = std::thread::spawn(move || {
            first_gate.wait();
            first_runtime.retry_start()
        });
        let second_runtime = runtime.clone();
        let second_gate = gate.clone();
        let second = std::thread::spawn(move || {
            second_gate.wait();
            second_runtime.retry_start()
        });
        gate.wait();

        for status in [
            first.join().expect("first retry thread should not panic"),
            second.join().expect("second retry thread should not panic"),
        ] {
            assert_eq!(status.status, ProxyAvailability::Ready);
            assert!(!status.can_retry);
        }
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            2,
            "the initial failure plus one serialized retry must be the only starts"
        );
    }

    #[test]
    fn unavailable_runtime_never_exposes_startup_details_or_server_token() {
        let secret = "server-token-must-not-leak";
        let runtime = start_proxy_runtime_with_starter(
            Arc::new(|_| None),
            Arc::new(move |_| {
                Err(format!(
                    "bind local proxy at 127.0.0.1:50000 with token {secret}"
                ))
            }),
        );

        let public_status =
            serde_json::to_value(runtime.status()).expect("runtime status should serialize");
        assert_eq!(
            public_status,
            serde_json::json!({"status": "unavailable", "canRetry": true})
        );
        assert!(
            !public_status.to_string().contains(secret),
            "public runtime status must not expose a failed server token"
        );

        let error = runtime
            .issue_transport_info(
                "test-request",
                "demo",
                Instant::now() + Duration::from_secs(60),
                Arc::new(AtomicBool::new(false)),
            )
            .expect_err("unavailable runtime must not return proxy info");
        assert_eq!(error, PROXY_UNAVAILABLE_ERROR);
        assert!(!error.contains(secret));
        assert!(!error.contains("127.0.0.1"));
    }

    #[test]
    fn ready_runtime_returns_info_for_capability_bound_transport() {
        let runtime = start_proxy_runtime_with_starter(
            Arc::new(|_| None),
            Arc::new(|_| Ok(test_proxy_state())),
        );
        let cancelled = Arc::new(AtomicBool::new(false));

        let info = runtime
            .issue_transport_info(
                "test-request",
                "demo",
                Instant::now() + Duration::from_secs(60),
                cancelled,
            )
            .expect("ready runtime must expose loopback info to the Pi transport command");
        assert_eq!(info.base_url, "http://127.0.0.1:9");
        assert!(!info.token.is_empty());
        assert_eq!(
            runtime.status(),
            ProxyRuntimeStatus {
                status: ProxyAvailability::Ready,
                can_retry: false,
            }
        );
    }

    #[test]
    fn transport_token_is_scoped_to_one_request_provider_and_live_grant() {
        let state = test_proxy_state();
        let cancelled = Arc::new(AtomicBool::new(false));
        let first = state
            .issue_transport_info(
                "run-alpha",
                "provider-a",
                Instant::now() + Duration::from_secs(60),
                cancelled.clone(),
            )
            .expect("live grant should issue a token");
        assert!(state.authorizes_transport(&first.token, "run-alpha", "provider-a"));
        assert!(!state.authorizes_transport(&first.token, "run-beta", "provider-a"));
        assert!(!state.authorizes_transport(&first.token, "run-alpha", "provider-b"));

        let replacement = state
            .issue_transport_info(
                "run-alpha",
                "provider-a",
                Instant::now() + Duration::from_secs(60),
                cancelled.clone(),
            )
            .expect("a fresh handshake should rotate the run token");
        assert!(
            !state.authorizes_transport(&first.token, "run-alpha", "provider-a"),
            "a replacement handshake must revoke the prior token"
        );
        assert!(state.authorizes_transport(&replacement.token, "run-alpha", "provider-a"));

        cancelled.store(true, Ordering::SeqCst);
        assert!(
            !state.authorizes_transport(&replacement.token, "run-alpha", "provider-a"),
            "terminal/cancelled grants must reject new proxy requests"
        );
        assert!(state
            .issue_transport_info(
                "expired-run",
                "provider-a",
                Instant::now() - Duration::from_secs(1),
                Arc::new(AtomicBool::new(false)),
            )
            .is_err());
    }

    #[test]
    fn allows_only_configured_tauri_and_development_origins() {
        let mut expected_origins = vec![TAURI_WEBVIEW_ORIGIN];
        if cfg!(debug_assertions) {
            expected_origins.push(VITE_DEVELOPMENT_ORIGIN);
        }
        for expected_origin in expected_origins {
            let mut request_headers = HeaderMap::new();
            request_headers.insert(
                HeaderName::from_static(ORIGIN),
                HeaderValue::from_static(expected_origin),
            );
            let mut response_headers = HeaderMap::new();

            apply_cors_headers(&mut response_headers, &request_headers);

            assert_eq!(
                response_headers
                    .get("access-control-allow-origin")
                    .and_then(|value| value.to_str().ok()),
                Some(expected_origin),
                "allowed origin must be granted exactly"
            );
        }
    }

    #[test]
    fn denies_cors_to_untrusted_and_missing_origins() {
        for origin in [Some("https://untrusted.example"), None] {
            let mut request_headers = HeaderMap::new();
            if let Some(origin) = origin {
                request_headers.insert(
                    HeaderName::from_static(ORIGIN),
                    HeaderValue::from_static(origin),
                );
            }
            let mut response_headers = HeaderMap::new();

            apply_cors_headers(&mut response_headers, &request_headers);

            assert!(
                response_headers
                    .get("access-control-allow-origin")
                    .is_none(),
                "untrusted or absent origins must not receive CORS access"
            );
        }
    }

    #[test]
    fn preflight_headers_stay_on_the_fixed_allowlist() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static(ORIGIN),
            HeaderValue::from_static("http://tauri.localhost"),
        );
        headers.insert(
            HeaderName::from_static(ACCESS_CONTROL_REQUEST_HEADERS),
            HeaderValue::from_static("authorization,x-novavei-proxy-token,x-untrusted-header"),
        );
        let mut response_headers = HeaderMap::new();

        apply_cors_headers(&mut response_headers, &headers);

        assert_eq!(
            response_headers
                .get("access-control-allow-headers")
                .cloned(),
            Some(HeaderValue::from_static(DEFAULT_ALLOW_HEADERS))
        );
        assert!(
            !response_headers
                .get("access-control-allow-headers")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .contains("x-untrusted-header"),
            "untrusted preflight headers must not be reflected"
        );
    }

    #[test]
    fn preflight_allowlist_covers_browser_sdk_headers() {
        // OpenAI/Anthropic browser SDKs always attach Stainless telemetry headers.
        // If any are missing from the fixed allowlist, WebView CORS rejects the
        // preflight and the UI only shows the opaque "Connection error." message.
        let allowed = DEFAULT_ALLOW_HEADERS.to_ascii_lowercase();
        for required in [
            "authorization",
            "content-type",
            "x-novavei-proxy-token",
            "x-novavei-proxy-request-id",
            "x-novavei-upstream-origin",
            "x-stainless-retry-count",
            "x-stainless-timeout",
            "session_id",
            "x-client-request-id",
            "anthropic-beta",
            "x-api-key",
        ] {
            assert!(
                allowed.split(',').any(|item| item.trim() == required),
                "CORS allowlist missing browser SDK header: {required}"
            );
        }
    }

    fn test_proxy_state() -> Arc<ProxyServerState> {
        Arc::new(ProxyServerState {
            base_url: "http://127.0.0.1:9".to_string(),
            direct_client: reqwest::Client::new(),
            system_proxy_client: reqwest::Client::new(),
            credential_resolver: Some(Arc::new(|_| None)),
            transport_tokens: Mutex::new(HashMap::new()),
            available: AtomicBool::new(true),
        })
    }

    fn test_proxy_state_with_unreachable_upstream() -> Arc<ProxyServerState> {
        Arc::new(ProxyServerState {
            base_url: "http://127.0.0.1:9".to_string(),
            // Port zero cannot be a listening TCP endpoint.  Use a direct
            // client so host proxy environment variables cannot mask the
            // deliberately failing local request.
            direct_client: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("test direct client should build"),
            system_proxy_client: reqwest::Client::new(),
            credential_resolver: Some(Arc::new(|provider| {
                (provider == "demo").then(|| ProviderCredentials {
                    headers: vec![(
                        "authorization".to_string(),
                        "Bearer native-upstream-secret".to_string(),
                    )],
                    upstream_base_url: "http://127.0.0.1:0".to_string(),
                    use_system_proxy: false,
                })
            })),
            transport_tokens: Mutex::new(HashMap::new()),
            available: AtomicBool::new(true),
        })
    }

    #[tokio::test]
    async fn upstream_transport_failure_returns_only_a_fixed_redacted_body() {
        use axum::body::Body as AxumBody;
        use axum::http::Request as HttpRequest;

        let state = test_proxy_state_with_unreachable_upstream();
        let token = state
            .issue_transport_info(
                "test-request",
                "demo",
                Instant::now() + Duration::from_secs(60),
                Arc::new(AtomicBool::new(false)),
            )
            .expect("test transport token should issue")
            .token;
        let response = build_proxy_router(state)
            .oneshot(
                HttpRequest::builder()
                    .method(Method::POST)
                    .uri("/proxy/demo/v1/responses")
                    .header(ORIGIN, TAURI_WEBVIEW_ORIGIN)
                    .header(PROXY_TOKEN_HEADER, token)
                    .header(PROXY_REQUEST_ID_HEADER, "test-request")
                    .body(AxumBody::empty())
                    .expect("test proxy request should build"),
            )
            .await
            .expect("proxy router should return an HTTP response");

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = to_bytes(response.into_body(), MAX_REQUEST_BODY_BYTES)
            .await
            .expect("fixed proxy failure body should be readable");
        assert_eq!(body.as_ref(), PROVIDER_REQUEST_FAILED_ERROR.as_bytes());
        let body = String::from_utf8_lossy(&body);
        for sensitive_detail in ["127.0.0.1", "native-upstream-secret", "connection refused"] {
            assert!(
                !body.contains(sensitive_detail),
                "upstream transport detail must not reach the WebView: {sensitive_detail}"
            );
        }
    }

    #[tokio::test]
    async fn mismatched_transport_scope_never_resolves_provider_credentials() {
        use axum::body::Body as AxumBody;
        use axum::http::Request as HttpRequest;

        let resolver_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls = resolver_calls.clone();
        let state = Arc::new(ProxyServerState {
            base_url: "http://127.0.0.1:9".to_string(),
            direct_client: reqwest::Client::new(),
            system_proxy_client: reqwest::Client::new(),
            credential_resolver: Some(Arc::new(move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Some(ProviderCredentials::default())
            })),
            transport_tokens: Mutex::new(HashMap::new()),
            available: AtomicBool::new(true),
        });
        let token = state
            .issue_transport_info(
                "scope-request",
                "configured-provider",
                Instant::now() + Duration::from_secs(60),
                Arc::new(AtomicBool::new(false)),
            )
            .expect("test transport token should issue")
            .token;

        let response = build_proxy_router(state)
            .oneshot(
                HttpRequest::builder()
                    .method(Method::POST)
                    .uri("/proxy/other-provider/v1/responses")
                    .header(ORIGIN, TAURI_WEBVIEW_ORIGIN)
                    .header(PROXY_TOKEN_HEADER, token)
                    .header(PROXY_REQUEST_ID_HEADER, "scope-request")
                    .body(AxumBody::empty())
                    .expect("test proxy request should build"),
            )
            .await
            .expect("proxy router should return an HTTP response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(resolver_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn router_applies_cors_to_404_and_405_for_allowed_origin() {
        use axum::body::Body as AxumBody;
        use axum::http::Request as HttpRequest;

        let app = build_proxy_router(test_proxy_state());
        let not_found = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method(Method::GET)
                    .uri("/missing")
                    .header(ORIGIN, TAURI_WEBVIEW_ORIGIN)
                    .body(AxumBody::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(not_found.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            not_found
                .headers()
                .get("access-control-allow-origin")
                .and_then(|value| value.to_str().ok()),
            Some(TAURI_WEBVIEW_ORIGIN)
        );

        let secondary_origin = if cfg!(debug_assertions) {
            VITE_DEVELOPMENT_ORIGIN
        } else {
            TAURI_WEBVIEW_ORIGIN
        };
        let method_not_allowed = app
            .oneshot(
                HttpRequest::builder()
                    .method(Method::TRACE)
                    .uri("/proxy/demo")
                    .header(ORIGIN, secondary_origin)
                    .body(AxumBody::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // TRACE is not routed; middleware must still attach fixed CORS headers.
        assert!(
            method_not_allowed.status().is_client_error()
                || method_not_allowed.status().is_server_error()
                || method_not_allowed.status() == StatusCode::METHOD_NOT_ALLOWED
                || method_not_allowed.status() == StatusCode::NOT_FOUND
        );
        assert_eq!(
            method_not_allowed
                .headers()
                .get("access-control-allow-origin")
                .and_then(|value| value.to_str().ok()),
            Some(secondary_origin)
        );
    }

    #[tokio::test]
    async fn router_preflight_and_business_error_use_fixed_cors_policy() {
        use axum::body::Body as AxumBody;
        use axum::http::Request as HttpRequest;

        let app = build_proxy_router(test_proxy_state());
        let preflight = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method(Method::OPTIONS)
                    .uri("/proxy/demo")
                    .header(ORIGIN, TAURI_WEBVIEW_ORIGIN)
                    .header(
                        ACCESS_CONTROL_REQUEST_HEADERS,
                        "authorization,x-untrusted-header",
                    )
                    .body(AxumBody::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(preflight.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            preflight
                .headers()
                .get("access-control-allow-headers")
                .cloned(),
            Some(HeaderValue::from_static(DEFAULT_ALLOW_HEADERS))
        );

        let forbidden = app
            .oneshot(
                HttpRequest::builder()
                    .method(Method::GET)
                    .uri("/proxy/demo")
                    .header(ORIGIN, TAURI_WEBVIEW_ORIGIN)
                    .header(PROXY_TOKEN_HEADER, "wrong-token")
                    .header(UPSTREAM_ORIGIN_HEADER, "https://api.openai.com")
                    .body(AxumBody::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            forbidden
                .headers()
                .get("access-control-allow-origin")
                .and_then(|value| value.to_str().ok()),
            Some(TAURI_WEBVIEW_ORIGIN)
        );
    }

    #[test]
    fn normalizes_only_structural_http_origins() {
        assert_eq!(
            normalize_origin("HTTPS://API.EXAMPLE.COM/"),
            Some("https://api.example.com".to_string())
        );
        assert_eq!(
            normalize_origin("https://api.example.com/v1"),
            None,
            "provider origin must not include a path"
        );
        assert_ne!(
            normalize_origin("https://api.example.com"),
            normalize_origin("https://evil.example.com")
        );
    }
}
