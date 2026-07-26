//! Native MCP client runtime.
//!
//! This module is deliberately independent from the renderer.  Callers pass a
//! complete configuration assembled by native settings code; secrets remain in
//! this module and are never included in diagnostics or `Debug` output.

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::{Client, Response, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex as StdMutex};
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const MAX_TIMEOUT_MS: u64 = 10 * 60_000;
const MAX_RPC_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_RPC_REQUEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_HEADER_VALUE_BYTES: usize = 16 * 1024;
const MAX_STDIO_HEADER_LINE_BYTES: usize = 16 * 1024;
const MAX_SSE_EVENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_TOOL_COUNT: usize = 512;
const MAX_TOOL_NAME_BYTES: usize = 512;
const MAX_TOOL_DESCRIPTION_BYTES: usize = 64 * 1024;
const MAX_TOOL_SCHEMA_BYTES: usize = 512 * 1024;
const MAX_CONTENT_ITEMS: usize = 256;
const MAX_ID_BYTES: usize = 128;
const MAX_COMMAND_BYTES: usize = 32 * 1024;
const MAX_ARGUMENT_COUNT: usize = 256;
const MAX_ARGUMENT_BYTES: usize = 64 * 1024;
const STDIO_CHANNEL_CAPACITY: usize = 64;
const LEGACY_ENDPOINT_WAIT: Duration = Duration::from_secs(5);

fn default_timeout_ms() -> Option<u64> {
    Some(DEFAULT_TIMEOUT_MS)
}

fn default_allow_remote() -> bool {
    false
}

/// Native MCP server settings.  The env/header values are intentionally not
/// printed by the custom `Debug` implementation below.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    pub id: String,
    pub enabled: bool,
    pub transport: Option<String>,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: Option<BTreeMap<String, String>>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub message_url: Option<String>,
    /// HTTP/SSE connections are localhost-only unless this is explicitly set.
    #[serde(default = "default_allow_remote")]
    pub allow_remote: bool,
    /// `jsonl` is the MCP default. `content-length` is useful for servers
    /// which implement the older LSP-style stdio framing.
    #[serde(default)]
    pub stdio_framing: Option<String>,
}

impl fmt::Debug for McpServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpServerConfig")
            .field("id", &self.id)
            .field("enabled", &self.enabled)
            .field("transport", &self.transport)
            .field("command", &self.command)
            .field("args", &format_args!("<{} args>", self.args.len()))
            .field(
                "env",
                &self
                    .env
                    .as_ref()
                    .map(|v| format!("<{} secret entries>", v.len())),
            )
            .field("cwd", &self.cwd)
            .field("url", &self.url.as_ref().map(|_| "<configured>"))
            .field(
                "headers",
                &self
                    .headers
                    .as_ref()
                    .map(|v| format!("<{} secret entries>", v.len())),
            )
            .field("timeout_ms", &self.timeout_ms)
            .field(
                "message_url",
                &self.message_url.as_ref().map(|_| "<configured>"),
            )
            .field("allow_remote", &self.allow_remote)
            .field("stdio_framing", &self.stdio_framing)
            .finish()
    }
}

impl McpServerConfig {
    fn transport_name(&self) -> Result<TransportKind, String> {
        match self
            .transport
            .as_deref()
            .unwrap_or("stdio")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "stdio" | "command" => Ok(TransportKind::Stdio),
            "http" | "streamable-http" | "streamablehttp" => Ok(TransportKind::Http),
            "sse" | "legacy-sse" | "legacy_sse" => Ok(TransportKind::Sse),
            _ => Err("unsupported MCP transport".to_string()),
        }
    }

    fn timeout(&self) -> Result<Duration, String> {
        let value = self.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        if !(1..=MAX_TIMEOUT_MS).contains(&value) {
            return Err(format!(
                "MCP timeout must be between 1 and {MAX_TIMEOUT_MS} milliseconds"
            ));
        }
        Ok(Duration::from_millis(value))
    }

    fn validate(&self) -> Result<ValidatedConfig, String> {
        if self.id.trim().is_empty()
            || self.id.len() > MAX_ID_BYTES
            || self.id.chars().any(|c| c.is_control())
        {
            return Err("MCP server id is invalid".to_string());
        }
        let transport = self.transport_name()?;
        let timeout = self.timeout()?;
        let command = self.command.trim().to_string();
        if transport == TransportKind::Stdio {
            if command.is_empty() || command.len() > MAX_COMMAND_BYTES {
                return Err("MCP stdio command is required and bounded".to_string());
            }
            validate_text(&command, MAX_COMMAND_BYTES, "command")?;
            if self.args.len() > MAX_ARGUMENT_COUNT {
                return Err("MCP stdio argument count is too large".to_string());
            }
            for arg in &self.args {
                validate_text(arg, MAX_ARGUMENT_BYTES, "argument")?;
            }
            if self
                .message_url
                .as_deref()
                .is_some_and(|v| !v.trim().is_empty())
            {
                return Err("messageUrl is only valid for SSE transport".to_string());
            }
        }

        let cwd = self
            .cwd
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        if let Some(path) = &cwd {
            validate_text(&path.to_string_lossy(), MAX_COMMAND_BYTES, "cwd")?;
        }

        validate_environment(self.env.as_ref())?;
        let headers = build_header_map(self.headers.as_ref())?;

        let endpoint = match transport {
            TransportKind::Stdio => None,
            TransportKind::Http | TransportKind::Sse => {
                let raw = self
                    .url
                    .as_deref()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| "MCP HTTP/SSE transport requires url".to_string())?;
                Some(validate_url(raw, self.allow_remote, "url")?)
            }
        };

        let message_url = if transport == TransportKind::Sse {
            match self
                .message_url
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
            {
                None => None,
                Some(raw) => {
                    let base = endpoint
                        .as_ref()
                        .ok_or_else(|| "SSE messageUrl requires url".to_string())?;
                    let parsed = Url::parse(raw)
                        .or_else(|_| base.join(raw))
                        .map_err(|_| "MCP messageUrl is invalid".to_string())?;
                    Some(validate_same_origin(
                        base,
                        &parsed,
                        self.allow_remote,
                        "messageUrl",
                    )?)
                }
            }
        } else {
            None
        };

        let framing = match self
            .stdio_framing
            .as_deref()
            .unwrap_or("jsonl")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "jsonl" | "json-lines" | "line" => StdioFraming::JsonLines,
            "content-length" | "contentlength" | "lsp" => StdioFraming::ContentLength,
            _ => return Err("unsupported MCP stdio framing".to_string()),
        };

        Ok(ValidatedConfig {
            id: self.id.trim().to_string(),
            enabled: self.enabled,
            transport,
            command,
            args: self.args.clone(),
            env: self.env.clone().unwrap_or_default(),
            cwd,
            endpoint,
            message_url,
            headers,
            timeout,
            allow_remote: self.allow_remote,
            framing,
        })
    }

    /// Validate a persisted configuration and write it back in the one shape
    /// the runtime consumes.  Keeping this beside `validate` prevents the
    /// settings boundary from accepting aliases or values which only fail
    /// when a server is first used.
    pub(crate) fn normalised_for_settings(&self) -> Result<Self, String> {
        let validated = self.validate()?;
        let transport = validated.transport;
        let is_stdio = transport == TransportKind::Stdio;
        let is_sse = transport == TransportKind::Sse;

        let headers = if validated.headers.is_empty() {
            None
        } else {
            let mut entries = BTreeMap::new();
            for (name, value) in &validated.headers {
                let value = value
                    .to_str()
                    .map_err(|_| "MCP header value is invalid".to_string())?;
                entries.insert(name.as_str().to_string(), value.to_string());
            }
            Some(entries)
        };

        Ok(Self {
            id: validated.id,
            enabled: validated.enabled,
            transport: Some(transport.as_str().to_string()),
            command: if is_stdio {
                validated.command
            } else {
                String::new()
            },
            args: if is_stdio { validated.args } else { Vec::new() },
            env: (!validated.env.is_empty()).then_some(validated.env),
            cwd: validated
                .cwd
                .map(|path| path.to_string_lossy().into_owned()),
            url: validated.endpoint.map(|url| url.to_string()),
            headers,
            timeout_ms: Some(validated.timeout.as_millis() as u64),
            message_url: is_sse
                .then_some(validated.message_url)
                .flatten()
                .map(|url| url.to_string()),
            allow_remote: !is_stdio && validated.allow_remote,
            stdio_framing: is_stdio.then(|| validated.framing.as_str().to_string()),
        })
    }
}

fn validate_text(value: &str, max: usize, label: &str) -> Result<(), String> {
    if value.len() > max || value.as_bytes().contains(&0) || value.chars().any(|c| c.is_control()) {
        return Err(format!("MCP {label} contains invalid or oversized text"));
    }
    Ok(())
}

fn validate_environment(env: Option<&BTreeMap<String, String>>) -> Result<(), String> {
    let Some(env) = env else {
        return Ok(());
    };
    if env.len() > MAX_ARGUMENT_COUNT {
        return Err("MCP environment entry count is too large".to_string());
    }
    let mut total = 0usize;
    for (key, value) in env {
        validate_env_key(key)?;
        validate_text(value, MAX_ARGUMENT_BYTES, "environment value")?;
        total = total.saturating_add(key.len()).saturating_add(value.len());
    }
    if total > MAX_HEADER_BYTES {
        return Err("MCP environment is too large".to_string());
    }
    Ok(())
}

fn validate_env_key(key: &str) -> Result<(), String> {
    if key.is_empty()
        || key.len() > 256
        || key.as_bytes().contains(&0)
        || key.chars().any(|c| c.is_control() || c == '=')
    {
        return Err("MCP environment key is invalid".to_string());
    }
    Ok(())
}

fn validate_url(raw: &str, allow_remote: bool, label: &str) -> Result<Url, String> {
    if raw.len() > MAX_COMMAND_BYTES || raw.chars().any(|c| c.is_control()) {
        return Err(format!("MCP {label} is invalid"));
    }
    let url = Url::parse(raw).map_err(|_| format!("MCP {label} is invalid"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.username() != ""
        || url.password().is_some()
        || url.host_str().is_none()
        || url.fragment().is_some()
    {
        return Err(format!(
            "MCP {label} must be an HTTP(S) URL without credentials"
        ));
    }
    if !allow_remote && !is_local_host(url.host_str().unwrap_or_default()) {
        return Err(format!(
            "MCP {label} is remote; enable allowRemote explicitly"
        ));
    }
    Ok(url)
}

fn validate_same_origin(
    base: &Url,
    endpoint: &Url,
    allow_remote: bool,
    label: &str,
) -> Result<Url, String> {
    let endpoint = validate_url(endpoint.as_str(), allow_remote, label)?;
    let same = base.scheme().eq_ignore_ascii_case(endpoint.scheme())
        && base.host_str().map(|v| v.to_ascii_lowercase())
            == endpoint.host_str().map(|v| v.to_ascii_lowercase())
        && base.port_or_known_default() == endpoint.port_or_known_default();
    if !same {
        return Err(format!("MCP {label} must use the MCP server origin"));
    }
    Ok(endpoint)
}

fn is_local_host(host: &str) -> bool {
    let lower = host.trim_end_matches('.').to_ascii_lowercase();
    lower == "localhost" || lower == "127.0.0.1" || lower == "::1"
}

fn build_header_map(headers: Option<&BTreeMap<String, String>>) -> Result<HeaderMap, String> {
    let mut result = HeaderMap::new();
    let Some(headers) = headers else {
        return Ok(result);
    };
    if headers.len() > MAX_ARGUMENT_COUNT {
        return Err("MCP header count is too large".to_string());
    }
    let mut total = 0usize;
    let mut names = HashSet::with_capacity(headers.len());
    for (raw_name, raw_value) in headers {
        let name = HeaderName::from_bytes(raw_name.as_bytes())
            .map_err(|_| "MCP header name is invalid".to_string())?;
        let lower = name.as_str().to_ascii_lowercase();
        if !names.insert(lower.clone()) {
            return Err("MCP headers contain duplicate names".to_string());
        }
        if matches!(
            lower.as_str(),
            "host"
                | "content-length"
                | "connection"
                | "transfer-encoding"
                | "upgrade"
                | "accept"
                | "content-type"
                | "mcp-session-id"
                | "mcp-protocol-version"
        ) {
            return Err("MCP header is reserved by the protocol".to_string());
        }
        if raw_value.len() > MAX_HEADER_VALUE_BYTES {
            return Err("MCP header value is oversized".to_string());
        }
        let value = HeaderValue::from_str(raw_value)
            .map_err(|_| "MCP header value is invalid".to_string())?;
        total = total
            .saturating_add(raw_name.len())
            .saturating_add(raw_value.len());
        result.insert(name, value);
    }
    if total > MAX_HEADER_BYTES {
        return Err("MCP headers are oversized".to_string());
    }
    Ok(result)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransportKind {
    Stdio,
    Http,
    Sse,
}

impl TransportKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
            Self::Sse => "sse",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StdioFraming {
    JsonLines,
    ContentLength,
}

impl StdioFraming {
    fn as_str(self) -> &'static str {
        match self {
            Self::JsonLines => "jsonl",
            Self::ContentLength => "content-length",
        }
    }
}

#[derive(Clone)]
struct ValidatedConfig {
    id: String,
    enabled: bool,
    transport: TransportKind,
    command: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    cwd: Option<PathBuf>,
    endpoint: Option<Url>,
    message_url: Option<Url>,
    headers: HeaderMap,
    timeout: Duration,
    allow_remote: bool,
    framing: StdioFraming,
}

impl fmt::Debug for ValidatedConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ValidatedConfig")
            .field("id", &self.id)
            .field("transport", &self.transport)
            .field("command", &self.command)
            .field("args", &format_args!("<{} args>", self.args.len()))
            .field("env", &format_args!("<{} secret entries>", self.env.len()))
            .field("endpoint", &self.endpoint.as_ref().map(|_| "<configured>"))
            .field(
                "message_url",
                &self.message_url.as_ref().map(|_| "<configured>"),
            )
            .field(
                "headers",
                &format_args!("<{} secret entries>", self.headers.len()),
            )
            .field("timeout", &self.timeout)
            .field("allow_remote", &self.allow_remote)
            .field("framing", &self.framing)
            .finish()
    }
}

/// Hashes all configuration values so changing a credential also evicts the
/// old client.  Only the opaque hash is retained; no secret is formatted.
fn config_fingerprint(config: &McpServerConfig) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    config.id.hash(&mut hasher);
    config.enabled.hash(&mut hasher);
    config.transport.hash(&mut hasher);
    config.command.hash(&mut hasher);
    config.args.hash(&mut hasher);
    config.env.hash(&mut hasher);
    config.cwd.hash(&mut hasher);
    config.url.hash(&mut hasher);
    config.headers.hash(&mut hasher);
    config.timeout_ms.hash(&mut hasher);
    config.message_url.hash(&mut hasher);
    config.allow_remote.hash(&mut hasher);
    config.stdio_framing.hash(&mut hasher);
    hasher.finish()
}

// -------------------------------------------------------------------------
// Public DTOs

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolInfo {
    pub server_id: String,
    pub server_label: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCallToolRequest {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(flatten)]
    pub fields: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCallToolResponse {
    pub content: Vec<McpContent>,
    pub is_error: bool,
    pub details: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRuntimeStatus {
    pub server_id: String,
    pub running: bool,
    pub initialized: bool,
    pub transport: String,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStopServerResponse {
    pub server_id: String,
    pub stopped: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpDiagnosticToolInfo {
    pub server_id: String,
    pub server_label: String,
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRuntimeTestResponse {
    pub server_id: String,
    pub ok: bool,
    pub phase: String,
    pub transport: String,
    pub duration_ms: u128,
    pub running: bool,
    pub initialized: bool,
    pub tools_count: usize,
    pub tools: Vec<McpDiagnosticToolInfo>,
    pub error: Option<String>,
    pub stderr_tail: Option<String>,
}

// -------------------------------------------------------------------------
// Bounded stdio transport

enum StdioEvent {
    Message(Value),
    Failure(String),
    Closed,
}

fn truncate_utf8(value: &str, max: usize) -> &str {
    if value.len() <= max {
        return value;
    }
    let mut end = max;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn read_bounded_line<R: BufRead>(reader: &mut R, max: usize) -> io::Result<Option<Vec<u8>>> {
    let mut output = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return if output.is_empty() {
                Ok(None)
            } else {
                Ok(Some(output))
            };
        }
        let take = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(buffer.len());
        if output.len().saturating_add(take) > max {
            reader.consume(take);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MCP stdio line exceeds the configured limit",
            ));
        }
        output.extend_from_slice(&buffer[..take]);
        reader.consume(take);
        if output.last() == Some(&b'\n') {
            return Ok(Some(output));
        }
    }
}

fn strip_line_ending(mut line: &[u8]) -> &[u8] {
    if line.last() == Some(&b'\n') {
        line = &line[..line.len() - 1];
    }
    if line.last() == Some(&b'\r') {
        line = &line[..line.len() - 1];
    }
    line
}

fn content_length_header(line: &[u8]) -> Option<Result<usize, String>> {
    let line = strip_line_ending(line);
    let colon = line.iter().position(|byte| *byte == b':')?;
    if !line[..colon].eq_ignore_ascii_case(b"content-length") {
        return None;
    }
    let raw = std::str::from_utf8(&line[colon + 1..]).ok()?.trim();
    Some(
        raw.parse::<usize>()
            .map_err(|_| "invalid MCP Content-Length header".to_string()),
    )
}

fn read_stdio_payload<R: BufRead>(reader: &mut R) -> Result<Option<Vec<u8>>, String> {
    loop {
        let first = match read_bounded_line(reader, MAX_RPC_MESSAGE_BYTES + 1)
            .map_err(|_| "failed to read bounded MCP stdio output".to_string())?
        {
            Some(line) => line,
            None => return Ok(None),
        };
        if strip_line_ending(&first).is_empty() {
            continue;
        }

        if let Some(length) = content_length_header(&first) {
            let length = length?;
            if length == 0 || length > MAX_RPC_MESSAGE_BYTES {
                return Err("MCP Content-Length is outside the allowed range".to_string());
            }
            let mut header_bytes = first.len();
            loop {
                let line = read_bounded_line(reader, MAX_STDIO_HEADER_LINE_BYTES)
                    .map_err(|_| "failed to read MCP stdio headers".to_string())?
                    .ok_or_else(|| "MCP stdio headers ended unexpectedly".to_string())?;
                header_bytes = header_bytes.saturating_add(line.len());
                if header_bytes > MAX_HEADER_BYTES {
                    return Err("MCP stdio headers are oversized".to_string());
                }
                if strip_line_ending(&line).is_empty() {
                    break;
                }
                if content_length_header(&line).is_some() {
                    return Err("duplicate MCP Content-Length header".to_string());
                }
            }
            let mut payload = vec![0u8; length];
            reader
                .read_exact(&mut payload)
                .map_err(|_| "MCP stdio body ended unexpectedly".to_string())?;
            return Ok(Some(payload));
        }

        if first.len() > MAX_RPC_MESSAGE_BYTES {
            return Err("MCP JSON line is oversized".to_string());
        }
        return Ok(Some(strip_line_ending(&first).to_vec()));
    }
}

fn parse_stdio_messages<R: Read>(source: R, sender: mpsc::Sender<StdioEvent>) {
    let mut reader = BufReader::new(source);
    loop {
        match read_stdio_payload(&mut reader) {
            Ok(Some(payload)) => match serde_json::from_slice::<Value>(&payload) {
                Ok(message) => {
                    if sender.blocking_send(StdioEvent::Message(message)).is_err() {
                        return;
                    }
                }
                Err(_) => {
                    let _ = sender.blocking_send(StdioEvent::Failure(
                        "MCP stdio emitted invalid JSON".to_string(),
                    ));
                    return;
                }
            },
            Ok(None) => {
                let _ = sender.blocking_send(StdioEvent::Closed);
                return;
            }
            Err(error) => {
                let _ = sender.blocking_send(StdioEvent::Failure(error));
                return;
            }
        }
    }
}

#[cfg(windows)]
fn is_windows_batch_program(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("cmd") || value.eq_ignore_ascii_case("bat"))
}

#[cfg(windows)]
fn windows_cmd_quote_arg(value: &str) -> Result<String, String> {
    if value
        .chars()
        .any(|c| matches!(c, '\0' | '\r' | '\n' | '"' | '%' | '!'))
    {
        return Err("unsafe character in Windows batch argument".to_string());
    }
    // Metacharacters are inert inside quotes. /V:OFF prevents delayed `!`
    // expansion, and `%`/quotes are rejected above because cmd.exe expands
    // them even in quoted strings.
    Ok(format!("\"{value}\""))
}

#[cfg(windows)]
fn windows_batch_command_line(program: &Path, args: &[String]) -> Result<String, String> {
    let program = program.to_string_lossy().into_owned();
    std::iter::once(program.as_str())
        .chain(args.iter().map(String::as_str))
        .map(windows_cmd_quote_arg)
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join(" "))
}

fn build_stdio_command(config: &ValidatedConfig) -> Result<Command, String> {
    let program = Path::new(&config.command);
    #[cfg(windows)]
    let mut command = if is_windows_batch_program(program) {
        let mut command = Command::new("cmd.exe");
        command
            .arg("/D")
            .arg("/V:OFF")
            .arg("/S")
            .arg("/C")
            .arg(windows_batch_command_line(program, &config.args)?);
        command
    } else {
        let mut command = Command::new(program);
        command.args(&config.args);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new(program);
        command.args(&config.args);
        command
    };

    // Do not give a third-party MCP process the desktop application's ambient
    // environment. It may contain cloud, CI, package-registry, or proxy
    // credentials unrelated to this configured server. Keep only the small
    // launch baseline required to resolve normal programs, then layer the
    // explicitly configured MCP environment on top.
    command.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    let temporary_directory = std::env::temp_dir();
    command
        .env("TMPDIR", &temporary_directory)
        .env("TMP", &temporary_directory)
        .env("TEMP", &temporary_directory);
    #[cfg(windows)]
    for key in ["SystemRoot", "WINDIR", "ComSpec", "PATHEXT"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(&config.env);
    if let Some(cwd) = &config.cwd {
        command.current_dir(cwd);
    }
    configure_child_process_group(&mut command);
    Ok(command)
}

#[cfg(windows)]
fn configure_child_process_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_child_process_group(_command: &mut Command) {}

fn terminate_child_tree_sync(child: &Arc<StdMutex<Option<Child>>>) {
    let Ok(mut guard) = child.lock() else {
        return;
    };
    let Some(mut child) = guard.take() else {
        return;
    };
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let taskkill = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("System32")
            .join("taskkill.exe");
        if taskkill.is_file() {
            let mut killer = Command::new(taskkill);
            killer
                .args(["/PID", &child.id().to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW);
            if let Ok(mut process) = killer.spawn() {
                let deadline = Instant::now() + Duration::from_secs(3);
                loop {
                    match process.try_wait() {
                        Ok(Some(_)) => break,
                        Ok(None) if Instant::now() < deadline => {
                            thread::sleep(Duration::from_millis(20));
                        }
                        _ => {
                            let _ = process.kill();
                            let _ = process.wait();
                            break;
                        }
                    }
                }
            }
        }
    }

    let _ = child.kill();
    let _ = child.wait();
}

struct StdioTransport {
    child: Arc<StdMutex<Option<Child>>>,
    stdin: Arc<StdMutex<Option<ChildStdin>>>,
    receiver: mpsc::Receiver<StdioEvent>,
    framing: StdioFraming,
}

impl StdioTransport {
    fn spawn(config: &ValidatedConfig) -> Result<Self, String> {
        if let Some(cwd) = &config.cwd {
            if !cwd.is_dir() {
                return Err("MCP stdio cwd does not exist or is not a directory".to_string());
            }
        }
        let mut command = build_stdio_command(config)?;
        let mut process = command
            .spawn()
            .map_err(|_| format!("failed to start MCP server {}", config.id))?;
        let stdin = match process.stdin.take() {
            Some(value) => value,
            None => {
                let _ = process.kill();
                let _ = process.wait();
                return Err("failed to open MCP server stdin".to_string());
            }
        };
        let stdout = match process.stdout.take() {
            Some(value) => value,
            None => {
                let _ = process.kill();
                let _ = process.wait();
                return Err("failed to open MCP server stdout".to_string());
            }
        };
        let stderr = match process.stderr.take() {
            Some(value) => value,
            None => {
                let _ = process.kill();
                let _ = process.wait();
                return Err("failed to open MCP server stderr".to_string());
            }
        };

        let (sender, receiver) = mpsc::channel(STDIO_CHANNEL_CAPACITY);
        thread::spawn(move || parse_stdio_messages(stdout, sender));

        // Drain stderr so a noisy child cannot block on a full pipe, but do
        // not retain or return it. A child process is not a trusted error
        // source and could otherwise exfiltrate inherited or runtime secrets
        // through renderer-visible diagnostics.
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            loop {
                match read_bounded_line(&mut reader, MAX_ARGUMENT_BYTES) {
                    Ok(Some(_)) => {}
                    Ok(None) => return,
                    Err(_) => return,
                }
            }
        });

        Ok(Self {
            child: Arc::new(StdMutex::new(Some(process))),
            stdin: Arc::new(StdMutex::new(Some(stdin))),
            receiver,
            framing: config.framing,
        })
    }

    fn is_running(&self) -> bool {
        let Ok(mut child) = self.child.lock() else {
            return false;
        };
        child
            .as_mut()
            .is_some_and(|process| matches!(process.try_wait(), Ok(None)))
    }

    async fn write_message(&self, message: &Value) -> Result<(), String> {
        let payload =
            serde_json::to_vec(message).map_err(|_| "failed to encode MCP request".to_string())?;
        if payload.len() > MAX_RPC_REQUEST_BYTES {
            return Err("MCP request is oversized".to_string());
        }
        let frame = match self.framing {
            StdioFraming::JsonLines => {
                let mut frame = payload;
                frame.push(b'\n');
                frame
            }
            StdioFraming::ContentLength => {
                let mut frame = format!("Content-Length: {}\r\n\r\n", payload.len()).into_bytes();
                frame.extend_from_slice(&payload);
                frame
            }
        };
        let stdin = self.stdin.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let mut stdin = stdin
                .lock()
                .map_err(|_| "MCP stdin lock was poisoned".to_string())?;
            let stdin = stdin
                .as_mut()
                .ok_or_else(|| "MCP server stdin is closed".to_string())?;
            stdin
                .write_all(&frame)
                .and_then(|_| stdin.flush())
                .map_err(|_| "failed to write MCP request to server".to_string())
        })
        .await
        .map_err(|_| "MCP stdin writer task failed".to_string())?
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        if !self.is_running() {
            return Err(self.with_stderr("MCP stdio server is not running"));
        }
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .await
        .map_err(|error| self.with_stderr(&error))
    }

    async fn request(
        &mut self,
        timeout: Duration,
        id: u64,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        if !self.is_running() {
            return Err(self.with_stderr("MCP stdio server is not running"));
        }
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await
        .map_err(|error| self.with_stderr(&error))?;

        let wait = async {
            loop {
                match self.receiver.recv().await {
                    Some(StdioEvent::Message(message)) => {
                        if message.get("id").is_none() {
                            continue;
                        }
                        ensure_matching_id(method, id, &message)?;
                        return Ok(message);
                    }
                    Some(StdioEvent::Failure(error)) => return Err(error),
                    Some(StdioEvent::Closed) | None => {
                        return Err("MCP stdio stream closed before a response".to_string())
                    }
                }
            }
        };
        match tokio::time::timeout(timeout, wait).await {
            Ok(result) => result.map_err(|error| self.with_stderr(&error)),
            Err(_) => {
                let error = self.with_stderr(&format!("MCP request timed out: method={method}"));
                self.stop().await;
                Err(error)
            }
        }
    }

    fn with_stderr(&self, message: &str) -> String {
        // Stderr is deliberately drained and discarded. See `spawn` above.
        message.to_string()
    }

    async fn stop(&mut self) {
        if let Ok(mut stdin) = self.stdin.lock() {
            *stdin = None;
        }
        let child = self.child.clone();
        let _ =
            tauri::async_runtime::spawn_blocking(move || terminate_child_tree_sync(&child)).await;
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        if let Ok(mut stdin) = self.stdin.lock() {
            *stdin = None;
        }
        terminate_child_tree_sync(&self.child);
    }
}

fn ensure_matching_id(method: &str, id: u64, message: &Value) -> Result<(), String> {
    if message.get("id") == Some(&Value::from(id)) {
        Ok(())
    } else {
        Err(format!("MCP response id mismatch for method {method}"))
    }
}

fn parse_jsonrpc_result(method: &str, id: u64, message: &Value) -> Result<Value, String> {
    ensure_matching_id(method, id, message)?;
    if let Some(error) = message.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(-1);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .map(redact_error_text)
            .unwrap_or_else(|| "MCP server returned an error".to_string());
        return Err(format!("MCP call failed: code={code} message={message}"));
    }
    message
        .get("result")
        .cloned()
        .ok_or_else(|| "MCP response did not contain a result".to_string())
}

fn redact_error_text(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    if [
        "authorization",
        "bearer ",
        "api_key",
        "apikey",
        "api-key",
        "access_token",
        "refresh_token",
        "password",
        "passwd",
        "secret",
        "cookie",
        "private_key",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return "[redacted sensitive MCP error]".to_string();
    }
    truncate_utf8(raw, MAX_TOOL_DESCRIPTION_BYTES).to_string()
}

#[derive(Default)]
struct SseDecoder {
    buffer: Vec<u8>,
    event_name: Option<String>,
    data_lines: Vec<String>,
    event_bytes: usize,
}

struct SseEvent {
    event_name: String,
    data: String,
}

impl SseDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, String> {
        if self.buffer.len().saturating_add(chunk.len()) > MAX_SSE_EVENT_BYTES {
            return Err("MCP SSE event buffer is oversized".to_string());
        }
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(index) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line = self.buffer.drain(..=index).collect::<Vec<_>>();
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.event_bytes = self.event_bytes.saturating_add(line.len());
            if self.event_bytes > MAX_SSE_EVENT_BYTES {
                return Err("MCP SSE event is oversized".to_string());
            }
            self.consume_line(&line, &mut events)?;
        }
        Ok(events)
    }

    fn finish(&mut self) -> Result<Vec<SseEvent>, String> {
        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            self.consume_line(&line, &mut Vec::new())?;
        }
        let mut events = Vec::new();
        self.dispatch(&mut events);
        Ok(events)
    }

    fn consume_line(&mut self, line: &[u8], events: &mut Vec<SseEvent>) -> Result<(), String> {
        if line.is_empty() {
            self.dispatch(events);
            return Ok(());
        }
        if line.first() == Some(&b':') {
            return Ok(());
        }
        let text =
            std::str::from_utf8(line).map_err(|_| "MCP SSE line is not UTF-8".to_string())?;
        if let Some(value) = text.strip_prefix("event:") {
            let value = value.trim().to_string();
            if value.len() > 256 {
                return Err("MCP SSE event name is oversized".to_string());
            }
            self.event_name = Some(value);
        } else if let Some(value) = text.strip_prefix("data:") {
            let value = value.strip_prefix(' ').unwrap_or(value).to_string();
            self.event_bytes = self.event_bytes.saturating_add(value.len());
            if self.event_bytes > MAX_SSE_EVENT_BYTES {
                return Err("MCP SSE data is oversized".to_string());
            }
            self.data_lines.push(value);
        }
        Ok(())
    }

    fn dispatch(&mut self, events: &mut Vec<SseEvent>) {
        if self.data_lines.is_empty() {
            self.event_name = None;
            self.event_bytes = 0;
            return;
        }
        events.push(SseEvent {
            event_name: self
                .event_name
                .take()
                .unwrap_or_else(|| "message".to_string()),
            data: self.data_lines.drain(..).collect::<Vec<_>>().join("\n"),
        });
        self.event_bytes = 0;
    }
}

fn validate_response_length(response: &Response) -> Result<(), String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RPC_MESSAGE_BYTES as u64)
    {
        return Err("MCP HTTP response exceeds the configured limit".to_string());
    }
    Ok(())
}

fn response_content_type(response: &Response) -> String {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

async fn read_response_bytes(mut response: Response) -> Result<Vec<u8>, String> {
    validate_response_length(&response)?;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "failed to read MCP HTTP response".to_string())?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RPC_MESSAGE_BYTES {
            return Err("MCP HTTP response exceeds the configured limit".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn read_sse_response(
    mut response: Response,
    timeout: Duration,
    id: u64,
    method: &str,
) -> Result<Value, String> {
    validate_response_length(&response)?;
    let mut decoder = SseDecoder::default();
    let read = async {
        let mut total_bytes = 0usize;
        loop {
            let chunk = response
                .chunk()
                .await
                .map_err(|_| "failed to read MCP SSE response".to_string())?;
            let Some(chunk) = chunk else {
                for event in decoder.finish()? {
                    if let Ok(value) = serde_json::from_str::<Value>(&event.data) {
                        if value.get("id").is_some() {
                            ensure_matching_id(method, id, &value)?;
                            return Ok(value);
                        }
                    }
                }
                return Err("MCP SSE response closed before the matching response".to_string());
            };
            total_bytes = total_bytes.saturating_add(chunk.len());
            if total_bytes > MAX_RPC_MESSAGE_BYTES {
                return Err("MCP SSE response exceeds the configured limit".to_string());
            }
            for event in decoder.push(&chunk)? {
                if event.data == "[DONE]" {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<Value>(&event.data) {
                    if value.get("id").is_none() {
                        continue;
                    }
                    ensure_matching_id(method, id, &value)?;
                    return Ok(value);
                }
            }
        }
    };
    tokio::time::timeout(timeout, read)
        .await
        .map_err(|_| format!("MCP SSE request timed out: method={method}"))?
}

fn build_http_client(timeout: Duration) -> Result<Client, String> {
    build_http_client_from_builder(Client::builder(), timeout)
}

fn build_http_client_from_builder(
    builder: reqwest::ClientBuilder,
    timeout: Duration,
) -> Result<Client, String> {
    // MCP configuration can name local endpoints and carries credentials in
    // its headers. Do not allow ambient proxy configuration to redirect either
    // streamable HTTP or legacy SSE traffic to an unrelated intermediary.
    builder
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(timeout.min(Duration::from_secs(30)))
        .build()
        .map_err(|_| "failed to create MCP HTTP client".to_string())
}

struct HttpTransport {
    endpoint: Url,
    client: Client,
    headers: HeaderMap,
    timeout: Duration,
    session_id: Option<String>,
    protocol_version: Option<String>,
}

impl HttpTransport {
    fn new(config: &ValidatedConfig) -> Result<Self, String> {
        Ok(Self {
            endpoint: config
                .endpoint
                .clone()
                .ok_or_else(|| "MCP HTTP endpoint is missing".to_string())?,
            client: build_http_client(config.timeout)?,
            headers: config.headers.clone(),
            timeout: config.timeout,
            session_id: None,
            protocol_version: None,
        })
    }

    fn request_builder(&self, body: Vec<u8>, method: &str) -> reqwest::RequestBuilder {
        let mut builder = self
            .client
            .post(self.endpoint.clone())
            .headers(self.headers.clone())
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .timeout(self.timeout);
        if let Some(version) = &self.protocol_version {
            builder = builder.header("MCP-Protocol-Version", version);
        } else if method == "initialize" {
            builder = builder.header("MCP-Protocol-Version", "2025-03-26");
        }
        if method != "initialize" {
            if let Some(session) = &self.session_id {
                builder = builder.header("MCP-Session-Id", session);
            }
        }
        builder
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let body = encode_rpc(None, method, params)?;
        let response = self
            .request_builder(body, method)
            .send()
            .await
            .map_err(|_| format!("MCP HTTP notification failed: method={method}"))?;
        if response.status().is_redirection() || !response.status().is_success() {
            return Err(format!(
                "MCP HTTP notification failed: method={method} status={}",
                response.status()
            ));
        }
        // A notification may receive a long-lived SSE acknowledgement. Drop
        // that response rather than waiting for its stream to close.
        if response_content_type(&response) != "text/event-stream" {
            let _ = read_response_bytes(response).await?;
        }
        Ok(())
    }

    async fn request(&mut self, id: u64, method: &str, params: Value) -> Result<Value, String> {
        let body = encode_rpc(Some(id), method, params)?;
        let response = self
            .request_builder(body, method)
            .send()
            .await
            .map_err(|_| format!("MCP HTTP request failed: method={method}"))?;
        if response.status() == StatusCode::NOT_FOUND && self.session_id.is_some() {
            self.session_id = None;
            return Err("MCP HTTP session expired".to_string());
        }
        if response.status().is_redirection() || !response.status().is_success() {
            return Err(format!(
                "MCP HTTP request failed: method={method} status={}",
                response.status()
            ));
        }
        let session = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let content_type = response_content_type(&response);
        let message = if content_type == "text/event-stream" {
            read_sse_response(response, self.timeout, id, method).await?
        } else {
            let body = read_response_bytes(response).await?;
            if body.is_empty() {
                return Err(format!("MCP HTTP response was empty: method={method}"));
            }
            serde_json::from_slice::<Value>(&body)
                .map_err(|_| format!("MCP HTTP response was not valid JSON: method={method}"))?
        };
        let result = parse_jsonrpc_result(method, id, &message)?;
        if let Some(session) = session {
            self.session_id = Some(session);
        }
        if method == "initialize" {
            self.protocol_version = result
                .get("protocolVersion")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| Some("2025-03-26".to_string()));
        }
        Ok(result)
    }
}

fn encode_rpc(id: Option<u64>, method: &str, params: Value) -> Result<Vec<u8>, String> {
    let mut request = Map::new();
    request.insert("jsonrpc".to_string(), Value::String("2.0".to_string()));
    if let Some(id) = id {
        request.insert("id".to_string(), Value::from(id));
    }
    request.insert("method".to_string(), Value::String(method.to_string()));
    request.insert("params".to_string(), params);
    let body = serde_json::to_vec(&Value::Object(request))
        .map_err(|_| "failed to encode MCP request".to_string())?;
    if body.len() > MAX_RPC_REQUEST_BYTES {
        return Err("MCP request is oversized".to_string());
    }
    Ok(body)
}

struct LegacySseTransport {
    endpoint: Url,
    configured_message_url: Option<Url>,
    discovered_message_url: Option<Url>,
    client: Client,
    headers: HeaderMap,
    timeout: Duration,
    allow_remote: bool,
    stream: Option<Response>,
    decoder: SseDecoder,
    pending_events: VecDeque<SseEvent>,
    pending_messages: VecDeque<Value>,
    session_id: Option<String>,
}

impl LegacySseTransport {
    fn new(config: &ValidatedConfig) -> Result<Self, String> {
        Ok(Self {
            endpoint: config
                .endpoint
                .clone()
                .ok_or_else(|| "MCP SSE endpoint is missing".to_string())?,
            configured_message_url: config.message_url.clone(),
            discovered_message_url: None,
            client: build_http_client(config.timeout)?,
            headers: config.headers.clone(),
            timeout: config.timeout,
            allow_remote: config.allow_remote,
            stream: None,
            decoder: SseDecoder::default(),
            pending_events: VecDeque::new(),
            pending_messages: VecDeque::new(),
            session_id: None,
        })
    }

    async fn ensure_stream(&mut self) -> Result<(), String> {
        if self.stream.is_some() {
            return Ok(());
        }
        self.decoder = SseDecoder::default();
        self.pending_events.clear();
        let request = self
            .client
            .get(self.endpoint.clone())
            .headers(self.headers.clone())
            .header(ACCEPT, "text/event-stream");
        let response = tokio::time::timeout(self.timeout, request.send())
            .await
            .map_err(|_| "MCP SSE connection timed out".to_string())?
            .map_err(|_| "failed to connect to MCP SSE endpoint".to_string())?;
        if response.status().is_redirection() || !response.status().is_success() {
            return Err(format!(
                "MCP SSE connection failed: status={}",
                response.status()
            ));
        }
        if response_content_type(&response) != "text/event-stream" {
            return Err("MCP SSE endpoint did not return text/event-stream".to_string());
        }
        validate_response_length(&response)?;
        self.session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        self.stream = Some(response);
        Ok(())
    }

    async fn next_event(&mut self, timeout: Duration) -> Result<SseEvent, String> {
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(event);
        }
        self.ensure_stream().await?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("MCP SSE stream timed out".to_string());
            }
            let stream = self
                .stream
                .as_mut()
                .ok_or_else(|| "MCP SSE stream is unavailable".to_string())?;
            let chunk = tokio::time::timeout(remaining, stream.chunk())
                .await
                .map_err(|_| "MCP SSE stream timed out".to_string())?
                .map_err(|_| "failed to read MCP SSE stream".to_string())?;
            let Some(chunk) = chunk else {
                self.stream = None;
                return Err("MCP SSE stream closed unexpectedly".to_string());
            };
            self.pending_events.extend(self.decoder.push(&chunk)?);
            if let Some(event) = self.pending_events.pop_front() {
                return Ok(event);
            }
        }
    }

    async fn message_endpoint(&mut self) -> Result<Url, String> {
        self.ensure_stream().await?;
        if let Some(url) = &self.configured_message_url {
            return Ok(url.clone());
        }
        if let Some(url) = &self.discovered_message_url {
            return Ok(url.clone());
        }

        let wait = self.timeout.min(LEGACY_ENDPOINT_WAIT);
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline {
            let event = match self
                .next_event(deadline.saturating_duration_since(Instant::now()))
                .await
            {
                Ok(event) => event,
                Err(error) if error.contains("timed out") => break,
                Err(error) => return Err(error),
            };
            if event.event_name.eq_ignore_ascii_case("endpoint") {
                let raw = event.data.trim();
                let parsed = Url::parse(raw)
                    .or_else(|_| self.endpoint.join(raw))
                    .map_err(|_| "MCP SSE endpoint event contained an invalid URL".to_string())?;
                let parsed = validate_same_origin(
                    &self.endpoint,
                    &parsed,
                    self.allow_remote,
                    "SSE message endpoint",
                )?;
                self.discovered_message_url = Some(parsed.clone());
                return Ok(parsed);
            }
            if let Ok(message) = serde_json::from_str::<Value>(&event.data) {
                self.pending_messages.push_back(message);
                if self.pending_messages.len() > STDIO_CHANNEL_CAPACITY {
                    return Err("MCP SSE pending response limit exceeded".to_string());
                }
            }
        }

        if let Some(prefix) = self.endpoint.path().strip_suffix("/sse") {
            let mut inferred = self.endpoint.clone();
            inferred.set_path(&format!("{prefix}/message"));
            inferred.set_query(None);
            let inferred = validate_same_origin(
                &self.endpoint,
                &inferred,
                self.allow_remote,
                "SSE message endpoint",
            )?;
            self.discovered_message_url = Some(inferred.clone());
            return Ok(inferred);
        }
        Err("MCP SSE server did not provide a message endpoint".to_string())
    }

    async fn post_message(
        &mut self,
        id: Option<u64>,
        method: &str,
        params: Value,
    ) -> Result<Option<Value>, String> {
        let endpoint = self.message_endpoint().await?;
        let body = encode_rpc(id, method, params)?;
        let mut request = self
            .client
            .post(endpoint)
            .headers(self.headers.clone())
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .timeout(self.timeout);
        if let Some(session) = &self.session_id {
            request = request.header("MCP-Session-Id", session);
        }
        let response = request
            .send()
            .await
            .map_err(|_| format!("MCP SSE POST failed: method={method}"))?;
        if response.status().is_redirection() || !response.status().is_success() {
            return Err(format!(
                "MCP SSE POST failed: method={method} status={}",
                response.status()
            ));
        }
        if let Some(session) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            self.session_id = Some(session.to_string());
        }

        let content_type = response_content_type(&response);
        if let Some(id) = id {
            if content_type == "text/event-stream" {
                return read_sse_response(response, self.timeout, id, method)
                    .await
                    .map(Some);
            }
            let body = read_response_bytes(response).await?;
            if body.is_empty() {
                return Ok(None);
            }
            let message = serde_json::from_slice::<Value>(&body)
                .map_err(|_| "MCP SSE POST returned invalid JSON".to_string())?;
            ensure_matching_id(method, id, &message)?;
            Ok(Some(message))
        } else {
            if content_type != "text/event-stream" {
                let _ = read_response_bytes(response).await?;
            }
            Ok(None)
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.post_message(None, method, params).await.map(|_| ())
    }

    async fn request(&mut self, id: u64, method: &str, params: Value) -> Result<Value, String> {
        if let Some(message) = self.post_message(Some(id), method, params).await? {
            return parse_jsonrpc_result(method, id, &message);
        }

        let deadline = Instant::now() + self.timeout;
        let mut consumed = 0usize;
        loop {
            let message = if let Some(message) = self.pending_messages.pop_front() {
                message
            } else {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(format!("MCP SSE request timed out: method={method}"));
                }
                let event = self.next_event(remaining).await?;
                consumed = consumed.saturating_add(event.data.len());
                if consumed > MAX_RPC_MESSAGE_BYTES {
                    return Err("MCP SSE response exceeds the configured limit".to_string());
                }
                if event.event_name.eq_ignore_ascii_case("endpoint") {
                    continue;
                }
                match serde_json::from_str::<Value>(&event.data) {
                    Ok(message) => message,
                    Err(_) => continue,
                }
            };
            if message.get("id").is_none() {
                continue;
            }
            ensure_matching_id(method, id, &message)?;
            return parse_jsonrpc_result(method, id, &message);
        }
    }

    fn stop(&mut self) {
        self.stream = None;
        self.pending_events.clear();
        self.pending_messages.clear();
        self.session_id = None;
    }
}

enum McpTransport {
    Stdio(StdioTransport),
    Http(Box<HttpTransport>),
    Sse(Box<LegacySseTransport>),
}

impl McpTransport {
    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        match self {
            Self::Stdio(transport) => transport.notify(method, params).await,
            Self::Http(transport) => transport.notify(method, params).await,
            Self::Sse(transport) => transport.notify(method, params).await,
        }
    }

    async fn request(
        &mut self,
        timeout: Duration,
        id: u64,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        match self {
            Self::Stdio(transport) => {
                let message = transport.request(timeout, id, method, params).await?;
                parse_jsonrpc_result(method, id, &message)
            }
            Self::Http(transport) => transport.request(id, method, params).await,
            Self::Sse(transport) => transport.request(id, method, params).await,
        }
    }

    fn is_running(&self) -> bool {
        match self {
            Self::Stdio(transport) => transport.is_running(),
            Self::Http(_) | Self::Sse(_) => true,
        }
    }

    async fn stop(&mut self) {
        match self {
            Self::Stdio(transport) => transport.stop().await,
            Self::Http(_) => {}
            Self::Sse(transport) => transport.stop(),
        }
    }
}

struct McpClient {
    config: ValidatedConfig,
    transport: Option<McpTransport>,
    next_id: u64,
    initialized: bool,
    last_error: Option<String>,
}

impl McpClient {
    fn new(config: ValidatedConfig) -> Self {
        Self {
            config,
            transport: None,
            next_id: 1,
            initialized: false,
            last_error: None,
        }
    }

    async fn ensure_transport(&mut self) -> Result<(), String> {
        if self
            .transport
            .as_ref()
            .is_some_and(McpTransport::is_running)
        {
            return Ok(());
        }
        if let Some(mut old) = self.transport.take() {
            old.stop().await;
        }
        let transport = match self.config.transport {
            TransportKind::Stdio => McpTransport::Stdio(StdioTransport::spawn(&self.config)?),
            TransportKind::Http => McpTransport::Http(Box::new(HttpTransport::new(&self.config)?)),
            TransportKind::Sse => {
                McpTransport::Sse(Box::new(LegacySseTransport::new(&self.config)?))
            }
        };
        self.transport = Some(transport);
        self.initialized = false;
        Ok(())
    }

    fn next_rpc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        id
    }

    fn remember_error(&mut self, error: String) -> String {
        let error = redact_error_text(&error);
        self.last_error = Some(error.clone());
        error
    }

    async fn request_raw(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.ensure_transport()
            .await
            .map_err(|error| self.remember_error(error))?;
        let id = self.next_rpc_id();
        let timeout = self.config.timeout;
        let result = self
            .transport
            .as_mut()
            .ok_or_else(|| "MCP transport is unavailable".to_string())?
            .request(timeout, id, method, params)
            .await;
        match result {
            Ok(value) => {
                self.last_error = None;
                Ok(value)
            }
            Err(error) => {
                if !self
                    .transport
                    .as_ref()
                    .is_some_and(McpTransport::is_running)
                {
                    self.initialized = false;
                }
                Err(self.remember_error(error))
            }
        }
    }

    async fn notify_raw(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.ensure_transport()
            .await
            .map_err(|error| self.remember_error(error))?;
        let result = self
            .transport
            .as_mut()
            .ok_or_else(|| "MCP transport is unavailable".to_string())?
            .notify(method, params)
            .await;
        match result {
            Ok(()) => Ok(()),
            Err(error) => Err(self.remember_error(error)),
        }
    }

    async fn ensure_initialized(&mut self) -> Result<(), String> {
        if self.initialized
            && self
                .transport
                .as_ref()
                .is_some_and(McpTransport::is_running)
        {
            return Ok(());
        }
        let result = self
            .request_raw(
                "initialize",
                json!({
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "novavei",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                }),
            )
            .await?;
        let object = result.as_object().ok_or_else(|| {
            self.remember_error("MCP initialize result was not an object".to_string())
        })?;
        if let Some(protocol) = object.get("protocolVersion") {
            let protocol = protocol.as_str().ok_or_else(|| {
                self.remember_error("MCP initialize protocolVersion was invalid".to_string())
            })?;
            validate_text(protocol, 128, "protocol version")?;
        }
        self.notify_raw("notifications/initialized", json!({}))
            .await?;
        self.initialized = true;
        self.last_error = None;
        Ok(())
    }

    async fn list_tools(&mut self) -> Result<Vec<McpToolInfo>, String> {
        self.ensure_initialized().await?;
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        let mut seen_tools = HashSet::new();

        for _ in 0..64 {
            let params = match &cursor {
                Some(cursor) => json!({ "cursor": cursor }),
                None => json!({}),
            };
            let result = self.request_raw("tools/list", params).await?;
            let (page, next_cursor) = parse_tools_page(&self.config.id, &result)?;
            for tool in page {
                if !seen_tools.insert(tool.name.clone()) {
                    return Err(self.remember_error(
                        "MCP tools/list returned a duplicate tool name".to_string(),
                    ));
                }
                tools.push(tool);
                if tools.len() > MAX_TOOL_COUNT {
                    return Err(self.remember_error(
                        "MCP tools/list exceeded the tool count limit".to_string(),
                    ));
                }
            }
            let Some(next) = next_cursor else {
                return Ok(tools);
            };
            if !seen_cursors.insert(next.clone()) {
                return Err(
                    self.remember_error("MCP tools/list returned a repeated cursor".to_string())
                );
            }
            cursor = Some(next);
        }
        Err(self.remember_error("MCP tools/list exceeded the pagination limit".to_string()))
    }

    async fn call_tool(
        &mut self,
        request: McpCallToolRequest,
    ) -> Result<McpCallToolResponse, String> {
        validate_text(&request.name, MAX_TOOL_NAME_BYTES, "tool name")?;
        if request.name.trim().is_empty() {
            return Err("MCP tool name is required".to_string());
        }
        if !request.arguments.is_null() && !request.arguments.is_object() {
            return Err("MCP tool arguments must be an object".to_string());
        }
        let arguments = if request.arguments.is_null() {
            json!({})
        } else {
            request.arguments
        };
        let encoded = serde_json::to_vec(&arguments)
            .map_err(|_| "failed to encode MCP tool arguments".to_string())?;
        if encoded.len() > MAX_RPC_REQUEST_BYTES {
            return Err("MCP tool arguments are oversized".to_string());
        }
        self.ensure_initialized().await?;
        let result = self
            .request_raw(
                "tools/call",
                json!({
                    "name": request.name,
                    "arguments": arguments,
                }),
            )
            .await?;
        parse_call_tool_response(result)
    }

    fn status(&self) -> McpRuntimeStatus {
        McpRuntimeStatus {
            server_id: self.config.id.clone(),
            running: self
                .transport
                .as_ref()
                .is_some_and(McpTransport::is_running),
            initialized: self.initialized,
            transport: self.config.transport.as_str().to_string(),
            last_error: self.last_error.clone(),
        }
    }

    async fn stop(&mut self) {
        if let Some(mut transport) = self.transport.take() {
            transport.stop().await;
        }
        self.initialized = false;
    }
}

fn parse_tools_page(
    server_id: &str,
    result: &Value,
) -> Result<(Vec<McpToolInfo>, Option<String>), String> {
    let object = result
        .as_object()
        .ok_or_else(|| "MCP tools/list result was not an object".to_string())?;
    let raw_tools = object
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| "MCP tools/list result did not contain tools".to_string())?;
    if raw_tools.len() > MAX_TOOL_COUNT {
        return Err("MCP tools/list page exceeded the tool count limit".to_string());
    }
    let mut tools = Vec::with_capacity(raw_tools.len());
    for raw in raw_tools {
        let raw = raw
            .as_object()
            .ok_or_else(|| "MCP tool descriptor was not an object".to_string())?;
        let name = raw
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "MCP tool descriptor did not contain a name".to_string())?;
        if name.trim().is_empty() || name.len() > MAX_TOOL_NAME_BYTES {
            return Err("MCP tool name is invalid or oversized".to_string());
        }
        let description = raw
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if description.len() > MAX_TOOL_DESCRIPTION_BYTES {
            return Err("MCP tool description is oversized".to_string());
        }
        let input_schema = raw
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object" }));
        if !input_schema.is_object()
            || serde_json::to_vec(&input_schema)
                .map_err(|_| "failed to encode MCP tool schema".to_string())?
                .len()
                > MAX_TOOL_SCHEMA_BYTES
        {
            return Err("MCP tool input schema is invalid or oversized".to_string());
        }
        tools.push(McpToolInfo {
            server_id: server_id.to_string(),
            server_label: server_id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            input_schema,
        });
    }

    let next_cursor = match object.get("nextCursor") {
        Some(Value::String(value)) if !value.is_empty() && value.len() <= MAX_TOOL_NAME_BYTES => {
            Some(value.clone())
        }
        Some(Value::Null) | None => None,
        Some(_) => return Err("MCP tools/list nextCursor was invalid".to_string()),
    };
    Ok((tools, next_cursor))
}

fn parse_call_tool_response(result: Value) -> Result<McpCallToolResponse, String> {
    let object = result
        .as_object()
        .ok_or_else(|| "MCP tools/call result was not an object".to_string())?;
    let raw_content: &[Value] = match object.get("content") {
        Some(Value::Array(content)) => content.as_slice(),
        Some(_) => return Err("MCP tools/call content was not an array".to_string()),
        None => &[],
    };
    if raw_content.len() > MAX_CONTENT_ITEMS {
        return Err("MCP tools/call returned too many content items".to_string());
    }
    let mut content = Vec::with_capacity(raw_content.len());
    for item in raw_content {
        let mut fields = item
            .as_object()
            .cloned()
            .ok_or_else(|| "MCP content item was not an object".to_string())?;
        let content_type = fields
            .remove("type")
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .ok_or_else(|| "MCP content item did not contain a type".to_string())?;
        if content_type.is_empty() || content_type.len() > 128 {
            return Err("MCP content type was invalid".to_string());
        }
        let item_size = serde_json::to_vec(&fields)
            .map_err(|_| "failed to encode MCP content item".to_string())?
            .len();
        if item_size > MAX_RPC_MESSAGE_BYTES {
            return Err("MCP content item was oversized".to_string());
        }
        content.push(McpContent {
            content_type,
            fields,
        });
    }
    Ok(McpCallToolResponse {
        content,
        is_error: object
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        details: result,
    })
}

struct ManagedClient {
    fingerprint: u64,
    client: Arc<tokio::sync::Mutex<McpClient>>,
}

/// Owns reusable native MCP clients.  The map is keyed only by native server
/// id; each client has its own async mutex, making all requests to that server
/// strictly serial.
#[derive(Default)]
pub struct McpRuntimeManager {
    clients: tokio::sync::Mutex<HashMap<String, ManagedClient>>,
    // The backend holds this across native configuration/trust resolution and
    // the corresponding runtime operation. That prevents a settings save from
    // revoking a configuration while a stale request is about to create it.
    execution_gate: tokio::sync::Mutex<()>,
}

impl McpRuntimeManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Serialize configuration changes with all MCP runtime operations that
    /// can create, use, or stop a client. The caller deliberately owns the
    /// guard so it can cover native config resolution as well as execution.
    pub async fn lock_execution_gate(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.execution_gate.lock().await
    }

    async fn client_for(
        &self,
        config: McpServerConfig,
    ) -> Result<Arc<tokio::sync::Mutex<McpClient>>, String> {
        let fingerprint = config_fingerprint(&config);
        let validated = config.validate()?;
        let mut clients = self.clients.lock().await;

        if !validated.enabled {
            if let Some(old) = clients.remove(&validated.id) {
                old.client.lock().await.stop().await;
            }
            return Err("MCP server is disabled".to_string());
        }

        if let Some(existing) = clients.get(&validated.id) {
            if existing.fingerprint == fingerprint {
                return Ok(existing.client.clone());
            }
        }
        if let Some(old) = clients.remove(&validated.id) {
            old.client.lock().await.stop().await;
        }
        let id = validated.id.clone();
        let client = Arc::new(tokio::sync::Mutex::new(McpClient::new(validated)));
        clients.insert(
            id,
            ManagedClient {
                fingerprint,
                client: client.clone(),
            },
        );
        Ok(client)
    }

    pub async fn list_tools(&self, config: McpServerConfig) -> Result<Vec<McpToolInfo>, String> {
        let client = self.client_for(config).await?;
        let mut client = client.lock().await;
        client.list_tools().await
    }

    pub async fn call_tool(
        &self,
        config: McpServerConfig,
        request: McpCallToolRequest,
    ) -> Result<McpCallToolResponse, String> {
        let client = self.client_for(config).await?;
        let mut client = client.lock().await;
        client.call_tool(request).await
    }

    pub async fn runtime_status(&self, server_id: &str) -> McpRuntimeStatus {
        let client = {
            let clients = self.clients.lock().await;
            clients.get(server_id).map(|entry| entry.client.clone())
        };
        match client {
            Some(client) => client.lock().await.status(),
            None => McpRuntimeStatus {
                server_id: server_id.to_string(),
                running: false,
                initialized: false,
                transport: "unknown".to_string(),
                last_error: None,
            },
        }
    }

    pub async fn stop_server(&self, server_id: &str) -> McpStopServerResponse {
        let client = self.clients.lock().await.remove(server_id);
        let stopped = client.is_some();
        if let Some(client) = client {
            client.client.lock().await.stop().await;
        }
        McpStopServerResponse {
            server_id: server_id.to_string(),
            stopped,
        }
    }

    pub async fn restart_server(
        &self,
        config: McpServerConfig,
    ) -> Result<McpRuntimeStatus, String> {
        let server_id = config.id.trim().to_string();
        self.stop_server(&server_id).await;
        let client = self.client_for(config).await?;
        let mut client = client.lock().await;
        client.ensure_initialized().await?;
        Ok(client.status())
    }

    pub async fn test_server(&self, config: McpServerConfig) -> McpRuntimeTestResponse {
        let started = Instant::now();
        let server_id = config.id.trim().to_string();
        let transport = config
            .transport_name()
            .map(TransportKind::as_str)
            .unwrap_or("invalid")
            .to_string();
        if let Err(error) = config.validate() {
            return McpRuntimeTestResponse {
                server_id,
                ok: false,
                phase: "validation".to_string(),
                transport,
                duration_ms: started.elapsed().as_millis(),
                running: false,
                initialized: false,
                tools_count: 0,
                tools: Vec::new(),
                error: Some(redact_error_text(&error)),
                stderr_tail: None,
            };
        }

        let result = self.list_tools(config).await;
        let status = self.runtime_status(&server_id).await;
        match result {
            Ok(tools) => {
                let diagnostics = tools
                    .into_iter()
                    .map(|tool| McpDiagnosticToolInfo {
                        server_id: tool.server_id,
                        server_label: tool.server_label,
                        name: tool.name,
                        description: tool.description,
                        input_schema: Some(tool.input_schema),
                    })
                    .collect::<Vec<_>>();
                McpRuntimeTestResponse {
                    server_id,
                    ok: true,
                    phase: "ready".to_string(),
                    transport,
                    duration_ms: started.elapsed().as_millis(),
                    running: status.running,
                    initialized: status.initialized,
                    tools_count: diagnostics.len(),
                    tools: diagnostics,
                    error: None,
                    // Keep the field for IPC compatibility, but never expose
                    // arbitrary child-process diagnostics to the WebView.
                    stderr_tail: None,
                }
            }
            Err(error) => McpRuntimeTestResponse {
                server_id,
                ok: false,
                phase: if status.initialized {
                    "tools-list".to_string()
                } else {
                    "initialize".to_string()
                },
                transport,
                duration_ms: started.elapsed().as_millis(),
                running: status.running,
                initialized: status.initialized,
                tools_count: 0,
                tools: Vec::new(),
                error: Some(redact_error_text(&error)),
                stderr_tail: None,
            },
        }
    }

    pub async fn shutdown_all(&self) {
        let clients = {
            let mut clients = self.clients.lock().await;
            clients
                .drain()
                .map(|(_, entry)| entry.client)
                .collect::<Vec<_>>()
        };
        for client in clients {
            client.lock().await.stop().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn base_config() -> McpServerConfig {
        McpServerConfig {
            id: "local-tools".to_string(),
            enabled: true,
            transport: Some("stdio".to_string()),
            command: "mcp-server".to_string(),
            args: Vec::new(),
            env: None,
            cwd: None,
            url: None,
            headers: None,
            timeout_ms: Some(1_000),
            message_url: None,
            allow_remote: false,
            stdio_framing: None,
        }
    }

    #[test]
    fn validates_local_urls_and_rejects_unsafe_urls() {
        let mut config = base_config();
        config.transport = Some("http".to_string());
        config.command.clear();
        config.url = Some("http://127.0.0.1:43123/mcp".to_string());
        assert!(config.validate().is_ok());

        config.url = Some("https://example.com/mcp".to_string());
        assert!(config.validate().is_err());
        config.allow_remote = true;
        assert!(config.validate().is_ok());

        config.url = Some("https://user:password@example.com/mcp".to_string());
        assert!(config.validate().is_err());
        config.url = Some("file:///tmp/mcp.sock".to_string());
        assert!(config.validate().is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mcp_http_client_bypasses_an_explicit_proxy() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let target_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target_listener.local_addr().unwrap();
        let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = proxy_listener.local_addr().unwrap();

        let target_task = tokio::spawn(async move {
            let (mut socket, _) = target_listener.accept().await.unwrap();
            let mut request = [0_u8; 1_024];
            let _ = socket.read(&mut request).await.unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let proxy_task = tokio::spawn(async move {
            let (mut socket, _) = proxy_listener.accept().await.unwrap();
            let mut request = [0_u8; 1_024];
            let _ = socket.read(&mut request).await;
            socket
                .write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });

        let client = build_http_client_from_builder(
            Client::builder().proxy(reqwest::Proxy::all(format!("http://{proxy_addr}")).unwrap()),
            Duration::from_secs(1),
        )
        .unwrap();
        let response = client
            .get(format!("http://{target_addr}/"))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        target_task.await.unwrap();
        proxy_task.abort();
    }

    #[test]
    fn rejects_cross_origin_sse_message_endpoint_and_bad_timeout() {
        let mut config = base_config();
        config.transport = Some("sse".to_string());
        config.command.clear();
        config.url = Some("http://localhost:3000/sse".to_string());
        config.message_url = Some("http://localhost:3001/message".to_string());
        assert!(config.validate().is_err());

        config.message_url = Some("/message".to_string());
        assert!(config.validate().is_ok());
        config.timeout_ms = Some(0);
        assert!(config.validate().is_err());
        config.timeout_ms = Some(MAX_TIMEOUT_MS + 1);
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_reserved_or_malformed_headers() {
        let mut config = base_config();
        config.transport = Some("http".to_string());
        config.command.clear();
        config.url = Some("http://localhost/mcp".to_string());
        config.headers = Some(BTreeMap::from([(
            "Content-Length".to_string(),
            "2".to_string(),
        )]));
        assert!(config.validate().is_err());
        config.headers = Some(BTreeMap::from([(
            "Authorization".to_string(),
            "Bearer abc\r\ninjected: true".to_string(),
        )]));
        assert!(config.validate().is_err());

        config.headers = Some(BTreeMap::from([
            ("X-Token".to_string(), "first".to_string()),
            ("x-token".to_string(), "second".to_string()),
        ]));
        assert!(config.validate().is_err());
    }

    #[test]
    fn normalises_settings_to_the_validated_runtime_shape() {
        let mut config = base_config();
        config.transport = Some("streamable-http".to_string());
        config.command = "unused".to_string();
        config.args = vec!["unused".to_string()];
        config.url = Some("http://LOCALHOST:43123/mcp".to_string());
        config.headers = Some(BTreeMap::from([(
            "X-Token".to_string(),
            "configured".to_string(),
        )]));
        config.stdio_framing = Some("lsp".to_string());

        let normalised = config.normalised_for_settings().unwrap();
        assert_eq!(normalised.transport.as_deref(), Some("http"));
        assert!(normalised.command.is_empty());
        assert!(normalised.args.is_empty());
        assert_eq!(normalised.stdio_framing, None);
        assert_eq!(
            normalised
                .headers
                .as_ref()
                .and_then(|headers| headers.get("x-token"))
                .map(String::as_str),
            Some("configured")
        );
    }

    #[test]
    fn fingerprint_and_debug_never_expose_secret_values() {
        let mut first = base_config();
        first.env = Some(BTreeMap::from([(
            "API_KEY".to_string(),
            "top-secret-environment-value".to_string(),
        )]));
        first.headers = Some(BTreeMap::from([(
            "Authorization".to_string(),
            "Bearer top-secret-header-value".to_string(),
        )]));
        let debug = format!("{first:?}");
        assert!(!debug.contains("top-secret-environment-value"));
        assert!(!debug.contains("top-secret-header-value"));

        let first_fingerprint = config_fingerprint(&first);
        first
            .env
            .as_mut()
            .unwrap()
            .insert("API_KEY".to_string(), "rotated-value".to_string());
        assert_ne!(first_fingerprint, config_fingerprint(&first));
        assert_eq!(
            redact_error_text("Authorization: Bearer should-not-leak"),
            "[redacted sensitive MCP error]"
        );
    }

    #[test]
    fn parses_json_line_and_content_length_stdio_frames() {
        let json_line = br#"{"jsonrpc":"2.0","id":1,"result":{}}
"#;
        let parsed = read_stdio_payload(&mut Cursor::new(json_line))
            .unwrap()
            .unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&parsed).unwrap()["id"], 1);

        let body = br#"{"jsonrpc":"2.0","id":2,"result":{"ok":true}}"#;
        let mut framed =
            format!("Content-Length: {}\r\nX-Test: 1\r\n\r\n", body.len()).into_bytes();
        framed.extend_from_slice(body);
        let parsed = read_stdio_payload(&mut Cursor::new(framed))
            .unwrap()
            .unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&parsed).unwrap()["id"], 2);
    }

    #[test]
    fn rejects_oversized_or_duplicate_content_length_frames() {
        let oversized = format!("Content-Length: {}\r\n\r\n", MAX_RPC_MESSAGE_BYTES + 1);
        assert!(read_stdio_payload(&mut Cursor::new(oversized)).is_err());
        let duplicate = b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}";
        assert!(read_stdio_payload(&mut Cursor::new(duplicate)).is_err());
    }

    #[test]
    fn decodes_split_and_multiline_sse_events() {
        let mut decoder = SseDecoder::default();
        assert!(decoder.push(b"event: mes").unwrap().is_empty());
        let events = decoder
            .push(b"sage\r\ndata: {\"jsonrpc\":\"2.0\",\r\ndata: \"id\":7}\r\n\r\n")
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_name, "message");
        assert_eq!(events[0].data, "{\"jsonrpc\":\"2.0\",\n\"id\":7}");
    }

    #[test]
    fn parses_tool_pages_and_call_results() {
        let page = json!({
            "tools": [{
                "name": "search",
                "description": "Search files",
                "inputSchema": {"type": "object", "properties": {}}
            }],
            "nextCursor": "page-two"
        });
        let (tools, cursor) = parse_tools_page("server", &page).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "search");
        assert_eq!(cursor.as_deref(), Some("page-two"));

        let response = parse_call_tool_response(json!({
            "content": [{"type": "text", "text": "done"}],
            "isError": false,
            "structuredContent": {"matches": 1}
        }))
        .unwrap();
        assert!(!response.is_error);
        assert_eq!(response.content[0].content_type, "text");
        assert_eq!(response.content[0].fields["text"], "done");
    }

    #[cfg(windows)]
    #[test]
    fn windows_batch_quoting_contains_metacharacters_and_rejects_expansion() {
        assert_eq!(windows_cmd_quote_arg("a&b").unwrap(), "\"a&b\"");
        assert!(windows_cmd_quote_arg("%PATH%").is_err());
        assert!(windows_cmd_quote_arg("a\"b").is_err());
        assert!(windows_cmd_quote_arg("line\nbreak").is_err());
    }
}
