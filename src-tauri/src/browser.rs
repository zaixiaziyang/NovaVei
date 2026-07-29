//! Native child-WebView browser surface.
//!
//! The browser is deliberately a separate child WebView rather than an
//! iframe in the application renderer.  Many sites reject embedding, and the
//! child keeps arbitrary page origins out of the NovaVei UI DOM.  Its webview
//! label is intentionally excluded from the default capability, so a loaded
//! page cannot invoke the application's local commands.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
#[cfg(feature = "desktop")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "desktop")]
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
#[cfg(feature = "desktop")]
use tauri::AppHandle;
use tauri::Url;
#[cfg(not(feature = "desktop"))]
type AppHandle<R = tauri::test::MockRuntime> = tauri::AppHandle<R>;

#[cfg(feature = "desktop")]
use tauri::{
    webview::{NewWindowResponse, WebviewBuilder},
    LogicalPosition, LogicalSize, Manager, WebviewUrl,
};

const MAX_BROWSER_URL_BYTES: usize = 2_048;
const MAX_BROWSER_RESOLVED_ADDRESSES: usize = 64;
#[cfg(feature = "desktop")]
const MAX_BROWSER_INPUT_CHARS: usize = 2_000;
#[cfg(feature = "desktop")]
const MAX_BROWSER_FINGERPRINT_BYTES: usize = 2_048;
#[cfg(feature = "desktop")]
const EVALUATION_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    pub available: bool,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct BrowserViewport {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub visible: bool,
}

fn browser_url(url: &str) -> Result<Url, String> {
    browser_url_with_resolver(url, &resolve_browser_host)
}

fn browser_url_with_resolver<F>(url: &str, resolver: &F) -> Result<Url, String>
where
    F: Fn(&str) -> Result<Vec<IpAddr>, ()>,
{
    let trimmed = url.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_BROWSER_URL_BYTES {
        return Err("browser URL is empty or exceeds the limit".to_string());
    }
    let parsed = Url::parse(trimmed).map_err(|_| "browser URL is invalid".to_string())?;
    if !matches!(parsed.scheme(), "https" | "http")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !browser_host_is_public_with_resolver(&parsed, resolver)
    {
        return Err("browser supports only credential-free public http or https URLs".to_string());
    }
    Ok(parsed)
}

fn allowed_browser_navigation(url: &Url) -> bool {
    allowed_browser_navigation_with_resolver(url, &resolve_browser_host)
}

fn allowed_browser_navigation_with_resolver<F>(url: &Url, resolver: &F) -> bool
where
    F: Fn(&str) -> Result<Vec<IpAddr>, ()>,
{
    matches!(url.scheme(), "https" | "http")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && browser_host_is_public_with_resolver(url, resolver)
}

/// Reject destinations that can address the local machine or a private
/// network.  Browser snapshots are supplied to an agent, so treating a
/// browser navigation as generic HTTP would otherwise turn this WebView into
/// a local-service reader.  This is deliberately applied both to the initial
/// URL and to every redirect/navigation callback.
fn browser_host_is_public_with_resolver<F>(url: &Url, resolver: &F) -> bool
where
    F: Fn(&str) -> Result<Vec<IpAddr>, ()>,
{
    let Some(host) = url.host_str() else {
        return false;
    };

    // `Url::host_str` keeps brackets around an IPv6 literal. Remove them
    // before parsing so loopback and other non-public IPv6 destinations do
    // not accidentally fall through as DNS names.
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    let normalized = host.trim_end_matches('.').to_ascii_lowercase();
    if normalized.is_empty()
        || normalized == "localhost"
        || normalized.ends_with(".localhost")
        || matches!(
            normalized.as_str(),
            "local" | "localdomain" | "internal" | "home" | "lan" | "corp"
        )
        || [
            ".local",
            ".localdomain",
            ".internal",
            ".home",
            ".lan",
            ".corp",
        ]
        .iter()
        .any(|suffix| normalized.ends_with(suffix))
    {
        return false;
    }

    match normalized.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => public_ipv4(address),
        Ok(IpAddr::V6(address)) => public_ipv6(address),
        Err(_) => {
            let Ok(addresses) = resolver(&normalized) else {
                return false;
            };
            !addresses.is_empty()
                && addresses.len() <= MAX_BROWSER_RESOLVED_ADDRESSES
                && addresses.into_iter().all(public_ip)
        }
    }
}

/// Resolve both A and AAAA answers through the operating system. An empty,
/// failed, or unexpectedly large result is denied rather than being treated
/// as a public hostname. The WebView performs its own later lookup, so this is
/// a fail-closed preflight/recheck rather than DNS pinning; see `open` below.
fn resolve_browser_host(host: &str) -> Result<Vec<IpAddr>, ()> {
    let addresses = (host, 0_u16)
        .to_socket_addrs()
        .map_err(|_| ())?
        .take(MAX_BROWSER_RESOLVED_ADDRESSES + 1)
        .map(|address| address.ip())
        .collect::<Vec<_>>();
    if addresses.is_empty() || addresses.len() > MAX_BROWSER_RESOLVED_ADDRESSES {
        return Err(());
    }
    Ok(addresses)
}

fn public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => public_ipv4(address),
        IpAddr::V6(address) => public_ipv6(address),
    }
}

fn public_ipv4(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    !address.is_private()
        && !address.is_loopback()
        && !address.is_link_local()
        && !address.is_broadcast()
        && !address.is_documentation()
        && !address.is_unspecified()
        && !address.is_multicast()
        // 0/8, carrier-grade NAT, IETF protocol assignments, benchmark
        // networks, and future-use space are not public Internet targets.
        && octets[0] != 0
        && !(octets[0] == 100 && (64..=127).contains(&octets[1]))
        && !(octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        && !(octets[0] == 192 && octets[1] == 31 && octets[2] == 196)
        && !(octets[0] == 192 && octets[1] == 52 && octets[2] == 193)
        && !(octets[0] == 192 && octets[1] == 88 && octets[2] == 99)
        && !(octets[0] == 192 && octets[1] == 175 && octets[2] == 48)
        && !(octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        && octets[0] < 240
}

fn public_ipv6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    // Stay fail-closed to the currently allocated global-unicast block.
    // IPv4-mapped/compatible addresses and every other special or reserved
    // prefix are outside 2000::/3 and are therefore denied without relying on
    // a platform's interpretation of their embedded destination.
    segments[0] & 0xe000 == 0x2000
        && !address.is_unspecified()
        && !address.is_loopback()
        && !address.is_unique_local()
        && !address.is_unicast_link_local()
        && !address.is_multicast()
        // IPv4-compatible, translation, discard-only, IETF protocol,
        // documentation, 6to4, SRv6, AS112, and deprecated site-local ranges
        // are all special-use rather than browser-reachable public targets.
        && segments[0] != 0
        && !(segments[0] == 0x0064
            && segments[1] == 0xff9b
            && segments[2..6].iter().all(|segment| *segment == 0))
        && !(segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2] == 1)
        && !(segments[0] == 0x0100 && segments[1] == 0)
        && !(segments[0] == 0x2001 && segments[1] <= 0x01ff)
        && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
        && segments[0] != 0x2002
        && segments[0] & 0xfff0 != 0x3ff0
        && segments[0] != 0x5f00
        && (segments[0] & 0xffc0 != 0xfec0)
        && !(segments[0] == 0x2620 && segments[1] == 0x004f && segments[2] == 0x8000)
}

#[cfg(feature = "desktop")]
fn browser_webview_label(owner_label: &str) -> String {
    format!("browser-{owner_label}")
}

#[cfg(feature = "desktop")]
fn browser_state(app: &AppHandle, owner_label: &str) -> BrowserState {
    let browser_label = browser_webview_label(owner_label);
    let Some(webview) = app.get_webview(&browser_label) else {
        return BrowserState {
            available: false,
            url: None,
        };
    };
    BrowserState {
        available: true,
        url: webview.url().ok().map(|url| url.to_string()),
    }
}

/// Return the compact, renderer-safe state of the native browser surface.
/// A missing child WebView is a normal unopened state rather than an error.
#[cfg(feature = "desktop")]
pub fn status(app: AppHandle, owner_label: String) -> BrowserState {
    browser_state(&app, &owner_label)
}

/// Create the native WebView on a worker thread. WebView2 can deadlock when a
/// child is created synchronously from an invoke handler, so this deliberately
/// follows Tauri's documented asynchronous creation route.
#[cfg(feature = "desktop")]
pub async fn open(
    app: AppHandle,
    owner_label: String,
    url: String,
) -> Result<BrowserState, String> {
    let url = browser_url(&url)?;
    let app_for_worker = app.clone();
    let owner_label_for_worker = owner_label.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let browser_label = browser_webview_label(&owner_label_for_worker);
        if let Some(webview) = app_for_worker.get_webview(&browser_label) {
            webview
                .navigate(url)
                .map_err(|_| "browser navigation failed".to_string())?;
            return Ok(());
        }
        let main_webview = app_for_worker
            .get_webview(&owner_label_for_worker)
            .ok_or_else(|| "calling NovaVei WebView is unavailable".to_string())?;
        let browser_builder = WebviewBuilder::new(browser_label, WebviewUrl::External(url))
            .on_navigation(allowed_browser_navigation)
            // A site may not create detached windows from inside the
            // dock. It can navigate the one visible browser surface.
            .on_new_window(|_, _| NewWindowResponse::Deny)
            // Downloads need a dedicated, user-mediated save flow;
            // do not let a page write files as a side effect here.
            .on_download(|_, _| false);
        // Every top-level NovaVei window owns one browser child, so its
        // profile must be distinct even in installed mode. Otherwise two
        // WebView2 children can contend for one profile directory.
        let browser_builder = browser_builder.data_directory(
            crate::storage::application_data_dir()
                .join("webview")
                .join(format!("browser-{owner_label_for_worker}")),
        );
        // `add_child` belongs to Tauri's native `Window`, not its
        // `WebviewWindow` facade. Obtain the owning native window from the
        // main webview so the browser stays a child surface rather than a
        // separately privileged top-level WebviewWindow.
        let browser = main_webview
            .window()
            .add_child(
                browser_builder,
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(1.0, 1.0),
            )
            .map_err(|_| "browser WebView could not be created".to_string())?;
        // The renderer supplies the exact dock viewport before showing it.
        browser
            .hide()
            .map_err(|_| "browser WebView could not be initialized".to_string())?;
        Ok(())
    })
    .await
    .map_err(|_| "browser WebView task did not complete".to_string())??;
    Ok(browser_state(&app, &owner_label))
}

#[cfg(feature = "desktop")]
pub fn layout(
    app: AppHandle,
    owner_label: String,
    viewport: BrowserViewport,
) -> Result<BrowserState, String> {
    if !viewport.x.is_finite()
        || !viewport.y.is_finite()
        || !viewport.width.is_finite()
        || !viewport.height.is_finite()
        || viewport.x < 0.0
        || viewport.y < 0.0
        || viewport.width < 1.0
        || viewport.height < 1.0
        || viewport.x > 16_384.0
        || viewport.y > 16_384.0
        || viewport.width > 16_384.0
        || viewport.height > 16_384.0
    {
        return Err("browser viewport is outside the supported window bounds".to_string());
    }
    let browser_label = browser_webview_label(&owner_label);
    let Some(webview) = app.get_webview(&browser_label) else {
        return Ok(browser_state(&app, &owner_label));
    };
    webview
        .set_position(LogicalPosition::new(viewport.x, viewport.y))
        .map_err(|_| "browser viewport position could not be updated".to_string())?;
    webview
        .set_size(LogicalSize::new(viewport.width, viewport.height))
        .map_err(|_| "browser viewport size could not be updated".to_string())?;
    if viewport.visible {
        webview
            .show()
            .map_err(|_| "browser WebView could not be shown".to_string())?;
    } else {
        webview
            .hide()
            .map_err(|_| "browser WebView could not be hidden".to_string())?;
    }
    Ok(browser_state(&app, &owner_label))
}

#[cfg(feature = "desktop")]
pub fn back(app: AppHandle, owner_label: String) -> Result<BrowserState, String> {
    let browser_label = browser_webview_label(&owner_label);
    let webview = app
        .get_webview(&browser_label)
        .ok_or_else(|| "browser has not opened a page yet".to_string())?;
    webview
        .eval("history.back()")
        .map_err(|_| "browser could not go back".to_string())?;
    Ok(browser_state(&app, &owner_label))
}

#[cfg(feature = "desktop")]
pub fn reload(app: AppHandle, owner_label: String) -> Result<BrowserState, String> {
    let browser_label = browser_webview_label(&owner_label);
    let webview = app
        .get_webview(&browser_label)
        .ok_or_else(|| "browser has not opened a page yet".to_string())?;
    webview
        .reload()
        .map_err(|_| "browser could not reload".to_string())?;
    Ok(browser_state(&app, &owner_label))
}

#[cfg(feature = "desktop")]
async fn evaluate(app: AppHandle, owner_label: String, script: String) -> Result<Value, String> {
    let browser_label = browser_webview_label(&owner_label);
    let webview = app
        .get_webview(&browser_label)
        .ok_or_else(|| "browser has not opened a page yet".to_string())?;
    let current_url = webview
        .url()
        .map_err(|_| "browser page URL is unavailable".to_string())?;
    // Re-resolve immediately before any Agent-visible read or action. The
    // navigation callback below performs the same fail-closed check for every
    // redirect/click navigation, while this closes the ordinary window between
    // page load and snapshot/click/type as much as WebView2 permits.
    if !allowed_browser_navigation(&current_url) {
        return Err("browser page no longer resolves only to public addresses".to_string());
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    // Tauri retains an `Fn` callback and may invoke it more than once. Keep
    // the one-shot sender in an Option so only the first result is delivered
    // without moving it out of the callback capture.
    let sender = Arc::new(Mutex::new(Some(sender)));
    webview
        .eval_with_callback(script, move |result| {
            let sender = sender.lock().ok().and_then(|mut sender| sender.take());
            if let Some(sender) = sender {
                let _ = sender.send(result);
            }
        })
        .map_err(|_| "browser page script could not start".to_string())?;
    let raw = tokio::time::timeout(EVALUATION_TIMEOUT, receiver)
        .await
        .map_err(|_| "browser page script timed out".to_string())?
        .map_err(|_| "browser page script did not return a result".to_string())?;
    serde_json::from_str(&raw).map_err(|_| "browser page returned an invalid result".to_string())
}

#[cfg(feature = "desktop")]
const INTERACTIVES_SCRIPT: &str = r#"
  const compact = (value, limit) => String(value || '').replace(/\s+/g, ' ').trim().slice(0, limit);
  const elements = Array.from(document.querySelectorAll('a,button,input,textarea,select,[role="button"],[role="link"],[contenteditable="true"]'))
    .filter((element) => {
      if (!(element instanceof HTMLElement) || element.getClientRects().length === 0) return false;
      if (element instanceof HTMLInputElement && element.type === 'hidden') return false;
      return !element.matches('[disabled],[aria-disabled="true"]');
    });
  const describeInteractive = (element) => ({
    tag: element.tagName.toLowerCase(),
    role: element.getAttribute('role') || element.tagName.toLowerCase(),
    text: compact(element.getAttribute('aria-label') || element.getAttribute('title') || element.textContent || element.getAttribute('placeholder') || '', 240),
    type: element instanceof HTMLInputElement ? element.type : undefined,
    href: element instanceof HTMLAnchorElement ? compact(element.href, 512) : undefined
  });
  const elementFingerprint = (element) => JSON.stringify(describeInteractive(element));
  const isBlockedInput = (element) => element instanceof HTMLInputElement
    && ['password', 'file', 'hidden', 'checkbox', 'radio', 'button', 'submit', 'reset', 'image'].includes(element.type);
"#;

#[cfg(feature = "desktop")]
pub async fn snapshot(app: AppHandle, owner_label: String) -> Result<Value, String> {
    evaluate(
        app,
        owner_label,
        format!(
            r#"(() => {{
              try {{
                {INTERACTIVES_SCRIPT}
                const text = compact(document.body?.innerText || '', 12000);
                return {{
                  untrustedPageContent: true,
                  url: location.href,
                  title: compact(document.title, 300),
                  text,
                  interactives: elements.slice(0, 80).map((element, index) => ({{
                    ref: `r${{index + 1}}`,
                    ...describeInteractive(element),
                    fingerprint: elementFingerprint(element)
                  }}))
                }};
              }} catch (error) {{
                return {{ ok: false, error: String(error).slice(0, 240) }};
              }}
            }})()"#
        ),
    )
    .await
}

#[cfg(feature = "desktop")]
fn browser_ref(value: &str) -> Result<usize, String> {
    let number = value
        .strip_prefix('r')
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=80).contains(value))
        .ok_or_else(|| "browser element reference is invalid".to_string())?;
    Ok(number)
}

#[cfg(feature = "desktop")]
fn encode_expected_fingerprint(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_BROWSER_FINGERPRINT_BYTES {
        return Err("browser element fingerprint is invalid or exceeds the limit".to_string());
    }
    let parsed = serde_json::from_str::<Value>(trimmed)
        .map_err(|_| "browser element fingerprint is invalid".to_string())?;
    if !parsed.is_object() {
        return Err("browser element fingerprint is invalid".to_string());
    }
    serde_json::to_string(trimmed)
        .map_err(|_| "browser element fingerprint encoding failed".to_string())
}

#[cfg(feature = "desktop")]
pub async fn click(
    app: AppHandle,
    owner_label: String,
    reference: String,
    expected_url: String,
    expected_fingerprint: String,
) -> Result<Value, String> {
    let index = browser_ref(&reference)?;
    let expected_url = browser_url(&expected_url)?.to_string();
    let expected_url = serde_json::to_string(&expected_url)
        .map_err(|_| "browser URL encoding failed".to_string())?;
    let expected_fingerprint = encode_expected_fingerprint(&expected_fingerprint)?;
    evaluate(
        app,
        owner_label,
        format!(
            r#"(() => {{
              try {{
                if (location.href !== {expected_url}) return {{ ok: false, error: 'page changed; take a new browser snapshot' }};
                {INTERACTIVES_SCRIPT}
                const element = elements[{index_minus_one}];
                if (!element) return {{ ok: false, error: 'element is no longer available; take a new browser snapshot' }};
                if (elementFingerprint(element) !== {expected_fingerprint}) return {{ ok: false, error: 'element changed; take a new browser snapshot' }};
                if (isBlockedInput(element)) return {{ ok: false, error: 'this input cannot be clicked by the agent' }};
                element.scrollIntoView({{ block: 'center', inline: 'nearest' }});
                element.focus();
                element.click();
                return {{ ok: true, action: 'clicked', ref: 'r{index}' }};
              }} catch (error) {{
                return {{ ok: false, error: String(error).slice(0, 240) }};
              }}
            }})()"#,
            index_minus_one = index - 1,
        ),
    )
    .await
}

#[cfg(feature = "desktop")]
pub async fn type_text(
    app: AppHandle,
    owner_label: String,
    reference: String,
    expected_url: String,
    expected_fingerprint: String,
    text: String,
) -> Result<Value, String> {
    let index = browser_ref(&reference)?;
    if text.chars().count() > MAX_BROWSER_INPUT_CHARS || text.chars().any(char::is_control) {
        return Err("browser text input is invalid or exceeds the limit".to_string());
    }
    let expected_url = browser_url(&expected_url)?.to_string();
    let expected_url = serde_json::to_string(&expected_url)
        .map_err(|_| "browser URL encoding failed".to_string())?;
    let expected_fingerprint = encode_expected_fingerprint(&expected_fingerprint)?;
    let text =
        serde_json::to_string(&text).map_err(|_| "browser text encoding failed".to_string())?;
    evaluate(
        app,
        owner_label,
        format!(
            r#"(() => {{
              try {{
                if (location.href !== {expected_url}) return {{ ok: false, error: 'page changed; take a new browser snapshot' }};
                {INTERACTIVES_SCRIPT}
                const element = elements[{index_minus_one}];
                if (!element) return {{ ok: false, error: 'element is no longer available; take a new browser snapshot' }};
                if (elementFingerprint(element) !== {expected_fingerprint}) return {{ ok: false, error: 'element changed; take a new browser snapshot' }};
                if (element instanceof HTMLInputElement) {{
                  if (isBlockedInput(element)) {{
                    return {{ ok: false, error: 'this input type cannot be filled by the agent' }};
                  }}
                  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
                  if (setter) setter.call(element, {text}); else element.value = {text};
                }} else if (element instanceof HTMLTextAreaElement) {{
                  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
                  if (setter) setter.call(element, {text}); else element.value = {text};
                }} else if (element instanceof HTMLElement && element.isContentEditable) {{
                  element.textContent = {text};
                }} else {{
                  return {{ ok: false, error: 'element is not a supported text input' }};
                }}
                element.focus();
                element.dispatchEvent(new Event('input', {{ bubbles: true }}));
                element.dispatchEvent(new Event('change', {{ bubbles: true }}));
                return {{ ok: true, action: 'typed', ref: 'r{index}' }};
              }} catch (error) {{
                return {{ ok: false, error: String(error).slice(0, 240) }};
              }}
            }})()"#,
            index_minus_one = index - 1,
        ),
    )
    .await
}

// The native test profile purposefully omits Wry/WebView2. Keep the command
// contract present there so pure native tests exercise the same backend
// registration without accidentally loading a desktop event loop.
#[cfg(not(feature = "desktop"))]
fn browser_unavailable() -> String {
    "native browser requires the desktop WebView runtime".to_string()
}

#[cfg(not(feature = "desktop"))]
pub fn status(_app: AppHandle, _owner_label: String) -> BrowserState {
    BrowserState {
        available: false,
        url: None,
    }
}

#[cfg(not(feature = "desktop"))]
pub async fn open(
    _app: AppHandle,
    _owner_label: String,
    _url: String,
) -> Result<BrowserState, String> {
    Err(browser_unavailable())
}

#[cfg(not(feature = "desktop"))]
pub fn layout(
    _app: AppHandle,
    _owner_label: String,
    _viewport: BrowserViewport,
) -> Result<BrowserState, String> {
    Ok(BrowserState {
        available: false,
        url: None,
    })
}

#[cfg(not(feature = "desktop"))]
pub fn back(_app: AppHandle, _owner_label: String) -> Result<BrowserState, String> {
    Err(browser_unavailable())
}

#[cfg(not(feature = "desktop"))]
pub fn reload(_app: AppHandle, _owner_label: String) -> Result<BrowserState, String> {
    Err(browser_unavailable())
}

#[cfg(not(feature = "desktop"))]
pub async fn snapshot(_app: AppHandle, _owner_label: String) -> Result<Value, String> {
    Err(browser_unavailable())
}

#[cfg(not(feature = "desktop"))]
pub async fn click(
    _app: AppHandle,
    _owner_label: String,
    _reference: String,
    _expected_url: String,
    _expected_fingerprint: String,
) -> Result<Value, String> {
    Err(browser_unavailable())
}

#[cfg(not(feature = "desktop"))]
pub async fn type_text(
    _app: AppHandle,
    _owner_label: String,
    _reference: String,
    _expected_url: String,
    _expected_fingerprint: String,
    _text: String,
) -> Result<Value, String> {
    Err(browser_unavailable())
}

#[cfg(test)]
mod tests {
    use super::{
        allowed_browser_navigation_with_resolver, browser_host_is_public_with_resolver,
        browser_url_with_resolver, MAX_BROWSER_RESOLVED_ADDRESSES,
    };
    use std::net::IpAddr;
    use tauri::Url;

    fn addresses(values: &[&str]) -> Result<Vec<IpAddr>, ()> {
        values
            .iter()
            .map(|value| value.parse::<IpAddr>().map_err(|_| ()))
            .collect()
    }

    #[test]
    fn accepts_credential_free_public_http_urls() {
        let public_dns = |_host: &str| addresses(&["93.184.216.34", "2606:4700:4700::1111"]);
        for url in [
            "https://example.com/path?q=1",
            "http://8.8.8.8/",
            "https://[2606:4700:4700::1111]/",
        ] {
            assert!(browser_url_with_resolver(url, &public_dns).is_ok(), "{url}");
        }
    }

    #[test]
    fn rejects_local_private_and_special_use_browser_targets() {
        for url in [
            "http://localhost:3000/",
            "http://tauri.localhost/",
            "http://service.localhost/",
            "http://printer.local/",
            "http://0.0.0.1/",
            "http://10.0.0.1/",
            "http://100.64.0.1/",
            "http://127.0.0.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://172.16.0.1/",
            "http://192.0.0.1/",
            "http://192.0.2.1/",
            "http://192.168.1.1/",
            "http://198.18.0.1/",
            "http://198.51.100.1/",
            "http://203.0.113.1/",
            "http://192.31.196.1/",
            "http://192.52.193.1/",
            "http://192.88.99.1/",
            "http://192.175.48.1/",
            "http://224.0.0.1/",
            "http://240.0.0.1/",
            "http://255.255.255.255/",
            "http://[::]/",
            "http://[::1]/",
            "http://[fc00::1]/",
            "http://[fe80::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:8.8.8.8]/",
            "http://[64:ff9b::1]/",
            "http://[64:ff9b:1::1]/",
            "http://[100::1]/",
            "http://[2001::1]/",
            "http://[2002::1]/",
            "http://[3fff::1]/",
            "http://[5f00::1]/",
            "http://[4000::1]/",
            "http://[fec0::1]/",
            "http://[ff02::1]/",
            "http://[2620:4f:8000::1]/",
        ] {
            assert!(
                browser_url_with_resolver(url, &|_| addresses(&["93.184.216.34"])).is_err(),
                "{url}"
            );
        }
    }

    #[test]
    fn dns_names_fail_closed_and_reject_every_mixed_private_answer() {
        let target = Url::parse("http://127.0.0.1.nip.io/").unwrap();
        assert!(!browser_host_is_public_with_resolver(&target, &|_| Err(())));
        assert!(!browser_host_is_public_with_resolver(&target, &|_| Ok(
            Vec::new()
        )));
        assert!(!browser_host_is_public_with_resolver(&target, &|_| {
            addresses(&["93.184.216.34", "127.0.0.1"])
        }));
        assert!(!browser_host_is_public_with_resolver(&target, &|_| {
            addresses(&["2606:4700:4700::1111", "fe80::1"])
        }));
        assert!(!browser_host_is_public_with_resolver(&target, &|_| {
            Ok(vec![
                "93.184.216.34".parse().unwrap();
                MAX_BROWSER_RESOLVED_ADDRESSES + 1
            ])
        }));
        assert!(
            browser_url_with_resolver(target.as_str(), &|_| { addresses(&["127.0.0.1"]) }).is_err()
        );
        assert!(browser_host_is_public_with_resolver(&target, &|_| {
            addresses(&["93.184.216.34", "2606:4700:4700::1111"])
        }));
    }

    #[test]
    fn navigation_policy_rechecks_redirect_destinations() {
        let public = Url::parse("https://example.com/next").unwrap();
        let rebinding = Url::parse("http://localtest.me/admin").unwrap();
        let unresolved = Url::parse("https://unresolved.example/next").unwrap();
        let local = Url::parse("https://localhost/next").unwrap();
        let private = Url::parse("http://192.168.1.10/next").unwrap();
        let credentials = Url::parse("https://user:password@example.com/").unwrap();
        let resolver = |host: &str| match host {
            "example.com" => addresses(&["93.184.216.34", "2606:4700:4700::1111"]),
            "localtest.me" => addresses(&["127.0.0.1"]),
            _ => Err(()),
        };

        assert!(allowed_browser_navigation_with_resolver(&public, &resolver));
        assert!(!allowed_browser_navigation_with_resolver(
            &rebinding, &resolver
        ));
        assert!(!allowed_browser_navigation_with_resolver(
            &unresolved,
            &resolver
        ));
        assert!(!allowed_browser_navigation_with_resolver(&local, &resolver));
        assert!(!allowed_browser_navigation_with_resolver(
            &private, &resolver
        ));
        assert!(!allowed_browser_navigation_with_resolver(
            &credentials,
            &resolver
        ));
    }
}
