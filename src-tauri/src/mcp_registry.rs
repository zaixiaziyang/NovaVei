//! Read-only client for the official MCP Registry.
//!
//! The registry is a public discovery service, but its payload is publisher
//! controlled.  Keep this boundary native and project responses into a small
//! DTO instead of passing arbitrary JSON into the renderer.  In particular,
//! package command arguments and environment values are descriptors only; the
//! client never guesses an executable command from registry metadata.

use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::time::Duration;

pub const MCP_REGISTRY_BASE_URL: &str = "https://registry.modelcontextprotocol.io";

const MCP_REGISTRY_SERVERS_PATH: &str = "/v0.1/servers";
const DEFAULT_LIMIT: usize = 24;
const MAX_LIMIT: usize = 100;
const MAX_SEARCH_BYTES: usize = 200;
const MAX_CURSOR_BYTES: usize = 2 * 1024;
const MAX_NAME_BYTES: usize = 200;
const MAX_VERSION_BYTES: usize = 255;
const MAX_URL_BYTES: usize = 4 * 1024;
const MAX_DESCRIPTION_BYTES: usize = 8 * 1024;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_INPUTS: usize = 256;
const MAX_PACKAGES: usize = 64;
const MAX_REMOTES: usize = 64;
const DEFAULT_DRAFT_TIMEOUT_MS: u64 = 60_000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRegistryListResponse {
    pub servers: Vec<McpRegistryServer>,
    pub next_cursor: Option<String>,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRegistryServer {
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub version: String,
    pub website_url: Option<String>,
    pub repository: Option<McpRegistryRepository>,
    pub remotes: Vec<McpRegistryRemote>,
    pub packages: Vec<McpRegistryPackage>,
    pub status: Option<String>,
    pub status_message: Option<String>,
    pub is_latest: Option<bool>,
    pub published_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRegistryRepository {
    pub url: Option<String>,
    pub source: Option<String>,
    pub id: Option<String>,
    pub subdirectory: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRegistryRemote {
    pub transport: String,
    pub url: String,
    pub headers: Vec<McpRegistryInput>,
    pub variables: Vec<McpRegistryInput>,
    /// A remote can be imported only when its endpoint is a concrete HTTPS
    /// URL without query credentials or template substitution.
    pub importable: bool,
    pub requires_configuration: bool,
    /// Query text is removed before this DTO reaches the renderer.  A remote
    /// with a removed query can never create a draft.
    pub query_redacted: bool,
    pub incompatibility_reason: Option<String>,
}

/// A non-executable, non-secret starting point for the existing native MCP
/// settings editor.  A registry draft is deliberately disabled and cannot
/// reach a non-local endpoint until the user reviews and explicitly changes
/// both toggles in the editor.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRegistryRemoteDraft {
    pub registry_name: String,
    pub registry_version: String,
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub transport: String,
    pub url: String,
    pub allow_remote: bool,
    pub timeout_ms: u64,
    /// Input descriptors are display-only.  No supplied header/environment
    /// value is copied into this draft or serialized to the renderer.
    pub headers: Vec<McpRegistryInput>,
    pub variables: Vec<McpRegistryInput>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRegistryPackage {
    pub registry_type: String,
    pub identifier: String,
    pub version: Option<String>,
    pub runtime_hint: Option<String>,
    pub transport: Option<String>,
    pub environment_variables: Vec<McpRegistryInput>,
    pub package_arguments: Vec<McpRegistryArgument>,
    pub runtime_arguments: Vec<McpRegistryArgument>,
    /// Registry packages are reference metadata only.  The desktop shell
    /// neither imports package records nor infers commands from them.
    pub importable: bool,
    pub incompatibility_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRegistryInput {
    pub name: String,
    pub description: Option<String>,
    pub required: bool,
    pub secret: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRegistryArgument {
    pub argument_type: Option<String>,
    pub name: Option<String>,
    pub value_hint: Option<String>,
    pub description: Option<String>,
    pub required: bool,
    pub secret: bool,
    pub has_variables: bool,
}

#[derive(Debug, Deserialize)]
struct WireListResponse {
    #[serde(default)]
    servers: Vec<WireServerEnvelope>,
    #[serde(default)]
    metadata: WireMetadata,
}

#[derive(Debug, Deserialize, Default)]
struct WireMetadata {
    #[serde(rename = "nextCursor")]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireServerEnvelope {
    server: WireServer,
    #[serde(default)]
    _meta: Value,
}

#[derive(Debug, Deserialize)]
struct WireServer {
    name: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    version: String,
    #[serde(rename = "websiteUrl", default)]
    website_url: Option<String>,
    #[serde(default)]
    repository: Option<WireRepository>,
    #[serde(default)]
    remotes: Vec<WireRemote>,
    #[serde(default)]
    packages: Vec<WirePackage>,
}

#[derive(Debug, Deserialize)]
struct WireRepository {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    subdirectory: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireRemote {
    #[serde(rename = "type")]
    transport: String,
    url: String,
    #[serde(default)]
    headers: Vec<Value>,
    #[serde(default)]
    variables: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
struct WirePackage {
    #[serde(rename = "registryType")]
    registry_type: String,
    identifier: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(rename = "runtimeHint", default)]
    runtime_hint: Option<String>,
    #[serde(default)]
    transport: Option<Value>,
    #[serde(rename = "environmentVariables", default)]
    environment_variables: Vec<Value>,
    #[serde(rename = "packageArguments", default)]
    package_arguments: Vec<Value>,
    #[serde(rename = "runtimeArguments", default)]
    runtime_arguments: Vec<Value>,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_registry_list(
    search: Option<String>,
    cursor: Option<String>,
    limit: Option<u32>,
) -> Result<McpRegistryListResponse, String> {
    let search = validate_search(search)?;
    let cursor = validate_cursor(cursor)?;
    let limit = validate_limit(limit)?;
    let url = build_list_url(search.as_deref(), cursor.as_deref(), limit)?;
    let body = fetch_json(url).await?;
    parse_list_response(&body)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_registry_get(
    name: String,
    version: Option<String>,
) -> Result<McpRegistryServer, String> {
    let name = validate_server_name(&name)?;
    let version = require_latest_version(version)?;
    let url = build_detail_url(&name, &version)?;
    let body = fetch_json(url).await?;
    parse_detail_response(&body)
}

/// Fetch the current official record again and create a display-only draft for
/// one concrete remote endpoint.  The renderer never supplies endpoint text,
/// headers, commands, package data, or a version to this command.
#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_registry_remote_draft(
    name: String,
    remote_index: u32,
) -> Result<McpRegistryRemoteDraft, String> {
    let name = validate_server_name(&name)?;
    let remote_index = usize::try_from(remote_index)
        .map_err(|_| "MCP Registry remote selection is invalid".to_string())?;
    if remote_index >= MAX_REMOTES {
        return Err("MCP Registry remote selection is invalid".to_string());
    }
    let version = require_latest_version(None)?;
    let url = build_detail_url(&name, &version)?;
    let body = fetch_json(url).await?;
    let server = parse_detail_response(&body)?;
    let remote = server
        .remotes
        .get(remote_index)
        .ok_or_else(|| "MCP Registry remote was not found".to_string())?;
    build_remote_draft(&server, remote)
}

fn registry_client() -> Result<Client, String> {
    Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .user_agent("NovaVei-McpRegistry/0.1")
        .build()
        .map_err(|error| format!("build MCP Registry client: {error}"))
}

async fn fetch_json(url: Url) -> Result<Vec<u8>, String> {
    // `url` is constructed only from the fixed registry origin and validated
    // path/query components above.  Never accept a renderer-provided URL.
    let client = registry_client()?;
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| format!("MCP Registry request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(registry_status_error(status));
    }
    read_bounded_body(response).await
}

async fn read_bounded_body(mut response: reqwest::Response) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err("MCP Registry response is too large".to_string());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("read MCP Registry response: {error}"))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err("MCP Registry response is too large".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    if body.is_empty() {
        return Err("MCP Registry returned an empty response".to_string());
    }
    Ok(body)
}

fn registry_status_error(status: StatusCode) -> String {
    match status {
        StatusCode::NOT_FOUND => "MCP Registry server or version was not found".to_string(),
        StatusCode::TOO_MANY_REQUESTS => {
            "MCP Registry rate limit reached; try again later".to_string()
        }
        status if status.is_server_error() => "MCP Registry is temporarily unavailable".to_string(),
        status => format!("MCP Registry request was rejected ({status})"),
    }
}

fn build_list_url(search: Option<&str>, cursor: Option<&str>, limit: usize) -> Result<Url, String> {
    let mut url = Url::parse(&format!(
        "{MCP_REGISTRY_BASE_URL}{MCP_REGISTRY_SERVERS_PATH}"
    ))
    .map_err(|_| "MCP Registry base URL is invalid".to_string())?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("limit", &limit.to_string());
        // The UI is a discovery surface, not a historical version browser.
        // Keep pagination stable by asking the Registry for one latest entry
        // per server and filtering lifecycle status below.
        query.append_pair("version", "latest");
        if let Some(search) = search {
            query.append_pair("search", search);
        }
        if let Some(cursor) = cursor {
            // Cursors are opaque.  Pass the exact validated value unchanged.
            query.append_pair("cursor", cursor);
        }
    }
    Ok(url)
}

fn build_detail_url(name: &str, version: &str) -> Result<Url, String> {
    let mut url = Url::parse(&format!(
        "{MCP_REGISTRY_BASE_URL}{MCP_REGISTRY_SERVERS_PATH}"
    ))
    .map_err(|_| "MCP Registry base URL is invalid".to_string())?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "MCP Registry URL cannot be modified".to_string())?;
        segments.push(name).push("versions").push(version);
    }
    Ok(url)
}

fn parse_list_response(body: &[u8]) -> Result<McpRegistryListResponse, String> {
    let wire: WireListResponse = serde_json::from_slice(body)
        .map_err(|_| "MCP Registry returned invalid JSON".to_string())?;
    if wire.servers.len() > MAX_LIMIT {
        return Err("MCP Registry returned too many servers".to_string());
    }
    let mut servers = Vec::with_capacity(wire.servers.len());
    for envelope in wire.servers {
        let server = project_server(envelope.server, &envelope._meta)?;
        // Discovery deliberately excludes every record that is not explicitly
        // marked active *and* latest by the official metadata.  Treating a
        // missing lifecycle field as current would let a historical record
        // reach the import-draft path.
        if is_current_active_server(&server) {
            servers.push(server);
        }
    }
    let count = servers.len();
    Ok(McpRegistryListResponse {
        servers,
        next_cursor: validate_cursor(wire.metadata.next_cursor)?,
        count,
    })
}

fn parse_detail_response(body: &[u8]) -> Result<McpRegistryServer, String> {
    let envelope: WireServerEnvelope = serde_json::from_slice(body)
        .map_err(|_| "MCP Registry returned invalid server JSON".to_string())?;
    let server = project_server(envelope.server, &envelope._meta)?;
    if !is_current_active_server(&server) {
        return Err("MCP Registry server is not the current active version".to_string());
    }
    Ok(server)
}

fn is_current_active_server(server: &McpRegistryServer) -> bool {
    server.status.as_deref() == Some("active") && server.is_latest == Some(true)
}

fn require_latest_version(requested: Option<String>) -> Result<String, String> {
    if requested.as_deref().unwrap_or("latest") != "latest" {
        return Err("MCP Registry details are limited to the latest version".to_string());
    }
    validate_version("latest")
}

fn build_remote_draft(
    server: &McpRegistryServer,
    remote: &McpRegistryRemote,
) -> Result<McpRegistryRemoteDraft, String> {
    if !is_current_active_server(server) {
        return Err("MCP Registry server is not the current active version".to_string());
    }
    if !remote.importable || !is_concrete_https_url(&remote.url) {
        return Err("Only a concrete HTTPS MCP remote can create an import draft".to_string());
    }
    let transport = match remote.transport.as_str() {
        "streamable-http" => "http",
        "sse" => "sse",
        _ => return Err("This MCP remote transport is not supported by the editor".to_string()),
    };
    let label = server.title.clone().unwrap_or_else(|| server.name.clone());
    Ok(McpRegistryRemoteDraft {
        registry_name: server.name.clone(),
        registry_version: server.version.clone(),
        id: pi_safe_server_id(&server.name),
        label,
        // A registry entry can never activate itself or opt into a remote
        // network connection through this discovery-only command.
        enabled: false,
        transport: transport.to_string(),
        url: remote.url.clone(),
        allow_remote: false,
        timeout_ms: DEFAULT_DRAFT_TIMEOUT_MS,
        headers: remote.headers.clone(),
        variables: remote.variables.clone(),
    })
}

/// Produce an ASCII id accepted by Pi-oriented MCP configuration consumers.
/// Registry names may contain dots and a namespace slash, neither of which is
/// a stable config identifier in all clients.  The source name remains
/// separately available for display and provenance.
fn pi_safe_server_id(name: &str) -> String {
    let mut id = String::from("mcp");
    let mut pending_separator = true;
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() {
            if pending_separator {
                id.push('-');
                pending_separator = false;
            }
            id.push(byte.to_ascii_lowercase() as char);
        } else {
            pending_separator = true;
        }
    }
    if id == "mcp" {
        id.push_str("-server");
    }
    id.truncate(128);
    id.trim_end_matches('-').to_string()
}

fn project_server(server: WireServer, meta: &Value) -> Result<McpRegistryServer, String> {
    let name = validate_server_name(&server.name)?;
    let version = validate_version(&server.version)?;
    if server.remotes.len() > MAX_REMOTES || server.packages.len() > MAX_PACKAGES {
        return Err("MCP Registry server metadata is too large".to_string());
    }
    let description = bounded_text(
        server.description.as_deref().unwrap_or(""),
        MAX_DESCRIPTION_BYTES,
        "server description",
    )?;
    let title = optional_bounded_text(
        server.title.as_deref(),
        MAX_DESCRIPTION_BYTES,
        "server title",
    )?;
    let website_url = optional_public_url(server.website_url.as_deref(), "website URL")?;
    let repository = server.repository.map(project_repository).transpose()?;
    let remotes = server
        .remotes
        .into_iter()
        .map(project_remote)
        .collect::<Result<Vec<_>, _>>()?;
    let packages = server
        .packages
        .into_iter()
        .map(project_package)
        .collect::<Result<Vec<_>, _>>()?;
    let official = meta
        .get("io.modelcontextprotocol.registry/official")
        .and_then(Value::as_object);
    let status = optional_bounded_text(
        official
            .and_then(|value| value.get("status"))
            .and_then(Value::as_str),
        64,
        "server status",
    )?;
    let status_message = optional_bounded_text(
        official
            .and_then(|value| value.get("statusMessage"))
            .and_then(Value::as_str),
        512,
        "server status message",
    )?;
    let is_latest = official
        .and_then(|value| value.get("isLatest"))
        .and_then(Value::as_bool);
    let published_at = optional_bounded_text(
        official
            .and_then(|value| value.get("publishedAt"))
            .and_then(Value::as_str),
        128,
        "published timestamp",
    )?;
    let updated_at = optional_bounded_text(
        official
            .and_then(|value| value.get("updatedAt"))
            .and_then(Value::as_str),
        128,
        "updated timestamp",
    )?;
    Ok(McpRegistryServer {
        name,
        title,
        description,
        version,
        website_url,
        repository,
        remotes,
        packages,
        status,
        status_message,
        is_latest,
        published_at,
        updated_at,
    })
}

fn project_repository(repository: WireRepository) -> Result<McpRegistryRepository, String> {
    Ok(McpRegistryRepository {
        url: optional_public_url(repository.url.as_deref(), "repository URL")?,
        source: optional_bounded_text(repository.source.as_deref(), 64, "repository source")?,
        id: optional_bounded_text(repository.id.as_deref(), 512, "repository id")?,
        subdirectory: optional_bounded_text(
            repository.subdirectory.as_deref(),
            MAX_URL_BYTES,
            "repository subdirectory",
        )?,
    })
}

fn project_remote(remote: WireRemote) -> Result<McpRegistryRemote, String> {
    let transport = bounded_text(&remote.transport, 64, "remote transport")?.to_ascii_lowercase();
    let raw_url = bounded_text(&remote.url, MAX_URL_BYTES, "remote URL")?;
    let query_redacted = Url::parse(&raw_url)
        .ok()
        .and_then(|url| url.query().map(|_| ()))
        .is_some();
    // A registry record may accidentally include a publisher token in a URL
    // query.  Never pass that query to the renderer, even when the record is
    // otherwise non-importable.
    let url = if query_redacted {
        redact_remote_query(&raw_url)?
    } else {
        raw_url
    };
    let headers = project_inputs(&remote.headers, None)?;
    let variables = remote
        .variables
        .iter()
        .map(|(name, value)| project_input(value, Some(name)))
        .collect::<Result<Vec<_>, _>>()?;
    if headers.len() > MAX_INPUTS || variables.len() > MAX_INPUTS {
        return Err("MCP Registry remote metadata is too large".to_string());
    }
    let concrete_url = !query_redacted && is_concrete_https_url(&url);
    let supported_transport = matches!(transport.as_str(), "streamable-http" | "sse");
    let importable = concrete_url && supported_transport;
    let requires_configuration = !headers.is_empty() || !variables.is_empty();
    let incompatibility_reason = if !supported_transport {
        Some("This remote transport is not supported by the MCP editor.".to_string())
    } else if query_redacted {
        Some("Endpoint query parameters were removed and cannot be imported.".to_string())
    } else if !concrete_url {
        Some("This endpoint contains variables or is not a concrete HTTPS URL.".to_string())
    } else {
        None
    };
    Ok(McpRegistryRemote {
        transport,
        url,
        headers,
        variables,
        importable,
        requires_configuration,
        query_redacted,
        incompatibility_reason,
    })
}

fn project_package(package: WirePackage) -> Result<McpRegistryPackage, String> {
    let registry_type = bounded_text(&package.registry_type, 64, "package registry type")?;
    let identifier = bounded_text(&package.identifier, MAX_URL_BYTES, "package identifier")?;
    let version = optional_bounded_text(
        package.version.as_deref(),
        MAX_VERSION_BYTES,
        "package version",
    )?;
    let runtime_hint = optional_bounded_text(
        package.runtime_hint.as_deref(),
        MAX_URL_BYTES,
        "package runtime hint",
    )?;
    let transport = package
        .transport
        .as_ref()
        .and_then(transport_type)
        .map(|value| value.to_ascii_lowercase());
    let environment_variables = project_inputs(&package.environment_variables, None)?;
    let package_arguments = project_arguments(&package.package_arguments)?;
    let runtime_arguments = project_arguments(&package.runtime_arguments)?;
    if environment_variables.len() + package_arguments.len() + runtime_arguments.len() > MAX_INPUTS
    {
        return Err("MCP Registry package metadata is too large".to_string());
    }
    Ok(McpRegistryPackage {
        registry_type,
        identifier,
        version,
        runtime_hint,
        transport,
        environment_variables,
        package_arguments,
        runtime_arguments,
        importable: false,
        incompatibility_reason: Some(
            "Registry package metadata is for reference only and cannot be imported or executed."
                .to_string(),
        ),
    })
}

fn transport_type(value: &Value) -> Option<&str> {
    match value {
        Value::String(value) => Some(value.as_str()),
        Value::Object(object) => object.get("type").and_then(Value::as_str),
        Value::Array(values) => values.iter().find_map(transport_type),
        _ => None,
    }
}

fn project_inputs(
    values: &[Value],
    fallback: Option<&str>,
) -> Result<Vec<McpRegistryInput>, String> {
    values
        .iter()
        .map(|value| project_input(value, fallback))
        .collect()
}

fn project_arguments(values: &[Value]) -> Result<Vec<McpRegistryArgument>, String> {
    values.iter().map(project_argument).collect()
}

fn project_argument(value: &Value) -> Result<McpRegistryArgument, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "MCP Registry package argument is invalid".to_string())?;
    let secret = object
        .get("isSecret")
        .or_else(|| object.get("secret"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let text_field = |key: &str, max: usize, label: &str| {
        optional_bounded_text(object.get(key).and_then(Value::as_str), max, label)
    };
    let argument_type =
        text_field("type", 64, "package argument type")?.map(|value| value.to_ascii_lowercase());
    let name = text_field("name", 512, "package argument name")?;
    let raw_value_hint = text_field("valueHint", 512, "package argument value hint")?;
    let raw_description = text_field(
        "description",
        MAX_DESCRIPTION_BYTES,
        "package argument description",
    )?;
    // Read only to detect a template; never serialize publisher-provided
    // values/defaults to the renderer.  A registry publisher controls its
    // own secret marker, so treating unmarked values as safe would be an
    // unnecessary data-leak path.
    let raw_value = text_field("value", MAX_URL_BYTES, "package argument value")?;
    let raw_default_value = text_field("default", MAX_URL_BYTES, "package argument default")?;
    let required = object
        .get("isRequired")
        .or_else(|| object.get("required"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let has_variables = object
        .get("variables")
        .and_then(Value::as_object)
        .is_some_and(|variables| !variables.is_empty())
        || raw_value
            .as_deref()
            .or(raw_default_value.as_deref())
            .is_some_and(|value| value.contains('{') || value.contains('}'));
    Ok(McpRegistryArgument {
        argument_type,
        name,
        value_hint: (!secret).then_some(raw_value_hint).flatten(),
        description: (!secret).then_some(raw_description).flatten(),
        required,
        secret,
        has_variables,
    })
}

fn project_input(value: &Value, fallback: Option<&str>) -> Result<McpRegistryInput, String> {
    let object = value.as_object();
    let name = object
        .and_then(|value| {
            ["name", "valueHint", "value_hint", "key"]
                .iter()
                .find_map(|key| value.get(*key).and_then(Value::as_str))
        })
        .or(fallback)
        .unwrap_or("input");
    let name = bounded_text(name, 512, "registry input name")?;
    let raw_description = optional_bounded_text(
        object
            .and_then(|value| value.get("description"))
            .and_then(Value::as_str),
        MAX_DESCRIPTION_BYTES,
        "registry input description",
    )?;
    let required = object
        .and_then(|value| {
            ["isRequired", "required"]
                .iter()
                .find_map(|key| value.get(*key).and_then(Value::as_bool))
        })
        .unwrap_or(false);
    let secret = object
        .and_then(|value| {
            ["isSecret", "secret"]
                .iter()
                .find_map(|key| value.get(*key).and_then(Value::as_bool))
        })
        .unwrap_or(false);
    Ok(McpRegistryInput {
        name,
        // Secret input values are never present in this DTO.  Do not surface
        // their free-form description either, because publishers sometimes
        // put example credentials in descriptions.
        description: (!secret).then_some(raw_description).flatten(),
        required,
        secret,
    })
}

fn validate_search(search: Option<String>) -> Result<Option<String>, String> {
    search
        .map(|value| {
            let value = value.trim().to_string();
            if value.is_empty() {
                return Ok(None);
            }
            bounded_text(&value, MAX_SEARCH_BYTES, "search").map(Some)
        })
        .transpose()
        .map(|value| value.flatten())
}

fn validate_cursor(cursor: Option<String>) -> Result<Option<String>, String> {
    cursor
        .map(|value| {
            if value.is_empty() {
                return Ok(None);
            }
            // Do not trim opaque cursors: the exact returned value must be
            // sent back to the Registry on the next page.
            bounded_text(&value, MAX_CURSOR_BYTES, "cursor").map(Some)
        })
        .transpose()
        .map(|value| value.flatten())
}

fn validate_limit(limit: Option<u32>) -> Result<usize, String> {
    let limit = limit.map(|value| value as usize).unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(format!(
            "MCP Registry limit must be between 1 and {MAX_LIMIT}"
        ));
    }
    Ok(limit)
}

fn validate_server_name(name: &str) -> Result<String, String> {
    let name = bounded_text(name, MAX_NAME_BYTES, "server name")?;
    if !(3..=MAX_NAME_BYTES).contains(&name.len()) {
        return Err("MCP Registry server name has an invalid length".to_string());
    }
    let mut parts = name.split('/');
    let namespace = parts.next().unwrap_or_default();
    let server = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || namespace.is_empty()
        || server.is_empty()
        || !namespace
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        || !server
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err("MCP Registry server name is invalid".to_string());
    }
    Ok(name)
}

fn validate_version(version: &str) -> Result<String, String> {
    let version = bounded_text(version, MAX_VERSION_BYTES, "server version")?;
    if version.is_empty() || version.contains('/') || version.contains('\\') {
        return Err("MCP Registry server version is invalid".to_string());
    }
    Ok(version)
}

fn bounded_text(value: &str, max: usize, label: &str) -> Result<String, String> {
    if value.is_empty()
        || value.len() > max
        || value.as_bytes().contains(&0)
        || value.chars().any(char::is_control)
    {
        return Err(format!("MCP Registry {label} is invalid or oversized"));
    }
    Ok(value.to_string())
}

fn optional_bounded_text(
    value: Option<&str>,
    max: usize,
    label: &str,
) -> Result<Option<String>, String> {
    value
        .filter(|value| !value.is_empty())
        .map(|value| bounded_text(value, max, label))
        .transpose()
}

fn is_http_url(value: &str) -> bool {
    Url::parse(value)
        .map(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
        })
        .unwrap_or(false)
}

fn is_https_url(value: &str) -> bool {
    Url::parse(value)
        .map(|url| {
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
        })
        .unwrap_or(false)
}

fn is_concrete_https_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    !value.contains('{')
        && !value.contains('}')
        // Percent-encoded braces are still a template marker for the
        // registry formats we support.  Reject them rather than handing an
        // unresolved endpoint to the settings editor.
        && !lower.contains("%7b")
        && !lower.contains("%7d")
        && Url::parse(value)
            .map(|url| url.query().is_none())
            .unwrap_or(false)
        && is_https_url(value)
}

fn redact_remote_query(value: &str) -> Result<String, String> {
    let mut url =
        Url::parse(value).map_err(|_| "MCP Registry remote URL is invalid".to_string())?;
    url.set_query(None);
    Ok(url.to_string())
}

fn optional_public_url(value: Option<&str>, label: &str) -> Result<Option<String>, String> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let value = bounded_text(value, MAX_URL_BYTES, label)?;
    if !is_http_url(&value) {
        return Ok(None);
    }
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST_JSON: &str = r#"{
      "servers": [{
        "server": {
          "name": "io.example/weather",
          "title": "Weather",
          "description": "Weather data",
          "version": "1.2.3",
          "websiteUrl": "https://example.com/weather",
          "repository": {"url": "https://github.com/example/weather", "source": "github"},
          "remotes": [{
           "type": "streamable-http",
           "url": "https://example.com/mcp",
            "headers": [{"name": "X-Api-Key", "value": "header-secret", "isSecret": true}],
            "variables": {"token": {"isRequired": true, "isSecret": true, "description": "API token"}}
          }, {
            "type": "sse",
            "url": "https://example.com/sse"
          }],
          "packages": [{
            "registryType": "npm",
            "identifier": "@example/weather",
            "version": "1.2.3",
            "runtimeHint": "npx",
            "transport": {"type": "stdio"},
            "environmentVariables": [{"name": "WEATHER_KEY", "value": "top-secret", "isSecret": true, "isRequired": true}],
            "packageArguments": [{"type": "named", "name": "--city", "value": "must-not-leak", "default": "also-not-leak"}],
            "runtimeArguments": []
          }]
        },
        "_meta": {"io.modelcontextprotocol.registry/official": {"status": "active", "isLatest": true}}
      }],
      "metadata": {"nextCursor": "io.example/weather:1.2.3", "count": 1}
    }"#;

    #[test]
    fn projection_is_display_only_and_never_serializes_input_values() {
        let response = parse_list_response(LIST_JSON.as_bytes()).unwrap();
        assert_eq!(response.count, 1);
        assert_eq!(
            response.next_cursor.as_deref(),
            Some("io.example/weather:1.2.3")
        );
        let server = &response.servers[0];
        assert_eq!(server.name, "io.example/weather");
        assert_eq!(server.status.as_deref(), Some("active"));
        assert!(server.is_latest.unwrap_or(false));
        assert!(server.remotes[0].importable);
        assert!(server.remotes[0].requires_configuration);
        assert_eq!(server.remotes[0].variables[0].name, "token");
        assert!(server.remotes[0].variables[0].description.is_none());
        assert!(!server.packages[0].importable);
        assert!(server.packages[0].incompatibility_reason.is_some());
        let serialized = serde_json::to_string(server).unwrap();
        assert!(serialized.contains("WEATHER_KEY"));
        assert!(!serialized.contains("top-secret"));
        assert!(!serialized.contains("header-secret"));
        assert!(!serialized.contains("must-not-leak"));
        assert!(!serialized.contains("also-not-leak"));
        assert!(!serialized.contains("API token"));
    }

    #[test]
    fn hides_inactive_historical_and_unmarked_records() {
        let mut inactive: Value = serde_json::from_str(LIST_JSON).unwrap();
        inactive["servers"][0]["_meta"]["io.modelcontextprotocol.registry/official"]["status"] =
            Value::String("inactive".to_string());
        assert_eq!(
            parse_list_response(serde_json::to_vec(&inactive).unwrap().as_slice())
                .unwrap()
                .count,
            0
        );

        let mut historical: Value = serde_json::from_str(LIST_JSON).unwrap();
        historical["servers"][0]["_meta"]["io.modelcontextprotocol.registry/official"]
            ["isLatest"] = Value::Bool(false);
        assert_eq!(
            parse_list_response(serde_json::to_vec(&historical).unwrap().as_slice())
                .unwrap()
                .count,
            0
        );

        let mut unmarked: Value = serde_json::from_str(LIST_JSON).unwrap();
        unmarked["servers"][0]["_meta"]["io.modelcontextprotocol.registry/official"]
            .as_object_mut()
            .unwrap()
            .remove("isLatest");
        assert_eq!(
            parse_list_response(serde_json::to_vec(&unmarked).unwrap().as_slice())
                .unwrap()
                .count,
            0
        );

        let detail = historical["servers"][0].clone();
        assert!(parse_detail_response(serde_json::to_vec(&detail).unwrap().as_slice()).is_err());
    }

    #[test]
    fn only_concrete_https_remotes_produce_disabled_local_only_drafts() {
        let response = parse_list_response(LIST_JSON.as_bytes()).unwrap();
        let server = &response.servers[0];
        let draft = build_remote_draft(server, &server.remotes[0]).unwrap();
        assert_eq!(draft.id, "mcp-io-example-weather");
        assert_eq!(draft.transport, "http");
        assert_eq!(draft.url, "https://example.com/mcp");
        assert!(!draft.enabled);
        assert!(!draft.allow_remote);
        assert_eq!(draft.timeout_ms, DEFAULT_DRAFT_TIMEOUT_MS);
        assert_eq!(draft.headers[0].name, "X-Api-Key");
        assert!(draft.headers[0].description.is_none());
        let serialized = serde_json::to_string(&draft).unwrap();
        assert!(!serialized.contains("header-secret"));

        let mut value: Value = serde_json::from_str(LIST_JSON).unwrap();
        value["servers"][0]["server"]["name"] = Value::String("bad/name/extra".to_string());
        assert!(parse_list_response(serde_json::to_vec(&value).unwrap().as_slice()).is_err());

        let remote = project_remote(WireRemote {
            transport: "streamable-http".to_string(),
            url: "https://example.com/{token}".to_string(),
            headers: Vec::new(),
            variables: Map::new(),
        })
        .unwrap();
        assert!(!remote.importable);
        assert!(remote.incompatibility_reason.is_some());

        let http_remote = project_remote(WireRemote {
            transport: "streamable-http".to_string(),
            url: "http://example.com/mcp".to_string(),
            headers: Vec::new(),
            variables: Map::new(),
        })
        .unwrap();
        assert!(!http_remote.importable);
        assert!(build_remote_draft(server, &http_remote).is_err());

        let query_remote = project_remote(WireRemote {
            transport: "streamable-http".to_string(),
            url: "https://example.com/mcp?token=publisher-token".to_string(),
            headers: Vec::new(),
            variables: Map::new(),
        })
        .unwrap();
        assert!(!query_remote.importable);
        assert!(query_remote.query_redacted);
        assert!(!query_remote.url.contains('?'));
        assert!(!serde_json::to_string(&query_remote)
            .unwrap()
            .contains("publisher-token"));
        assert!(build_remote_draft(server, &query_remote).is_err());

        let encoded_template = project_remote(WireRemote {
            transport: "sse".to_string(),
            url: "https://example.com/%7Btoken%7D".to_string(),
            headers: Vec::new(),
            variables: Map::new(),
        })
        .unwrap();
        assert!(!encoded_template.importable);
        assert!(build_remote_draft(server, &encoded_template).is_err());
    }

    #[test]
    fn cursor_is_not_trimmed_and_query_path_is_encoded() {
        let url = build_list_url(Some("weather"), Some("opaque cursor"), 7).unwrap();
        assert_eq!(url.host_str(), Some("registry.modelcontextprotocol.io"));
        assert!(url.as_str().contains("version=latest"));
        assert_eq!(
            url.query_pairs()
                .find_map(|(key, value)| (key == "cursor").then(|| value.into_owned())),
            Some("opaque cursor".to_string())
        );
        let detail = build_detail_url("io.example/weather", "1.0.0+build").unwrap();
        assert_eq!(
            detail
                .path_segments()
                .map(|segments| segments.collect::<Vec<_>>()),
            Some(vec![
                "v0.1",
                "servers",
                "io.example%2Fweather",
                "versions",
                "1.0.0+build"
            ])
        );
        assert_eq!(require_latest_version(None).unwrap(), "latest");
        assert!(require_latest_version(Some("1.0.0".to_string())).is_err());
    }

    #[test]
    fn validates_query_limits_and_public_urls() {
        assert!(validate_limit(Some(0)).is_err());
        assert!(validate_limit(Some((MAX_LIMIT + 1) as u32)).is_err());
        assert!(validate_cursor(Some("bad\nvalue".to_string())).is_err());
        assert!(validate_server_name("io.example/weather").is_ok());
        assert!(validate_server_name("io.example/weather/name").is_err());
        assert!(optional_public_url(Some("javascript:alert(1)"), "website")
            .unwrap()
            .is_none());
    }

    #[test]
    fn pi_safe_ids_are_ascii_bounded_and_stable() {
        assert_eq!(
            pi_safe_server_id("io.example/weather"),
            "mcp-io-example-weather"
        );
        assert_eq!(pi_safe_server_id("--A__B..C--"), "mcp-a-b-c");
        let long = pi_safe_server_id(&"a".repeat(400));
        assert!(long.len() <= 128);
        assert!(long
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'));
    }
}
