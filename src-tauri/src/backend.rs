//! Native capabilities used by the NovaVei shell.
//!
//! The Pi runtime intentionally lives in the WebView. This module owns the
//! parts that must remain native: persistence, workspace/file access, shell
//! execution, and the small event transport used by an embedded Pi runner.

use crate::diagnostics;
use crate::history_store::{
    HistoryStore, StoredHistoryHeader, StoredHistorySegment, StoredMessage, StoredSegmentedHistory,
    StoredSession, StoredSessionGoal, StoredTurnTrace, StoredTurnTraceTool,
    DEFAULT_UI_MESSAGE_PAGE_SIZE, MAX_SESSION_GOAL_TEXT_CHARS, MAX_UI_MESSAGE_PAGE_SIZE,
};
use crate::local_services::{LocalServices, MemoryCreateInput, MemoryEntry};
use crate::mcp_runtime::{
    McpCallToolRequest, McpCallToolResponse, McpRuntimeManager, McpRuntimeStatus,
    McpRuntimeTestResponse, McpServerConfig, McpStopServerResponse, McpToolInfo,
};
use crate::path_display::{path_for_display, path_string_for_display};
use crate::secret_store::{protect_settings, unprotect_settings};
use crate::subagent_store::{
    worktree_cleanup_terminal_status, NewSubagentTask, StoredSubagentMessage, StoredSubagentTask,
    SubagentStore, SubagentTaskStatus, WorktreeCleanupDisposition,
};
use crate::worktree_runtime::{self, WorktreeLease, WorktreePatch};
use chrono::Utc;
use parking_lot::Mutex;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
#[cfg(feature = "desktop")]
use tauri::{AppHandle, WebviewWindow};
#[cfg(not(feature = "desktop"))]
type AppHandle<R = tauri::test::MockRuntime> = tauri::AppHandle<R>;
#[cfg(not(feature = "desktop"))]
type WebviewWindow<R = tauri::test::MockRuntime> = tauri::WebviewWindow<R>;
use tauri::{Emitter, State};
use uuid::Uuid;
use walkdir::WalkDir;

const MAX_READ_BYTES: usize = 4 * 1024 * 1024;
const MAX_WRITE_BYTES: usize = 16 * 1024 * 1024;
const MAX_COMMAND_OUTPUT_BYTES: usize = 512 * 1024;
const MAX_PROVIDER_TEST_BODY_BYTES: usize = 4 * 1024 * 1024;
// Model discovery is a read-only convenience operation, not a general
// provider proxy. Keep both the response and the renderer-facing projection
// deliberately small even when a custom gateway returns an unexpectedly
// large catalogue.
const MAX_PROVIDER_MODELS_BODY_BYTES: usize = 1024 * 1024;
const MAX_PROVIDER_MODELS_RETURNED: usize = 256;
const MAX_PROVIDER_MODEL_ID_BYTES: usize = 256;
const MAX_PROVIDER_MODEL_LABEL_BYTES: usize = 256;
const MAX_PROVIDER_MODEL_PAGES: usize = 4;
const MAX_PROVIDER_MODEL_CURSOR_BYTES: usize = 1024;
const MAX_PROVIDER_ID_BYTES: usize = 128;
const MAX_PROVIDER_NAME_CHARS: usize = 128;
const MAX_PROVIDER_CUSTOM_HEADERS: usize = 32;
const MAX_PROVIDER_HEADER_NAME_BYTES: usize = 128;
const MAX_PROVIDER_HEADER_VALUE_BYTES: usize = 8 * 1024;
const MAX_PROVIDER_API_KEY_BYTES: usize = 8 * 1024;
const PROVIDER_CONNECTION_DRAFT_TTL: Duration = Duration::from_secs(5 * 60);
// A Full capability is exceptionally broad. Its native-memory-only grant is
// deliberately short-lived, one-use, and kept outside renderer-controlled
// settings.
const FULL_PERMISSION_GRANT_TTL: Duration = Duration::from_secs(2 * 60);
const MAX_PENDING_FULL_PERMISSION_GRANTS: usize = 64;
const MAX_FULL_PERMISSION_RUN_ID_CHARS: usize = 256;
const MAX_FULL_PERMISSION_PROMPT_CHARS: usize = 2_000;
// Third-party provider imports are deliberately a small, user-selected
// interchange boundary. They are never a directory scan or a way to ferry
// credentials through the WebView.
const MAX_PROVIDER_IMPORT_FILE_BYTES: u64 = 1024 * 1024;
const MAX_PROVIDER_IMPORT_RECORDS: usize = 128;
const MAX_PROVIDER_IMPORT_MODELS: usize = 256;
const MAX_PROVIDER_IMPORT_NAME_CHARS: usize = 128;
const MAX_PROVIDER_IMPORT_URL_BYTES: usize = 2048;
const MAX_PROVIDER_IMPORT_PREVIEWS: usize = 8;
const PROVIDER_IMPORT_PREVIEW_TTL: Duration = Duration::from_secs(5 * 60);
// A third-party import is deliberately narrower than manually editing a
// provider. These are the public API roots that can be safely preserved in a
// renderer-visible provider setting; arbitrary path segments could themselves
// be exported credentials or tenant secrets.
const PROVIDER_IMPORT_SAFE_API_PATHS: &[&str] = &[
    "",
    "/v1",
    "/v1beta",
    "/v1alpha",
    "/api/v1",
    "/openai/v1",
    "/compatible-mode/v1",
    "/api/paas/v4",
];
// Session model selection is intentionally a tiny, canonical DTO. The
// renderer must never use history metadata as a transport for provider
// credentials, custom headers, or arbitrary provider configuration.
const MAX_SESSION_MODEL_SELECTION_JSON_BYTES: usize =
    MAX_PROVIDER_ID_BYTES + MAX_PROVIDER_MODEL_ID_BYTES + 64;
// Projects are durable identities, not path-derived keys.  A project can move
// while its historical sessions still retain the old workspace metadata, so a
// path hash must never be the sole identifier for project preferences.
const PROJECT_SETTINGS_VERSION: u64 = 2;
const PROJECT_STABLE_ID_PREFIX: &str = "project-";
pub const SECONDARY_LAUNCH_FOCUS_EXISTING: &str = "focus-existing";
pub const SECONDARY_LAUNCH_NEW_WINDOW: &str = "new-window";
// This private scope is native-only. It lets a portable copy distinguish a
// deliberate project list on this computer from paths carried over with a
// copied USB/database. It is never returned to the WebView.
const PORTABLE_PROJECT_SCOPE: &str = "__portableProjectScope";
const PORTABLE_PROJECT_SCOPE_VERSION: u64 = 1;
const MAX_PICKED_FILES: usize = 20;
const MAX_PICKED_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PICKED_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
// Composer media is copied into session-scoped application storage before the
// WebView can render it.  This keeps native file-picker paths out of the DOM
// and bounds the bytes that can cross the IPC boundary for previews.
const MAX_COMPOSER_MEDIA_BYTES: u64 = 8 * 1024 * 1024;
const MAX_COMPOSER_MEDIA_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_COMPOSER_MEDIA_NAME_CHARS: usize = 160;
const MAX_COMPOSER_MEDIA_METADATA_BYTES: u64 = 8 * 1024;
const MAX_COMPOSER_MEDIA_IPC_HEADER_BYTES: usize = 1024;
// The pasted-image request carries workspace-bound metadata in a small binary
// envelope ahead of the raw image bytes. It is larger than the renderer-facing
// response header because a canonical workspace path may be several KiB.
const MAX_COMPOSER_PASTED_IMAGE_IPC_HEADER_BYTES: usize = 16 * 1024;
const COMPOSER_PASTED_IMAGE_IPC_MAGIC: &[u8; 4] = b"NVPI";
const COMPOSER_PASTED_IMAGE_IPC_VERSION: u8 = 1;
const COMPOSER_PASTED_IMAGE_IPC_PREFIX_BYTES: usize = 9;
const MAX_COMPOSER_IMAGE_DIMENSION: u32 = 16_384;
const MAX_COMPOSER_IMAGE_PIXELS: u64 = 40_000_000;
const MAX_COMPOSER_IMAGE_FRAMES: u32 = 120;
const MAX_COMPOSER_IMAGE_DECODED_PIXELS: u64 = 160_000_000;
const MAX_COMPOSER_MEDIA_MARKER_BYTES: usize = 64 * 1024;
const COMPOSER_MEDIA_DIRECTORY: &str = "composer-media";
const COMPOSER_MEDIA_MARKER_PREFIX: &str = "[novavei-media:";
const DEFAULT_SHELL_TIMEOUT_MS: u64 = 120_000;
const MAX_SHELL_TIMEOUT_MS: u64 = 10 * 60_000;
// Git Review is a deliberately small native capability, not a general shell.
// Keep the read path quick and give a confirmed commit enough time for normal
// local hooks without allowing a stalled hook to hold the desktop UI forever.
const GIT_STATUS_TIMEOUT_MS: u64 = 10_000;
const GIT_COMMIT_TIMEOUT_MS: u64 = 60_000;
const MAX_GIT_STATUS_ENTRIES: usize = 1_000;
const MAX_GIT_COMMIT_MESSAGE_CHARS: usize = 4_000;
const GIT_COMMIT_GRANT_TTL: Duration = Duration::from_secs(2 * 60);
const MAX_PENDING_GIT_COMMIT_GRANTS: usize = 64;
// A shell descendant can retain a copied stdout/stderr handle after its direct
// parent has exited. Never let that keep a Tauri IPC request open forever.
const SHELL_STDIO_DRAIN_TIMEOUT_MS: u64 = 250;
const SHELL_REAP_TIMEOUT_MS: u64 = 3_000;
const MAX_MCP_SERVER_ID_BYTES: usize = 128;
const MAX_MCP_PERMISSION_TOOL_NAME_BYTES: usize = 1024;
const MAX_SESSION_GOAL_SESSION_ID_BYTES: usize = 128;
const MAX_HISTORY_TRACE_SESSION_ID_BYTES: usize = 128;
const MAX_HISTORY_TRACE_MESSAGE_ID_BYTES: usize = 256;
const MAX_HISTORY_TRACE_TURN_ID_BYTES: usize = 128;
const MAX_HISTORY_TRACE_TOOL_NAME_CHARS: usize = 160;
const MAX_HISTORY_TRACE_TOOL_STATUS_CHARS: usize = 48;
const MAX_HISTORY_SEARCH_QUERY_CHARS: usize = 256;
const MAX_HISTORY_SEARCH_RESULTS: usize = 50;
const MAX_HISTORY_SEARCH_PREVIEW_CHARS: usize = 240;
const MAX_HISTORY_SEARCH_TITLE_CHARS: usize = 200;
const MAX_HISTORY_SEARCH_MESSAGE_CHARS: usize = 100_000;
// Search must never create a second plaintext durable store: conversation
// segments are protected at rest.  This bounded-lifetime index exists only in
// the process alongside the already-decrypted session cache and vanishes when
// NovaVei exits.
const HISTORY_SEARCH_INDEX_GRAM_CHARS: usize = 3;
// Workspace paths are renderer-visible history metadata. Bound every path
// probe and metadata search before it reaches filesystem APIs or a large
// in-memory sort.
const MAX_WORKSPACE_STATUS_PATHS: usize = 512;
const MAX_WORKSPACE_PATH_CHARS: usize = 4 * 1024;
const MAX_SESSION_METADATA_SEARCH_RESULTS: usize = 50;
// A relocation has a different trust boundary from opening a project.  The
// selected destination must come from a fresh native picker invocation that
// was explicitly initiated for the historical source path, and cannot be
// replayed as a generic previously-approved workspace.
const RELOCATION_PICKER_GRANT_TTL: Duration = Duration::from_secs(5 * 60);
const RELOCATION_CONFLICT_GRANT_TTL: Duration = Duration::from_secs(2 * 60);
const MAX_PENDING_RELOCATION_CONFLICT_GRANTS: usize = 64;
const MAX_GLOBAL_SYSTEM_PROMPT_CHARS: usize = 32_000;
const MAX_SUBAGENT_TASK_CHARS: usize = 4_000;
const MAX_CONCURRENT_SUBAGENTS_PER_PARENT: usize = 4;
// The renderer may choose only the project-root policy.  An earlier prototype
// exposed an `extra` option without a native per-session allowlist or a way to
// bind an extra root to a capability.  Treating that value as a grant would
// make the picker state an implicit, cross-session filesystem authority.
const WORKDIR_POLICY_PROJECT: &str = "project";
const WORKDIR_POLICY_EXTRA: &str = "extra";
// Native-only metadata used to prevent a redacted settings payload from
// carrying a credential across provider endpoints or auth protocol families.
const PROVIDER_CREDENTIAL_BINDING_KEY: &str = "__credentialBinding";

type PersistenceSnapshot = (
    Vec<StoredSession>,
    Vec<StoredMessage>,
    HashMap<String, Value>,
);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub updated_at: i64,
    #[serde(default)]
    pub message_count: i64,
    #[serde(default)]
    pub is_pinned: bool,
    #[serde(default)]
    pub is_archived: bool,
    /// Last durable Pi terminal outcome, projected from the local turn audit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_finished_at: Option<i64>,
}

/// A deliberately small filesystem-health projection for an already-known
/// workspace path. It never includes an operating-system error string: those
/// can reveal local machine details and are not actionable in the shell.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePathStatus {
    pub path: String,
    pub accessible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePathsStatusResponse {
    pub paths: Vec<WorkspacePathStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRecord {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: i64,
    #[serde(default)]
    pub turn_id: Option<String>,
    /// Model id used for this assistant turn (projected from turns on read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Reasoning level for this assistant turn (projected from turns on read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Turn completion time in ms (projected from turns.finished_at on read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    /// Redacted provider thinking projected from the completed terminal event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionGoalStatus {
    Active,
    Completed,
}

impl SessionGoalStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionGoal {
    pub text: String,
    pub status: SessionGoalStatus,
    pub progress: u8,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionRecord {
    summary: SessionSummary,
    /// Working cache for the active/open session only; may be a partial page.
    #[serde(default)]
    messages: Vec<MessageRecord>,
    /// Durable message count from SQLite (or in-memory for brand-new sessions).
    #[serde(default)]
    message_count: i64,
    /// True when `messages` reflects at least a page from DB (or an empty new session).
    #[serde(default = "default_true")]
    messages_loaded: bool,
    /// True when `messages` holds the full transcript (or empty).
    #[serde(default = "default_true")]
    messages_complete: bool,
    #[serde(default = "default_provider_id")]
    provider_id: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    selected_model_json: Option<String>,
    #[serde(default)]
    pinned_at: Option<i64>,
    #[serde(default)]
    archived_at: Option<i64>,
    #[serde(default)]
    share_enabled: bool,
    #[serde(default)]
    share_token: Option<String>,
    #[serde(default)]
    share_created_at: Option<i64>,
    #[serde(default)]
    share_updated_at: Option<i64>,
    #[serde(default = "default_redact_tool_content")]
    redact_tool_content: bool,
    #[serde(default)]
    goal: Option<SessionGoal>,
}

#[derive(Debug, Clone)]
struct InMemoryHistorySearchEntry {
    conversation_id: String,
    conversation_title: String,
    message_id: String,
    role: String,
    content: String,
    search_content: String,
    created_at: i64,
}

/// An encrypted SQLite database cannot safely use ordinary FTS: its auxiliary
/// tables would expose transcript terms. This is a process-lifetime index of
/// the same user/assistant text already loaded for the renderer. It maintains
/// a small character-ngram posting map so repeated searches do not repeatedly
/// normalize and scan every loaded message. Nothing from this struct is ever
/// serialized or persisted.
#[derive(Debug, Default)]
struct InMemoryHistorySearchIndex {
    source_stamp: Option<[u8; 32]>,
    entries: Vec<InMemoryHistorySearchEntry>,
    grams: HashMap<String, Vec<usize>>,
}

impl InMemoryHistorySearchIndex {
    fn search(
        &mut self,
        sessions: &HashMap<String, SessionRecord>,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Vec<Value> {
        let source_stamp = history_search_source_stamp(sessions);
        if self.source_stamp != Some(source_stamp) {
            self.rebuild(sessions, source_stamp);
        }

        let query_grams = history_search_grams(query);
        let candidate_indices = if query_grams.is_empty() {
            (0..self.entries.len()).collect::<Vec<_>>()
        } else {
            let mut postings = Vec::with_capacity(query_grams.len());
            for gram in &query_grams {
                let Some(indices) = self.grams.get(gram) else {
                    return Vec::new();
                };
                postings.push(indices);
            }
            postings.sort_by_key(|indices| indices.len());
            let mut candidates = postings[0].clone();
            for indices in postings.into_iter().skip(1) {
                candidates.retain(|candidate| indices.binary_search(candidate).is_ok());
                if candidates.is_empty() {
                    break;
                }
            }
            candidates
        };

        let mut matches = candidate_indices
            .into_iter()
            .filter_map(|index| self.entries.get(index))
            .filter(|entry| entry.search_content.contains(query))
            .map(|entry| {
                (
                    entry.created_at,
                    json!({
                        "conversationId": entry.conversation_id,
                        "conversationTitle": entry.conversation_title,
                        "messageId": entry.message_id,
                        "role": entry.role,
                        "text": history_search_preview(&entry.content, query),
                        "updatedAt": entry.created_at,
                    }),
                )
            })
            .collect::<Vec<_>>();
        matches.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        matches
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|(_, value)| value)
            .collect()
    }

    fn rebuild(&mut self, sessions: &HashMap<String, SessionRecord>, source_stamp: [u8; 32]) {
        self.entries.clear();
        self.grams.clear();
        for record in sessions.values() {
            let conversation_id = record.summary.id.trim();
            if conversation_id.is_empty()
                || conversation_id.len() > MAX_HISTORY_TRACE_SESSION_ID_BYTES
                || conversation_id.chars().any(char::is_control)
            {
                continue;
            }
            let conversation_title =
                history_search_normalize(&record.summary.title, MAX_HISTORY_SEARCH_TITLE_CHARS);
            for message in &record.messages {
                let role = message.role.to_ascii_lowercase();
                if !matches!(role.as_str(), "user" | "assistant") {
                    continue;
                }
                let message_id = message.id.trim();
                if message_id.is_empty()
                    || message_id.len() > MAX_HISTORY_TRACE_MESSAGE_ID_BYTES
                    || message_id.chars().any(char::is_control)
                {
                    continue;
                }
                let content =
                    history_search_normalize(&message.content, MAX_HISTORY_SEARCH_MESSAGE_CHARS);
                if content.is_empty() {
                    continue;
                }
                let search_content = content.to_lowercase();
                let entry_index = self.entries.len();
                for gram in history_search_grams(&search_content) {
                    self.grams.entry(gram).or_default().push(entry_index);
                }
                self.entries.push(InMemoryHistorySearchEntry {
                    conversation_id: conversation_id.to_string(),
                    conversation_title: conversation_title.clone(),
                    message_id: message_id.to_string(),
                    role,
                    content,
                    search_content,
                    created_at: message.created_at,
                });
            }
        }
        self.source_stamp = Some(source_stamp);
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SessionModelSelection {
    provider_id: String,
    model_id: String,
}

fn default_provider_id() -> String {
    "embedded".to_string()
}

fn default_redact_tool_content() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedState {
    version: u32,
    #[serde(default)]
    sessions: Vec<SessionRecord>,
    #[serde(default)]
    settings: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
struct ActiveRun {
    session_id: String,
    conversation_id: String,
    turn_id: String,
    request_id: String,
    // Resolved from the native provider snapshot at registration time. This
    // is deliberately distinct from the renderer's requested provider id:
    // proxy credentials may only be released for this canonical selection.
    proxy_provider_id: Option<String>,
    capability_token: String,
    cancelled: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
struct CapabilityGrant {
    session_id: String,
    conversation_id: String,
    turn_id: String,
    request_id: String,
    workdir: PathBuf,
    permission_mode: PermissionMode,
    expires_at: Instant,
    cancelled: Arc<AtomicBool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FullPermissionRunBinding {
    provider_id: String,
    model_id: String,
    reasoning: Option<String>,
}

/// Native-memory-only grant for one exact Full-access run. This is
/// intentionally not serializable and never persisted in `system` settings:
/// a WebView can write generic settings, so those settings cannot constitute
/// an authorization boundary.
#[derive(Debug, Clone)]
struct FullPermissionRunGrant {
    session_id: String,
    conversation_id: String,
    request_id: String,
    workdir: PathBuf,
    prompt_digest: [u8; 32],
    run_binding: FullPermissionRunBinding,
    expires_at: Instant,
}

/// A child uses a capability distinct from its parent run. This token can
/// authorize only the explicit read-only workspace registry and is revoked
/// whenever the parent turn ends.
#[derive(Debug, Clone)]
struct SubagentCapabilityGrant {
    task_id: String,
    session_id: String,
    parent_turn_id: String,
    parent_request_id: String,
    proxy_request_id: String,
    workdir: PathBuf,
    mode: SubagentCapabilityMode,
    /// Granted only after the delegation confirmation card explicitly includes
    /// global read access. This never grants any filesystem mutation.
    allow_global_read: bool,
    expires_at: Instant,
    cancelled: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubagentCapabilityMode {
    Readonly,
    Worktree,
}

#[derive(Debug, Clone)]
struct WorkspaceCapabilityGrant {
    session_id: String,
    workdir: PathBuf,
    expires_at: Instant,
}

#[derive(Debug, Clone)]
struct GitCommitCapabilityGrant {
    session_id: String,
    workdir: PathBuf,
    message_digest: [u8; 32],
    staged_digest: [u8; 32],
    staged_count: usize,
    expires_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingPermission {
    capability_token: String,
    session_id: String,
    conversation_id: String,
    turn_id: String,
    request_id: String,
    tool_name: String,
    /// Only delegated tasks may request this. It is copied into the one-use
    /// approval after the user accepts the visible confirmation card.
    subagent_global_read: bool,
}

#[derive(Debug, Clone)]
struct ToolApprovalGrant {
    capability_token: String,
    action: ToolAction,
    // Tool calls with the MCP action are dynamically named. Keep the exact
    // name from the permission request so an approval for one configured MCP
    // tool cannot be replayed against another server or tool.
    tool_name: String,
    /// A delegation approval is bound to whether it may mint a child that can
    /// use GlobalRead. Ordinary approvals always keep this false.
    subagent_global_read: bool,
    expires_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionMode {
    Readonly,
    Ask,
    Full,
}

impl PermissionMode {
    fn parse(value: Option<&str>) -> Self {
        match value.unwrap_or("ask").trim().to_ascii_lowercase().as_str() {
            "readonly" | "read-only" | "只读" => Self::Readonly,
            "full" | "完全访问权限" => Self::Full,
            _ => Self::Ask,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Readonly => "readonly",
            Self::Ask => "ask",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ToolAction {
    Read,
    Write,
    /// Durable memory is a separate mutation class so a one-time approval for
    /// it cannot be replayed as a workspace file write.
    MemoryWrite,
    Edit,
    Delete,
    Shell,
    /// Browser automation can read page content and trigger remote actions.
    /// It is always separate from workspace and shell permissions.
    Browser,
    /// Remote/local MCP effects are never treated as a read-only operation.
    Mcp,
    /// Delegating local workspace analysis sends derived data to a provider.
    /// It is therefore never equivalent to a local `Read` operation.
    Subagent,
    /// A user-approved detached Git worktree lets a child propose changes.
    /// This is kept distinct from ordinary write actions and never enables a
    /// direct write into the parent workspace.
    Worktree,
    /// A Git commit mutates repository history and may execute project hooks.
    /// It is not equivalent to a read-only Git status check.
    GitCommit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionRequirement {
    Allow,
    Approval,
    Deny,
}

fn permission_requirement(mode: PermissionMode, action: ToolAction) -> PermissionRequirement {
    if action == ToolAction::Read {
        return PermissionRequirement::Allow;
    }
    if action == ToolAction::Worktree {
        return match mode {
            PermissionMode::Readonly => PermissionRequirement::Deny,
            _ => PermissionRequirement::Approval,
        };
    }
    match mode {
        PermissionMode::Readonly => PermissionRequirement::Deny,
        PermissionMode::Full => PermissionRequirement::Allow,
        PermissionMode::Ask => PermissionRequirement::Approval,
    }
}

fn tool_action(name: &str) -> Option<ToolAction> {
    let normalized = name.trim().to_ascii_lowercase();
    if normalized.starts_with("mcp__") && normalized.len() > "mcp__".len() {
        return Some(ToolAction::Mcp);
    }
    match normalized.as_str() {
        "read" | "projectread" | "globalread" | "list" | "grep" | "memorysearch" | "skillslist"
        | "skillread" => Some(ToolAction::Read),
        "write" => Some(ToolAction::Write),
        "memorysave" => Some(ToolAction::MemoryWrite),
        "edit" => Some(ToolAction::Edit),
        "delete" | "remove" => Some(ToolAction::Delete),
        "bash" | "shell" | "command" => Some(ToolAction::Shell),
        "browsernavigate" | "browsersnapshot" | "browserclick" | "browsertype" => {
            Some(ToolAction::Browser)
        }
        "delegatereadonly" => Some(ToolAction::Subagent),
        "delegateworktree" => Some(ToolAction::Worktree),
        "gitcommit" | "git_commit" => Some(ToolAction::GitCommit),
        _ => None,
    }
}

fn pending_permission_belongs_to_live_grant(
    state: &AppState,
    pending: &PendingPermission,
    grant: &CapabilityGrant,
) -> bool {
    if pending.session_id != grant.session_id
        || pending.conversation_id != grant.conversation_id
        || pending.turn_id != grant.turn_id
        || pending.request_id != grant.request_id
        || grant.cancelled.load(Ordering::SeqCst)
        || grant.expires_at <= Instant::now()
    {
        return false;
    }
    state.active_runs.lock().values().any(|run| {
        run.capability_token == pending.capability_token
            && run.session_id == grant.session_id
            && run.conversation_id == grant.conversation_id
            && run.turn_id == grant.turn_id
            && run.request_id == grant.request_id
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupPersistenceDecision {
    Persist,
    SkipForStorageRecovery,
    SkipForSettingsRecovery,
}

fn startup_persistence_decision(
    storage_ready: bool,
    settings_ready: bool,
) -> StartupPersistenceDecision {
    if !storage_ready {
        StartupPersistenceDecision::SkipForStorageRecovery
    } else if !settings_ready {
        StartupPersistenceDecision::SkipForSettingsRecovery
    } else {
        StartupPersistenceDecision::Persist
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageRecoveryStatus {
    pub state: String,
    pub errors: Vec<String>,
}

impl StorageRecoveryStatus {
    fn ready() -> Self {
        Self {
            state: "ready".to_string(),
            errors: Vec::new(),
        }
    }

    fn degraded(errors: Vec<String>) -> Self {
        let mut codes = Vec::new();
        for error in errors {
            let code = storage_recovery_error_code(&error).to_string();
            if !codes.contains(&code) {
                codes.push(code);
            }
        }
        Self {
            state: "degraded".to_string(),
            // This legacy compatibility field intentionally contains only
            // stable categories. Raw SQLite/filesystem/DPAPI details remain
            // in native diagnostics and are never sent to the WebView.
            errors: codes,
        }
    }

    fn is_ready(&self) -> bool {
        self.state == "ready"
    }
}

fn storage_recovery_error_code(error: &str) -> &'static str {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("history")
        || normalized.contains("sqlite")
        || normalized.contains("session")
    {
        "session_store_unavailable"
    } else if normalized.contains("settings") || normalized.contains("decrypt") {
        "settings_unavailable"
    } else {
        "local_storage_unavailable"
    }
}

fn initial_persist_failure_recovery_status() -> StorageRecoveryStatus {
    // The first encrypted snapshot is the point at which an in-memory startup
    // projection becomes durable. If it fails, do not leave the renderer with
    // a ready-looking state that will lose its next mutation on restart.
    StorageRecoveryStatus::degraded(vec![
        "initial persistence failed: recovery is required before local state can be changed"
            .to_string(),
    ])
}

/// Renderer-safe health projection for the capabilities that are required to
/// create or persist a local conversation.  This deliberately contains only
/// closed, stable enum values: underlying SQLite, filesystem, DPAPI, proxy,
/// and operating-system errors stay in native diagnostics instead of becoming
/// a WebView disclosure channel.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppHealth {
    pub session_store: AppHealthSessionStore,
    pub settings: AppHealthSettings,
    pub writes: AppHealthWrites,
    pub proxy: AppHealthProxy,
    pub recovery_guidance: AppHealthRecoveryGuidance,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppHealthSessionStore {
    Ready,
    RecoveryRequired,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppHealthSettings {
    Ready,
    Locked,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppHealthWrites {
    Enabled,
    Blocked,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppHealthProxy {
    Ready,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppHealthRecoveryGuidance {
    None,
    RestartAndCheckLocalStorage,
    UnlockProtectedSettings,
}

/// State shared by all native commands.  The maps are intentionally small and
/// local; long-lived Pi state remains owned by the WebView runtime.
pub struct AppState {
    data_file: PathBuf,
    history: HistoryStore,
    subagent_tasks: SubagentStore,
    sessions: Mutex<HashMap<String, SessionRecord>>,
    history_search_index: Mutex<InMemoryHistorySearchIndex>,
    settings: Mutex<HashMap<String, Value>>,
    // The entries contain only normalized, non-secret provider settings from
    // a user-picked JSON export.  They are short-lived and consumed exactly
    // once by the import-apply command.
    provider_import_previews: Mutex<HashMap<String, ProviderImportDraft>>,
    // Provider connection drafts are an in-memory, short-lived bridge for
    // testing unsaved editor fields. The renderer receives an opaque token,
    // never a reusable native connection configuration.
    provider_connection_drafts: Mutex<HashMap<String, ProviderConnectionDraft>>,
    persist_lock: Mutex<()>,
    settings_locked: AtomicBool,
    storage_recovery: StorageRecoveryStatus,
    // Only roots deliberately approved by the user (a durable project, the
    // process workdir, or this-process picker selection) belong here. Durable
    // session cwd metadata remains visible, but is never an implicit grant.
    approved_workdirs: Mutex<HashSet<PathBuf>>,
    // Picker roots are process-local and intentionally distinct from durable
    // project registrations. Rebuilding `approved_workdirs` after a project
    // settings save must not accidentally retain a removed project, nor lose
    // a folder the user explicitly selected in this running process.
    picker_workdirs: Mutex<HashSet<PathBuf>>,
    relocation_picker_grants: Mutex<HashMap<String, RelocationPickerGrant>>,
    relocation_conflict_grants: Mutex<HashMap<String, RelocationConflictGrant>>,
    active_runs: Mutex<HashMap<String, ActiveRun>>,
    capabilities: Mutex<HashMap<String, CapabilityGrant>>,
    full_permission_grants: Mutex<HashMap<String, FullPermissionRunGrant>>,
    subagent_capabilities: Mutex<HashMap<String, SubagentCapabilityGrant>>,
    workspace_capabilities: Mutex<HashMap<String, WorkspaceCapabilityGrant>>,
    git_commit_capabilities: Mutex<HashMap<String, GitCommitCapabilityGrant>>,
    pending_permissions: Mutex<HashMap<String, PendingPermission>>,
    tool_approvals: Mutex<HashMap<String, ToolApprovalGrant>>,
    // Keep the cancellation flag and its capability owner in one lock-protected
    // record.  A cancellation must never observe a run after the process has
    // started but before its owner has been published.
    shell_runs: Mutex<HashMap<String, ShellRunRegistration>>,
}

#[derive(Debug, Clone)]
struct ShellRunRegistration {
    capability_token: String,
    cancelled: Arc<AtomicBool>,
}

/// A short-lived, one-use proof that the native folder picker selected this
/// exact destination while relocating one exact historical workspace key.
#[derive(Debug, Clone)]
struct RelocationPickerGrant {
    to_workdir: PathBuf,
    expires_at: Instant,
}

/// Renderer-opaque pending-conflict proof for one exact relocation snapshot.
/// It proves a fresh picker result and lets native code detect settings drift;
/// it is deliberately not itself approval for either destructive resolution.
#[derive(Debug, Clone)]
struct RelocationConflictGrant {
    from_key: String,
    to_workdir: PathBuf,
    conflict: WorkspaceRelocationConflict,
    expires_at: Instant,
}

#[derive(Debug, Clone)]
struct ProviderImportDraft {
    providers: Vec<Value>,
    existing_public: HashMap<String, Option<Value>>,
    created_at: Instant,
}

#[derive(Debug, Clone)]
struct ProviderConnectionDraft {
    provider: Value,
    created_at: Instant,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDraftPrepareResult {
    pub draft_token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderImportPreview {
    pub import_token: String,
    pub providers: Vec<ProviderImportPreviewItem>,
    pub skipped: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderImportPreviewItem {
    pub id: String,
    pub name: String,
    pub host: String,
    // This is only the public, allowlisted path from the normalized base URL;
    // never the complete URL, query, fragment, credential, or custom header.
    pub api_root: String,
    pub protocol: String,
    pub model_count: usize,
    pub conflict: bool,
    pub has_credential: bool,
    // A local credential exists, but native code will not retain it because
    // this import changes the endpoint or authentication family.
    pub requires_credential_reentry: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderImportApplyResult {
    pub added: usize,
    pub updated: usize,
}

impl AppState {
    pub fn new() -> Self {
        let data_dir = diagnostics::application_data_dir();
        let mut storage_errors = Vec::new();
        if let Err(error) = fs::create_dir_all(&data_dir) {
            storage_errors.push(format!("create application data directory: {error}"));
        }
        let legacy_file = data_dir.join("state.json");
        let data_file = data_dir.join("chat-history.sqlite3");
        let history = HistoryStore::new(data_file.clone());
        let subagent_tasks = SubagentStore::new(data_file.clone());
        // Startup password protection is a native boundary, not only a
        // renderer overlay. While locked, initialize database handles but do
        // not project or mutate durable user state until the password command
        // succeeds and hydrates the in-memory view.
        let startup_unlock_required = crate::secret_store::app_security_needs_unlock();
        if let Err(error) = history.initialize() {
            storage_errors.push(format!("initialize history store: {error}"));
        } else if startup_unlock_required {
            if let Err(error) = subagent_tasks.initialize() {
                storage_errors.push(format!("initialize subagent task store: {error}"));
            }
        } else if let Err(error) = history.mark_interrupted_turns(now_ms()) {
            // Do not continue into the subagent store after an interrupted-turn
            // recovery failure. Both stores share the persistence boundary, and
            // proceeding would make a partially recovered startup look usable.
            storage_errors.push(format!("mark interrupted turns: {error}"));
        } else if let Err(error) = subagent_tasks.initialize() {
            storage_errors.push(format!("initialize subagent task store: {error}"));
        } else if let Err(error) = subagent_tasks.mark_interrupted_tasks(now_ms()) {
            storage_errors.push(format!("mark interrupted subagent tasks: {error}"));
        }
        let legacy_persisted = match fs::read_to_string(&legacy_file) {
            Ok(text) => match serde_json::from_str::<PersistedState>(&text) {
                Ok(persisted) => Some(persisted),
                Err(error) => {
                    storage_errors.push(format!("parse legacy state: {error}"));
                    None
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                storage_errors.push(format!("read legacy state: {error}"));
                None
            }
        };
        let legacy_loaded = legacy_persisted.is_some();
        let persisted = legacy_persisted.unwrap_or_default();
        // A locked startup can contain password-protected settings and
        // transcripts. Do not even project its session metadata into memory
        // until the user has supplied the password; otherwise a startup path
        // could create a replacement default session before the protected
        // corpus is available.
        let stored_sessions = if startup_unlock_required {
            Vec::new()
        } else {
            match history.load_sessions() {
                Ok(records) => records,
                Err(error) => {
                    storage_errors.push(format!("load sessions: {error}"));
                    Vec::new()
                }
            }
        };
        let stored_session_run_summaries = if startup_unlock_required {
            HashMap::new()
        } else {
            match history.load_session_run_summaries() {
                Ok(summaries) => summaries,
                Err(error) => {
                    storage_errors.push(format!("load session run summaries: {error}"));
                    HashMap::new()
                }
            }
        };
        // Session startup needs only message counts; fetching one count per
        // record opens a SQLite connection for every historical session. Load
        // the complete count projection once so long local histories do not
        // make first paint scale with the number of conversations.
        let stored_message_counts = if startup_unlock_required {
            HashMap::new()
        } else {
            match history.load_message_counts() {
                Ok(counts) => counts,
                Err(error) => {
                    storage_errors.push(format!("load session message counts: {error}"));
                    HashMap::new()
                }
            }
        };
        let stored_goals = if startup_unlock_required {
            HashMap::new()
        } else {
            match history.load_session_goals() {
                Ok(goals) => goals,
                Err(error) => {
                    storage_errors.push(format!("load session goals: {error}"));
                    HashMap::new()
                }
            }
        };
        let stored_settings = if startup_unlock_required {
            HashMap::new()
        } else {
            match history.load_settings() {
                Ok(settings) => settings,
                Err(error) => {
                    storage_errors.push(format!("load settings: {error}"));
                    HashMap::new()
                }
            }
        };
        let mut sessions = HashMap::new();
        for record in stored_sessions {
            let last_run = stored_session_run_summaries.get(&record.id);
            let goal = stored_goals
                .get(&record.id)
                .and_then(session_goal_from_stored);
            // Startup loads metadata and counts only. Transcript pages are
            // fetched on demand via sessions_get so large corpora stay off the
            // AppState hot path.
            let message_count = stored_message_counts.get(&record.id).copied().unwrap_or(0);
            let messages_empty = message_count == 0;
            sessions.insert(
                record.id.clone(),
                SessionRecord {
                    summary: SessionSummary {
                        id: record.id,
                        title: record.title,
                        cwd: record.cwd,
                        updated_at: record.updated_at,
                        message_count,
                        is_pinned: false,
                        is_archived: false,
                        last_run_status: last_run.map(|summary| summary.status.clone()),
                        last_run_finished_at: last_run.map(|summary| summary.finished_at),
                    },
                    messages: Vec::new(),
                    message_count,
                    messages_loaded: messages_empty,
                    messages_complete: messages_empty,
                    provider_id: record.provider_id,
                    model: record.model,
                    selected_model_json: record.selected_model_json,
                    pinned_at: record.pinned_at,
                    archived_at: record.archived_at,
                    share_enabled: record.share_enabled,
                    share_token: record.share_token,
                    share_created_at: record.share_created_at,
                    share_updated_at: record.share_updated_at,
                    redact_tool_content: record.redact_tool_content,
                    goal,
                },
            );
        }
        if !startup_unlock_required {
            merge_legacy_sessions(&mut sessions, persisted.sessions);
        }
        let mut raw_settings = if startup_unlock_required {
            HashMap::new()
        } else {
            persisted.settings
        };
        // SQLite wins for scopes already migrated, while a legacy snapshot
        // can still supply scopes that were never written to SQLite.
        raw_settings.extend(stored_settings);
        let (mut settings, settings_ready) = if startup_unlock_required {
            (HashMap::new(), false)
        } else {
            match unprotect_settings(&raw_settings) {
                Ok(settings) => (settings, true),
                Err(error) => {
                    // Never treat ciphertext as a provider credential. Keep the
                    // legacy file/database intact so the user can recover it
                    // under the original Windows profile.
                    let _ = error;
                    diagnostics::record_event("storage", "settings_unlock_failed", "failure", None);
                    // The renderer must not proceed with an apparently writable
                    // shell when every protected settings write will be rejected.
                    // Keep the recovery reason generic: the underlying crypto or
                    // Windows-profile error may reveal sensitive local details.
                    storage_errors.push(
                    "unlock protected settings: recovery is required before local state can be changed"
                        .to_string(),
                );
                    (HashMap::new(), false)
                }
            }
        };
        let settings_reconciled_at_startup = if settings_ready {
            let settings_before_reconciliation = settings.clone();
            reconcile_portable_projects_at_startup(&mut settings);
            settings != settings_before_reconciliation
        } else {
            false
        };
        let mut created_startup_session_id = None;
        if sessions.is_empty() && !startup_unlock_required {
            let record = new_session_record("本地工具执行与权限".to_string(), current_workdir());
            created_startup_session_id = Some(record.summary.id.clone());
            sessions.insert(record.summary.id.clone(), record);
        }

        let storage_recovery = if storage_errors.is_empty() {
            StorageRecoveryStatus::ready()
        } else {
            for error in &storage_errors {
                let _ = error;
                diagnostics::record_event("storage", "recovery_required", "failure", None);
            }
            StorageRecoveryStatus::degraded(storage_errors)
        };

        // Historical session cwd values are display/persistence metadata, not
        // evidence of a current filesystem grant. Only a durable project may
        // restore a previously chosen root at startup.
        let mut approved_workdirs = approved_project_workdirs(&settings);
        if let Ok(current) = canonical_workdir(&current_workdir()) {
            approved_workdirs.insert(current);
        }

        let mut state = Self {
            data_file,
            history,
            subagent_tasks,
            sessions: Mutex::new(sessions),
            history_search_index: Mutex::new(InMemoryHistorySearchIndex::default()),
            settings: Mutex::new(settings),
            provider_import_previews: Mutex::new(HashMap::new()),
            provider_connection_drafts: Mutex::new(HashMap::new()),
            persist_lock: Mutex::new(()),
            settings_locked: AtomicBool::new(!settings_ready),
            storage_recovery,
            approved_workdirs: Mutex::new(approved_workdirs),
            picker_workdirs: Mutex::new(HashSet::new()),
            relocation_picker_grants: Mutex::new(HashMap::new()),
            relocation_conflict_grants: Mutex::new(HashMap::new()),
            active_runs: Mutex::new(HashMap::new()),
            capabilities: Mutex::new(HashMap::new()),
            full_permission_grants: Mutex::new(HashMap::new()),
            subagent_capabilities: Mutex::new(HashMap::new()),
            workspace_capabilities: Mutex::new(HashMap::new()),
            git_commit_capabilities: Mutex::new(HashMap::new()),
            pending_permissions: Mutex::new(HashMap::new()),
            tool_approvals: Mutex::new(HashMap::new()),
            shell_runs: Mutex::new(HashMap::new()),
        };
        // A legacy JSON snapshot still needs one complete projection write.
        // An ordinary SQLite startup has already loaded the durable metadata,
        // so write only a newly-created starter session or reconciled settings.
        // In particular, do not hydrate and replace every transcript merely
        // because the process was restarted.
        if !startup_unlock_required
            && startup_persistence_decision(state.storage_recovery.is_ready(), settings_ready)
                == StartupPersistenceDecision::Persist
        {
            let startup_persist_result = if legacy_loaded {
                state.persist()
            } else {
                (|| -> Result<(), String> {
                    if let Some(session_id) = created_startup_session_id.as_deref() {
                        state.persist_session_locked(session_id)?;
                    }
                    if settings_reconciled_at_startup {
                        state.persist_settings_locked()?;
                    }
                    Ok(())
                })()
            };
            match startup_persist_result {
                Ok(()) => match state.history.secure_compact_once() {
                    Ok(_) if legacy_loaded => retire_legacy_state(&legacy_file),
                    Ok(_) => {}
                    Err(error) => {
                        let _ = error;
                        diagnostics::record_event(
                            "storage",
                            "secure_compaction_failed",
                            "failure",
                            None,
                        )
                    }
                },
                Err(_) => {
                    diagnostics::record_event("storage", "initial_persist_failed", "failure", None);
                    state.storage_recovery = initial_persist_failure_recovery_status();
                }
            }
            // Best-effort: encrypt any plaintext transcripts left from older builds.
            // Do not block the shell when migration fails; diagnostics retain the event.
            if state.storage_recovery.is_ready() {
                match state.history.encrypt_legacy_transcripts_once() {
                    Ok(true) => {
                        // The migration has replaced plaintext rows after the
                        // initial startup compaction. Vacuum once more so the
                        // old SQLite/WAL pages do not retain those values.
                        if state.history.secure_compact_after_protection().is_err() {
                            diagnostics::record_event(
                                "storage",
                                "transcript_encryption_compaction_failed",
                                "failure",
                                None,
                            );
                        }
                    }
                    Ok(false) => {}
                    Err(error) => {
                        let _ = error;
                        diagnostics::record_event(
                            "storage",
                            "transcript_encryption_migration_failed",
                            "failure",
                            None,
                        );
                    }
                }
            }
        }
        state
    }

    pub fn storage_recovery_status(&self) -> StorageRecoveryStatus {
        self.storage_recovery.clone()
    }

    pub fn app_health(&self, proxy_ready: bool) -> AppHealth {
        let session_store_ready = self.storage_recovery.is_ready();
        let settings_ready = !self.settings_locked.load(Ordering::Acquire);
        AppHealth {
            session_store: if session_store_ready {
                AppHealthSessionStore::Ready
            } else {
                AppHealthSessionStore::RecoveryRequired
            },
            settings: if settings_ready {
                AppHealthSettings::Ready
            } else {
                AppHealthSettings::Locked
            },
            writes: if session_store_ready && settings_ready {
                AppHealthWrites::Enabled
            } else {
                AppHealthWrites::Blocked
            },
            proxy: if proxy_ready {
                AppHealthProxy::Ready
            } else {
                AppHealthProxy::Unavailable
            },
            recovery_guidance: if !session_store_ready {
                AppHealthRecoveryGuidance::RestartAndCheckLocalStorage
            } else if !settings_ready {
                AppHealthRecoveryGuidance::UnlockProtectedSettings
            } else {
                AppHealthRecoveryGuidance::None
            },
        }
    }

    /// Native mutation guard. UI controls are only a usability aid; a
    /// recovery-required store must reject every durable mutation before an
    /// in-memory projection can become a ghost session or reply.
    fn require_durable_mutation(&self) -> Result<(), String> {
        if !self.storage_recovery_status().is_ready() {
            return Err(
                "NovaVei storage recovery is required before state can be persisted".to_string(),
            );
        }
        if self.settings_locked.load(Ordering::Acquire) {
            return Err(
                "NovaVei settings are locked because protected values could not be decrypted"
                    .to_string(),
            );
        }
        Ok(())
    }

    // Existing persistence helpers share this guard while callers migrate to
    // the explicit durable-mutation terminology.
    fn require_persistence_ready(&self) -> Result<(), String> {
        self.require_durable_mutation()
    }

    fn persist(&self) -> Result<(), String> {
        let _persist_guard = self.persist_lock.lock();
        self.persist_locked()
    }

    fn persist_locked(&self) -> Result<(), String> {
        let (sessions, messages, protected_settings) = self.snapshot_for_persistence_locked()?;
        self.history
            .replace_snapshot(&sessions, &messages, &protected_settings)
    }

    /// Persist one session's metadata and messages without rewriting the corpus.
    ///
    /// When the in-memory cache is incomplete, never use the full replace
    /// projection (which DELETEs sibling messages). Instead upsert metadata and
    /// INSERT OR REPLACE only the messages currently held in cache.
    fn persist_session_locked(&self, session_id: &str) -> Result<(), String> {
        self.require_persistence_ready()?;
        let record = self
            .sessions
            .lock()
            .get(session_id)
            .cloned()
            .ok_or_else(|| "session not found".to_string())?;
        let session = stored_session_from_record(&record);
        if record.messages_complete {
            let messages = record
                .messages
                .iter()
                .map(|message| stored_message_from_record(&record.summary.id, message))
                .collect::<Vec<_>>();
            self.history.upsert_session_projection(&session, &messages)
        } else {
            let messages = record
                .messages
                .iter()
                .map(|message| stored_message_from_record(&record.summary.id, message))
                .collect::<Vec<_>>();
            self.history
                .upsert_session_metadata_and_messages(&session, &messages)
        }
    }

    /// Persist session fields that do not alter its transcript. Keeping this
    /// separate from `persist_session_locked` prevents a rename, pin, model,
    /// or share preference from re-encrypting and replacing cached messages.
    fn persist_session_metadata_locked(&self, session_id: &str) -> Result<(), String> {
        self.require_persistence_ready()?;
        let record = self
            .sessions
            .lock()
            .get(session_id)
            .cloned()
            .ok_or_else(|| "session not found".to_string())?;
        self.history
            .upsert_session_metadata(&stored_session_from_record(&record))
    }

    /// Persist settings only; never rewrites conversation messages.
    fn persist_settings_locked(&self) -> Result<(), String> {
        self.require_persistence_ready()?;
        let settings = self.settings.lock().clone();
        let protected_settings = protect_settings(&settings)?;
        self.history.upsert_settings(&protected_settings)
    }

    fn snapshot_for_persistence_locked(&self) -> Result<PersistenceSnapshot, String> {
        self.require_persistence_ready()?;
        let session_records = self.sessions.lock().values().cloned().collect::<Vec<_>>();
        let settings = self.settings.lock().clone();
        let sessions = session_records
            .iter()
            .map(|record| StoredSession {
                id: record.summary.id.clone(),
                title: record.summary.title.clone(),
                cwd: record.summary.cwd.clone(),
                updated_at: record.summary.updated_at,
                provider_id: record.provider_id.clone(),
                model: record.model.clone(),
                selected_model_json: record.selected_model_json.clone(),
                pinned_at: record.pinned_at,
                archived_at: record.archived_at,
                share_enabled: record.share_enabled,
                share_token: record.share_token.clone(),
                share_created_at: record.share_created_at,
                share_updated_at: record.share_updated_at,
                redact_tool_content: record.redact_tool_content,
            })
            .collect::<Vec<_>>();
        // Full corpus rewrite must never drop durable rows that are only on
        // disk. Incomplete caches hydrate from SQLite and merge the working set.
        let mut messages = Vec::new();
        for record in &session_records {
            if record.messages_complete {
                messages.extend(
                    record
                        .messages
                        .iter()
                        .map(|message| stored_message_from_record(&record.summary.id, message)),
                );
                continue;
            }
            let mut durable = self.history.load_messages(&record.summary.id)?;
            for message in &record.messages {
                let stored = stored_message_from_record(&record.summary.id, message);
                if let Some(existing) = durable.iter_mut().find(|row| row.id == stored.id) {
                    *existing = stored;
                } else {
                    durable.push(stored);
                }
            }
            durable.sort_by(|left, right| {
                left.created_at
                    .cmp(&right.created_at)
                    .then_with(|| left.id.cmp(&right.id))
            });
            messages.extend(durable);
        }
        let protected_settings = protect_settings(&settings)?;
        Ok((sessions, messages, protected_settings))
    }

    /// Unlock the portable key envelope, then hydrate the durable projection
    /// that startup deliberately withheld. The password is consumed by the
    /// secret store and never retained by AppState or returned to the WebView.
    pub fn unlock_portable_storage(
        &self,
        password: &str,
        recovery: Option<crate::secret_store::PortableRecoverySetup>,
    ) -> Result<crate::secret_store::PortableStorageStatus, String> {
        let _persist_guard = self.persist_lock.lock();
        if !self.storage_recovery.is_ready() {
            return Err(
                "portable storage recovery is required before it can be unlocked".to_string(),
            );
        }

        crate::secret_store::unlock_portable_storage(password, recovery)?;
        self.hydrate_unlocked_portable_storage()
    }

    /// Restore the current portable data key from all recovery answers, update
    /// its password wrapper, and hydrate the same durable projection as a
    /// normal password unlock. The database ciphertext is never replaced.
    pub fn recover_portable_storage(
        &self,
        answers: &[String],
        new_password: &str,
    ) -> Result<crate::secret_store::PortableStorageStatus, String> {
        let _persist_guard = self.persist_lock.lock();
        if !self.storage_recovery.is_ready() {
            return Err(
                "portable storage recovery is required before it can be unlocked".to_string(),
            );
        }
        crate::secret_store::recover_portable_storage(answers, new_password)?;
        self.hydrate_unlocked_portable_storage()
    }

    pub fn auto_unlock_portable_storage(
        &self,
    ) -> Result<crate::secret_store::PortableStorageStatus, String> {
        let _persist_guard = self.persist_lock.lock();
        if !self.storage_recovery.is_ready() {
            return Err(
                "portable storage recovery is required before it can be unlocked".to_string(),
            );
        }
        crate::secret_store::auto_unlock_portable_storage()?;
        self.hydrate_unlocked_portable_storage()
    }

    pub fn unlock_installed_app_storage(
        &self,
        password: &str,
    ) -> Result<crate::secret_store::AppSecurityStatus, String> {
        let _persist_guard = self.persist_lock.lock();
        crate::secret_store::unlock_app_password(password)?;
        if let Err(error) = self.hydrate_unlocked_installed_storage() {
            crate::secret_store::clear_app_password_unlock();
            return Err(error);
        }
        crate::secret_store::app_security_status()
    }

    pub fn set_installed_app_password(
        &self,
        current_password: Option<&str>,
        new_password: &str,
    ) -> Result<crate::secret_store::AppSecurityStatus, String> {
        let _persist_guard = self.persist_lock.lock();
        crate::secret_store::set_installed_app_password(current_password, new_password)?;
        if let Err(error) = self.hydrate_unlocked_installed_storage() {
            crate::secret_store::clear_app_password_unlock();
            return Err(error);
        }
        crate::secret_store::app_security_status()
    }

    pub fn disable_installed_app_password(
        &self,
        current_password: Option<&str>,
    ) -> Result<crate::secret_store::AppSecurityStatus, String> {
        let _persist_guard = self.persist_lock.lock();
        crate::secret_store::disable_installed_app_password(current_password)?;
        if let Err(error) = self.hydrate_unlocked_installed_storage() {
            crate::secret_store::clear_app_password_unlock();
            return Err(error);
        }
        crate::secret_store::app_security_status()
    }

    pub fn set_portable_password_requirement(
        &self,
        required: bool,
        current_password: Option<&str>,
        new_password: Option<&str>,
        recovery_setup: Option<crate::secret_store::PortableRecoverySetup>,
    ) -> Result<crate::secret_store::AppSecurityStatus, String> {
        let _persist_guard = self.persist_lock.lock();
        self.require_durable_mutation()?;
        crate::secret_store::set_portable_password_requirement(
            required,
            current_password,
            new_password,
            recovery_setup,
        )?;
        crate::secret_store::app_security_status()
    }

    /// Hydrate installed-mode user state after the startup password has been
    /// verified. The caller holds `persist_lock`, matching the portable
    /// unlock path so a renderer cannot observe a partially restored memory
    /// projection.
    fn hydrate_unlocked_installed_storage(&self) -> Result<(), String> {
        if !self.settings_locked.load(Ordering::Acquire) {
            return Ok(());
        }
        if !self.storage_recovery.is_ready() {
            return Err(
                "NovaVei storage recovery is required before state can be loaded".to_string(),
            );
        }

        let previous_sessions = self.sessions.lock().clone();
        let previous_settings = self.settings.lock().clone();
        let previous_approved_workdirs = self.approved_workdirs.lock().clone();
        let previous_settings_locked = self.settings_locked.load(Ordering::Acquire);

        let hydrated = (|| -> Result<(), String> {
            self.history
                .mark_interrupted_turns(now_ms())
                .map_err(|_| "mark interrupted turns failed".to_string())?;
            self.subagent_tasks
                .mark_interrupted_tasks(now_ms())
                .map_err(|_| "mark interrupted subagent tasks failed".to_string())?;

            let legacy_file = diagnostics::application_data_dir().join("state.json");
            let legacy_persisted = match fs::read_to_string(&legacy_file) {
                Ok(text) => Some(
                    serde_json::from_str::<PersistedState>(&text)
                        .map_err(|_| "parse legacy state failed".to_string())?,
                ),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(_) => return Err("read legacy state failed".to_string()),
            };
            let legacy_loaded = legacy_persisted.is_some();
            let persisted = legacy_persisted.unwrap_or_default();

            let mut sessions = load_session_projection(&self.history)?;
            merge_legacy_sessions(&mut sessions, persisted.sessions);

            let mut raw_settings = persisted.settings;
            let stored_settings = self
                .history
                .load_settings()
                .map_err(|_| "read settings failed".to_string())?;
            raw_settings.extend(stored_settings);
            let settings = unprotect_settings(&raw_settings)
                .map_err(|_| "unlock installed settings failed".to_string())?;

            if sessions.is_empty() {
                let record =
                    new_session_record("本地工具执行与权限".to_string(), current_workdir());
                self.history
                    .upsert_session_metadata(&stored_session_from_record(&record))
                    .map_err(|_| "initialize conversation store failed".to_string())?;
                sessions.insert(record.summary.id.clone(), record);
            }

            let mut approved_workdirs = approved_project_workdirs(&settings);
            if let Ok(current) = canonical_workdir(&current_workdir()) {
                approved_workdirs.insert(current);
            }
            *self.sessions.lock() = sessions;
            *self.settings.lock() = settings;
            *self.approved_workdirs.lock() = approved_workdirs;
            self.settings_locked.store(false, Ordering::Release);

            if legacy_loaded {
                self.persist_locked()?;
                if self.history.secure_compact_once().is_ok() {
                    retire_legacy_state(&legacy_file);
                } else {
                    diagnostics::record_event(
                        "storage",
                        "secure_compaction_failed",
                        "failure",
                        None,
                    );
                }
            }
            match self.history.encrypt_legacy_transcripts_once() {
                Ok(true) => {
                    if self.history.secure_compact_after_protection().is_err() {
                        diagnostics::record_event(
                            "storage",
                            "transcript_encryption_compaction_failed",
                            "failure",
                            None,
                        );
                    }
                }
                Ok(false) => {}
                Err(_) => {
                    diagnostics::record_event(
                        "storage",
                        "transcript_encryption_migration_failed",
                        "failure",
                        None,
                    );
                }
            }
            Ok(())
        })();

        if let Err(error) = hydrated {
            *self.sessions.lock() = previous_sessions;
            *self.settings.lock() = previous_settings;
            *self.approved_workdirs.lock() = previous_approved_workdirs;
            self.settings_locked
                .store(previous_settings_locked, Ordering::Release);
            return Err(error);
        }
        Ok(())
    }

    /// Hydration only publishes these projections at its end, but retain the
    /// pre-unlock state as a final fail-closed guard if publishing the key
    /// readiness flag itself fails. The caller holds `persist_lock` and has
    /// already installed a pending portable key.
    fn hydrate_unlocked_portable_storage(
        &self,
    ) -> Result<crate::secret_store::PortableStorageStatus, String> {
        // Hydration only publishes these projections at its end, but retain the
        // pre-unlock state as a final fail-closed guard if publishing the key
        // readiness flag itself fails.
        let previous_sessions = self.sessions.lock().clone();
        let previous_settings = self.settings.lock().clone();
        let previous_approved_workdirs = self.approved_workdirs.lock().clone();

        let hydrated = (|| -> Result<(), String> {
            let mut settings = unprotect_settings(
                &self
                    .history
                    .load_settings()
                    .map_err(|_| "read portable settings failed".to_string())?,
            )
            .map_err(|_| "unlock portable settings failed".to_string())?;
            reconcile_portable_projects_at_startup(&mut settings);
            let mut sessions = load_session_projection(&self.history)?;

            // A new portable directory has no durable conversation. Persist its
            // starter record before publishing it to memory so a failed USB write
            // cannot leave the renderer holding a ghost session.
            if sessions.is_empty() {
                let record =
                    new_session_record("本地工具执行与权限".to_string(), current_workdir());
                self.history
                    .upsert_session_metadata(&stored_session_from_record(&record))
                    .map_err(|_| "initialize portable conversation store failed".to_string())?;
                sessions.insert(record.summary.id.clone(), record);
            }

            // Migrate legacy portable rows only after the password-derived key is
            // present. The migration is idempotent and never falls back to DPAPI.
            let protected_settings = protect_settings(&settings)?;
            self.history
                .upsert_settings(&protected_settings)
                .map_err(|_| "secure portable settings failed".to_string())?;
            self.history
                .encrypt_legacy_transcripts_once()
                .map_err(|_| "secure portable conversation store failed".to_string())?;
            // A copied database can carry an old compaction marker while its
            // current freelist/WAL still contains plaintext. Always compact the
            // portable copy after this unlock-time migration.
            self.history
                .secure_compact_after_protection()
                .map_err(|_| "secure portable database compaction failed".to_string())?;

            let mut approved_workdirs = approved_project_workdirs(&settings);
            if let Ok(current) = canonical_workdir(&current_workdir()) {
                approved_workdirs.insert(current);
            }
            *self.sessions.lock() = sessions;
            *self.settings.lock() = settings;
            *self.approved_workdirs.lock() = approved_workdirs;
            self.settings_locked.store(false, Ordering::Release);
            Ok(())
        })();
        if let Err(error) = hydrated {
            crate::secret_store::clear_portable_storage_key();
            return Err(error);
        }
        if let Err(error) = crate::secret_store::complete_portable_storage_unlock() {
            crate::secret_store::clear_portable_storage_key();
            *self.sessions.lock() = previous_sessions;
            *self.settings.lock() = previous_settings;
            *self.approved_workdirs.lock() = previous_approved_workdirs;
            self.settings_locked.store(true, Ordering::Release);
            return Err(error);
        }
        Ok(crate::secret_store::portable_storage_status())
    }
}

fn load_session_projection(
    history: &HistoryStore,
) -> Result<HashMap<String, SessionRecord>, String> {
    let stored_sessions = history
        .load_sessions()
        .map_err(|_| "read sessions failed".to_string())?;
    let run_summaries = history
        .load_session_run_summaries()
        .map_err(|_| "read session run summaries failed".to_string())?;
    let goals = history
        .load_session_goals()
        .map_err(|_| "read session goals failed".to_string())?;
    let message_counts = history
        .load_message_counts()
        .map_err(|_| "read session message counts failed".to_string())?;
    let mut sessions = HashMap::new();
    for record in stored_sessions {
        let message_count = message_counts.get(&record.id).copied().unwrap_or(0);
        let messages_empty = message_count == 0;
        let last_run = run_summaries.get(&record.id);
        let goal = goals.get(&record.id).and_then(session_goal_from_stored);
        sessions.insert(
            record.id.clone(),
            SessionRecord {
                summary: SessionSummary {
                    id: record.id,
                    title: record.title,
                    cwd: record.cwd,
                    updated_at: record.updated_at,
                    message_count,
                    is_pinned: false,
                    is_archived: false,
                    last_run_status: last_run.map(|summary| summary.status.clone()),
                    last_run_finished_at: last_run.map(|summary| summary.finished_at),
                },
                messages: Vec::new(),
                message_count,
                messages_loaded: messages_empty,
                messages_complete: messages_empty,
                provider_id: record.provider_id,
                model: record.model,
                selected_model_json: record.selected_model_json,
                pinned_at: record.pinned_at,
                archived_at: record.archived_at,
                share_enabled: record.share_enabled,
                share_token: record.share_token,
                share_created_at: record.share_created_at,
                share_updated_at: record.share_updated_at,
                redact_tool_content: record.redact_tool_content,
                goal,
            },
        );
    }
    Ok(sessions)
}

fn stored_session_from_record(record: &SessionRecord) -> StoredSession {
    StoredSession {
        id: record.summary.id.clone(),
        title: record.summary.title.clone(),
        cwd: record.summary.cwd.clone(),
        updated_at: record.summary.updated_at,
        provider_id: record.provider_id.clone(),
        model: record.model.clone(),
        selected_model_json: record.selected_model_json.clone(),
        pinned_at: record.pinned_at,
        archived_at: record.archived_at,
        share_enabled: record.share_enabled,
        share_token: record.share_token.clone(),
        share_created_at: record.share_created_at,
        share_updated_at: record.share_updated_at,
        redact_tool_content: record.redact_tool_content,
    }
}

fn stored_message_from_record(session_id: &str, message: &MessageRecord) -> StoredMessage {
    StoredMessage {
        id: message.id.clone(),
        session_id: session_id.to_string(),
        role: message.role.clone(),
        content: message.content.clone(),
        created_at: message.created_at,
        turn_id: message.turn_id.clone(),
    }
}

fn message_record_from_stored(message: StoredMessage) -> MessageRecord {
    MessageRecord {
        id: message.id,
        role: message.role,
        content: message.content,
        created_at: message.created_at,
        turn_id: message.turn_id,
        model: None,
        reasoning: None,
        finished_at: None,
        thinking: None,
    }
}

fn clamp_ui_message_page_limit(limit: Option<u32>) -> usize {
    limit
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_UI_MESSAGE_PAGE_SIZE)
        .clamp(1, MAX_UI_MESSAGE_PAGE_SIZE)
}

fn merge_legacy_sessions(
    sessions: &mut HashMap<String, SessionRecord>,
    legacy_sessions: Vec<SessionRecord>,
) {
    for mut record in legacy_sessions {
        // Legacy JSON snapshots carried the full transcript. Treat that as a
        // complete cache when present so a one-time migration does not drop rows.
        if !record.messages.is_empty() {
            record.message_count = record.messages.len() as i64;
            record.messages_loaded = true;
            record.messages_complete = true;
        } else if record.message_count == 0 {
            record.messages_loaded = true;
            record.messages_complete = true;
        }
        sessions.entry(record.summary.id.clone()).or_insert(record);
    }
}

fn retire_legacy_state(path: &Path) {
    if fs::remove_file(path).is_ok() || !path.exists() {
        return;
    }
    let cleared = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .and_then(|mut file| {
            file.write_all(b"{\"version\":2,\"migrated\":true}\n")?;
            file.sync_all()
        });
    if let Err(error) = cleared {
        eprintln!("failed to clear legacy NovaVei state: {error}");
    }
}

fn current_workdir() -> String {
    if let Some(portable_dir) = crate::storage::portable_application_dir() {
        return path_for_display(&portable_dir);
    }
    std::env::current_dir()
        .map(|path| path_for_display(&path))
        .unwrap_or_else(|_| ".".to_string())
}

fn portable_machine_identity() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

/// Removable drives regularly change letters between hosts and even between
/// plug-ins on one host. For scope comparison only, treat `E:\NovaVei` and
/// `F:\NovaVei` as the same portable folder; the machine identity check still
/// distinguishes computers. UNC and relative keys keep exact comparison.
fn drive_agnostic_scope_key(key: &str) -> &str {
    let bytes = key.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        &key[2..]
    } else {
        key
    }
}

fn workspace_drive_letter(path: &str) -> Option<char> {
    let bytes = path.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        Some(bytes[0].to_ascii_uppercase() as char)
    } else {
        None
    }
}

/// Rewrite a project entry's path from the portable drive's previous letter
/// to its current one. The remap is deliberately conservative: an entry moves
/// only when its saved path is no longer reachable while the same path on the
/// current portable drive exists, so fixed-disk projects that legitimately
/// share the old letter are never rewritten.
fn remap_project_entry_drive(entry: &mut Value, previous_drive: char, current_drive: char) {
    let Some(object) = entry.as_object_mut() else {
        return;
    };
    let Some(path) = object.get("path").and_then(Value::as_str) else {
        return;
    };
    if workspace_drive_letter(path) != Some(previous_drive) {
        return;
    }
    if canonical_workdir(path).is_ok() {
        return;
    }
    let remapped: String = current_drive.to_string() + &path[1..];
    if canonical_workdir(&remapped).is_err() {
        return;
    }
    object.insert("path".to_string(), Value::String(remapped));
}

fn portable_project_scope_matches(
    value: Option<&Value>,
    root_key: &str,
    machine: Option<&str>,
) -> bool {
    let Some(value) = value else {
        return true;
    };
    let Some(scope) = value.as_object() else {
        return false;
    };
    if scope.get("version").and_then(Value::as_u64) != Some(PORTABLE_PROJECT_SCOPE_VERSION) {
        return false;
    }
    let Some(saved_root_key) = scope
        .get("root")
        .and_then(Value::as_str)
        .and_then(workspace_path_key)
    else {
        return false;
    };
    if drive_agnostic_scope_key(&saved_root_key) != drive_agnostic_scope_key(root_key) {
        return false;
    }
    match (
        scope.get("machine").and_then(Value::as_str).map(str::trim),
        machine,
    ) {
        (Some(saved), Some(current)) if !saved.is_empty() => saved.eq_ignore_ascii_case(current),
        _ => true,
    }
}

/// A copied portable data directory may contain project paths belonging to a
/// different computer. Keep their transcript rows durable, but do not restore
/// them as projects when the executable folder or machine identity changes.
/// The executable's directory is always the first current project so the app
/// opens somewhere usable even when all carried paths are stale.
fn reconcile_portable_projects_at_startup(settings: &mut HashMap<String, Value>) {
    if !crate::storage::is_portable() {
        return;
    }
    let Ok(root) = canonical_workdir(&current_workdir()) else {
        return;
    };
    let root = path_for_display(&root);
    let Some(root_key) = workspace_path_key(&root) else {
        return;
    };
    let machine = portable_machine_identity();
    let has_portable_scope = settings.contains_key(PORTABLE_PROJECT_SCOPE);
    let scope_matches = portable_project_scope_matches(
        settings.get(PORTABLE_PROJECT_SCOPE),
        &root_key,
        machine.as_deref(),
    );
    let saved_scope_root = settings
        .get(PORTABLE_PROJECT_SCOPE)
        .and_then(Value::as_object)
        .and_then(|scope| scope.get("root"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut entries = if scope_matches {
        let mut entries = settings
            .get("projects")
            .and_then(Value::as_object)
            .and_then(|projects| projects.get("entries"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // The portable drive may have received a different letter on this
        // host. Project paths that lived on the old portable drive are moved
        // to the new letter; paths on other drives are untouched.
        if let Some(previous_drive) = saved_scope_root.as_deref().and_then(workspace_drive_letter) {
            if let Some(current_drive) = workspace_drive_letter(&root) {
                if previous_drive != current_drive {
                    for entry in &mut entries {
                        remap_project_entry_drive(entry, previous_drive, current_drive);
                    }
                }
            }
        }
        entries
    } else {
        // A different machine (or an unrecognized scope) starts from a clean
        // list, but the carried entries are preserved under a backup key so a
        // false-negative scope check never destroys the user's project list.
        let previous_entries = settings
            .get("projects")
            .and_then(Value::as_object)
            .and_then(|projects| projects.get("entries"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !previous_entries.is_empty() {
            settings.insert(
                // This key remains native-only. Preserve rather than discard
                // the prior list so a false-negative scope check (machine
                // rename or unusual drive setup) never destroys user data.
                "__portableProjectScopeBackup".to_string(),
                json!({
                    "version": PORTABLE_PROJECT_SCOPE_VERSION,
                    "savedAt": now_ms(),
                    "previousScope": settings.get(PORTABLE_PROJECT_SCOPE).cloned(),
                    "entries": previous_entries,
                }),
            );
        }
        Vec::new()
    };
    if !has_portable_scope {
        // First launch after this migration has no machine marker to compare.
        // Retain only paths that exist on this computer; future launches use
        // the explicit portable scope above and do not depend on this probe.
        entries.retain(|entry| {
            entry
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| canonical_workdir(path).is_ok())
        });
    }
    let contains_root = entries.iter().any(|entry| {
        entry
            .get("path")
            .and_then(Value::as_str)
            .and_then(workspace_path_key)
            .as_deref()
            == Some(root_key.as_str())
    });
    if !contains_root {
        entries.insert(
            0,
            json!({
                "id": new_stable_project_id(),
                "name": default_project_name(Path::new(&root)),
                "path": root.clone(),
                "lastSessionId": Value::Null,
                "pinned": false,
            }),
        );
    }
    settings.insert(
        "projects".to_string(),
        json!({
            "version": PROJECT_SETTINGS_VERSION,
            "initialized": true,
            "entries": entries,
        }),
    );
    settings.insert("system".to_string(), {
        let mut system = settings
            .get("system")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        system.insert("workdir".to_string(), Value::String(root.clone()));
        Value::Object(system)
    });
    settings.insert(
        PORTABLE_PROJECT_SCOPE.to_string(),
        json!({
            "version": PORTABLE_PROJECT_SCOPE_VERSION,
            "root": root,
            "machine": machine,
        }),
    );
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn validate_session_goal_session_id(value: &str) -> Result<&str, String> {
    let id = value.trim();
    if id.is_empty()
        || id.len() > MAX_SESSION_GOAL_SESSION_ID_BYTES
        || id.len() != value.len()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("session goal session id is invalid".to_string());
    }
    Ok(id)
}

fn normalize_session_goal_text(value: String) -> Result<String, String> {
    let text = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let length = text.chars().count();
    if length == 0 || length > MAX_SESSION_GOAL_TEXT_CHARS {
        return Err(format!(
            "session goal text must contain 1 to {MAX_SESSION_GOAL_TEXT_CHARS} characters"
        ));
    }
    Ok(text)
}

fn validate_session_goal_state(status: SessionGoalStatus, progress: u8) -> Result<(), String> {
    match status {
        SessionGoalStatus::Active if progress < 100 => Ok(()),
        SessionGoalStatus::Completed if progress == 100 => Ok(()),
        _ => Err("session goal status and progress are inconsistent".to_string()),
    }
}

fn session_goal_from_stored(value: &StoredSessionGoal) -> Option<SessionGoal> {
    let status = match value.status.as_str() {
        "active" => SessionGoalStatus::Active,
        "completed" => SessionGoalStatus::Completed,
        _ => return None,
    };
    validate_session_goal_state(status, value.progress).ok()?;
    let text = normalize_session_goal_text(value.text.clone()).ok()?;
    if value.updated_at <= 0 {
        return None;
    }
    Some(SessionGoal {
        text,
        status,
        progress: value.progress,
        updated_at: value.updated_at,
    })
}

fn new_session_record(title: String, cwd: String) -> SessionRecord {
    let id = Uuid::new_v4().to_string();
    SessionRecord {
        summary: SessionSummary {
            id,
            title,
            cwd,
            updated_at: now_ms(),
            message_count: 0,
            is_pinned: false,
            is_archived: false,
            last_run_status: None,
            last_run_finished_at: None,
        },
        messages: Vec::new(),
        message_count: 0,
        messages_loaded: true,
        messages_complete: true,
        provider_id: default_provider_id(),
        model: String::new(),
        selected_model_json: None,
        pinned_at: None,
        archived_at: None,
        share_enabled: false,
        share_token: None,
        share_created_at: None,
        share_updated_at: None,
        redact_tool_content: true,
        goal: None,
    }
}

fn title_from_prompt(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars = collapsed.chars().collect::<Vec<_>>();
    if chars.len() > 36 {
        format!("{}…", chars[..36].iter().collect::<String>())
    } else if collapsed.is_empty() {
        "新建对话".to_string()
    } else {
        collapsed
    }
}

fn ensure_session(
    state: &AppState,
    session_id: Option<String>,
    title: Option<String>,
    cwd: Option<String>,
) -> String {
    let mut sessions = state.sessions.lock();
    if let Some(id) = session_id.filter(|id| sessions.contains_key(id)) {
        return id;
    }
    let record = new_session_record(
        title.unwrap_or_else(|| "新建对话".to_string()),
        cwd.filter(|value| !value.trim().is_empty())
            .unwrap_or_else(current_workdir),
    );
    let id = record.summary.id.clone();
    sessions.insert(id.clone(), record);
    id
}

fn append_message(
    state: &AppState,
    session_id: &str,
    role: &str,
    content: String,
    turn_id: Option<String>,
) -> Result<(), String> {
    let mut sessions = state.sessions.lock();
    let session = sessions
        .get_mut(session_id)
        .ok_or_else(|| "session not found".to_string())?;
    if role == "user" && session.summary.title == "新建对话" {
        session.summary.title = title_from_prompt(&content);
    }
    session.summary.updated_at = now_ms();
    let message = MessageRecord {
        id: Uuid::new_v4().to_string(),
        role: role.to_string(),
        content,
        created_at: now_ms(),
        turn_id,
        model: None,
        reasoning: None,
        finished_at: None,
        thinking: None,
    };
    // Always stage the new turn for incomplete-safe insert. When the cache was
    // never loaded, keep messages_complete=false so persist never full-replaces.
    if !session.messages_loaded {
        session.messages.push(message);
        session.messages_loaded = true;
        session.messages_complete = false;
        session.message_count = session.message_count.saturating_add(1);
    } else {
        session.messages.push(message);
        if session.messages_complete {
            session.message_count = session.messages.len() as i64;
        } else {
            session.message_count = session.message_count.saturating_add(1);
        }
    }
    Ok(())
}

fn persist_terminal_projection_locked(
    state: &AppState,
    run: &ActiveRun,
    payload: &Value,
) -> Result<(), String> {
    let event_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
    if !matches!(event_type, "done" | "error" | "cancelled") {
        return Ok(());
    }
    let assistant_text = if event_type == "done" {
        Some(
            payload
                .get("text")
                .or_else(|| payload.get("assistantText"))
                .and_then(Value::as_str)
                .ok_or_else(|| "terminal completion text is required".to_string())?,
        )
    } else {
        None
    };
    let previous = state
        .sessions
        .lock()
        .get(&run.session_id)
        .cloned()
        .ok_or_else(|| "terminal projection session not found".to_string())?;
    let finished_at = now_ms();
    record_terminal_session_result(state, &run.session_id, event_type, finished_at);
    {
        let mut sessions = state.sessions.lock();
        let session = sessions
            .get_mut(&run.session_id)
            .ok_or_else(|| "terminal projection session not found".to_string())?;
        if let Some(text) = assistant_text {
            let message_id = format!("assistant:{}", run.request_id);
            let created_at = payload
                .get("timestamp")
                .and_then(Value::as_i64)
                .unwrap_or(finished_at);
            if session.messages_loaded {
                if let Some(message) = session.messages.iter_mut().find(|message| {
                    message.id == message_id
                        || (message.role == "assistant"
                            && message.turn_id.as_deref() == Some(run.turn_id.as_str()))
                }) {
                    message.role = "assistant".to_string();
                    message.content = text.to_string();
                    message.turn_id = Some(run.turn_id.clone());
                } else {
                    session.messages.push(MessageRecord {
                        id: message_id,
                        role: "assistant".to_string(),
                        content: text.to_string(),
                        created_at,
                        turn_id: Some(run.turn_id.clone()),
                        model: None,
                        reasoning: None,
                        finished_at: None,
                        thinking: None,
                    });
                    if session.messages_complete {
                        session.message_count = session.messages.len() as i64;
                    } else {
                        session.message_count = session.message_count.saturating_add(1);
                    }
                }
            } else {
                // Unloaded cache: stage one row for incomplete-safe insert_message.
                session.messages.push(MessageRecord {
                    id: message_id,
                    role: "assistant".to_string(),
                    content: text.to_string(),
                    created_at,
                    turn_id: Some(run.turn_id.clone()),
                    model: None,
                    reasoning: None,
                    finished_at: None,
                    thinking: None,
                });
                session.messages_loaded = true;
                session.messages_complete = false;
                session.message_count = session.message_count.saturating_add(1);
            }
            session.summary.updated_at = session.summary.updated_at.max(created_at);
        } else {
            // Error and cancelled outcomes are durable terminal states too,
            // but neither fabricates an assistant message.
            session.summary.updated_at = session.summary.updated_at.max(finished_at);
        }
    }
    if let Err(error) = state.persist_session_locked(&run.session_id) {
        state
            .sessions
            .lock()
            .insert(run.session_id.clone(), previous);
        return Err(format!("persist terminal outcome: {error}"));
    }
    Ok(())
}

fn record_terminal_session_result(
    state: &AppState,
    session_id: &str,
    event_type: &str,
    finished_at: i64,
) {
    let status = match event_type {
        "done" => "completed",
        "error" => "error",
        "cancelled" => "cancelled",
        _ => return,
    };
    if let Some(session) = state.sessions.lock().get_mut(session_id) {
        session.summary.last_run_status = Some(status.to_string());
        session.summary.last_run_finished_at = Some(finished_at);
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub product: String,
    pub skin: String,
    pub version: String,
    pub backend: String,
    pub pi_runtime: String,
}

/// The process keeps its current storage root fixed, while this DTO makes a
/// user-selected mode for the *next* launch explicit to the renderer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageModeStatus {
    pub current_mode: crate::storage::StorageMode,
    pub next_launch_mode: crate::storage::StorageMode,
    pub restart_required: bool,
}

fn current_storage_mode_status() -> Result<StorageModeStatus, String> {
    let current_mode = crate::storage::layout().mode;
    let next_launch_mode = crate::storage::next_launch_mode()?;
    Ok(StorageModeStatus {
        current_mode,
        next_launch_mode,
        restart_required: current_mode != next_launch_mode,
    })
}

#[tauri::command]
pub fn system_info() -> SystemInfo {
    SystemInfo {
        product: "NovaVei".to_string(),
        skin: "Luminous Quiet".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        backend: "tauri-rust".to_string(),
        pi_runtime: "embedded-webview".to_string(),
    }
}

/// Read the active storage mode and the one selected for the next launch.
/// No storage paths or encryption metadata are exposed to the WebView.
#[tauri::command]
pub fn storage_mode_status() -> Result<StorageModeStatus, String> {
    current_storage_mode_status()
}

/// Schedule the storage mode for the next application launch.  The marker is
/// the only changed file; existing installed and portable data roots remain
/// untouched and deliberately separate.
#[tauri::command]
pub fn storage_mode_set(mode: crate::storage::StorageMode) -> Result<StorageModeStatus, String> {
    crate::storage::set_next_launch_mode(mode)?;
    current_storage_mode_status()
}

/// A picker-only export path keeps the renderer from choosing a filesystem
/// destination or reading back the report contents.
#[tauri::command]
pub async fn diagnostics_export(
    state: State<'_, Arc<AppState>>,
) -> Result<Option<diagnostics::DiagnosticsExportResponse>, String> {
    let history = state.history.clone();
    let storage_status = state.storage_recovery_status();
    tauri::async_runtime::spawn_blocking(move || {
        diagnostics::export_diagnostics(
            &history,
            &storage_status.state,
            storage_status.errors.len(),
        )
    })
    .await
    .map_err(|_| "native diagnostics export picker did not complete".to_string())?
}

/// Startup persistence failures are recoverable, but the renderer must know
/// about them before it exposes controls that promise durable changes.
#[tauri::command]
pub fn storage_recovery_status(state: State<'_, Arc<AppState>>) -> StorageRecoveryStatus {
    state.storage_recovery_status()
}

/// Reveal only whether this executable is a portable build and whether its
/// in-memory key has been unlocked. Paths, salts, verifiers, and passwords
/// remain native-only.
#[tauri::command]
pub fn portable_storage_status() -> crate::secret_store::PortableStorageStatus {
    crate::secret_store::portable_storage_status()
}

#[tauri::command]
pub fn app_security_status() -> Result<crate::secret_store::AppSecurityStatus, String> {
    crate::secret_store::app_security_status()
}

#[tauri::command(rename_all = "camelCase")]
pub async fn app_security_unlock(
    state: State<'_, Arc<AppState>>,
    password: Option<String>,
) -> Result<crate::secret_store::AppSecurityStatus, String> {
    let state = Arc::clone(&state);
    tokio::task::spawn_blocking(move || {
        if crate::storage::is_portable() {
            let portable = crate::secret_store::portable_storage_status();
            if portable.password_required {
                let password = password
                    .as_deref()
                    .ok_or_else(|| "portable storage password is required".to_string())?;
                state.unlock_portable_storage(password, None)?;
            } else {
                state.auto_unlock_portable_storage()?;
            }
            crate::secret_store::app_security_status()
        } else {
            let password = password
                .as_deref()
                .ok_or_else(|| "application password is required".to_string())?;
            state.unlock_installed_app_storage(password)
        }
    })
    .await
    .map_err(|_| "application security unlock did not complete".to_string())?
}

#[tauri::command(rename_all = "camelCase")]
pub async fn app_security_set_password(
    state: State<'_, Arc<AppState>>,
    current_password: Option<String>,
    new_password: String,
    recovery_setup: Option<crate::secret_store::PortableRecoverySetup>,
) -> Result<crate::secret_store::AppSecurityStatus, String> {
    let state = Arc::clone(&state);
    tokio::task::spawn_blocking(move || {
        if crate::storage::is_portable() {
            state.set_portable_password_requirement(
                true,
                current_password.as_deref(),
                Some(new_password.as_str()),
                recovery_setup,
            )
        } else {
            state.set_installed_app_password(current_password.as_deref(), &new_password)
        }
    })
    .await
    .map_err(|_| "application security update did not complete".to_string())?
}

#[tauri::command(rename_all = "camelCase")]
pub async fn app_security_disable_password(
    state: State<'_, Arc<AppState>>,
    current_password: Option<String>,
) -> Result<crate::secret_store::AppSecurityStatus, String> {
    let state = Arc::clone(&state);
    tokio::task::spawn_blocking(move || {
        if crate::storage::is_portable() {
            state.set_portable_password_requirement(false, current_password.as_deref(), None, None)
        } else {
            state.disable_installed_app_password(current_password.as_deref())
        }
    })
    .await
    .map_err(|_| "application security update did not complete".to_string())?
}

/// Unlock or initialize the password-protected portable data root, then
/// hydrate the durable state that was intentionally withheld during startup.
/// Argon2 derivation, legacy-row re-encryption, and the follow-up VACUUM can
/// take seconds on removable media, so the work runs on a blocking thread
/// instead of the main event loop.
#[tauri::command(rename_all = "camelCase")]
pub async fn portable_storage_unlock(
    state: State<'_, Arc<AppState>>,
    password: String,
    recovery: Option<crate::secret_store::PortableRecoverySetup>,
) -> Result<crate::secret_store::PortableStorageStatus, String> {
    let state = Arc::clone(&state);
    tokio::task::spawn_blocking(move || state.unlock_portable_storage(&password, recovery))
        .await
        .map_err(|_| "portable storage unlock did not complete".to_string())?
}

/// Recover a portable data key from all three custom recovery answers, then
/// set a new password wrapper before hydrating the encrypted projection.
/// Runs on a blocking thread for the same reason as the password unlock.
#[tauri::command(rename_all = "camelCase")]
pub async fn portable_storage_recover(
    state: State<'_, Arc<AppState>>,
    answers: Vec<String>,
    new_password: String,
) -> Result<crate::secret_store::PortableStorageStatus, String> {
    let state = Arc::clone(&state);
    tokio::task::spawn_blocking(move || state.recover_portable_storage(&answers, &new_password))
        .await
        .map_err(|_| "portable storage recovery did not complete".to_string())?
}

/// Return the closed health DTO before a renderer reads sessions or settings.
/// Proxy transport metadata is intentionally kept inside `ProxyRuntime`; the
/// WebView sees only its stable availability enum through this aggregate.
#[tauri::command]
pub fn app_health(
    state: State<'_, Arc<AppState>>,
    proxy: State<'_, Arc<crate::proxy::ProxyRuntime>>,
) -> AppHealth {
    state.app_health(matches!(
        proxy.status().status,
        crate::proxy::ProxyAvailability::Ready
    ))
}

/// Open or navigate the user-visible side browser for the calling NovaVei
/// window. The external page still runs in a different child WebView without
/// this application's command capability.
#[tauri::command(rename_all = "camelCase")]
pub async fn browser_open(
    app: AppHandle,
    webview_window: WebviewWindow,
    url: String,
) -> Result<crate::browser::BrowserState, String> {
    crate::browser::open(app, webview_window.label().to_string(), url).await
}

/// Keep the native child WebView aligned with the renderer-owned browser
/// viewport. A missing browser is intentionally a no-op so opening the dock
/// does not trigger a network request.
#[tauri::command(rename_all = "camelCase")]
pub fn browser_layout(
    app: AppHandle,
    webview_window: WebviewWindow,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    visible: bool,
) -> Result<crate::browser::BrowserState, String> {
    crate::browser::layout(
        app,
        webview_window.label().to_string(),
        crate::browser::BrowserViewport {
            x,
            y,
            width,
            height,
            visible,
        },
    )
}

#[tauri::command]
pub fn browser_back(
    app: AppHandle,
    webview_window: WebviewWindow,
) -> Result<crate::browser::BrowserState, String> {
    crate::browser::back(app, webview_window.label().to_string())
}

#[tauri::command]
pub fn browser_reload(
    app: AppHandle,
    webview_window: WebviewWindow,
) -> Result<crate::browser::BrowserState, String> {
    crate::browser::reload(app, webview_window.label().to_string())
}

#[tauri::command]
pub fn browser_status(
    app: AppHandle,
    webview_window: WebviewWindow,
) -> crate::browser::BrowserState {
    crate::browser::status(app, webview_window.label().to_string())
}

fn require_browser_agent_capability(
    state: &AppState,
    capability_token: &str,
    workdir: &str,
    tool_call_id: &str,
    expected_tool_name: &str,
) -> Result<PathBuf, String> {
    require_capability_for_named(
        state,
        Some(capability_token),
        workdir,
        ToolAction::Browser,
        Some(tool_call_id),
        Some(expected_tool_name),
    )
}

/// Agent browser tools are all capability-bound to the current active turn.
/// They are intentionally distinct from the user-operated browser commands:
/// browser content can be untrusted and remote page actions can have effects.
#[tauri::command(rename_all = "camelCase")]
pub async fn browser_agent_navigate(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    webview_window: WebviewWindow,
    workdir: String,
    capability_token: String,
    tool_call_id: String,
    url: String,
) -> Result<crate::browser::BrowserState, String> {
    let canonical_workdir = require_browser_agent_capability(
        &state,
        &capability_token,
        &workdir,
        &tool_call_id,
        "BrowserNavigate",
    )?;
    let result = crate::browser::open(app, webview_window.label().to_string(), url).await?;
    recheck_mutation_capability(
        &state,
        Some(&capability_token),
        &canonical_workdir,
        ToolAction::Browser,
    )?;
    Ok(result)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn browser_agent_snapshot(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    webview_window: WebviewWindow,
    workdir: String,
    capability_token: String,
    tool_call_id: String,
) -> Result<Value, String> {
    let canonical_workdir = require_browser_agent_capability(
        &state,
        &capability_token,
        &workdir,
        &tool_call_id,
        "BrowserSnapshot",
    )?;
    let result = crate::browser::snapshot(app, webview_window.label().to_string()).await?;
    recheck_mutation_capability(
        &state,
        Some(&capability_token),
        &canonical_workdir,
        ToolAction::Browser,
    )?;
    Ok(result)
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub async fn browser_agent_click(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    webview_window: WebviewWindow,
    workdir: String,
    capability_token: String,
    tool_call_id: String,
    reference: String,
    expected_url: String,
    expected_fingerprint: String,
) -> Result<Value, String> {
    let canonical_workdir = require_browser_agent_capability(
        &state,
        &capability_token,
        &workdir,
        &tool_call_id,
        "BrowserClick",
    )?;
    let result = crate::browser::click(
        app,
        webview_window.label().to_string(),
        reference,
        expected_url,
        expected_fingerprint,
    )
    .await?;
    recheck_mutation_capability(
        &state,
        Some(&capability_token),
        &canonical_workdir,
        ToolAction::Browser,
    )?;
    Ok(result)
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub async fn browser_agent_type(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    webview_window: WebviewWindow,
    workdir: String,
    capability_token: String,
    tool_call_id: String,
    reference: String,
    expected_url: String,
    expected_fingerprint: String,
    text: String,
) -> Result<Value, String> {
    let canonical_workdir = require_browser_agent_capability(
        &state,
        &capability_token,
        &workdir,
        &tool_call_id,
        "BrowserType",
    )?;
    let result = crate::browser::type_text(
        app,
        webview_window.label().to_string(),
        reference,
        expected_url,
        expected_fingerprint,
        text,
    )
    .await?;
    recheck_mutation_capability(
        &state,
        Some(&capability_token),
        &canonical_workdir,
        ToolAction::Browser,
    )?;
    Ok(result)
}

#[tauri::command]
pub fn sessions_list(state: State<'_, Arc<AppState>>) -> Vec<SessionSummary> {
    let mut list = state
        .sessions
        .lock()
        .values()
        .map(session_summary_value)
        .collect::<Vec<_>>();
    list.sort_by_key(|entry| std::cmp::Reverse(entry.updated_at));
    list
}

#[tauri::command(rename_all = "camelCase")]
pub fn sessions_create(
    state: State<'_, Arc<AppState>>,
    title: Option<String>,
    cwd: Option<String>,
) -> Result<SessionSummary, String> {
    state.require_persistence_ready()?;
    // Project removal and session creation share this gate. Requiring the
    // durable registration only after acquiring it prevents a renderer from
    // passing the check, losing the project in another request, and then
    // persisting a new session against read-only historical metadata.
    let _persist_guard = state.persist_lock.lock();
    let cwd = cwd
        .as_deref()
        .ok_or_else(|| "session creation requires a registered project workspace".to_string())
        .and_then(canonical_workdir)?;
    require_registered_project_workdir(&state, &cwd)?;
    let cwd = Some(path_for_display(&cwd));
    let id = ensure_session(&state, None, title, cwd);
    if let Err(error) = state.persist_session_locked(&id) {
        // `ensure_session` creates the in-memory record before the encrypted
        // snapshot can be committed. Roll it back on every persistence error
        // so a recovery-required or disk-full host cannot present a ghost
        // conversation as if it will survive a restart.
        state.sessions.lock().remove(&id);
        return Err(error);
    }
    state
        .sessions
        .lock()
        .get(&id)
        .map(session_summary_value)
        .ok_or_else(|| "session creation failed".to_string())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsGetResponse {
    pub messages: Vec<MessageRecord>,
    pub total_count: i64,
    pub has_more_before: bool,
}

#[tauri::command(rename_all = "camelCase")]
pub fn sessions_get(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    limit: Option<u32>,
    before_created_at: Option<i64>,
    before_id: Option<String>,
) -> Result<SessionsGetResponse, String> {
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Err("session not found".to_string());
    }
    {
        let sessions = state.sessions.lock();
        if !sessions.contains_key(&session_id) {
            return Err("session not found".to_string());
        }
    }
    let page_limit = clamp_ui_message_page_limit(limit);
    let before_id = before_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    if let (Some(before_created_at), Some(before_id)) = (before_created_at, before_id) {
        let page = state.history.load_messages_before(
            &session_id,
            before_created_at,
            &before_id,
            page_limit,
        )?;
        let messages = page
            .messages
            .into_iter()
            .map(message_record_from_stored)
            .collect::<Vec<_>>();
        let enriched = enrich_messages_with_turn_metadata(&state.history, &session_id, &messages)?;
        // Optionally prepend into the active session cache when a page is already loaded.
        {
            let mut sessions = state.sessions.lock();
            if let Some(record) = sessions.get_mut(&session_id) {
                record.message_count = page.total_count;
                if record.messages_loaded {
                    let existing_ids = record
                        .messages
                        .iter()
                        .map(|message| message.id.clone())
                        .collect::<HashSet<_>>();
                    let mut older = enriched
                        .iter()
                        .filter(|message| !existing_ids.contains(&message.id))
                        .cloned()
                        .collect::<Vec<_>>();
                    older.append(&mut record.messages);
                    record.messages = older;
                    if !page.has_more_before {
                        record.messages_complete = true;
                    }
                }
            }
        }
        return Ok(SessionsGetResponse {
            messages: enriched,
            total_count: page.total_count,
            has_more_before: page.has_more_before,
        });
    }

    let page = state
        .history
        .load_messages_recent(&session_id, page_limit)?;
    let messages = page
        .messages
        .into_iter()
        .map(message_record_from_stored)
        .collect::<Vec<_>>();
    let enriched = enrich_messages_with_turn_metadata(&state.history, &session_id, &messages)?;
    {
        let mut sessions = state.sessions.lock();
        if let Some(record) = sessions.get_mut(&session_id) {
            // Preserve any in-memory tail not yet reflected if a concurrent
            // append happened after the SQLite read. Prefer DB order as base.
            let mut cache = enriched.clone();
            if record.messages_loaded {
                for message in &record.messages {
                    if !cache.iter().any(|item| item.id == message.id) {
                        cache.push(message.clone());
                    }
                }
            }
            record.messages = cache;
            record.message_count = page.total_count.max(record.messages.len() as i64);
            record.messages_loaded = true;
            record.messages_complete =
                !page.has_more_before && record.messages.len() as i64 >= record.message_count;
        }
    }
    let has_more_before = page.has_more_before;
    let total_count = page.total_count;
    Ok(SessionsGetResponse {
        messages: enriched,
        total_count,
        has_more_before,
    })
}

/// Read the compact, local-only goal record for one durable session. This
/// deliberately returns no transcript, provider credentials, or agent state.
#[tauri::command(rename_all = "camelCase")]
pub fn session_goal_get(
    state: State<'_, Arc<AppState>>,
    session_id: String,
) -> Result<Option<SessionGoal>, String> {
    let id = validate_session_goal_session_id(&session_id)?;
    state
        .sessions
        .lock()
        .get(id)
        .map(|record| record.goal.clone())
        .ok_or_else(|| "session not found".to_string())
}

/// Update or clear a goal record with a fixed DTO. The renderer cannot pass
/// arbitrary metadata, provider configuration, transcript text, or a caller
/// supplied timestamp through this boundary.
#[tauri::command(rename_all = "camelCase")]
pub fn session_goal_set(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    text: Option<String>,
    status: Option<SessionGoalStatus>,
    progress: Option<u8>,
    clear: Option<bool>,
) -> Result<Option<SessionGoal>, String> {
    set_session_goal(&state, session_id, text, status, progress, clear)
}

/// Advance only an existing goal's status/progress. The Agent-facing caller
/// cannot supply goal text, clear the goal, or target a different session via
/// tool arguments. `expected_updated_at` prevents a concurrent user edit from
/// being overwritten by a stale tool call.
#[tauri::command(rename_all = "camelCase")]
pub fn session_goal_progress_update(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    status: SessionGoalStatus,
    progress: u8,
    expected_updated_at: i64,
) -> Result<SessionGoal, String> {
    update_existing_session_goal_progress(&state, session_id, status, progress, expected_updated_at)
}

fn update_existing_session_goal_progress(
    state: &AppState,
    session_id: String,
    status: SessionGoalStatus,
    progress: u8,
    expected_updated_at: i64,
) -> Result<SessionGoal, String> {
    state.require_persistence_ready()?;
    let id = validate_session_goal_session_id(&session_id)?.to_string();
    validate_session_goal_state(status, progress)?;
    if expected_updated_at <= 0 {
        return Err("session goal expected updated timestamp is invalid".to_string());
    }

    let _persist_guard = state.persist_lock.lock();
    let existing = state
        .sessions
        .lock()
        .get(&id)
        .ok_or_else(|| "session not found".to_string())?
        .goal
        .clone()
        .ok_or_else(|| "session goal does not exist".to_string())?;
    if existing.updated_at != expected_updated_at {
        return Err("session goal changed before progress update; read it again".to_string());
    }
    let goal = SessionGoal {
        text: existing.text,
        status,
        progress,
        updated_at: now_ms().max(existing.updated_at.saturating_add(1)),
    };
    let stored = StoredSessionGoal {
        text: goal.text.clone(),
        status: goal.status.as_str().to_string(),
        progress: goal.progress,
        updated_at: goal.updated_at,
    };
    state.history.upsert_session_goal(&id, &stored)?;
    let mut sessions = state.sessions.lock();
    let record = sessions
        .get_mut(&id)
        .ok_or_else(|| "session not found".to_string())?;
    record.goal = Some(goal.clone());
    Ok(goal)
}

fn set_session_goal(
    state: &AppState,
    session_id: String,
    text: Option<String>,
    status: Option<SessionGoalStatus>,
    progress: Option<u8>,
    clear: Option<bool>,
) -> Result<Option<SessionGoal>, String> {
    state.require_persistence_ready()?;
    let id = validate_session_goal_session_id(&session_id)?.to_string();
    let clear = clear.unwrap_or(false);
    if clear {
        if text.is_some() || status.is_some() || progress.is_some() {
            return Err("clear session goal cannot include text, status, or progress".to_string());
        }
        let _persist_guard = state.persist_lock.lock();
        if !state.sessions.lock().contains_key(&id) {
            return Err("session not found".to_string());
        }
        state.history.clear_session_goal(&id)?;
        if let Some(record) = state.sessions.lock().get_mut(&id) {
            record.goal = None;
        }
        return Ok(None);
    }

    let text = normalize_session_goal_text(
        text.ok_or_else(|| "session goal text is required".to_string())?,
    )?;
    let status = status.ok_or_else(|| "session goal status is required".to_string())?;
    let progress = progress.ok_or_else(|| "session goal progress is required".to_string())?;
    validate_session_goal_state(status, progress)?;
    let goal = SessionGoal {
        text,
        status,
        progress,
        updated_at: now_ms(),
    };
    let stored = StoredSessionGoal {
        text: goal.text.clone(),
        status: goal.status.as_str().to_string(),
        progress: goal.progress,
        updated_at: goal.updated_at,
    };

    let _persist_guard = state.persist_lock.lock();
    if !state.sessions.lock().contains_key(&id) {
        return Err("session not found".to_string());
    }
    state.history.upsert_session_goal(&id, &stored)?;
    if let Some(record) = state.sessions.lock().get_mut(&id) {
        record.goal = Some(goal.clone());
    }
    Ok(Some(goal))
}

/// Return the durable conversation transcript in the shape expected by the
/// embedded Pi adapter. The visual HTML is never used as runtime context.
#[tauri::command(rename_all = "camelCase")]
pub fn history_context_load(
    state: State<'_, Arc<AppState>>,
    session_id: String,
) -> Result<Value, String> {
    let id = session_id.trim();
    if id.is_empty() {
        return Ok(Value::Array(Vec::new()));
    }
    let context = state.history.load_runtime_context_with_metadata(id)?;
    let mut response = json!({ "messages": context.messages });
    if let Some(metadata) = context.manual_compaction {
        response["manualCompaction"] = metadata;
    }
    Ok(response)
}

/// Return the full durable source used to create a replacement manual summary.
/// This intentionally bypasses the currently active summary so repeated
/// `/compact` commands stay rooted in original local history.
#[tauri::command(rename_all = "camelCase")]
pub fn history_context_compaction_source_load(
    state: State<'_, Arc<AppState>>,
    session_id: String,
) -> Result<Value, String> {
    let id = validate_session_goal_session_id(&session_id)?;
    state
        .history
        .load_runtime_context_source(id)
        .map(Value::Array)
}

/// Persist a renderer-generated deterministic reference only after verifying
/// that its covered source boundary still matches the current session. The
/// summary is encrypted at rest by `HistoryStore`; the native host never asks a
/// provider to summarize or changes the original transcript.
#[tauri::command(rename_all = "camelCase")]
pub fn session_context_compaction_set(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    summary: String,
    metadata: Value,
) -> Result<Value, String> {
    state.require_persistence_ready()?;
    let id = validate_session_goal_session_id(&session_id)?.to_string();
    let source_message_count = metadata
        .get("sourceMessageEnd")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| "manual context summary source count is invalid".to_string())?;
    let _persist_guard = state.persist_lock.lock();
    if !state.sessions.lock().contains_key(&id) {
        return Err("session not found".to_string());
    }
    let source = state.history.load_runtime_context_source(&id)?;
    if source_message_count == 0 || source_message_count >= source.len() {
        return Err(
            "manual context summary must preserve at least one recent context message".to_string(),
        );
    }
    let stored = state
        .history
        .upsert_session_context_compaction(&id, &summary, &metadata)?;
    Ok(json!({
        "sourceMessageCount": stored.source_message_count,
        "metadata": stored.metadata,
    }))
}

/// Remove the manual summary record and make the next Pi turn load the full
/// original transcript again. No conversation rows are deleted.
#[tauri::command(rename_all = "camelCase")]
pub fn session_context_compaction_clear(
    state: State<'_, Arc<AppState>>,
    session_id: String,
) -> Result<(), String> {
    state.require_persistence_ready()?;
    let id = validate_session_goal_session_id(&session_id)?.to_string();
    let _persist_guard = state.persist_lock.lock();
    if !state.sessions.lock().contains_key(&id) {
        return Err("session not found".to_string());
    }
    state.history.clear_session_context_compaction(&id)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    pub session_id: String,
    pub user_text: String,
    pub assistant_text: String,
    pub mode: String,
    pub request_id: Option<String>,
}

fn validate_turn_reasoning(value: Option<String>) -> Result<Option<String>, String> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let reasoning = raw.trim().to_ascii_lowercase();
    if reasoning.is_empty() {
        return Ok(None);
    }
    match reasoning.as_str() {
        "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" => Ok(Some(reasoning)),
        _ => {
            Err("reasoning must be one of: off, minimal, low, medium, high, xhigh, max".to_string())
        }
    }
}

fn enrich_messages_with_turn_metadata(
    history: &HistoryStore,
    session_id: &str,
    messages: &[MessageRecord],
) -> Result<Vec<MessageRecord>, String> {
    let turn_meta = history.load_turn_metadata(session_id)?;
    Ok(messages
        .iter()
        .map(|message| {
            let mut projected = message.clone();
            // Clear any stale projection fields before re-applying durable turn data.
            projected.model = None;
            projected.reasoning = None;
            projected.finished_at = None;
            projected.thinking = None;
            if projected.role != "assistant" {
                return projected;
            }
            let Some(turn_id) = projected.turn_id.as_deref() else {
                return projected;
            };
            let Some(meta) = turn_meta.get(turn_id) else {
                return projected;
            };
            projected.model = meta.model.clone();
            projected.reasoning = meta.reasoning.clone();
            projected.finished_at = meta.finished_at;
            projected.thinking = meta.thinking.clone();
            projected
        })
        .collect())
}

const FULL_PERMISSION_GRANT_REQUIRED: &str =
    "full access requires a current grant for this exact run";

fn normalize_full_permission_run_identity(value: &str, field: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > MAX_FULL_PERMISSION_RUN_ID_CHARS
        || value.chars().any(|character| character.is_control())
    {
        return Err(format!("{field} is invalid"));
    }
    Ok(value.to_string())
}

fn full_permission_prompt_digest(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

fn full_permission_prompt_fingerprint(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_full_permission_prompt_hidden_formatting(character: char) -> bool {
    matches!(
        character,
        '\u{00ad}'
            | '\u{034f}'
            | '\u{061c}'
            | '\u{070f}'
            | '\u{17b4}'..='\u{17b5}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{e0000}'..='\u{e007f}'
    )
}

/// A Full grant is issued only for a bounded, unambiguous prompt payload.
fn validate_full_permission_prompt(text: &str) -> Result<(), String> {
    let mut character_count = 0usize;
    for character in text.chars() {
        character_count += 1;
        if character_count > MAX_FULL_PERMISSION_PROMPT_CHARS {
            return Err(format!(
                "full-access prompt must be at most {MAX_FULL_PERMISSION_PROMPT_CHARS} characters"
            ));
        }
        if (character.is_control() && character != '\n' && character != '\r' && character != '\t')
            || is_full_permission_prompt_hidden_formatting(character)
        {
            return Err(format!(
                "full-access prompt contains an invisible or bidirectional formatting character (U+{:04X})",
                character as u32
            ));
        }
    }
    Ok(())
}

fn normalize_full_permission_run_binding(
    provider_id: Option<&str>,
    model_id: Option<&str>,
    reasoning: Option<&str>,
) -> Result<FullPermissionRunBinding, String> {
    let provider_id = provider_id
        .and_then(valid_provider_id)
        .ok_or_else(|| "full access requires a valid provider id".to_string())?
        .to_string();
    let model_id = model_id
        .and_then(bounded_model_id)
        .ok_or_else(|| "full access requires a valid model id".to_string())?;
    if model_id
        .chars()
        .any(is_full_permission_prompt_hidden_formatting)
    {
        return Err(
            "full-access model id contains invisible or bidirectional formatting".to_string(),
        );
    }
    let reasoning = validate_turn_reasoning(reasoning.map(str::to_string))?;
    Ok(FullPermissionRunBinding {
        provider_id,
        model_id,
        reasoning,
    })
}

/// Resolve the renderer's requested Full-access transport against the native
/// provider registry. Syntactically valid IPC strings are not sufficient to
/// authorize a run: the provider and model must still be configured and
/// enabled, and the native resolver must return the exact requested route.
fn resolve_full_permission_run_binding(
    state: &AppState,
    provider_id: Option<&str>,
    model_id: Option<&str>,
    reasoning: Option<&str>,
) -> Result<FullPermissionRunBinding, String> {
    let requested = normalize_full_permission_run_binding(provider_id, model_id, reasoning)?;
    let runtime_config = provider_runtime_config_for(
        state,
        Some(requested.provider_id.as_str()),
        Some(requested.model_id.as_str()),
    )?;
    let provider = runtime_config
        .get("provider")
        .and_then(Value::as_object)
        .ok_or_else(|| "native provider runtime configuration is invalid".to_string())?;
    let resolved_provider_id = provider
        .get("id")
        .or_else(|| provider.get("providerId"))
        .or_else(|| provider.get("provider_id"))
        .and_then(Value::as_str)
        .and_then(valid_provider_id)
        .ok_or_else(|| "native provider runtime id is invalid".to_string())?
        .to_string();
    let resolved_model_id = provider
        .get("defaultModel")
        .and_then(Value::as_str)
        .and_then(bounded_model_id)
        .ok_or_else(|| "native provider runtime model is invalid".to_string())?;
    if resolved_provider_id != requested.provider_id || resolved_model_id != requested.model_id {
        return Err(
            "native provider runtime selection does not match the requested Full-access run"
                .to_string(),
        );
    }
    Ok(FullPermissionRunBinding {
        provider_id: resolved_provider_id,
        model_id: resolved_model_id,
        reasoning: requested.reasoning,
    })
}

fn prune_full_permission_grants(grants: &mut HashMap<String, FullPermissionRunGrant>) {
    let now = Instant::now();
    grants.retain(|_, grant| grant.expires_at > now);
}

/// Consume a native-memory-only, one-use Full-access grant. Removing the
/// record before comparing its bindings makes a token replay-safe even when a
/// caller supplies the wrong run details.
#[allow(clippy::too_many_arguments)]
fn consume_full_permission_grant(
    state: &AppState,
    token: Option<&str>,
    session_id: &str,
    conversation_id: &str,
    request_id: &str,
    workdir: &Path,
    text: &str,
    run_binding: &FullPermissionRunBinding,
) -> Result<(), String> {
    let token = token.map(str::trim).filter(|value| {
        !value.is_empty() && value.len() <= 128 && value.starts_with("full-permission-")
    });
    let Some(token) = token else {
        return Err(FULL_PERMISSION_GRANT_REQUIRED.to_string());
    };
    let grant = {
        let mut grants = state.full_permission_grants.lock();
        prune_full_permission_grants(&mut grants);
        grants
            .remove(token)
            .ok_or_else(|| FULL_PERMISSION_GRANT_REQUIRED.to_string())?
    };
    if grant.expires_at <= Instant::now()
        || grant.session_id != session_id
        || grant.conversation_id != conversation_id
        || grant.request_id != request_id
        || grant.workdir != workdir
        || grant.prompt_digest != full_permission_prompt_digest(text)
        || grant.run_binding.provider_id != run_binding.provider_id
        || grant.run_binding.model_id != run_binding.model_id
        || grant.run_binding.reasoning != run_binding.reasoning
    {
        return Err(FULL_PERMISSION_GRANT_REQUIRED.to_string());
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FullPermissionGrantResult {
    /// Opaque, native-memory-only token. It must be sent as
    /// `fullPermissionGrant` to the matching `agent_run` or `sessions_send`.
    pub grant_token: String,
    pub workdir: String,
    pub expires_at_ms: i64,
}

/// Issue a native-memory-only, one-use grant for one exact Full-access run.
/// No modal is shown. Generic renderer-writable settings are intentionally
/// never read here; they cannot constitute an authorization boundary.
#[cfg(feature = "desktop")]
#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub fn full_permission_confirm(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    session_id: String,
    conversation_id: Option<String>,
    request_id: String,
    text: String,
    provider_id: Option<String>,
    model: Option<String>,
    reasoning: Option<String>,
) -> Result<FullPermissionGrantResult, String> {
    state.require_persistence_ready()?;
    let workdir = canonical_workdir(&workdir)?;
    require_registered_project_workdir(&state, &workdir)?;
    let session_id = normalize_full_permission_run_identity(&session_id, "session id")?;
    let conversation_id = conversation_id
        .as_deref()
        .map(|value| normalize_full_permission_run_identity(value, "conversation id"))
        .transpose()?
        .unwrap_or_else(|| session_id.clone());
    let request_id = normalize_full_permission_run_identity(&request_id, "request id")?;
    let text = text.trim();
    if text.is_empty() {
        return Err("empty prompt".to_string());
    }
    validate_full_permission_prompt(text)?;
    let prompt_digest = full_permission_prompt_digest(text);
    let run_binding = resolve_full_permission_run_binding(
        &state,
        provider_id.as_deref(),
        model.as_deref(),
        reasoning.as_deref(),
    )?;

    let session_workdir = state
        .sessions
        .lock()
        .get(&session_id)
        .cloned()
        .ok_or_else(|| "session not found".to_string())
        .and_then(|session| canonical_workdir(&session.summary.cwd))?;
    if session_workdir != workdir {
        return Err("full access grant workdir does not match the session workspace".to_string());
    }

    // Revalidate both native bindings immediately before minting the grant.
    let current_session_workdir = state
        .sessions
        .lock()
        .get(&session_id)
        .cloned()
        .ok_or_else(|| "session changed while preparing the full-access grant".to_string())
        .and_then(|session| canonical_workdir(&session.summary.cwd))?;
    if current_session_workdir != workdir {
        return Err("session workspace changed while preparing the full-access grant".to_string());
    }
    require_registered_project_workdir(&state, &workdir)?;
    let current_run_binding = resolve_full_permission_run_binding(
        &state,
        provider_id.as_deref(),
        model.as_deref(),
        reasoning.as_deref(),
    )
    .map_err(|error| {
        format!("provider configuration changed while preparing the full-access grant: {error}")
    })?;
    if current_run_binding != run_binding {
        return Err("provider selection changed while preparing the full-access grant".to_string());
    }

    let grant_token = format!("full-permission-{}", Uuid::new_v4());
    let expires_at = Instant::now() + FULL_PERMISSION_GRANT_TTL;
    {
        let mut grants = state.full_permission_grants.lock();
        prune_full_permission_grants(&mut grants);
        if grants.len() >= MAX_PENDING_FULL_PERMISSION_GRANTS {
            return Err("too many pending full-access grants; wait for one to expire".to_string());
        }
        grants.insert(
            grant_token.clone(),
            FullPermissionRunGrant {
                session_id,
                conversation_id,
                request_id,
                workdir: workdir.clone(),
                prompt_digest,
                run_binding,
                expires_at,
            },
        );
    }
    Ok(FullPermissionGrantResult {
        grant_token,
        workdir: path_for_display(&workdir),
        expires_at_ms: now_ms() + FULL_PERMISSION_GRANT_TTL.as_millis() as i64,
    })
}

#[allow(clippy::too_many_arguments)]
fn start_transport_run<R: tauri::Runtime>(
    state: &AppState,
    app: &AppHandle<R>,
    session_id: Option<String>,
    conversation_id: Option<String>,
    text: String,
    display_text: Option<String>,
    permission: Option<String>,
    provider_id: Option<String>,
    model: Option<String>,
    reasoning: Option<String>,
    cwd: Option<String>,
    request_id: Option<String>,
    full_permission_grant: Option<String>,
) -> Result<AgentRunResult, String> {
    state.require_persistence_ready()?;
    // Reserve the run, write its initial projection, and publish the request
    // under one persistence gate.  A second invocation with the same request
    // id must never be able to overwrite the first capability or snapshot.
    let _persist_guard = state.persist_lock.lock();
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("empty prompt".to_string());
    }
    // Composer commands may expand a short user-facing slash command into a
    // richer instruction for Pi. Persist only the explicit display text so a
    // later history reload never exposes hidden command scaffolding or local
    // Skill content as though the user typed it.
    let display_text = display_text
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| text.clone());
    let reasoning = validate_turn_reasoning(reasoning)?;
    let permission_mode = PermissionMode::parse(permission.as_deref());
    let full_permission_run_binding = if permission_mode == PermissionMode::Full {
        Some(resolve_full_permission_run_binding(
            state,
            provider_id.as_deref(),
            model.as_deref(),
            reasoning.as_deref(),
        )?)
    } else {
        None
    };
    let requested_session_id = if permission_mode == PermissionMode::Full {
        session_id
            .as_deref()
            .map(|value| normalize_full_permission_run_identity(value, "session id"))
            .transpose()?
    } else {
        session_id.clone().filter(|value| !value.trim().is_empty())
    };
    let requested_workdir = cwd.as_deref().map(canonical_workdir).transpose()?;
    let workdir = if let Some(session_id) = requested_session_id.as_deref() {
        let stored = state
            .sessions
            .lock()
            .get(session_id)
            .cloned()
            .ok_or_else(|| "session not found".to_string())?;
        let stored = canonical_workdir(&stored.summary.cwd)?;
        if let Some(requested) = requested_workdir.as_ref() {
            if requested != &stored {
                return Err("agent cwd does not match the session workspace".to_string());
            }
        }
        stored
    } else {
        requested_workdir.unwrap_or_else(|| {
            canonical_workdir(&current_workdir()).expect("current workdir must be accessible")
        })
    };
    require_registered_project_workdir(state, &workdir)?;
    let workdir_string = path_for_display(&workdir);
    // A Full grant is always tied to a durable session. This keeps a
    // renderer from using an unbound, newly-created session as a way to widen
    // the scope of a confirmation that was shown for another conversation.
    if permission_mode == PermissionMode::Full && requested_session_id.is_none() {
        return Err(
            "full access requires an existing session and a current grant for this exact run"
                .to_string(),
        );
    }
    let (session_id, previous_session, pending_new_session) =
        if let Some(session_id) = requested_session_id {
            let previous = state
                .sessions
                .lock()
                .get(&session_id)
                .cloned()
                .ok_or_else(|| "session not found".to_string())?;
            (session_id, Some(previous), None)
        } else {
            let record = new_session_record("新建对话".to_string(), workdir_string.clone());
            let id = record.summary.id.clone();
            (id, None, Some(record))
        };
    let conversation_id = conversation_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| session_id.clone());
    let conversation_id = if permission_mode == PermissionMode::Full {
        normalize_full_permission_run_identity(&conversation_id, "conversation id")?
    } else {
        conversation_id
    };
    let turn_id = Uuid::new_v4().to_string();
    let request_id = request_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let request_id = if permission_mode == PermissionMode::Full {
        normalize_full_permission_run_identity(&request_id, "request id")?
    } else {
        request_id
    };
    if state.active_runs.lock().contains_key(&request_id) {
        return Err("request is already running".to_string());
    }
    let proxy_provider_id = if permission_mode == PermissionMode::Full {
        let run_binding = full_permission_run_binding
            .as_ref()
            .expect("Full mode must have a resolved run binding");
        consume_full_permission_grant(
            state,
            full_permission_grant.as_deref(),
            &session_id,
            &conversation_id,
            &request_id,
            &workdir,
            &text,
            run_binding,
        )?;
        Some(run_binding.provider_id.clone())
    } else {
        resolved_proxy_provider_id(state, provider_id.as_deref(), model.as_deref())
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    let capability_token = format!("cap-{}", Uuid::new_v4());
    let active = ActiveRun {
        session_id: session_id.clone(),
        conversation_id: conversation_id.clone(),
        turn_id: turn_id.clone(),
        request_id: request_id.clone(),
        proxy_provider_id,
        capability_token: capability_token.clone(),
        cancelled: cancelled.clone(),
    };
    state.active_runs.lock().insert(request_id.clone(), active);
    state.capabilities.lock().insert(
        capability_token.clone(),
        CapabilityGrant {
            session_id: session_id.clone(),
            conversation_id: conversation_id.clone(),
            turn_id: turn_id.clone(),
            request_id: request_id.clone(),
            workdir: workdir.clone(),
            permission_mode,
            expires_at: Instant::now() + Duration::from_secs(24 * 60 * 60),
            cancelled,
        },
    );
    if let Some(record) = pending_new_session {
        state.sessions.lock().insert(session_id.clone(), record);
    }

    if let Err(error) = append_message(
        state,
        &session_id,
        "user",
        display_text,
        Some(turn_id.clone()),
    ) {
        revoke_run(state, &request_id);
        restore_session_snapshot(state, &session_id, previous_session.as_ref());
        return Err(error);
    }
    if let Some(record) = state.sessions.lock().get_mut(&session_id) {
        if let Some(provider_id) = provider_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            record.provider_id = provider_id.to_string();
        }
        if let Some(model) = model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            record.model = model.to_string();
        }
    }
    if let Err(error) = state.persist_session_locked(&session_id) {
        revoke_run(state, &request_id);
        restore_session_snapshot(state, &session_id, previous_session.as_ref());
        return Err(error);
    }
    if let Err(error) = state.history.upsert_turn(
        &session_id,
        &conversation_id,
        &turn_id,
        &request_id,
        "running",
        provider_id.as_deref(),
        model.as_deref(),
        reasoning.as_deref(),
        Some(workdir_string.as_str()),
        now_ms(),
    ) {
        revoke_run(state, &request_id);
        restore_session_snapshot(state, &session_id, previous_session.as_ref());
        if let Err(rollback_error) = state.persist_session_locked(&session_id) {
            eprintln!("failed to roll back agent run snapshot: {rollback_error}");
        }
        return Err(error);
    }

    let payload = json!({
        "type": "request",
        "requestId": request_id,
        "request_id": request_id,
        "sessionId": session_id,
        "session_id": session_id,
        "conversationId": conversation_id,
        "conversation_id": conversation_id,
        "turnId": turn_id,
        "turn_id": turn_id,
        "text": text,
        "permission": permission_mode.as_str(),
        "providerId": provider_id,
        "provider_id": provider_id,
        "model": model,
        "cwd": workdir_string,
    });
    // This is a transport hook only.  A WebView Pi runner may consume it, but
    // no native code fabricates an assistant answer.
    if let Err(error) = app.emit("agent:request", payload) {
        revoke_run(state, &request_id);
        if let Err(delete_error) = state.history.delete_turn(&turn_id) {
            eprintln!("failed to roll back agent turn: {delete_error}");
        }
        restore_session_snapshot(state, &session_id, previous_session.as_ref());
        if let Err(rollback_error) = state.persist_session_locked(&session_id) {
            eprintln!("failed to roll back agent request snapshot: {rollback_error}");
        }
        return Err(format!("emit agent request: {error}"));
    }

    Ok(AgentRunResult {
        session_id,
        conversation_id,
        turn_id,
        request_id,
        capability_token,
        mode: "transport-hook".to_string(),
    })
}

fn restore_session_snapshot(state: &AppState, session_id: &str, previous: Option<&SessionRecord>) {
    let mut sessions = state.sessions.lock();
    match previous {
        Some(record) => {
            sessions.insert(session_id.to_string(), record.clone());
        }
        None => {
            sessions.remove(session_id);
        }
    }
}

fn revoke_run(state: &AppState, request_id: &str) {
    if let Some(run) = state.active_runs.lock().remove(request_id) {
        run.cancelled.store(true, Ordering::SeqCst);
        clear_capability_state(state, &run.capability_token);
    }
}

fn clear_capability_state(state: &AppState, capability_token: &str) {
    state.capabilities.lock().remove(capability_token);
    state
        .pending_permissions
        .lock()
        .retain(|_, permission| permission.capability_token != capability_token);
    state
        .tool_approvals
        .lock()
        .retain(|_, approval| approval.capability_token != capability_token);
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunResult {
    pub session_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub request_id: String,
    pub capability_token: String,
    pub mode: String,
}

#[cfg(feature = "desktop")]
#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub fn agent_run(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    session_id: Option<String>,
    conversation_id: Option<String>,
    text: String,
    display_text: Option<String>,
    permission: Option<String>,
    provider_id: Option<String>,
    model: Option<String>,
    reasoning: Option<String>,
    cwd: Option<String>,
    request_id: Option<String>,
    full_permission_grant: Option<String>,
) -> Result<AgentRunResult, String> {
    start_transport_run(
        &state,
        &app,
        session_id,
        conversation_id,
        text,
        display_text,
        permission,
        provider_id,
        model,
        reasoning,
        cwd,
        request_id,
        full_permission_grant,
    )
}

#[cfg(feature = "desktop")]
#[tauri::command(rename_all = "camelCase")]
pub fn agent_cancel(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    session_id: Option<String>,
    conversation_id: Option<String>,
    turn_id: Option<String>,
    request_id: Option<String>,
) -> Result<CancelResult, String> {
    agent_cancel_inner(state, app, session_id, conversation_id, turn_id, request_id)
}

// Keep the Tauri command concrete for IPC registration, while allowing the
// same state-transition path to be exercised against Tauri's mock runtime.
fn agent_cancel_inner<R: tauri::Runtime>(
    state: State<'_, Arc<AppState>>,
    app: AppHandle<R>,
    session_id: Option<String>,
    conversation_id: Option<String>,
    turn_id: Option<String>,
    request_id: Option<String>,
) -> Result<CancelResult, String> {
    state.require_persistence_ready()?;
    let _terminal_guard = state.persist_lock.lock();
    let key = request_id
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            state
                .active_runs
                .lock()
                .values()
                .find(|run| {
                    session_id
                        .as_deref()
                        .map(|id| id == run.session_id)
                        .unwrap_or(true)
                        && conversation_id
                            .as_deref()
                            .map(|id| id == run.conversation_id)
                            .unwrap_or(true)
                        && turn_id
                            .as_deref()
                            .map(|id| id == run.turn_id)
                            .unwrap_or(true)
                })
                .map(|run| run.request_id.clone())
        });
    let Some(key) = key else {
        return Ok(CancelResult { cancelled: false });
    };
    let Some(run) = state.active_runs.lock().get(&key).cloned() else {
        return Ok(CancelResult { cancelled: false });
    };
    let identity_matches = session_id
        .as_deref()
        .map(|value| value == run.session_id)
        .unwrap_or(true)
        && conversation_id
            .as_deref()
            .map(|value| value == run.conversation_id)
            .unwrap_or(true)
        && turn_id
            .as_deref()
            .map(|value| value == run.turn_id)
            .unwrap_or(true);
    if !identity_matches {
        return Err("cancellation identity does not match the active run".to_string());
    }
    run.cancelled.store(true, Ordering::SeqCst);
    let owned_shells = state
        .shell_runs
        .lock()
        .values()
        .filter(|shell| shell.capability_token == run.capability_token)
        .map(|shell| shell.cancelled.clone())
        .collect::<Vec<_>>();
    for cancellation in owned_shells {
        cancellation.store(true, Ordering::SeqCst);
    }
    cancel_subagent_tasks_for_parent(&state, &app, &run.request_id, now_ms());
    let payload = agent_event_payload(
        "cancelled",
        &run.session_id,
        &run.conversation_id,
        &run.turn_id,
        &run.request_id,
        json!({"cancelled": true}),
    );
    // Do not announce a cancellation until the session summary has been
    // durably updated. If this fails, retain the active run so the renderer
    // does not mistake an in-memory cancellation for a saved terminal state.
    persist_terminal_projection_locked(&state, &run, &payload)?;
    let finished_at = now_ms();
    let history_new = state.history.append_event(&payload, finished_at)?;
    if !history_new {
        return Ok(CancelResult { cancelled: false });
    }
    state.active_runs.lock().remove(&key);
    clear_capability_state(&state, &run.capability_token);
    app.emit("agent:event", payload)
        .map_err(|error| format!("emit cancellation event: {error}"))?;
    Ok(CancelResult { cancelled: true })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelResult {
    pub cancelled: bool,
}

fn agent_event_payload(
    event_type: &str,
    session_id: &str,
    conversation_id: &str,
    turn_id: &str,
    request_id: &str,
    extra: Value,
) -> Value {
    let mut object = match extra {
        Value::Object(value) => value,
        _ => Map::new(),
    };
    object.insert("type".to_string(), Value::String(event_type.to_string()));
    object.insert(
        "sessionId".to_string(),
        Value::String(session_id.to_string()),
    );
    object.insert(
        "session_id".to_string(),
        Value::String(session_id.to_string()),
    );
    object.insert(
        "conversationId".to_string(),
        Value::String(conversation_id.to_string()),
    );
    object.insert(
        "conversation_id".to_string(),
        Value::String(conversation_id.to_string()),
    );
    object.insert("turnId".to_string(), Value::String(turn_id.to_string()));
    object.insert("turn_id".to_string(), Value::String(turn_id.to_string()));
    object.insert(
        "requestId".to_string(),
        Value::String(request_id.to_string()),
    );
    object.insert(
        "request_id".to_string(),
        Value::String(request_id.to_string()),
    );
    Value::Object(object)
}

fn known_event_type(value: &str) -> bool {
    matches!(
        value,
        "run_started"
            | "text_delta"
            | "thinking_delta"
            | "tool_call"
            | "tool_update"
            | "tool_result"
            | "permission_requested"
            | "done"
            | "error"
            | "cancelled"
    )
}

/// Forward an event produced by the embedded Pi runtime.  This command is
/// intentionally narrow: arbitrary event types are rejected, and terminal
/// events remove the corresponding native run record.
#[cfg(feature = "desktop")]
#[tauri::command]
pub fn agent_emit_event(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    payload: Value,
) -> Result<(), String> {
    agent_emit_event_inner(state, app, payload)
}

// See `agent_cancel_inner`: command functions use the desktop runtime, but
// storage and capability invariants must also be testable with MockRuntime.
fn agent_emit_event_inner<R: tauri::Runtime>(
    state: State<'_, Arc<AppState>>,
    app: AppHandle<R>,
    payload: Value,
) -> Result<(), String> {
    state.require_persistence_ready()?;
    // Treat event ingestion as an untrusted renderer boundary too. Embedded
    // Pi redacts before IPC, but direct callers must not put credentials into
    // SQLite or the native event bus either.
    let payload = redact_event_value(&payload, false);
    let event_type = payload
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "agent event type is required".to_string())?;
    if !known_event_type(event_type) {
        return Err(format!("unsupported agent event type: {event_type}"));
    }
    let terminal = matches!(event_type, "done" | "error" | "cancelled");
    // Serialize event ingestion with snapshots and cancellation. A failed
    // terminal projection/audit write leaves the run active so the renderer
    // can retry the exact terminal event; non-terminal events cannot race a
    // capability revocation and land after the terminal event.
    let _event_guard = state.persist_lock.lock();
    let request_id = payload
        .get("requestId")
        .or_else(|| payload.get("request_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| "agent event request id is required".to_string())?;
    let active_identity = state.active_runs.lock().get(request_id).cloned();
    let mut replay_terminal = false;
    if let Some(run) = active_identity.as_ref() {
        if run.cancelled.load(Ordering::SeqCst) && event_type != "cancelled" {
            return Err("agent cancellation is pending persistence".to_string());
        }
        let session = payload
            .get("sessionId")
            .or_else(|| payload.get("session_id"))
            .and_then(Value::as_str);
        let turn = payload
            .get("turnId")
            .or_else(|| payload.get("turn_id"))
            .and_then(Value::as_str);
        if session != Some(run.session_id.as_str()) || turn != Some(run.turn_id.as_str()) {
            return Err("agent event identity does not match the active run".to_string());
        }
    } else if !terminal {
        return Err("agent run is no longer active".to_string());
    } else {
        match state.history.load_terminal_event(request_id)? {
            Some(stored) if stored == payload => replay_terminal = true,
            Some(_) => return Ok(()),
            None => return Err("agent run is no longer active".to_string()),
        }
    }
    if terminal {
        if let Some(run) = active_identity.as_ref() {
            persist_terminal_projection_locked(&state, run, &payload)?;
        }
    }
    let event_persisted_at = now_ms();
    let history_new = state.history.append_event(&payload, event_persisted_at)?;
    if terminal {
        if !history_new {
            if replay_terminal {
                return app
                    .emit("agent:event", payload)
                    .map_err(|error| format!("re-emit terminal agent event: {error}"));
            }
            return Ok(());
        }
        // The terminal event is now durable. Flip the shared revocation flag
        // before removing the active record so a concurrent transport
        // handshake cannot squeeze between those two terminal steps.
        if let Some(run) = active_identity.as_ref() {
            run.cancelled.store(true, Ordering::SeqCst);
        }
        let removed = state.active_runs.lock().remove(request_id);
        if let Some(run) = removed.as_ref() {
            // This flag is shared with every issued proxy token. A successful
            // completion/error is terminal just like a user cancellation:
            // neither may leave a bearer token able to start another request.
            cancel_subagent_tasks_for_parent(&state, &app, &run.request_id, event_persisted_at);
            clear_capability_state(&state, &run.capability_token);
        }
    }
    if let Some(request_id) = payload
        .get("requestId")
        .or_else(|| payload.get("request_id"))
        .and_then(Value::as_str)
    {
        if event_type == "permission_requested" {
            if let Some(permission) = payload.get("permission") {
                if let Some(permission_id) = permission
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    if let Some(run) = state.active_runs.lock().get(request_id).cloned() {
                        let tool_name = permission
                            .get("toolName")
                            .or_else(|| permission.get("tool_name"))
                            .and_then(Value::as_str)
                            .unwrap_or("tool")
                            .trim()
                            .to_string();
                        if tool_action(&tool_name).is_none() {
                            return Err("permission request names an unsupported tool".to_string());
                        }
                        let subagent_global_read = tool_name
                            .eq_ignore_ascii_case("delegatereadonly")
                            || tool_name.eq_ignore_ascii_case("delegateworktree");
                        let subagent_global_read = subagent_global_read
                            && permission
                                .get("arguments")
                                .and_then(Value::as_object)
                                .and_then(|arguments| {
                                    arguments
                                        .get("allow_global_read")
                                        .or_else(|| arguments.get("allowGlobalRead"))
                                })
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                        state.pending_permissions.lock().insert(
                            permission_id.to_string(),
                            PendingPermission {
                                capability_token: run.capability_token,
                                session_id: run.session_id,
                                conversation_id: run.conversation_id,
                                turn_id: run.turn_id,
                                request_id: run.request_id,
                                tool_name,
                                subagent_global_read,
                            },
                        );
                    }
                }
            }
        }
        if event_type == "text_delta" {
            if let (Some(session_id), Some(delta)) = (
                payload
                    .get("sessionId")
                    .or_else(|| payload.get("session_id"))
                    .and_then(Value::as_str),
                payload
                    .get("delta")
                    .or_else(|| payload.get("text"))
                    .and_then(Value::as_str),
            ) {
                // Incremental text is kept in the transcript by Pi; storing
                // every delta here would duplicate history.  The final event
                // is persisted below.
                let _ = (session_id, delta);
            }
        }
    }
    app.emit("agent:event", payload)
        .map_err(|error| format!("emit agent event: {error}"))
}

/// Alias retained for adapters that use the shorter command name.
#[cfg(feature = "desktop")]
#[tauri::command]
pub fn agent_event(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    event: Value,
) -> Result<(), String> {
    agent_emit_event_inner(state, app, event)
}

#[cfg(feature = "desktop")]
#[tauri::command(rename_all = "camelCase")]
pub fn agent_permission(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    request_id: String,
    decision: String,
) -> Result<(), String> {
    agent_permission_inner(state, app, request_id, decision)
}

fn agent_permission_inner<R: tauri::Runtime>(
    state: State<'_, Arc<AppState>>,
    app: AppHandle<R>,
    request_id: String,
    decision: String,
) -> Result<(), String> {
    state.require_persistence_ready()?;
    let request_id = request_id.trim().to_string();
    if request_id.is_empty() {
        return Err("permission request id is required".to_string());
    }
    let decision = match decision.trim() {
        "allow" => "allow".to_string(),
        "deny" => "deny".to_string(),
        "cancel" => "cancel".to_string(),
        _ => return Err("permission decision is invalid".to_string()),
    };
    let pending = state
        .pending_permissions
        .lock()
        .get(&request_id)
        .cloned()
        .ok_or_else(|| "permission request is unknown or already resolved".to_string())?;
    let grant = state
        .capabilities
        .lock()
        .get(&pending.capability_token)
        .cloned();
    let Some(grant) = grant else {
        return Err("permission request no longer belongs to an active run".to_string());
    };
    if !pending_permission_belongs_to_live_grant(&state, &pending, &grant) {
        return Err("permission request no longer belongs to an active run".to_string());
    }
    let action = tool_action(&pending.tool_name)
        .ok_or_else(|| "permission request names an unsupported tool".to_string())?;
    let requires_explicit_approval = permission_requirement(grant.permission_mode, action)
        == PermissionRequirement::Approval
        || action == ToolAction::Read
        || pending.subagent_global_read;
    if !requires_explicit_approval {
        return Err("permission request is not required for this capability mode".to_string());
    }
    // The visible tool approval card is the explicit user decision for this
    // exact pending request. Native code still verifies the live capability,
    // claims the pending request atomically, persists the decision, and mints
    // a one-use approval below; showing a second OS dialog here only repeats
    // the same consent. The separate Full-access flow remains run-bound.
    let removed_pending = state.pending_permissions.lock().remove(&request_id);
    if removed_pending.as_ref() != Some(&pending) {
        return Err("permission request was resolved concurrently".to_string());
    }
    if state
        .history
        .resolve_permission(&request_id, &decision, now_ms())
        .is_err()
    {
        // The request was claimed above to serialize competing decisions. Put
        // it back before returning so the renderer can retry rather than
        // showing an accepted decision that was never recorded durably.
        state
            .pending_permissions
            .lock()
            .entry(request_id.clone())
            .or_insert(pending.clone());
        diagnostics::record_event(
            "storage",
            "permission_decision_persist_failed",
            "failure",
            None,
        );
        return Err(
            "NovaVei could not persist the permission decision; retry after storage recovery"
                .to_string(),
        );
    }
    if decision == "allow" {
        state.tool_approvals.lock().insert(
            request_id.clone(),
            ToolApprovalGrant {
                capability_token: pending.capability_token.clone(),
                action,
                tool_name: pending.tool_name.clone(),
                subagent_global_read: pending.subagent_global_read,
                expires_at: Instant::now() + Duration::from_secs(5 * 60),
            },
        );
    }
    app.emit(
        "agent:permission",
        json!({
            "requestId": request_id,
            "request_id": request_id,
            "sessionId": pending.session_id,
            "conversationId": pending.conversation_id,
            "turnId": pending.turn_id,
            "decision": decision,
        }),
    )
    .map_err(|error| format!("emit permission decision: {error}"))
}

#[cfg(feature = "desktop")]
#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub fn sessions_send(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    session_id: Option<String>,
    text: String,
    permission: Option<String>,
    provider_id: Option<String>,
    model: Option<String>,
    reasoning: Option<String>,
    request_id: Option<String>,
    full_permission_grant: Option<String>,
) -> Result<SendResult, String> {
    let result = start_transport_run(
        &state,
        &app,
        session_id,
        None,
        text.clone(),
        None,
        permission,
        provider_id,
        model,
        reasoning,
        None,
        request_id,
        full_permission_grant,
    )?;
    Ok(SendResult {
        session_id: result.session_id,
        user_text: text.trim().to_string(),
        assistant_text: "".to_string(),
        mode: result.mode,
        request_id: Some(result.request_id),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    pub status: String,
}

#[tauri::command]
pub fn providers_list(state: State<'_, Arc<AppState>>) -> Vec<ProviderInfo> {
    let configured = state.settings.lock().get("providers").cloned();
    let Some(Value::Array(items)) = configured else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            let id = object.get("id")?.as_str()?.to_string();
            let name = object
                .get("name")
                .or_else(|| object.get("label"))
                .and_then(Value::as_str)
                .unwrap_or(&id)
                .to_string();
            let protocol = object
                .get("protocol")
                .and_then(Value::as_str)
                .unwrap_or("openai")
                .to_string();
            let base_url = object
                .get("baseUrl")
                .or_else(|| object.get("base_url"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let status =
                if provider_has_credentials(object) || provider_allows_keyless_local(object) {
                    "configured"
                } else {
                    "needs-key"
                }
                .to_string();
            Some(ProviderInfo {
                id,
                name,
                protocol,
                base_url,
                status,
            })
        })
        .collect()
}

/// Native proxy state resolved from one settings snapshot. Keeping the endpoint,
/// credentials, and routing decision together prevents a concurrent settings
/// update from combining values belonging to different provider revisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProxyConfig {
    /// Full native-configured provider base URL, including its path prefix.
    /// Gateway paths can select tenants, so an origin alone is not sufficient.
    pub upstream_base_url: String,
    pub headers: Vec<(String, String)>,
    pub use_system_proxy: bool,
}

fn provider_record<'a>(
    settings: &'a HashMap<String, Value>,
    provider_id: &str,
) -> Option<&'a Map<String, Value>> {
    let providers = settings.get("providers")?;
    match providers {
        Value::Array(items) => items.iter().filter_map(Value::as_object).find(|object| {
            object
                .get("id")
                .or_else(|| object.get("providerId"))
                .and_then(Value::as_str)
                == Some(provider_id)
        }),
        Value::Object(items) => items.iter().find_map(|(key, value)| {
            let object = value.as_object()?;
            let id = object
                .get("id")
                .or_else(|| object.get("providerId"))
                .and_then(Value::as_str)
                .or(Some(key.as_str()));
            (id == Some(provider_id)).then_some(object)
        }),
        _ => None,
    }
}

/// Providers are opt-in at the native boundary.  A disabled record may remain
/// visible in Settings so the user can edit or re-enable it, but it must not
/// be selected for a completion, model catalogue request, or proxy credential
/// resolution.
fn provider_is_enabled(object: &Map<String, Value>) -> bool {
    object.get("enabled").and_then(Value::as_bool) != Some(false)
}

fn provider_proxy_config_from_settings(
    settings: &HashMap<String, Value>,
    provider_id: &str,
) -> Option<ProviderProxyConfig> {
    let object = provider_record(settings, provider_id)?;
    // Disabled providers must not retain a native proxy path to their
    // credentials. This is enforced here rather than relying on picker UI.
    if !provider_is_enabled(object) {
        return None;
    }
    Some(ProviderProxyConfig {
        upstream_base_url: provider_proxy_base_url(object)?,
        headers: provider_credentials_from_object(object),
        use_system_proxy: object
            .get("useSystemProxy")
            .or_else(|| object.get("use_system_proxy"))
            .or_else(|| object.get("systemProxy"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// Resolve all native proxy inputs while holding settings exactly once. The
/// returned headers never cross into the WebView, history, or Pi events.
pub fn provider_proxy_config(state: &AppState, provider_id: &str) -> Option<ProviderProxyConfig> {
    let settings = state.settings.lock();
    provider_proxy_config_from_settings(&settings, provider_id)
}

fn provider_credentials_from_object(object: &Map<String, Value>) -> Vec<(String, String)> {
    if !provider_secret_binding_matches(object) {
        // A stale persisted record must not silently send its key to the new
        // endpoint. Saving the provider again with an explicit key creates a
        // fresh native binding.
        return Vec::new();
    }
    provider_credentials_from_object_unchecked(object)
}

fn provider_test_credentials_from_object(
    object: &Map<String, Value>,
) -> Result<Vec<(String, String)>, String> {
    if !provider_secret_binding_matches(object)
        && !provider_credentials_from_object_unchecked(object).is_empty()
    {
        return Err(
            "provider endpoint or auth family changed; re-enter its credentials before testing"
                .to_string(),
        );
    }
    Ok(provider_credentials_from_object(object))
}

fn provider_credentials_from_object_unchecked(
    object: &Map<String, Value>,
) -> Vec<(String, String)> {
    let api_key = [
        "apiKey",
        "api_key",
        "key",
        "token",
        "accessToken",
        "access_token",
    ]
    .iter()
    .find_map(|key| object.get(*key).and_then(Value::as_str))
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(str::to_string);
    let protocol = provider_protocol(object);
    let mut headers = Vec::new();
    if let Some(api_key) = api_key.as_deref() {
        if protocol.contains("anthropic") || protocol.contains("claude") {
            headers.push(("x-api-key".to_string(), api_key.to_string()));
            headers.push(("anthropic-version".to_string(), "2023-06-01".to_string()));
        } else if protocol.contains("gemini") || protocol.contains("google") {
            headers.push(("x-goog-api-key".to_string(), api_key.to_string()));
        } else {
            headers.push(("authorization".to_string(), format!("Bearer {api_key}")));
        }
    }
    append_custom_headers(object, &mut headers);
    headers
}

fn provider_has_credentials(object: &Map<String, Value>) -> bool {
    if !provider_secret_binding_matches(object) {
        return false;
    }
    [
        "apiKey",
        "api_key",
        "key",
        "token",
        "accessToken",
        "access_token",
    ]
    .iter()
    .any(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    }) || {
        let mut headers = Vec::new();
        append_custom_headers(object, &mut headers);
        !headers.is_empty()
    }
}

fn provider_allows_keyless_local(object: &Map<String, Value>) -> bool {
    let base = object
        .get("baseUrl")
        .or_else(|| object.get("base_url"))
        .or_else(|| object.get("endpoint"))
        .or_else(|| object.get("url"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(host) = Url::parse(base.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
    else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host.to_ascii_lowercase().ends_with(".localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// Provider settings have existed in both `protocol`/`api` and
/// `type`/`requestFormat` forms. Resolve all of them before choosing the
/// native auth header so a renderer-controlled display label cannot change
/// credential semantics.
fn provider_protocol(object: &Map<String, Value>) -> String {
    let values = ["type", "protocol", "api", "requestFormat", "request_format"]
        .iter()
        .filter_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values
        .iter()
        .find(|value| {
            let lower = value.to_ascii_lowercase();
            lower.contains("anthropic")
                || lower.contains("claude")
                || lower.contains("gemini")
                || lower.contains("google")
                || lower.contains("openai")
                || lower.contains("codex")
        })
        .copied()
        .or_else(|| values.first().copied())
        .unwrap_or("openai")
        .to_ascii_lowercase()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderAuthFamily {
    OpenAi,
    Anthropic,
    Gemini,
}

impl ProviderAuthFamily {
    fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
        }
    }
}

fn provider_auth_family(object: &Map<String, Value>) -> ProviderAuthFamily {
    let protocol = provider_protocol(object);
    if protocol.contains("anthropic") || protocol.contains("claude") {
        ProviderAuthFamily::Anthropic
    } else if protocol.contains("gemini") || protocol.contains("google") {
        ProviderAuthFamily::Gemini
    } else {
        // OpenAI-compatible endpoints (including custom gateways) use the
        // bearer-key family unless they were explicitly classified above.
        ProviderAuthFamily::OpenAi
    }
}

fn provider_endpoint_url(object: &Map<String, Value>) -> Option<Url> {
    let family = provider_auth_family(object);
    let raw = object
        .get("baseUrl")
        .or_else(|| object.get("base_url"))
        .or_else(|| object.get("endpoint"))
        .or_else(|| object.get("url"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| match family {
            ProviderAuthFamily::Anthropic => "https://api.anthropic.com".to_string(),
            ProviderAuthFamily::Gemini => {
                "https://generativelanguage.googleapis.com/v1beta".to_string()
            }
            ProviderAuthFamily::OpenAi => "https://api.openai.com/v1".to_string(),
        });
    let url = Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.has_host()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(url)
}

fn provider_endpoint_origin(object: &Map<String, Value>) -> Option<String> {
    let url = provider_endpoint_url(object)?;
    Some(url.origin().ascii_serialization().to_ascii_lowercase())
}

/// Return the exact native endpoint accepted by the loopback proxy. In
/// particular, preserve its path because gateway paths commonly identify a
/// tenant. Legacy records with a query or fragment fail closed rather than
/// letting a renderer choose a different target below the same origin.
fn provider_proxy_base_url(object: &Map<String, Value>) -> Option<String> {
    let mut url = provider_endpoint_url(object)?;
    if url.query().is_some() || url.fragment().is_some() {
        return None;
    }
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(if path.is_empty() { "/" } else { &path });
    Some(url.to_string())
}

fn provider_credential_endpoint(object: &Map<String, Value>) -> Option<String> {
    let mut url = provider_endpoint_url(object)?;
    // URL fragments never reach the provider; omit them so cosmetic changes do
    // not create a distinct credential target. The path and query remain part
    // of the binding because they can route to distinct gateway tenants.
    url.set_fragment(None);
    Some(url.to_string())
}

fn provider_credential_context(
    object: &Map<String, Value>,
) -> Option<(String, ProviderAuthFamily)> {
    Some((
        provider_credential_endpoint(object)?,
        provider_auth_family(object),
    ))
}

fn provider_credential_binding_value(object: &Map<String, Value>) -> Value {
    let origin = provider_endpoint_origin(object)
        .map(Value::String)
        .unwrap_or(Value::Null);
    let endpoint = provider_credential_endpoint(object)
        .map(Value::String)
        .unwrap_or(Value::Null);
    json!({
        "origin": origin,
        "endpoint": endpoint,
        "authFamily": provider_auth_family(object).as_str(),
    })
}

/// A missing binding is accepted for legacy records. Once a provider is
/// saved, native code writes the binding and all subsequent credential reads
/// require an exact endpoint/family match. Legacy origin-only bindings remain
/// readable at their stored endpoint, but cannot carry a credential to a new
/// path because `provider_secret_context_matches` compares endpoint identity.
fn provider_secret_binding_matches(object: &Map<String, Value>) -> bool {
    let Some(binding) = object.get(PROVIDER_CREDENTIAL_BINDING_KEY) else {
        return true;
    };
    let Some(binding) = binding.as_object() else {
        return false;
    };
    let Some(current) = provider_credential_context(object) else {
        return false;
    };
    let Some(family) = binding.get("authFamily").and_then(Value::as_str) else {
        return false;
    };
    if family != current.1.as_str() {
        return false;
    }
    if let Some(endpoint) = binding.get("endpoint").and_then(Value::as_str) {
        return endpoint == current.0.as_str();
    }
    // Pre-endpoint-binding records are valid only at their own stored origin.
    // The later old/new context comparison makes path changes fail closed.
    binding.get("origin").and_then(Value::as_str) == provider_endpoint_origin(object).as_deref()
}

fn provider_secret_context_matches(
    existing: &Map<String, Value>,
    incoming: &Map<String, Value>,
) -> bool {
    provider_secret_binding_matches(existing)
        && provider_credential_context(existing) == provider_credential_context(incoming)
}

fn is_proxy_header(name: &str) -> bool {
    let lowered = name.trim().to_ascii_lowercase();
    lowered.starts_with("x-novavei-")
}

fn is_reserved_provider_header(name: &str) -> bool {
    let lowered = name.trim().to_ascii_lowercase();
    matches!(
        lowered.as_str(),
        "host"
            | "content-length"
            | "connection"
            | "keep-alive"
            | "proxy-connection"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "forwarded"
            | "x-forwarded"
            | "origin"
            | "referer"
    ) || lowered.starts_with("x-forwarded-")
        || is_proxy_header(&lowered)
}

fn append_header_pair(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    let name = name.trim();
    let value = value.trim();
    if name.is_empty() || value.is_empty() || is_reserved_provider_header(name) {
        return;
    }
    // Validate the shape here; the proxy performs the final HTTP header
    // validation before injection.
    if reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err()
        || reqwest::header::HeaderValue::from_str(value).is_err()
    {
        return;
    }
    headers.push((name.to_string(), value.to_string()));
}

fn append_custom_headers(object: &Map<String, Value>, headers: &mut Vec<(String, String)>) {
    for key in ["headers", "customHeaders", "custom_headers"] {
        let Some(value) = object.get(key) else {
            continue;
        };
        match value {
            Value::Object(entries) => {
                for (name, value) in entries {
                    if let Some(value) = value.as_str() {
                        append_header_pair(headers, name, value);
                    }
                }
            }
            Value::Array(entries) => {
                for entry in entries {
                    let Some(entry) = entry.as_object() else {
                        continue;
                    };
                    let name = entry
                        .get("key")
                        .or_else(|| entry.get("name"))
                        .and_then(Value::as_str);
                    let value = entry.get("value").and_then(Value::as_str);
                    if let (Some(name), Some(value)) = (name, value) {
                        append_header_pair(headers, name, value);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Normalize the one provider endpoint shape accepted from the renderer.
/// Gateway path prefixes are supported, but they cannot contain traversal or
/// encoded separators that would make the prefix check in the loopback proxy
/// ambiguous.
fn normalize_provider_base_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_PROVIDER_IMPORT_URL_BYTES {
        return Err("provider URL is invalid".to_string());
    }
    // `url` correctly canonicalizes special URLs, including literal dot
    // segments.  Reject the raw dangerous spellings first so a compromised
    // renderer cannot turn a configured tenant prefix into a sibling path by
    // relying on that canonicalization.
    let lowered_value = value.to_ascii_lowercase();
    if value.contains('\\')
        || value.contains("/./")
        || value.ends_with("/.")
        || value.contains("/../")
        || value.ends_with("/..")
        || lowered_value.contains("%2e")
        || lowered_value.contains("%2f")
        || lowered_value.contains("%5c")
    {
        return Err("provider URL path contains an unsafe traversal segment".to_string());
    }
    let parsed_url = Url::parse(value).map_err(|_| "provider URL is invalid".to_string())?;
    if !matches!(parsed_url.scheme(), "http" | "https")
        || parsed_url.host_str().is_none()
        || !parsed_url.username().is_empty()
        || parsed_url.password().is_some()
        || parsed_url.query().is_some()
        || parsed_url.fragment().is_some()
    {
        return Err(
            "provider URL must be an absolute http(s) URL without credentials, query, or fragment"
                .to_string(),
        );
    }

    let path = parsed_url.path();
    let lowered_path = path.to_ascii_lowercase();
    if path.starts_with("//")
        || path.contains('\\')
        || path.split('/').any(|segment| matches!(segment, "." | ".."))
        || lowered_path.contains("%2e")
        || lowered_path.contains("%2f")
        || lowered_path.contains("%5c")
    {
        return Err("provider URL path contains an unsafe traversal segment".to_string());
    }
    Ok(parsed_url.to_string())
}

fn normalize_provider_connection_draft(payload: Value) -> Result<Value, String> {
    let mut provider = payload
        .as_object()
        .cloned()
        .ok_or_else(|| "provider connection settings must be an object".to_string())?;

    let provider_id = provider
        .get("id")
        .or_else(|| provider.get("providerId"))
        .and_then(Value::as_str)
        .and_then(valid_provider_id)
        .ok_or_else(|| "provider id is invalid".to_string())?
        .to_string();
    let provider_name = provider
        .get("name")
        .or_else(|| provider.get("label"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.chars().count() <= MAX_PROVIDER_NAME_CHARS)
        .ok_or_else(|| "provider name is invalid".to_string())?
        .to_string();
    let provider_type = provider
        .get("type")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .filter(|value| matches!(value.as_str(), "codex" | "claude_code" | "gemini"))
        .ok_or_else(|| "provider type is invalid".to_string())?;
    let base_url = provider
        .get("baseUrl")
        .or_else(|| provider.get("base_url"))
        .or_else(|| provider.get("endpoint"))
        .or_else(|| provider.get("url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "provider URL is required".to_string())?;
    let base_url = normalize_provider_base_url(base_url)?;

    let models = provider
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| "provider needs at least one model".to_string())?;
    if models.is_empty() || models.len() > MAX_PROVIDER_MODELS_RETURNED {
        return Err("provider model count is invalid".to_string());
    }
    let mut normalized_models = Vec::with_capacity(models.len());
    let mut model_ids = HashSet::with_capacity(models.len());
    for model in models {
        let model_id = match model {
            Value::String(value) => bounded_model_id(value),
            Value::Object(object) => object
                .get("id")
                .or_else(|| object.get("modelId"))
                .or_else(|| object.get("model_id"))
                .and_then(Value::as_str)
                .and_then(bounded_model_id),
            _ => None,
        }
        .ok_or_else(|| "provider model id is invalid".to_string())?;
        if !model_ids.insert(model_id.clone()) {
            return Err("provider model ids must be unique".to_string());
        }
        normalized_models.push(match model {
            Value::Object(object) => {
                let mut normalized = object.clone();
                normalized.insert("id".to_string(), Value::String(model_id));
                Value::Object(normalized)
            }
            _ => Value::String(model_id),
        });
    }

    let request_format = provider
        .get("requestFormat")
        .or_else(|| provider.get("request_format"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("openai-responses")
        .to_string();
    if provider_type == "codex" {
        if !matches!(
            request_format.as_str(),
            "openai-responses" | "openai-completions"
        ) {
            return Err("OpenAI-compatible provider request format is invalid".to_string());
        }
        provider.insert(
            "requestFormat".to_string(),
            Value::String(request_format.clone()),
        );
        provider.insert("protocol".to_string(), Value::String(request_format));
    } else {
        provider.remove("requestFormat");
        provider.remove("request_format");
        provider.insert(
            "protocol".to_string(),
            Value::String(
                if provider_type == "gemini" {
                    "google-generative-ai"
                } else {
                    "anthropic-messages"
                }
                .to_string(),
            ),
        );
    }

    for field in ["useSystemProxy", "promptCachingEnabled", "enabled"] {
        if let Some(value) = provider.get(field) {
            if !value.is_boolean() {
                return Err(format!("provider {field} must be a boolean"));
            }
        }
    }
    for key in [
        "apiKey",
        "api_key",
        "key",
        "token",
        "accessToken",
        "access_token",
        "secret",
        "password",
        "privateKey",
        "private_key",
    ] {
        if let Some(value) = provider.get(key) {
            let value = value
                .as_str()
                .ok_or_else(|| format!("provider {key} must be a string"))?;
            if value.len() > MAX_PROVIDER_API_KEY_BYTES || value.chars().any(char::is_control) {
                return Err(format!("provider {key} is invalid"));
            }
        }
        let marker = format!("{key}Configured");
        if provider
            .get(&marker)
            .is_some_and(|value| !value.is_boolean())
        {
            return Err(format!("provider {marker} must be a boolean"));
        }
    }
    let header_sources = ["customHeaders", "headers", "custom_headers"]
        .iter()
        .filter_map(|key| provider.get(*key).map(|value| (*key, value)))
        .collect::<Vec<_>>();
    if header_sources.len() > 1 {
        return Err("provider custom headers must use one canonical field".to_string());
    }
    let normalized_custom_headers = header_sources
        .first()
        .map(|(_, value)| normalize_provider_connection_custom_headers(value))
        .transpose()?;
    if let Some(custom_headers) = normalized_custom_headers {
        provider.insert("customHeaders".to_string(), Value::Array(custom_headers));
    }
    provider.remove("headers");
    provider.remove("custom_headers");
    provider.insert("id".to_string(), Value::String(provider_id));
    provider.insert("name".to_string(), Value::String(provider_name));
    provider.insert("type".to_string(), Value::String(provider_type));
    provider.insert("baseUrl".to_string(), Value::String(base_url));
    provider.remove("base_url");
    provider.remove("endpoint");
    provider.remove("url");
    provider.insert("models".to_string(), Value::Array(normalized_models));

    Ok(Value::Object(provider))
}

fn normalize_provider_connection_custom_headers(headers: &Value) -> Result<Vec<Value>, String> {
    let headers = headers
        .as_array()
        .ok_or_else(|| "provider customHeaders must be an array".to_string())?;
    if headers.len() > MAX_PROVIDER_CUSTOM_HEADERS {
        return Err("provider customHeaders exceed the configured limit".to_string());
    }
    let mut normalized_headers = Vec::with_capacity(headers.len());
    let mut header_names = HashSet::with_capacity(headers.len());

    for header in headers {
        let object = header
            .as_object()
            .ok_or_else(|| "each provider custom header must be an object".to_string())?;
        let name = object
            .get("key")
            .or_else(|| object.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "each provider custom header needs a name".to_string())?;
        let value = object.get("value").and_then(Value::as_str).map(str::trim);
        let value_configured = object.get("valueConfigured").and_then(Value::as_bool) == Some(true);
        let normalized_name = name.to_ascii_lowercase();

        if name.len() > MAX_PROVIDER_HEADER_NAME_BYTES
            || value.is_some_and(|item| item.len() > MAX_PROVIDER_HEADER_VALUE_BYTES)
            || is_reserved_provider_header(name)
            || normalized_name == "authorization"
            || normalized_name == "proxy-authorization"
            || normalized_name == "cookie"
            || normalized_name == "transfer-encoding"
            || normalized_name == "connection"
            || normalized_name == "content-length"
            || normalized_name == "forwarded"
            || normalized_name.starts_with("x-forwarded-")
        {
            return Err("provider customHeaders contain a reserved header name".to_string());
        }
        if !header_names.insert(normalized_name) {
            return Err("provider customHeaders must not repeat a header name".to_string());
        }
        if reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err()
            || value.is_some_and(|item| reqwest::header::HeaderValue::from_str(item).is_err())
        {
            return Err("provider customHeaders contain an invalid HTTP header".to_string());
        }
        let value = value.filter(|item| !item.is_empty());
        if value.is_none() && !value_configured {
            return Err("each provider custom header needs a value".to_string());
        }
        let mut normalized =
            Map::from_iter([(String::from("key"), Value::String(name.to_string()))]);
        if let Some(value) = value {
            normalized.insert("value".to_string(), Value::String(value.to_string()));
        } else {
            normalized.insert("valueConfigured".to_string(), Value::Bool(true));
        }
        normalized_headers.push(Value::Object(normalized));
    }

    Ok(normalized_headers)
}

fn provider_connection_draft(
    state: &AppState,
    draft_token: &str,
) -> Result<ProviderConnectionDraft, String> {
    let draft_token = draft_token.trim();
    if draft_token.is_empty() {
        return Err("the provider connection draft token is invalid".to_string());
    }

    let mut drafts = state.provider_connection_drafts.lock();
    let now = Instant::now();
    drafts.retain(|_, draft| {
        now.checked_duration_since(draft.created_at)
            .is_some_and(|age| age < PROVIDER_CONNECTION_DRAFT_TTL)
    });

    drafts
        .get(draft_token)
        .cloned()
        .ok_or_else(|| "the provider connection draft expired or is unavailable".to_string())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTestResult {
    pub ok: bool,
    pub status: u16,
    pub latency_ms: u128,
    pub model_count: Option<usize>,
}

/// A narrowly classified model-list protocol. This is intentionally distinct
/// from the broader provider request protocol: an endpoint may support chat
/// generation without exposing a safe public catalogue endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderModelListProtocol {
    OpenAiCompatible,
    Anthropic,
    Gemini,
}

impl ProviderModelListProtocol {
    fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai-compatible",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
        }
    }
}

fn provider_model_list_protocol(protocol: &str) -> Option<ProviderModelListProtocol> {
    let protocol = protocol.trim().to_ascii_lowercase();
    if protocol.contains("anthropic") || protocol.contains("claude") {
        Some(ProviderModelListProtocol::Anthropic)
    } else if protocol.contains("gemini") || protocol.contains("google") {
        Some(ProviderModelListProtocol::Gemini)
    } else if protocol.contains("openai") || protocol.contains("codex") {
        // This includes both OpenAI's native APIs and configured
        // OpenAI-compatible gateways. The latter can still decline /models;
        // that response is represented as `unsupported` below rather than
        // guessing a model name or trying a generation endpoint.
        Some(ProviderModelListProtocol::OpenAiCompatible)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderModelDiscoveryAvailability {
    Available,
    Unsupported,
}

/// The only model fields that may cross the native/WebView boundary. Raw
/// provider responses often include owners, timestamps, descriptions and
/// vendor-specific capability blobs; none are needed to populate Pi's model
/// picker, so they stay native.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelMetadata {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_token: Option<u32>,
}

/// Result of a native model-catalogue request. The command accepts only an
/// already-saved provider ID; URLs, API keys and headers never appear in its
/// IPC input or output.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelsFetchResult {
    pub provider_id: String,
    pub protocol: String,
    pub availability: ProviderModelDiscoveryAvailability,
    pub status: Option<u16>,
    pub latency_ms: Option<u128>,
    pub models: Vec<ProviderModelMetadata>,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
struct ProviderModelProbeConfig {
    provider_id: String,
    protocol: String,
    list_protocol: Option<ProviderModelListProtocol>,
    base_url: String,
    credentials: Vec<(String, String)>,
    use_system_proxy: bool,
}

#[derive(Debug, Clone)]
struct ProviderModelPage {
    models: Vec<ProviderModelMetadata>,
    next_cursor: Option<String>,
    has_more: bool,
}

fn valid_provider_id(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= MAX_PROVIDER_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')))
    .then_some(value)
}

fn provider_default_base_url(protocol: &str) -> &'static str {
    if protocol.contains("anthropic") || protocol.contains("claude") {
        "https://api.anthropic.com"
    } else if protocol.contains("gemini") || protocol.contains("google") {
        "https://generativelanguage.googleapis.com/v1beta"
    } else {
        "https://api.openai.com/v1"
    }
}

fn provider_model_probe_config_from_settings(
    settings: &HashMap<String, Value>,
    provider_id: &str,
) -> Result<ProviderModelProbeConfig, String> {
    let provider_id =
        valid_provider_id(provider_id).ok_or_else(|| "provider id is invalid".to_string())?;
    let object = provider_record(settings, provider_id)
        .ok_or_else(|| "provider is not configured".to_string())?;
    if !provider_is_enabled(object) {
        return Err("provider is disabled".to_string());
    }
    let protocol = provider_protocol(object);
    let list_protocol = provider_model_list_protocol(&protocol);
    let base_url = object
        .get("baseUrl")
        .or_else(|| object.get("base_url"))
        .or_else(|| object.get("endpoint"))
        .or_else(|| object.get("url"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| provider_default_base_url(&protocol).to_string());
    Ok(ProviderModelProbeConfig {
        provider_id: provider_id.to_string(),
        list_protocol,
        protocol,
        base_url,
        // This is the same endpoint/auth-family binding check used by the
        // provider proxy and `provider_test`. A stale saved key is never sent
        // to a newly edited endpoint.
        credentials: list_protocol
            .map(|_| provider_test_credentials_from_object(object))
            .transpose()?
            .unwrap_or_default(),
        use_system_proxy: object
            .get("useSystemProxy")
            .or_else(|| object.get("use_system_proxy"))
            .or_else(|| object.get("systemProxy"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn provider_model_probe_config_from_draft(
    provider: &Value,
) -> Result<ProviderModelProbeConfig, String> {
    let object = provider
        .as_object()
        .ok_or_else(|| "provider connection draft is invalid".to_string())?;
    let provider_id = object
        .get("id")
        .and_then(Value::as_str)
        .and_then(valid_provider_id)
        .ok_or_else(|| "provider connection draft is invalid".to_string())?;
    let protocol = provider_protocol(object);
    let list_protocol = provider_model_list_protocol(&protocol);
    let base_url = object
        .get("baseUrl")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "provider connection draft is invalid".to_string())?
        .to_string();

    Ok(ProviderModelProbeConfig {
        provider_id: provider_id.to_string(),
        list_protocol,
        protocol,
        base_url,
        // The draft is held only in native memory after its preparation
        // command. Its credentials never return to the renderer.
        credentials: list_protocol
            .map(|_| provider_test_credentials_from_object(object))
            .transpose()?
            .unwrap_or_default(),
        use_system_proxy: object
            .get("useSystemProxy")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn bounded_model_id(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= MAX_PROVIDER_MODEL_ID_BYTES
        && !value.chars().any(char::is_control))
    .then(|| value.to_string())
}

fn provider_model_identifier(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| {
        value.as_object().and_then(|model| {
            model
                .get("id")
                .or_else(|| model.get("modelId"))
                .or_else(|| model.get("model_id"))
                .or_else(|| model.get("name"))
                .and_then(Value::as_str)
        })
    })
}

fn provider_default_model(object: &Map<String, Value>) -> Option<&str> {
    [
        "defaultModel",
        "default_model",
        "modelId",
        "model_id",
        "model",
    ]
    .iter()
    .find_map(|key| object.get(*key).and_then(Value::as_str))
    .map(str::trim)
    .filter(|model| bounded_model_id(model).is_some())
}

/// `activeModels` is the persisted model allowlist.  Old records that predate
/// it still use their `models` array, but malformed allowlists fail closed.
fn provider_model_is_enabled(object: &Map<String, Value>, model: &str) -> bool {
    let Some(model) = bounded_model_id(model) else {
        return false;
    };
    if let Some(active_models) = object
        .get("activeModels")
        .or_else(|| object.get("active_models"))
    {
        let Some(active_models) = active_models.as_array() else {
            return false;
        };
        if !active_models
            .iter()
            .filter_map(Value::as_str)
            .any(|active| active.trim() == model)
        {
            return false;
        }
    }

    match object.get("models") {
        Some(models) => {
            let Some(models) = models.as_array() else {
                return false;
            };
            let Some(configured_model) = models.iter().find(|candidate| {
                provider_model_identifier(candidate).is_some_and(|id| id.trim() == model)
            }) else {
                return false;
            };
            configured_model
                .as_object()
                .and_then(|candidate| candidate.get("enabled"))
                .and_then(Value::as_bool)
                != Some(false)
        }
        // A legacy single-model record has no model catalogue to constrain;
        // permit only its explicitly stored default rather than a caller's
        // arbitrary model string.
        None => provider_default_model(object) == Some(model.as_str()),
    }
}

fn provider_default_enabled_model(object: &Map<String, Value>) -> Option<String> {
    if let Some(model) = provider_default_model(object) {
        if provider_model_is_enabled(object, model) {
            return Some(model.to_string());
        }
    }
    object
        .get("models")
        .and_then(Value::as_array)
        .and_then(|models| {
            models.iter().find_map(|model| {
                let id = provider_model_identifier(model)?;
                provider_model_is_enabled(object, id).then(|| id.trim().to_string())
            })
        })
}

fn parse_session_model_selection(value: &str) -> Result<SessionModelSelection, String> {
    if value.len() > MAX_SESSION_MODEL_SELECTION_JSON_BYTES {
        return Err("selected model selection is too large".to_string());
    }
    let selection = serde_json::from_str::<SessionModelSelection>(value.trim())
        .map_err(|error| format!("selected model selection is invalid: {error}"))?;
    let provider_id = valid_provider_id(&selection.provider_id)
        .ok_or_else(|| "selected model provider id is invalid".to_string())?
        .to_string();
    let model_id = bounded_model_id(&selection.model_id)
        .ok_or_else(|| "selected model id is invalid".to_string())?;
    Ok(SessionModelSelection {
        provider_id,
        model_id,
    })
}

fn bounded_model_label(value: &str) -> Option<String> {
    let mut label = String::new();
    for character in value.trim().chars() {
        if character.is_control() {
            continue;
        }
        if label.len().saturating_add(character.len_utf8()) > MAX_PROVIDER_MODEL_LABEL_BYTES {
            break;
        }
        label.push(character);
    }
    let label = label.trim();
    (!label.is_empty()).then(|| label.to_string())
}

/// A misbehaving gateway can reflect a request credential inside an otherwise
/// valid-looking model label. Treat configured header values as sensitive too,
/// so the allowlisted model projection cannot become a secret echo channel.
fn provider_model_secret_values(credentials: &[(String, String)]) -> Vec<String> {
    let mut values = Vec::new();
    for (_, value) in credentials {
        let value = value.trim();
        if value.len() >= 4 {
            values.push(value.to_string());
            if let Some((scheme, token)) = value.split_once(char::is_whitespace) {
                if scheme.eq_ignore_ascii_case("bearer") && token.trim().len() >= 4 {
                    values.push(token.trim().to_string());
                }
            }
        }
    }
    // Replace longer values first when one header value contains another.
    values.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    values.dedup();
    values
}

fn model_metadata_contains_secret(value: &str, secrets: &[String]) -> bool {
    secrets.iter().any(|secret| value.contains(secret))
}

fn redact_model_metadata_label(value: String, secrets: &[String]) -> String {
    secrets.iter().fold(value, |redacted, secret| {
        redacted.replace(secret, "[redacted]")
    })
}

fn bounded_model_limit(object: &Map<String, Value>, keys: &[&str]) -> Option<u32> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
    })
}

fn provider_model_metadata(
    value: &Value,
    protocol: ProviderModelListProtocol,
    secrets: &[String],
) -> Option<ProviderModelMetadata> {
    let object = value.as_object()?;
    let raw_id = match protocol {
        ProviderModelListProtocol::Gemini => object
            .get("baseModelId")
            .or_else(|| object.get("base_model_id"))
            .or_else(|| object.get("name"))
            .and_then(Value::as_str)
            // Gemini list responses use resource names such as
            // `models/gemini-2.0-flash`; Pi expects the bare model ID so it
            // can construct exactly one `/models/` segment itself.
            .and_then(|value| value.strip_prefix("models/").or(Some(value))),
        ProviderModelListProtocol::OpenAiCompatible | ProviderModelListProtocol::Anthropic => {
            object
                .get("id")
                .or_else(|| object.get("model"))
                .and_then(Value::as_str)
        }
    };
    let id = bounded_model_id(raw_id?)?;
    if model_metadata_contains_secret(&id, secrets) {
        // A model ID must remain exactly usable by Pi, so redacting it would
        // make it invalid. Omit it instead of returning a possible key echo.
        return None;
    }
    let label = ["displayName", "display_name", "label", "name"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .and_then(bounded_model_label)
        .map(|label| redact_model_metadata_label(label, secrets))
        .filter(|label| label != &id);
    let context_window = bounded_model_limit(
        object,
        &[
            "inputTokenLimit",
            "input_token_limit",
            "contextWindow",
            "context_window",
        ],
    );
    let max_output_token = bounded_model_limit(
        object,
        &[
            "outputTokenLimit",
            "output_token_limit",
            "maxOutputToken",
            "max_output_token",
            "maxOutputTokens",
            "max_output_tokens",
        ],
    );
    Some(ProviderModelMetadata {
        id,
        label,
        context_window,
        max_output_token,
    })
}

fn bounded_provider_model_cursor(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= MAX_PROVIDER_MODEL_CURSOR_BYTES
        && !value.chars().any(char::is_control))
    .then(|| value.to_string())
}

fn provider_model_page(
    value: &Value,
    protocol: ProviderModelListProtocol,
    secrets: &[String],
) -> Result<ProviderModelPage, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "provider model discovery returned an invalid response".to_string())?;
    let entries = object
        .get("data")
        .or_else(|| object.get("models"))
        .and_then(Value::as_array)
        .ok_or_else(|| "provider model discovery returned an invalid response".to_string())?;
    let models = entries
        .iter()
        .filter_map(|entry| provider_model_metadata(entry, protocol, secrets))
        .collect();
    let (next_cursor, has_more) = match protocol {
        ProviderModelListProtocol::OpenAiCompatible => (None, false),
        ProviderModelListProtocol::Anthropic => {
            let has_more = object
                .get("has_more")
                .or_else(|| object.get("hasMore"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let next_cursor = has_more
                .then(|| {
                    object
                        .get("last_id")
                        .or_else(|| object.get("lastId"))
                        .and_then(Value::as_str)
                        .and_then(bounded_provider_model_cursor)
                })
                .flatten();
            (next_cursor, has_more)
        }
        ProviderModelListProtocol::Gemini => {
            let next_cursor = object
                .get("nextPageToken")
                .or_else(|| object.get("next_page_token"))
                .and_then(Value::as_str)
                .and_then(bounded_provider_model_cursor);
            let has_more = next_cursor.is_some()
                || object
                    .get("nextPageToken")
                    .or_else(|| object.get("next_page_token"))
                    .is_some();
            (next_cursor, has_more)
        }
    };
    Ok(ProviderModelPage {
        models,
        next_cursor,
        has_more,
    })
}

fn provider_models_page_url(
    base_url: &str,
    protocol: &str,
    list_protocol: ProviderModelListProtocol,
    cursor: Option<&str>,
) -> Result<Url, String> {
    let mut url = provider_models_url(base_url, protocol)?;
    match list_protocol {
        ProviderModelListProtocol::OpenAiCompatible => {}
        ProviderModelListProtocol::Anthropic => {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", "100");
            if let Some(cursor) = cursor {
                query.append_pair("after_id", cursor);
            }
        }
        ProviderModelListProtocol::Gemini => {
            let mut query = url.query_pairs_mut();
            query.append_pair("pageSize", "100");
            if let Some(cursor) = cursor {
                query.append_pair("pageToken", cursor);
            }
        }
    }
    Ok(url)
}

fn provider_model_client(use_system_proxy: bool) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20));
    if !use_system_proxy {
        builder = builder.no_proxy();
    }
    builder
        .build()
        .map_err(|_| "provider model discovery could not initialize a request client".to_string())
}

async fn provider_model_response(
    client: &reqwest::Client,
    url: Url,
    credentials: &[(String, String)],
) -> Result<Result<(u16, Value), u16>, String> {
    let mut request = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, "NovaVei/0.1");
    for (name, value) in credentials {
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| "provider model discovery has an invalid credential header".to_string())?;
        let value = reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| "provider model discovery has an invalid credential header".to_string())?;
        request = request.header(name, value);
    }
    let mut response = request
        .send()
        .await
        // Never surface reqwest's text: it can contain proxy/network details
        // that are neither useful to the picker nor part of the IPC contract.
        .map_err(|_| "provider model discovery request failed".to_string())?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Ok(Err(status));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PROVIDER_MODELS_BODY_BYTES as u64)
    {
        return Err("provider model discovery response exceeds the native size limit".to_string());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "provider model discovery response failed".to_string())?
    {
        if body.len().saturating_add(chunk.len()) > MAX_PROVIDER_MODELS_BODY_BYTES {
            return Err(
                "provider model discovery response exceeds the native size limit".to_string(),
            );
        }
        body.extend_from_slice(&chunk);
    }
    let value = serde_json::from_slice(&body)
        .map_err(|_| "provider model discovery returned an invalid response".to_string())?;
    Ok(Ok((status, value)))
}

fn provider_models_url(raw: &str, protocol: &str) -> Result<Url, String> {
    let mut url =
        Url::parse(raw.trim()).map_err(|error| format!("invalid provider URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") || !url.has_host() {
        return Err("provider URL must be an absolute http(s) URL".to_string());
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("provider URL may not contain credentials, query, or fragment".to_string());
    }
    let mut path = url.path().trim_end_matches('/').to_string();
    for suffix in [
        "/chat/completions",
        "/responses",
        "/messages",
        "/generateContent",
        "/streamGenerateContent",
    ] {
        if path.ends_with(suffix) {
            path.truncate(path.len() - suffix.len());
            break;
        }
    }
    if !path.ends_with("/models") {
        if protocol.contains("anthropic") || protocol.contains("claude") {
            if !path.ends_with("/v1") {
                path.push_str("/v1");
            }
        } else if protocol.contains("gemini") || protocol.contains("google") {
            if !path.ends_with("/v1") && !path.ends_with("/v1beta") {
                path.push_str("/v1beta");
            }
        } else if !path.ends_with("/v1") {
            path.push_str("/v1");
        }
        path.push_str("/models");
    }
    url.set_path(&path);
    Ok(url)
}

/// Probe the configured provider without moving its secret into the WebView.
/// The endpoint is deliberately limited to a model-list request, so pressing
/// "测试模型" cannot create a billable completion or execute a tool.
/// Validate an unsaved provider connection and retain it only in native
/// memory. Follow-up diagnostics receive the opaque token rather than a URL,
/// API key, or custom header values from the WebView.
#[tauri::command(rename_all = "camelCase")]
pub fn provider_draft_prepare(
    state: State<'_, Arc<AppState>>,
    provider: Value,
) -> Result<ProviderDraftPrepareResult, String> {
    let provider = normalize_provider_connection_draft(provider)?;
    let draft_token = format!("provider-draft-{}", Uuid::new_v4());
    let now = Instant::now();
    let mut drafts = state.provider_connection_drafts.lock();
    drafts.retain(|_, draft| {
        now.checked_duration_since(draft.created_at)
            .is_some_and(|age| age < PROVIDER_CONNECTION_DRAFT_TTL)
    });
    drafts.insert(
        draft_token.clone(),
        ProviderConnectionDraft {
            provider,
            created_at: now,
        },
    );
    Ok(ProviderDraftPrepareResult { draft_token })
}

/// Non-billable model-catalogue probe for a prepared native connection draft.
/// Credentials stay native-only; this never issues a completion request.
async fn provider_model_catalogue_test(
    config: ProviderModelProbeConfig,
) -> Result<ProviderTestResult, String> {
    let Some(list_protocol) = config.list_protocol else {
        return Err(
            "this provider protocol does not expose a native model catalogue endpoint".to_string(),
        );
    };
    let client = provider_model_client(config.use_system_proxy)?;
    let started = Instant::now();
    let url = provider_models_page_url(&config.base_url, &config.protocol, list_protocol, None)?;
    match provider_model_response(&client, url, &config.credentials).await? {
        Ok((status, value)) => {
            let secrets = provider_model_secret_values(&config.credentials);
            let page = provider_model_page(&value, list_protocol, &secrets)?;
            Ok(ProviderTestResult {
                ok: true,
                status,
                latency_ms: started.elapsed().as_millis(),
                model_count: Some(page.models.len()),
            })
        }
        Err(status) if matches!(status, 404 | 405 | 501) => Ok(ProviderTestResult {
            ok: true,
            status,
            latency_ms: started.elapsed().as_millis(),
            model_count: None,
        }),
        Err(status) => Err(format!("provider model catalogue returned HTTP {status}")),
    }
}

/// Perform the existing non-billable model-catalogue probe against a
/// prepared native draft. This does not send a completion request.
#[tauri::command(rename_all = "camelCase")]
pub async fn provider_draft_test(
    state: State<'_, Arc<AppState>>,
    draft_token: String,
) -> Result<ProviderTestResult, String> {
    let draft = provider_connection_draft(&state, &draft_token)?;
    let config = provider_model_probe_config_from_draft(&draft.provider)?;
    provider_model_catalogue_test(config).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn provider_test(
    state: State<'_, Arc<AppState>>,
    provider_id: String,
    _model: Option<String>,
) -> Result<ProviderTestResult, String> {
    let provider_id = provider_id.trim().to_string();
    if provider_id.is_empty() {
        return Err("provider id is required".to_string());
    }
    let (base_url, protocol, credentials, use_system_proxy) = {
        let settings = state.settings.lock();
        let providers = settings
            .get("providers")
            .cloned()
            .unwrap_or_else(|| json!([]));
        let candidates: Vec<Value> = match providers {
            Value::Array(items) => items,
            Value::Object(object) => object
                .into_iter()
                .filter_map(|(id, value)| match value {
                    Value::Object(mut record) => {
                        record.entry("id".to_string()).or_insert(Value::String(id));
                        Some(Value::Object(record))
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let record = candidates
            .iter()
            .find(|value| {
                value
                    .get("id")
                    .or_else(|| value.get("providerId"))
                    .and_then(Value::as_str)
                    == Some(provider_id.as_str())
            })
            .ok_or_else(|| "provider is not configured".to_string())?;
        let object = record
            .as_object()
            .ok_or_else(|| "provider configuration is invalid".to_string())?;
        if !provider_is_enabled(object) {
            return Err("provider is disabled".to_string());
        }
        let protocol = provider_protocol(object);
        let base_url = object
            .get("baseUrl")
            .or_else(|| object.get("base_url"))
            .or_else(|| object.get("endpoint"))
            .or_else(|| object.get("url"))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| match protocol.as_str() {
                value if value.contains("anthropic") || value.contains("claude") => {
                    "https://api.anthropic.com".to_string()
                }
                value if value.contains("gemini") || value.contains("google") => {
                    "https://generativelanguage.googleapis.com/v1beta".to_string()
                }
                _ => "https://api.openai.com/v1".to_string(),
            });
        let credentials = provider_test_credentials_from_object(object)?;
        let use_system_proxy = object
            .get("useSystemProxy")
            .or_else(|| object.get("use_system_proxy"))
            .or_else(|| object.get("systemProxy"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        (base_url, protocol, credentials, use_system_proxy)
    };
    let url = provider_models_url(&base_url, &protocol)?;
    let mut client_builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20));
    if !use_system_proxy {
        client_builder = client_builder.no_proxy();
    }
    let client = client_builder
        .build()
        .map_err(|error| format!("build provider test client: {error}"))?;
    let mut request = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, "NovaVei/0.1");
    for (name, value) in credentials {
        let header_name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| "provider credential header is invalid".to_string())?;
        let header_value = reqwest::header::HeaderValue::from_str(&value)
            .map_err(|_| "provider credential value is invalid".to_string())?;
        request = request.header(header_name, header_value);
    }
    let started = Instant::now();
    let mut response = request
        .send()
        .await
        .map_err(|error| format!("provider test request failed: {error}"))?;
    let status = response.status().as_u16();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PROVIDER_TEST_BODY_BYTES as u64)
    {
        return Err("provider test response exceeds the native size limit".to_string());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("provider test response failed: {error}"))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_PROVIDER_TEST_BODY_BYTES {
            return Err("provider test response exceeds the native size limit".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    if !(200..300).contains(&status) {
        return Err(format!("provider test returned HTTP {status}"));
    }
    let model_count = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("data")
                .or_else(|| value.get("models"))
                .and_then(Value::as_array)
                .map(Vec::len)
        });
    Ok(ProviderTestResult {
        ok: true,
        status,
        latency_ms: started.elapsed().as_millis(),
        model_count,
    })
}

async fn provider_models_fetch_with_config(
    config: ProviderModelProbeConfig,
) -> Result<ProviderModelsFetchResult, String> {
    let Some(list_protocol) = config.list_protocol else {
        // Do not turn an unknown chat protocol into a speculative HTTP
        // request. This is the explicit manual-entry path for providers which
        // do not publish a compatible model-list API (for example, a local
        // protocol with a different discovery endpoint).
        return Ok(ProviderModelsFetchResult {
            provider_id: config.provider_id,
            // Do not reflect an arbitrary legacy `protocol` setting back to
            // the renderer when it has no supported discovery contract.
            protocol: "unsupported".to_string(),
            availability: ProviderModelDiscoveryAvailability::Unsupported,
            status: None,
            latency_ms: None,
            models: Vec::new(),
            truncated: false,
        });
    };
    let client = provider_model_client(config.use_system_proxy)?;
    let model_secrets = provider_model_secret_values(&config.credentials);
    let started = Instant::now();
    let mut models = Vec::new();
    let mut seen_models = HashSet::new();
    let mut seen_cursors = HashSet::new();
    let mut cursor: Option<String> = None;
    let mut status = None;
    let mut truncated = false;

    for page_index in 0..MAX_PROVIDER_MODEL_PAGES {
        let url = provider_models_page_url(
            &config.base_url,
            &config.protocol,
            list_protocol,
            cursor.as_deref(),
        )?;
        let page = match provider_model_response(&client, url, &config.credentials).await? {
            Ok((page_status, value)) => {
                status = Some(page_status);
                provider_model_page(&value, list_protocol, &model_secrets)?
            }
            Err(page_status) if page_index == 0 && matches!(page_status, 404 | 405 | 501) => {
                // These are the normal signatures of an otherwise usable
                // provider that simply has no public catalogue endpoint.
                return Ok(ProviderModelsFetchResult {
                    provider_id: config.provider_id,
                    protocol: list_protocol.as_str().to_string(),
                    availability: ProviderModelDiscoveryAvailability::Unsupported,
                    status: Some(page_status),
                    latency_ms: Some(started.elapsed().as_millis()),
                    models: Vec::new(),
                    truncated: false,
                });
            }
            Err(page_status) => {
                return Err(format!(
                    "provider model discovery returned HTTP {page_status}"
                ));
            }
        };

        let mut skipped_for_limit = false;
        for model in page.models {
            if !seen_models.insert(model.id.clone()) {
                continue;
            }
            if models.len() >= MAX_PROVIDER_MODELS_RETURNED {
                skipped_for_limit = true;
                break;
            }
            models.push(model);
        }
        if skipped_for_limit {
            truncated = true;
            break;
        }
        let Some(next_cursor) = page.next_cursor else {
            // A malformed pagination response that says `has_more` but does
            // not give a usable cursor is still a valid partial discovery; do
            // not guess or send any extra request.
            truncated |= page.has_more;
            break;
        };
        if page_index + 1 >= MAX_PROVIDER_MODEL_PAGES
            || models.len() >= MAX_PROVIDER_MODELS_RETURNED
            || !seen_cursors.insert(next_cursor.clone())
        {
            truncated = true;
            break;
        }
        cursor = Some(next_cursor);
    }

    Ok(ProviderModelsFetchResult {
        provider_id: config.provider_id,
        protocol: list_protocol.as_str().to_string(),
        availability: ProviderModelDiscoveryAvailability::Available,
        status,
        latency_ms: Some(started.elapsed().as_millis()),
        models,
        truncated,
    })
}

/// Discover models for one already-saved provider. The renderer supplies only
/// `providerId`; native settings provide the endpoint, credentials, custom
/// headers and proxy choice. No fetched result mutates settings automatically:
/// the UI may explicitly choose which bounded model metadata to retain.
#[tauri::command(rename_all = "camelCase")]
pub async fn provider_models_fetch(
    state: State<'_, Arc<AppState>>,
    provider_id: String,
) -> Result<ProviderModelsFetchResult, String> {
    let config = {
        let settings = state.settings.lock();
        provider_model_probe_config_from_settings(&settings, &provider_id)?
    };
    provider_models_fetch_with_config(config).await
}

/// Discover models for a prepared native connection draft. The renderer may
/// supply only the opaque draft token; endpoint, credentials, and headers stay
/// native-only for the duration of the short-lived draft.
#[tauri::command(rename_all = "camelCase")]
pub async fn provider_draft_models_fetch(
    state: State<'_, Arc<AppState>>,
    draft_token: String,
) -> Result<ProviderModelsFetchResult, String> {
    let draft = provider_connection_draft(&state, &draft_token)?;
    let config = provider_model_probe_config_from_draft(&draft.provider)?;
    provider_models_fetch_with_config(config).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsLoadResponse {
    pub providers: Value,
    pub system: Value,
    pub projects: Value,
    pub mcp: Value,
    pub agents: Value,
    pub ssh: Value,
    pub remote: Value,
    pub memory: Value,
    pub default_workdir: String,
}

fn redact_settings_secrets(value: &Value) -> Value {
    redact_settings_value(value, false, false)
}

fn event_secret_key(key: &str) -> bool {
    let normalized = key.replace(['-', '_', '.'], "").to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "apikey"
            | "xapikey"
            | "xgoogapikey"
            | "authorization"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "capabilitytoken"
            | "proxytoken"
            | "secret"
            | "clientsecret"
            | "password"
            | "privatekey"
    )
}

fn redact_event_value(value: &Value, in_headers: bool) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_event_value(item, in_headers))
                .collect(),
        ),
        Value::Object(object) => {
            let header_entry = in_headers
                && (object.contains_key("key")
                    || object.contains_key("name")
                    || object.contains_key("value"));
            let mut output = Map::new();
            for (key, item) in object {
                let normalized = key.replace(['-', '_', '.'], "").to_ascii_lowercase();
                let header_value = in_headers
                    && ((header_entry && normalized == "value")
                        || (!header_entry && item.is_string()));
                if event_secret_key(key) || header_value {
                    output.insert(key.clone(), Value::String("[redacted]".to_string()));
                } else {
                    let child_headers = matches!(normalized.as_str(), "headers" | "customheaders");
                    output.insert(key.clone(), redact_event_value(item, child_headers));
                }
            }
            Value::Object(output)
        }
        other => other.clone(),
    }
}

fn redact_settings_value(value: &Value, in_headers: bool, in_secret_map: bool) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_settings_value(item, in_headers, in_secret_map))
                .collect(),
        ),
        Value::Object(object) => {
            let mut result = Map::new();
            let header_entry = in_headers
                && (object.contains_key("key") || object.contains_key("name"))
                && object.contains_key("value");
            for (key, item) in object {
                if key == PROVIDER_CREDENTIAL_BINDING_KEY {
                    // This is native bookkeeping, not renderer settings. It
                    // must never become an input-controlled credential hint.
                    continue;
                }
                let lowered = key.to_ascii_lowercase();
                if in_secret_map && item.is_string() {
                    let configured = item.as_str().is_some_and(|text| !text.trim().is_empty());
                    result.insert(
                        key.clone(),
                        if configured {
                            Value::String("[configured]".to_string())
                        } else {
                            Value::Null
                        },
                    );
                    continue;
                }
                if in_headers && !header_entry {
                    let configured = item.as_str().is_some_and(|text| !text.trim().is_empty());
                    result.insert(
                        key.clone(),
                        if configured {
                            Value::String("[configured]".to_string())
                        } else {
                            Value::Null
                        },
                    );
                    continue;
                }
                if in_headers
                    && matches!(
                        lowered.as_str(),
                        "value" | "authorization" | "x-api-key" | "x-goog-api-key"
                    )
                {
                    let configured = item.as_str().is_some_and(|text| !text.trim().is_empty());
                    result.insert(format!("{key}Configured"), Value::Bool(configured));
                    continue;
                }
                if in_headers && matches!(lowered.as_str(), "key" | "name") {
                    // Header names are not secrets.  The old generic redactor
                    // accidentally converted `{ key, value }` array entries
                    // into `{ keyConfigured }`, making them impossible to
                    // round-trip through settings.
                    result.insert(key.clone(), item.clone());
                    continue;
                }
                if secret_setting_key(&lowered) {
                    let configured = item.as_str().is_some_and(|text| !text.trim().is_empty());
                    result.insert(format!("{key}Configured"), Value::Bool(configured));
                    continue;
                }
                let child_in_headers = matches!(
                    lowered.as_str(),
                    "headers" | "customheaders" | "custom_headers"
                );
                let normalized = lowered.replace(['-', '_', '.'], "");
                let child_in_secret_map = in_secret_map
                    || matches!(
                        normalized.as_str(),
                        "env" | "environment" | "environmentvariables" | "environmentvars"
                    );
                result.insert(
                    key.clone(),
                    redact_settings_value(item, child_in_headers, child_in_secret_map),
                );
            }
            Value::Object(result)
        }
        other => other.clone(),
    }
}

fn default_setting(scope: &str, default_workdir: &str) -> Value {
    match scope {
        "providers" => json!([]),
        "system" => json!({
            "workdir": default_workdir,
            "theme": "dark",
            "showShortcutHints": true,
            "workdirPolicy": WORKDIR_POLICY_PROJECT,
            "defaultPermissionTier": "ask",
            "globalSystemPrompt": "",
            "security": {
                "requirePlanForMutableTools": true,
                "allowSubagentGlobalRead": false
            },
        }),
        "projects" => json!({
            "version": PROJECT_SETTINGS_VERSION,
            "initialized": false,
            "entries": []
        }),
        "mcp" => json!([]),
        "agents" => json!([]),
        "ssh" => json!([]),
        "remote" => json!({"enabled": false}),
        "memory" => json!({"enabled": true}),
        _ => Value::Null,
    }
}

fn validate_workdir_policy_value(value: Option<&Value>) -> Result<(), String> {
    let Some(value) = value else {
        // Older settings did not persist a policy. Their existing capability
        // behavior was exact project-root access, so retain that safe default.
        return Ok(());
    };
    let policy = value
        .as_str()
        .map(str::trim)
        .ok_or_else(|| "workdir policy must be a string".to_string())?;
    match policy {
        WORKDIR_POLICY_PROJECT => Ok(()),
        WORKDIR_POLICY_EXTRA => Err(
            "workdir policy \"extra\" is not supported: workspace access remains limited to the exact session project root"
                .to_string(),
        ),
        _ => Err("workdir policy is invalid; only \"project\" is supported".to_string()),
    }
}

fn validate_system_workdir_policy(system: Option<&Value>) -> Result<(), String> {
    let Some(system) = system else {
        return Ok(());
    };
    let object = system
        .as_object()
        .ok_or_else(|| "system settings are invalid for workspace access".to_string())?;
    let camel = object.get("workdirPolicy");
    let snake = object.get("workdir_policy");
    if let (Some(camel), Some(snake)) = (camel, snake) {
        if camel != snake {
            return Err("system workdir policy keys disagree".to_string());
        }
    }
    validate_workdir_policy_value(camel.or(snake))
}

fn require_project_root_workdir_policy(state: &AppState) -> Result<(), String> {
    let settings = state.settings.lock();
    validate_system_workdir_policy(settings.get("system"))
}

fn normalize_system_security_settings(
    security: Option<Value>,
    legacy_require_plan: Option<Value>,
    legacy_global_read: Option<Value>,
) -> Result<Value, String> {
    let mut require_plan = true;
    let mut allow_global_read = false;
    if let Some(security) = security {
        let object = security
            .as_object()
            .ok_or_else(|| "system security settings must be an object".to_string())?;
        if object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "requirePlanForMutableTools"
                    | "require_plan_for_mutable_tools"
                    | "allowSubagentGlobalRead"
                    | "allow_subagent_global_read"
            )
        }) {
            return Err("system security settings contain an unsupported field".to_string());
        }
        let camel = object.get("requirePlanForMutableTools");
        let snake = object.get("require_plan_for_mutable_tools");
        if let (Some(camel), Some(snake)) = (camel, snake) {
            if camel != snake {
                return Err("system security plan keys disagree".to_string());
            }
        }
        if let Some(value) = camel.or(snake) {
            require_plan = value
                .as_bool()
                .ok_or_else(|| "system security plan setting must be boolean".to_string())?;
        }
        let camel = object.get("allowSubagentGlobalRead");
        let snake = object.get("allow_subagent_global_read");
        if let (Some(camel), Some(snake)) = (camel, snake) {
            if camel != snake {
                return Err("system security global-read keys disagree".to_string());
            }
        }
        if let Some(value) = camel.or(snake) {
            allow_global_read = value
                .as_bool()
                .ok_or_else(|| "system security global-read setting must be boolean".to_string())?;
        }
    }
    if let Some(value) = legacy_require_plan {
        require_plan = value
            .as_bool()
            .ok_or_else(|| "system security plan setting must be boolean".to_string())?;
    }
    if let Some(value) = legacy_global_read {
        allow_global_read = value
            .as_bool()
            .ok_or_else(|| "system security global-read setting must be boolean".to_string())?;
    }
    Ok(json!({
        "requirePlanForMutableTools": require_plan,
        "allowSubagentGlobalRead": allow_global_read,
    }))
}

/// Canonicalize the small system-policy DTO before persisting it.  This is
/// deliberately separate from capability issuance so a renderer cannot save
/// an unsupported setting now and attempt to reinterpret it later.
fn normalize_system_settings_payload(mut payload: Value) -> Result<Value, String> {
    let object = payload
        .as_object_mut()
        .ok_or_else(|| "system settings payload must be an object".to_string())?;
    let global_system_prompt = object.remove("globalSystemPrompt");
    let legacy_global_system_prompt = object.remove("global_system_prompt");
    if let (Some(camel), Some(snake)) = (&global_system_prompt, &legacy_global_system_prompt) {
        if camel != snake {
            return Err("global system prompt keys disagree".to_string());
        }
    }
    match global_system_prompt.or(legacy_global_system_prompt) {
        None => {}
        Some(Value::String(prompt)) if prompt.chars().count() <= MAX_GLOBAL_SYSTEM_PROMPT_CHARS => {
            object.insert("globalSystemPrompt".to_string(), Value::String(prompt));
        }
        Some(Value::String(_)) => {
            return Err("global system prompt exceeds 32000 characters".to_string());
        }
        Some(_) => return Err("global system prompt must be a string".to_string()),
    }
    let camel = object.remove("workdirPolicy");
    let snake = object.remove("workdir_policy");
    if let (Some(camel), Some(snake)) = (&camel, &snake) {
        if camel != snake {
            return Err("system workdir policy keys disagree".to_string());
        }
    }
    validate_workdir_policy_value(camel.as_ref().or(snake.as_ref()))?;
    object.insert(
        "workdirPolicy".to_string(),
        Value::String(WORKDIR_POLICY_PROJECT.to_string()),
    );
    let camel_permission = object.remove("defaultPermissionTier");
    let snake_permission = object.remove("default_permission_tier");
    if let (Some(camel), Some(snake)) = (&camel_permission, &snake_permission) {
        if camel != snake {
            return Err("system permission tier keys disagree".to_string());
        }
    }
    let permission = camel_permission.or(snake_permission);
    // Legacy "auto-approve" values fall back to "ask": that tier was removed
    // from the product surface and must never be reintroduced by settings
    // normalization.
    let permission = match permission.as_ref().and_then(Value::as_str).map(str::trim) {
        None | Some("") | Some("ask") | Some("auto-approve") | Some("auto") | Some("auto_approve") => {
            "ask"
        }
        Some("readonly") | Some("read_only") => "readonly",
        Some("full") => "full",
        Some(_) => return Err("default permission tier is invalid".to_string()),
    };
    object.insert(
        "defaultPermissionTier".to_string(),
        Value::String(permission.to_string()),
    );
    let security = normalize_system_security_settings(
        object.remove("security"),
        object
            .remove("requirePlanForMutableTools")
            .or_else(|| object.remove("require_plan_for_mutable_tools")),
        object
            .remove("allowSubagentGlobalRead")
            .or_else(|| object.remove("allow_subagent_global_read")),
    )?;
    object.insert("security".to_string(), security);
    let camel_launch_behavior = object.remove("secondaryLaunchBehavior");
    let snake_launch_behavior = object.remove("secondary_launch_behavior");
    if let (Some(camel), Some(snake)) = (&camel_launch_behavior, &snake_launch_behavior) {
        if camel != snake {
            return Err("secondary launch behavior keys disagree".to_string());
        }
    }
    let launch_behavior = match camel_launch_behavior
        .or(snake_launch_behavior)
        .as_ref()
        .and_then(Value::as_str)
        .map(str::trim)
    {
        None | Some("") | Some(SECONDARY_LAUNCH_FOCUS_EXISTING) => SECONDARY_LAUNCH_FOCUS_EXISTING,
        Some(SECONDARY_LAUNCH_NEW_WINDOW) => SECONDARY_LAUNCH_NEW_WINDOW,
        Some(_) => return Err("secondary launch behavior is invalid".to_string()),
    };
    object.insert(
        "secondaryLaunchBehavior".to_string(),
        Value::String(launch_behavior.to_string()),
    );
    Ok(payload)
}

/// Whether an operating-system request to start NovaVei again should open a
/// second top-level window in this already-running process. The process keeps
/// the single database and protected settings owner; only the WebView profile
/// is distinct per window.
pub fn secondary_launch_opens_new_window(state: &AppState) -> bool {
    state
        .settings
        .lock()
        .get("system")
        .and_then(|value| value.get("secondaryLaunchBehavior"))
        .and_then(Value::as_str)
        .is_some_and(|value| value == SECONDARY_LAUNCH_NEW_WINDOW)
}

fn valid_project_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
}

fn new_stable_project_id() -> String {
    format!("{PROJECT_STABLE_ID_PREFIX}{}", Uuid::new_v4())
}

/// Accept the persisted UUID form only. Older path-hash IDs remain readable
/// through `valid_project_id` and are upgraded during the next successful
/// projects-settings save.
fn canonical_stable_project_id(value: &str) -> Option<String> {
    let value = value.trim();
    let uuid = value.strip_prefix(PROJECT_STABLE_ID_PREFIX)?;
    let uuid = Uuid::parse_str(uuid).ok()?;
    Some(format!("{PROJECT_STABLE_ID_PREFIX}{}", uuid.hyphenated()))
}

fn bounded_project_text(value: &str, max: usize) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.chars().count() <= max && !trimmed.chars().any(char::is_control)
}

/// Keep project preferences a closed, credential-free DTO.  These settings
/// are defaults for a project, not a second provider configuration channel.
/// Full access never appears here: it is a native-memory-only, one-use run
/// grant rather than a durable project preference.
fn normalize_project_permission_tier(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "readonly" | "read_only" | "read-only" => Some("readonly"),
        "ask" => Some("ask"),
        _ => None,
    }
}

fn normalize_project_preferences(entry: &Map<String, Value>) -> Result<Option<Value>, String> {
    let Some(preferences) = entry.get("preferences") else {
        return Ok(None);
    };
    let preferences = preferences
        .as_object()
        .ok_or_else(|| "project preferences must be an object".to_string())?;
    if preferences
        .keys()
        .any(|key| !matches!(key.as_str(), "model" | "reasoning" | "permission"))
    {
        return Err("project preferences contain an unsupported field".to_string());
    }

    let model = match preferences.get("model") {
        None => None,
        Some(value) => {
            let model = value
                .as_object()
                .ok_or_else(|| "project model preference must be an object".to_string())?;
            if model
                .keys()
                .any(|key| !matches!(key.as_str(), "providerId" | "modelId"))
            {
                return Err("project model preference contains an unsupported field".to_string());
            }
            let provider_id = model
                .get("providerId")
                .and_then(Value::as_str)
                .and_then(valid_provider_id)
                .ok_or_else(|| "project model preference provider id is invalid".to_string())?;
            let model_id = model
                .get("modelId")
                .and_then(Value::as_str)
                .and_then(bounded_model_id)
                .ok_or_else(|| "project model preference model id is invalid".to_string())?;
            Some(json!({"providerId": provider_id, "modelId": model_id}))
        }
    };

    let reasoning = match preferences.get("reasoning") {
        None => None,
        Some(Value::String(value)) => Some(
            validate_turn_reasoning(Some(value.clone()))?
                .ok_or_else(|| "project reasoning preference is invalid".to_string())?,
        ),
        Some(_) => return Err("project reasoning preference must be a string".to_string()),
    };

    let permission = match preferences.get("permission") {
        None => None,
        Some(Value::String(value)) => normalize_project_permission_tier(value)
            .map(str::to_string)
            .ok_or_else(|| "project permission preference is invalid".to_string())
            .map(Some)?,
        Some(_) => return Err("project permission preference must be a string".to_string()),
    };

    if model.is_none() && reasoning.is_none() && permission.is_none() {
        return Ok(None);
    }
    let mut normalized = Map::new();
    if let Some(model) = model {
        normalized.insert("model".to_string(), model);
    }
    if let Some(reasoning) = reasoning {
        normalized.insert("reasoning".to_string(), Value::String(reasoning));
    }
    if let Some(permission) = permission {
        normalized.insert("permission".to_string(), Value::String(permission));
    }
    Ok(Some(Value::Object(normalized)))
}

/// Return a stable display path for history/project metadata without requiring
/// the directory to be mounted. Unlike `canonical_workdir`, this is not a
/// capability grant and must never be used to access the filesystem.
fn historical_workspace_path_display(raw: &str) -> Result<String, String> {
    let display = path_string_for_display(raw);
    let value = display.trim();
    if value.is_empty()
        || value.chars().count() > MAX_WORKSPACE_PATH_CHARS
        || value.chars().any(char::is_control)
    {
        return Err("workspace path is invalid".to_string());
    }
    if !Path::new(value).is_absolute() {
        return Err("workspace path must be absolute".to_string());
    }
    Ok(value.to_string())
}

/// A comparison-only path key that still works after a directory was moved,
/// deleted, or a removable/network drive is disconnected. Existing paths are
/// always canonicalized before access; this key is only for binding durable
/// metadata to that historical string.
fn workspace_path_key(raw: &str) -> Option<String> {
    let display = historical_workspace_path_display(raw).ok()?;
    #[cfg(windows)]
    {
        Some(normalize_windows_workspace_path_key(&display))
    }
    #[cfg(not(windows))]
    {
        Some(normalize_non_windows_workspace_path_key(&display))
    }
}

/// Windows path comparison is case-insensitive. Normalize slash variants and
/// redundant separators without touching the filesystem, then remove trailing
/// separators except for a drive root (`C:\\`). This makes durable history
/// grouping stable after a path becomes unavailable as well.
#[cfg(windows)]
fn normalize_windows_workspace_path_key(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut previous_separator = false;
    for character in value.chars() {
        if matches!(character, '\\' | '/') {
            // Preserve the two leading separators of a UNC path, but collapse
            // every other separator run to one logical Windows separator.
            if !previous_separator || normalized == "\\" {
                normalized.push('\\');
            }
            previous_separator = true;
        } else {
            normalized.push(character);
            previous_separator = false;
        }
    }

    while normalized.ends_with('\\') && !windows_path_key_is_drive_root(&normalized) {
        normalized.pop();
    }
    normalized.to_ascii_lowercase()
}

#[cfg(windows)]
fn windows_path_key_is_drive_root(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\'
}

#[cfg(not(windows))]
fn normalize_non_windows_workspace_path_key(value: &str) -> String {
    let mut normalized = value.to_string();
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    normalized
}

#[derive(Debug, Clone)]
struct ProjectMetadata {
    id: String,
    name: String,
    path: String,
    last_session_id: Option<String>,
    pinned: bool,
}

fn project_metadata_from_value(value: Option<&Value>) -> Vec<ProjectMetadata> {
    value
        .and_then(Value::as_object)
        .and_then(|projects| projects.get("entries"))
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let entry = entry.as_object()?;
                    let id = entry.get("id")?.as_str()?.trim();
                    let name = entry.get("name")?.as_str()?.trim();
                    let path = entry.get("path")?.as_str()?.trim();
                    if !valid_project_id(id)
                        || !bounded_project_text(name, 200)
                        || workspace_path_key(path).is_none()
                    {
                        return None;
                    }
                    Some(ProjectMetadata {
                        id: id.to_string(),
                        name: name.to_string(),
                        path: path_string_for_display(path),
                        last_session_id: entry
                            .get("lastSessionId")
                            .or_else(|| entry.get("last_session_id"))
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string),
                        pinned: entry
                            .get("pinned")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Startup may restore only roots represented by a durable project record.
/// Sessions intentionally do not participate: their cwd is historical data
/// that can remain useful after a folder is moved, deleted, or recreated.
fn approved_project_workdirs(settings: &HashMap<String, Value>) -> HashSet<PathBuf> {
    approved_project_workdirs_from_value(settings.get("projects"))
}

fn approved_project_workdirs_from_value(value: Option<&Value>) -> HashSet<PathBuf> {
    project_metadata_from_value(value)
        .into_iter()
        .filter_map(|project| canonical_workdir(&project.path).ok())
        .collect()
}

fn project_path_keys_from_value(value: Option<&Value>) -> HashSet<String> {
    project_metadata_from_value(value)
        .into_iter()
        .filter_map(|project| workspace_path_key(&project.path))
        .collect()
}

/// Stable project identifiers may only move through the relocation command,
/// which also rewrites every affected session and revokes stale grants in one
/// transaction. The generic settings endpoint is intentionally limited to
/// metadata edits, additions, and removals; accepting an ID/path reassignment
/// here would strand history under the old cwd while presenting the project as
/// if it had already moved.
fn reject_direct_stable_project_path_changes(
    previous: Option<&Value>,
    next: &Value,
) -> Result<(), String> {
    let previous_paths = project_metadata_from_value(previous)
        .into_iter()
        .filter_map(|project| {
            let id = canonical_stable_project_id(&project.id)?;
            let path = workspace_path_key(&project.path)?;
            Some((id, path))
        })
        .collect::<HashMap<_, _>>();
    if previous_paths.is_empty() {
        return Ok(());
    }
    for project in project_metadata_from_value(Some(next)) {
        let Some(id) = canonical_stable_project_id(&project.id) else {
            continue;
        };
        let Some(previous_path) = previous_paths.get(&id) else {
            continue;
        };
        let Some(next_path) = workspace_path_key(&project.path) else {
            continue;
        };
        if previous_path != &next_path {
            return Err("project paths can only be changed with workspace relocation".to_string());
        }
    }
    Ok(())
}

/// Apply the same invariant to legacy path-hash identifiers before the
/// normalizer upgrades them to UUIDs. Without this pre-normalization check, a
/// final legacy settings write could change its path while receiving a fresh
/// stable ID and evade the retained-ID comparison above.
fn reject_direct_legacy_project_path_changes(
    previous: Option<&Value>,
    incoming: &Value,
) -> Result<(), String> {
    let previous_paths = project_metadata_from_value(previous)
        .into_iter()
        .map(|project| {
            (
                project.id.to_ascii_lowercase(),
                workspace_path_key(&project.path),
            )
        })
        .filter_map(|(id, path)| path.map(|path| (id, path)))
        .collect::<HashMap<_, _>>();
    for project in project_metadata_from_value(Some(incoming)) {
        let Some(previous_path) = previous_paths.get(&project.id.to_ascii_lowercase()) else {
            continue;
        };
        if workspace_path_key(&project.path).as_ref() != Some(previous_path) {
            return Err("project paths can only be changed with workspace relocation".to_string());
        }
    }
    Ok(())
}

/// A picker selection is permission to register one root, not a durable
/// project by itself. File, shell, attachment, branch, and new-session entry
/// points all call this helper so historical cwd metadata and stale picker
/// roots remain read-only until a project record has been persisted.
fn workdir_is_registered_project(state: &AppState, workdir: &Path) -> bool {
    let Some(key) = workspace_path_key(&path_for_display(workdir)) else {
        return false;
    };
    project_path_keys_from_value(state.settings.lock().get("projects")).contains(&key)
}

fn require_registered_project_workdir(state: &AppState, workdir: &Path) -> Result<(), String> {
    if workdir_is_registered_project(state, workdir) {
        Ok(())
    } else {
        Err("workspace is not a registered project; historical sessions are read-only".to_string())
    }
}

/// Rebuild the process-local picker/settings allowlist after a durable project
/// mutation. Picker roots are consumed once they become durable; removing a
/// project therefore cannot inherit an old picker approval indefinitely.
fn refresh_approved_workdirs_after_project_save(state: &AppState) {
    let durable = approved_project_workdirs(&state.settings.lock());
    let pending_picker_roots = {
        let mut picker_roots = state.picker_workdirs.lock();
        picker_roots.retain(|path| !durable.contains(path));
        picker_roots.clone()
    };
    let mut approved = durable;
    approved.extend(pending_picker_roots);
    if let Ok(current) = canonical_workdir(&current_workdir()) {
        approved.insert(current);
    }
    *state.approved_workdirs.lock() = approved;
}

fn normalize_project_path_for_settings(
    raw_path: &str,
    approved: &HashSet<PathBuf>,
    known_historical_paths: &HashSet<String>,
) -> Result<(String, String), String> {
    let display = historical_workspace_path_display(raw_path)?;
    if let Ok(canonical) = canonical_workdir(&display) {
        if !approved.contains(&canonical) {
            return Err(
                "project path has not been approved with the native folder picker".to_string(),
            );
        }
        let display = path_for_display(&canonical);
        let key = workspace_path_key(&display)
            .ok_or_else(|| "project path is invalid after canonicalization".to_string())?;
        return Ok((display, key));
    }

    let key = workspace_path_key(&display).ok_or_else(|| "project path is invalid".to_string())?;
    if !known_historical_paths.contains(&key) {
        return Err(
            "project path is unavailable and is not known to local history or existing projects"
                .to_string(),
        );
    }
    Ok((display, key))
}

fn normalize_projects_settings_payload(state: &AppState, payload: Value) -> Result<Value, String> {
    normalize_projects_settings_payload_with_extra_approved(state, payload, None)
}

/// Normalize the durable projects DTO against existing picker/project grants.
/// `workspace_register_project` may pass one additional canonical root after
/// proving it is already present in local history; that explicit user action
/// is the only non-picker route that can promote historical metadata into a
/// durable project.
fn normalize_projects_settings_payload_with_extra_approved(
    state: &AppState,
    payload: Value,
    extra_approved: Option<&Path>,
) -> Result<Value, String> {
    let object = payload
        .as_object()
        .ok_or_else(|| "projects settings payload must be an object".to_string())?;
    let initialized = object
        .get("initialized")
        .and_then(Value::as_bool)
        .ok_or_else(|| "projects initialized flag must be a boolean".to_string())?;
    let entries = object
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| "projects entries must be an array".to_string())?;
    if entries.len() > 256 {
        return Err("projects settings can contain at most 256 entries".to_string());
    }

    let mut approved = state.approved_workdirs.lock().clone();
    if let Some(extra_approved) = extra_approved {
        approved.insert(extra_approved.to_path_buf());
    }
    let session_workdirs = state
        .sessions
        .lock()
        .iter()
        .map(|(id, session)| (id.clone(), session.summary.cwd.clone()))
        .collect::<HashMap<_, _>>();
    let existing_project_paths = project_metadata_from_value(state.settings.lock().get("projects"));
    let mut known_historical_paths = session_workdirs
        .values()
        .filter_map(|path| workspace_path_key(path))
        .collect::<HashSet<_>>();
    known_historical_paths.extend(
        existing_project_paths
            .iter()
            .filter_map(|project| workspace_path_key(&project.path)),
    );
    let mut seen = HashSet::new();
    let mut seen_ids = HashSet::new();
    let mut normalized = Vec::with_capacity(entries.len());
    for entry in entries {
        let entry = entry
            .as_object()
            .ok_or_else(|| "project entry must be an object".to_string())?;
        let supplied_id = entry
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| valid_project_id(value))
            .ok_or_else(|| "project id is invalid".to_string())?;
        // Path-hash IDs were used by older renderer builds.  Keep accepting
        // them at this migration boundary, but replace them with a native
        // UUID before they become durable again.  The UUID is intentionally
        // independent from the path so a later relocate keeps its identity.
        let id = canonical_stable_project_id(supplied_id).unwrap_or_else(new_stable_project_id);
        if !seen_ids.insert(id.to_ascii_lowercase()) {
            return Err("project ids must be unique".to_string());
        }
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| bounded_project_text(value, 200))
            .ok_or_else(|| "project name is invalid".to_string())?;
        let raw_path = entry
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| "project path is required".to_string())?;
        let (path, path_key) =
            normalize_project_path_for_settings(raw_path, &approved, &known_historical_paths)?;
        if !seen.insert(path_key.clone()) {
            return Err("project paths must be unique".to_string());
        }
        let last_session_id = entry
            .get("lastSessionId")
            .or_else(|| entry.get("last_session_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(session_id) = last_session_id {
            if !bounded_project_text(session_id, 200) {
                return Err("project last session id is invalid".to_string());
            }
            if let Some(session_workdir) = session_workdirs.get(session_id) {
                let session_path = workspace_path_key(session_workdir)
                    .ok_or_else(|| "project last session workspace is invalid".to_string())?;
                if session_path != path_key {
                    return Err("project last session belongs to another workdir".to_string());
                }
            }
        }
        let preferences = normalize_project_preferences(entry)?;
        let mut normalized_entry = Map::new();
        normalized_entry.insert("id".to_string(), Value::String(id));
        normalized_entry.insert("name".to_string(), Value::String(name.to_string()));
        normalized_entry.insert("path".to_string(), Value::String(path));
        normalized_entry.insert(
            "lastSessionId".to_string(),
            last_session_id
                .map(str::to_string)
                .map(Value::String)
                .unwrap_or(Value::Null),
        );
        normalized_entry.insert(
            "pinned".to_string(),
            Value::Bool(
                entry
                    .get("pinned")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            ),
        );
        if let Some(preferences) = preferences {
            normalized_entry.insert("preferences".to_string(), preferences);
        }
        normalized.push(Value::Object(normalized_entry));
    }
    Ok(json!({
        "version": PROJECT_SETTINGS_VERSION,
        "initialized": initialized,
        "entries": normalized,
    }))
}

#[tauri::command]
pub fn settings_load_all(state: State<'_, Arc<AppState>>) -> SettingsLoadResponse {
    let settings = state.settings.lock();
    let cwd = settings
        .get("system")
        .and_then(|value| value.get("workdir"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(current_workdir);
    let get = |scope: &str| {
        settings
            .get(scope)
            .cloned()
            .unwrap_or_else(|| default_setting(scope, &cwd))
    };
    SettingsLoadResponse {
        providers: redact_settings_secrets(&get("providers")),
        system: redact_settings_secrets(&get("system")),
        projects: redact_settings_secrets(&get("projects")),
        mcp: redact_settings_secrets(&get("mcp")),
        agents: redact_settings_secrets(&get("agents")),
        ssh: redact_settings_secrets(&get("ssh")),
        remote: redact_settings_secrets(&get("remote")),
        memory: redact_settings_secrets(&get("memory")),
        default_workdir: cwd,
    }
}

/// Return only the selected provider's runtime configuration. This is the
/// narrow credential boundary for the embedded Pi runner; the general
/// settings payload above is always redacted before it reaches the WebView.
#[tauri::command(rename_all = "camelCase")]
pub fn provider_runtime_config(
    state: State<'_, Arc<AppState>>,
    provider_id: Option<String>,
    model: Option<String>,
) -> Result<Value, String> {
    provider_runtime_config_for(&state, provider_id.as_deref(), model.as_deref())
}

/// Resolve a provider selection from the native settings snapshot.  The same
/// helper backs the renderer-facing redacted DTO and the proxy grant binding,
/// so a transport token is tied to the final native selection rather than an
/// arbitrary provider route supplied later by the renderer.
fn provider_runtime_config_for(
    state: &AppState,
    provider_id: Option<&str>,
    model: Option<&str>,
) -> Result<Value, String> {
    let settings = state.settings.lock();
    let providers = settings
        .get("providers")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let requested_provider = provider_id.map(str::trim).filter(|value| !value.is_empty());
    let requested_model = model.map(str::trim).filter(|value| !value.is_empty());
    let candidates: Vec<Value> = match providers {
        Value::Array(items) => items,
        Value::Object(object) => object
            .into_iter()
            .filter_map(|(id, value)| match value {
                Value::Object(mut record) => {
                    record.entry("id".to_string()).or_insert(Value::String(id));
                    Some(Value::Object(record))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let selected = if let Some(requested) = requested_provider {
        candidates
            .iter()
            .filter_map(Value::as_object)
            .find(|record| {
                record
                    .get("id")
                    .or_else(|| record.get("providerId"))
                    .or_else(|| record.get("provider_id"))
                    .and_then(Value::as_str)
                    == Some(requested)
            })
            .ok_or_else(|| format!("provider is not configured: {requested}"))?
    } else {
        requested_model
            .and_then(|requested| {
                candidates
                    .iter()
                    .filter_map(Value::as_object)
                    .find(|record| {
                        provider_is_enabled(record) && provider_model_is_enabled(record, requested)
                    })
            })
            .or_else(|| {
                candidates
                    .iter()
                    .filter_map(Value::as_object)
                    .find(|record| {
                        provider_is_enabled(record)
                            && provider_default_enabled_model(record).is_some()
                            && record
                                .get("default")
                                .or_else(|| record.get("isDefault"))
                                .and_then(Value::as_bool)
                                == Some(true)
                    })
            })
            .or_else(|| {
                candidates
                    .iter()
                    .filter_map(Value::as_object)
                    .find(|record| {
                        provider_is_enabled(record)
                            && provider_default_enabled_model(record).is_some()
                    })
            })
            .ok_or_else(|| "no provider is configured".to_string())?
    };
    if !provider_is_enabled(selected) {
        return Err("provider is disabled".to_string());
    }
    let effective_model = if let Some(requested) = requested_model {
        if !provider_model_is_enabled(selected, requested) {
            return Err("provider model is disabled or unavailable".to_string());
        }
        requested.to_string()
    } else {
        provider_default_enabled_model(selected)
            .ok_or_else(|| "provider has no enabled model".to_string())?
    };
    let system = settings
        .get("system")
        .cloned()
        .unwrap_or_else(|| json!({"workdir": current_workdir()}));
    let default_workdir = selected
        .get("workdir")
        .and_then(Value::as_str)
        .or_else(|| {
            settings
                .get("system")
                .and_then(|value| value.get("workdir"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .unwrap_or_else(current_workdir);
    let mut public_provider = redact_settings_secrets(&Value::Object(selected.clone()));
    if let Some(object) = public_provider.as_object_mut() {
        // The Pi adapter must route through the native proxy.  Credentials are
        // resolved by Rust at request time and never cross the IPC boundary.
        object.insert("proxyRequired".to_string(), Value::Bool(true));
        object.insert(
            "credentialMode".to_string(),
            Value::String("native-proxy".to_string()),
        );
        // One run receives one native-authorized model.  Do not leave the Pi
        // adapter an opportunity to fall back from an unavailable persisted
        // selection to another model in the raw settings record.
        object.insert("activeModels".to_string(), json!([effective_model.clone()]));
        object.insert("defaultModel".to_string(), Value::String(effective_model));
    }
    Ok(json!({
        "provider": public_provider,
        "system": redact_settings_secrets(&system),
        "defaultWorkdir": default_workdir,
    }))
}

fn resolved_proxy_provider_id(
    state: &AppState,
    requested_provider_id: Option<&str>,
    requested_model: Option<&str>,
) -> Option<String> {
    provider_runtime_config_for(state, requested_provider_id, requested_model)
        .ok()?
        .get("provider")
        .and_then(|provider| {
            provider
                .get("id")
                .or_else(|| provider.get("providerId"))
                .or_else(|| provider.get("provider_id"))
        })
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|provider_id| !provider_id.is_empty())
        .map(str::to_string)
}

/// Resolve the provider/model used for one-click translation from the same
/// native selection rules as the interactive runtime, never trusting a
/// renderer-supplied model string on its own. Prefer the active session's
/// durable model selection when a session id is supplied.
fn resolve_translation_context(
    state: &AppState,
    session_id: Option<&str>,
    requested_model: Option<&str>,
) -> Result<(String, String), String> {
    let session_selection = session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|id| {
            let sessions = state.sessions.lock();
            let record = sessions.get(id)?;
            if let Some(raw) = record.selected_model_json.as_deref() {
                if let Ok(selection) = parse_session_model_selection(raw) {
                    return Some((selection.provider_id, selection.model_id));
                }
            }
            let provider_id = record.provider_id.trim();
            let model = record.model.trim();
            if !provider_id.is_empty()
                && provider_id != "embedded"
                && !model.is_empty()
                && model != "embedded"
            {
                return Some((provider_id.to_string(), model.to_string()));
            }
            None
        });
    let (requested_provider, requested_model) = match session_selection {
        Some((provider_id, model_id)) => (Some(provider_id), Some(model_id)),
        None => (
            None,
            requested_model
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
        ),
    };
    let config = provider_runtime_config_for(
        state,
        requested_provider.as_deref(),
        requested_model.as_deref(),
    )
    .map_err(|_| "no provider is configured for translation".to_string())?;
    let provider_id = config
        .pointer("/provider/id")
        .or_else(|| config.pointer("/provider/providerId"))
        .or_else(|| config.pointer("/provider/provider_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "translation provider is unavailable".to_string())?
        .to_string();
    let model = config
        .pointer("/provider/defaultModel")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "translation model is unavailable".to_string())?
        .to_string();
    Ok((provider_id, model))
}

/// Read the raw (unredacted) provider record by id. Credentials never cross
/// the IPC boundary; they are consumed natively inside the same call.
fn raw_translation_provider(
    settings: &HashMap<String, Value>,
    provider_id: &str,
) -> Result<Value, String> {
    let providers = settings
        .get("providers")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let candidates: Vec<Value> = match providers {
        Value::Array(items) => items,
        Value::Object(object) => object
            .into_iter()
            .filter_map(|(id, value)| match value {
                Value::Object(mut record) => {
                    record.entry("id".to_string()).or_insert(Value::String(id));
                    Some(Value::Object(record))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    candidates
        .into_iter()
        .find(|record| {
            record
                .get("id")
                .or_else(|| record.get("providerId"))
                .or_else(|| record.get("provider_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some_and(|id| id == provider_id)
        })
        .ok_or_else(|| "translation provider is unavailable".to_string())
}

fn translation_system_instruction(source_lang: &str, target_lang: &str) -> String {
    let target = target_lang.trim();
    if source_lang.eq_ignore_ascii_case("auto") || source_lang.trim().is_empty() {
        format!(
            "You are a professional translator. Detect the source language and translate \
             the user-provided text into {target}. Return only the translation without \
             explanations, quotes, or code fences. Preserve line breaks and technical terms."
        )
    } else {
        format!(
            "You are a professional translator. Translate the user-provided text from {source} \
             into {target}. Return only the translation without explanations, quotes, or code \
             fences. Preserve line breaks and technical terms.",
            source = source_lang.trim()
        )
    }
}

#[tauri::command(rename_all = "camelCase")]
pub fn get_active_translation_model(
    state: State<'_, Arc<AppState>>,
    session_id: Option<String>,
) -> Result<String, String> {
    Ok(resolve_translation_context(
        &state,
        session_id.as_deref(),
        None,
    )?
    .1)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn translate_text(
    state: State<'_, Arc<AppState>>,
    text: String,
    target_lang: Option<String>,
    source_lang: Option<String>,
    model: Option<String>,
    session_id: Option<String>,
) -> Result<String, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("translation text cannot be empty".to_string());
    }
    if text.chars().count() > crate::local_services::MAX_TRANSLATION_INPUT_CHARS {
        return Err("translation text is too long".to_string());
    }
    let target_lang = target_lang
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "zh".to_string());
    let source_lang = source_lang
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "auto".to_string());
    if target_lang.chars().count() > 32 || source_lang.chars().count() > 32 {
        return Err("translation language code is invalid".to_string());
    }
    let requested_model = model
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let (provider_id, resolved_model) = resolve_translation_context(
        &state,
        session_id.as_deref(),
        requested_model.as_deref(),
    )?;
    let provider = {
        let settings = state.settings.lock();
        let provider = raw_translation_provider(&settings, &provider_id)?;
        provider
            .as_object()
            .cloned()
            .ok_or_else(|| "translation provider is unavailable".to_string())?
    };
    let system = translation_system_instruction(&source_lang, &target_lang);
    crate::local_services::translate_provider_text(&provider, &resolved_model, &system, &text)
        .await
}

/// Return loopback transport metadata only to the live, capability-bound Pi
/// turn that needs it. The normal proxy status/retry APIs never return a port
/// or token. Keeping this handshake bound to `agent_run` prevents settings,
/// diagnostics, and arbitrary renderer calls from treating the transport
/// metadata as a public configuration value.
#[tauri::command(rename_all = "camelCase")]
pub fn proxy_transport_info(
    state: State<'_, Arc<AppState>>,
    proxy: State<'_, Arc<crate::proxy::ProxyRuntime>>,
    request_id: String,
    capability_token: String,
) -> Result<crate::proxy::ProxyServerInfo, String> {
    let request_id = request_id.trim();
    let capability_token = capability_token.trim();
    if request_id.is_empty() || capability_token.is_empty() {
        return Err(crate::proxy::PROXY_UNAVAILABLE_ERROR.to_string());
    }
    let Some(grant) = proxy_transport_grant(&state, request_id, capability_token) else {
        return Err(crate::proxy::PROXY_UNAVAILABLE_ERROR.to_string());
    };
    proxy.issue_transport_info(
        request_id,
        &grant.provider_id,
        grant.expires_at,
        grant.cancelled,
    )
}

/// The proxy listener is a credential-bearing transport, so its loopback
/// token may only be released to the exact live, unexpired agent grant. This
/// mirrors normal native tool checks and closes the window after cancellation
/// has started but before a terminal event removes the active run.
#[cfg(test)]
fn proxy_transport_authorized(state: &AppState, request_id: &str, capability_token: &str) -> bool {
    proxy_transport_grant(state, request_id, capability_token).is_some()
}

#[derive(Clone)]
struct ProxyTransportGrant {
    provider_id: String,
    expires_at: Instant,
    cancelled: Arc<AtomicBool>,
}

fn proxy_transport_grant(
    state: &AppState,
    request_id: &str,
    capability_token: &str,
) -> Option<ProxyTransportGrant> {
    proxy_transport_parent_grant(state, request_id, capability_token)
        .or_else(|| proxy_transport_subagent_grant(state, request_id, capability_token))
}

fn proxy_transport_parent_grant(
    state: &AppState,
    request_id: &str,
    capability_token: &str,
) -> Option<ProxyTransportGrant> {
    let run = state.active_runs.lock().get(request_id).cloned()?;
    if run.capability_token != capability_token || run.cancelled.load(Ordering::SeqCst) {
        return None;
    }
    let grant = state.capabilities.lock().get(capability_token).cloned()?;
    let provider_id = run.proxy_provider_id.clone()?;
    if grant.expires_at <= Instant::now()
        || grant.cancelled.load(Ordering::SeqCst)
        || run.cancelled.load(Ordering::SeqCst)
        || grant.session_id != run.session_id
        || grant.conversation_id != run.conversation_id
        || grant.turn_id != run.turn_id
        || grant.request_id != run.request_id
        // The resolver independently makes this check before it can read a
        // native credential. Checking here also avoids minting a loopback
        // token for a provider removed or disabled after run registration.
        || provider_proxy_config(state, &provider_id).is_none()
    {
        return None;
    }
    Some(ProxyTransportGrant {
        provider_id,
        expires_at: grant.expires_at,
        cancelled: run.cancelled,
    })
}

/// A subagent never inherits its parent's general capability token. It receives
/// only a native-issued proxy request identity paired with its own scoped
/// capability. The proxy token is released only while the exact child task and
/// its parent run are both live.
fn proxy_transport_subagent_grant(
    state: &AppState,
    request_id: &str,
    capability_token: &str,
) -> Option<ProxyTransportGrant> {
    let child = state
        .subagent_capabilities
        .lock()
        .get(capability_token)
        .cloned()?;
    if child.proxy_request_id != request_id
        || child.expires_at <= Instant::now()
        || child.cancelled.load(Ordering::SeqCst)
    {
        return None;
    }
    let parent = state
        .active_runs
        .lock()
        .get(&child.parent_request_id)
        .cloned()?;
    if parent.cancelled.load(Ordering::SeqCst)
        || parent.session_id != child.session_id
        || parent.turn_id != child.parent_turn_id
        || parent.request_id != child.parent_request_id
    {
        return None;
    }
    let parent_grant = state
        .capabilities
        .lock()
        .get(&parent.capability_token)
        .cloned()?;
    if parent_grant.expires_at <= Instant::now()
        || parent_grant.cancelled.load(Ordering::SeqCst)
        || parent_grant.session_id != parent.session_id
        || parent_grant.conversation_id != parent.conversation_id
        || parent_grant.turn_id != parent.turn_id
        || parent_grant.request_id != parent.request_id
    {
        return None;
    }
    let task_running = state
        .subagent_tasks
        .get_task(&child.session_id, &child.task_id)
        .ok()
        .flatten()
        .is_some_and(|task| task.status == SubagentTaskStatus::Running);
    if !task_running {
        return None;
    }
    let provider_id = parent.proxy_provider_id.clone()?;
    provider_proxy_config(state, &provider_id)?;
    Some(ProxyTransportGrant {
        provider_id,
        expires_at: child.expires_at.min(parent_grant.expires_at),
        // Parent termination calls `cancel_subagent_tasks_for_parent`, which
        // flips this same flag before removing the child capability.
        cancelled: child.cancelled,
    })
}

fn secret_setting_key(key: &str) -> bool {
    let normalized = key.replace(['-', '_', '.'], "").to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "apikey"
            | "key"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "authorization"
            | "xapikey"
            | "xgoogapikey"
            | "secret"
            | "clientsecret"
            | "credential"
            | "credentials"
            | "password"
            | "passphrase"
            | "privatekey"
            | "bearertoken"
    ) || normalized.ends_with("token")
}

fn provider_id(value: &Value) -> Option<&str> {
    value
        .get("id")
        .or_else(|| value.get("providerId"))
        .or_else(|| value.get("provider_id"))
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
}

fn header_name(value: &Value) -> Option<&str> {
    value
        .get("key")
        .or_else(|| value.get("name"))
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
}

fn merge_provider_record(
    existing: Option<&Map<String, Value>>,
    incoming: &Map<String, Value>,
) -> Value {
    let mut merged = incoming.clone();
    let can_inherit = existing
        .map(|old| provider_secret_context_matches(old, incoming))
        .unwrap_or(false);

    for key in [
        "apiKey",
        "api_key",
        "key",
        "token",
        "accessToken",
        "access_token",
        "secret",
        "password",
        "privateKey",
        "private_key",
    ] {
        let marker = format!("{key}Configured");
        let marker_value = incoming.get(&marker).and_then(Value::as_bool);
        let inherited = if can_inherit && marker_value != Some(false) {
            existing.and_then(|old| old.get(key)).cloned()
        } else {
            None
        };
        match incoming.get(key) {
            None => {
                if let Some(value) = inherited {
                    merged.insert(key.to_string(), value);
                } else {
                    merged.remove(key);
                }
            }
            Some(Value::String(value)) if value.trim().is_empty() => {
                // An explicit empty field is the one unambiguous way for the
                // settings UI to clear a previously stored credential.
                merged.remove(key);
            }
            Some(Value::String(value)) if value.trim() == "[configured]" => {
                if let Some(value) = inherited {
                    merged.insert(key.to_string(), value);
                } else {
                    merged.remove(key);
                }
            }
            Some(_) => {}
        }
        merged.remove(&marker);
    }

    for key in ["headers", "customHeaders", "custom_headers"] {
        let Some(incoming_headers) = incoming.get(key) else {
            continue;
        };
        let existing_headers = existing.and_then(|old| old.get(key));
        if let Some(incoming_items) = incoming_headers.as_array() {
            let mut existing_by_name = HashMap::<String, &Map<String, Value>>::new();
            if can_inherit {
                if let Some(existing_items) = existing_headers.and_then(Value::as_array) {
                    for item in existing_items {
                        if let Some(name) = header_name(item) {
                            if let Some(object) = item.as_object() {
                                existing_by_name.insert(name.to_ascii_lowercase(), object);
                            }
                        }
                    }
                }
            }
            let mut output = Vec::new();
            for item in incoming_items {
                let Some(object) = item.as_object() else {
                    continue;
                };
                let Some(name) = header_name(item) else {
                    continue;
                };
                let mut header = object.clone();
                let configured = object.get("valueConfigured").and_then(Value::as_bool);
                let placeholder = object
                    .get("value")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value.trim() == "[configured]");
                let missing_or_placeholder = object.get("value").is_none() || placeholder;
                if missing_or_placeholder {
                    header.remove("value");
                    if configured != Some(false) && can_inherit {
                        if let Some(old) = existing_by_name.get(&name.to_ascii_lowercase()) {
                            if let Some(value) = old
                                .get("value")
                                .and_then(Value::as_str)
                                .filter(|value| !value.trim().is_empty())
                            {
                                header
                                    .insert("value".to_string(), Value::String(value.to_string()));
                            }
                        }
                    }
                }
                header.remove("valueConfigured");
                if header
                    .get("value")
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty() && value.trim() != "[configured]")
                {
                    output.push(Value::Object(header));
                }
            }
            merged.insert(key.to_string(), Value::Array(output));
        } else if let Some(incoming_object) = incoming_headers.as_object() {
            let existing_object = if can_inherit {
                existing_headers.and_then(Value::as_object)
            } else {
                None
            };
            let mut output = Map::new();
            for (name, value) in incoming_object {
                let placeholder = value.is_null()
                    || value
                        .as_str()
                        .is_some_and(|text| text.trim() == "[configured]");
                if placeholder {
                    if let Some(old) = existing_object
                        .and_then(|values| values.get(name))
                        .and_then(Value::as_str)
                        .filter(|value| !value.trim().is_empty())
                    {
                        output.insert(name.clone(), Value::String(old.to_string()));
                    }
                } else if value.as_str().is_some_and(|text| !text.trim().is_empty()) {
                    output.insert(name.clone(), value.clone());
                }
            }
            merged.insert(key.to_string(), Value::Object(output));
        }
    }

    // Renderer input cannot choose or preserve this value. It is derived from
    // the resulting provider record after all explicit/inherited secrets are
    // resolved, so a changed endpoint/family gets no old credential binding.
    merged.remove(PROVIDER_CREDENTIAL_BINDING_KEY);
    if provider_record_has_secret_values(&merged) {
        merged.insert(
            PROVIDER_CREDENTIAL_BINDING_KEY.to_string(),
            provider_credential_binding_value(&merged),
        );
    }
    Value::Object(merged)
}

fn provider_record_has_secret_values(object: &Map<String, Value>) -> bool {
    [
        "apiKey",
        "api_key",
        "key",
        "token",
        "accessToken",
        "access_token",
        "secret",
        "password",
        "privateKey",
        "private_key",
    ]
    .iter()
    .any(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    }) || {
        let mut headers = Vec::new();
        append_custom_headers(object, &mut headers);
        !headers.is_empty()
    }
}

fn merge_provider_settings(existing: Option<&Value>, incoming: Value) -> Value {
    match incoming {
        Value::Array(items) => {
            let old_items = existing.and_then(Value::as_array);
            Value::Array(
                items
                    .iter()
                    .map(|item| {
                        let old = provider_id(item).and_then(|id| {
                            old_items
                                .into_iter()
                                .flatten()
                                .find(|candidate| provider_id(candidate) == Some(id))
                        });
                        item.as_object()
                            .map(|object| {
                                merge_provider_record(old.and_then(Value::as_object), object)
                            })
                            .unwrap_or_else(|| item.clone())
                    })
                    .collect(),
            )
        }
        Value::Object(items) if items.get("providers").is_some() => {
            let mut output = items;
            let old_nested = existing.and_then(|value| value.get("providers"));
            if let Some(nested) = output.remove("providers") {
                output.insert(
                    "providers".to_string(),
                    merge_provider_settings(old_nested, nested),
                );
            }
            Value::Object(output)
        }
        Value::Object(items) => {
            let old_items = existing.and_then(Value::as_object);
            Value::Object(
                items
                    .iter()
                    .map(|(id, item)| {
                        let old = old_items.and_then(|values| values.get(id));
                        let value = item
                            .as_object()
                            .map(|object| {
                                merge_provider_record(old.and_then(Value::as_object), object)
                            })
                            .unwrap_or_else(|| item.clone());
                        (id.clone(), value)
                    })
                    .collect(),
            )
        }
        other => other,
    }
}

fn provider_removal_ids(provider_ids: Option<Vec<String>>) -> Result<HashSet<String>, String> {
    let Some(provider_ids) = provider_ids else {
        return Ok(HashSet::new());
    };
    if provider_ids.len() > MAX_PROVIDER_IMPORT_RECORDS {
        return Err("too many provider removals were requested".to_string());
    }
    let mut removed = HashSet::with_capacity(provider_ids.len());
    for id in provider_ids {
        let id = id.trim();
        if id.is_empty()
            || id.len() > MAX_PROVIDER_ID_BYTES
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            || !removed.insert(id.to_string())
        {
            return Err("the removed provider identifiers are invalid".to_string());
        }
    }
    Ok(removed)
}

/// The WebView can only save the same canonical provider record used for a
/// native connection draft.  Keeping this validation at the persistence
/// boundary prevents an IPC caller from storing a URL shape that the proxy,
/// credential binding, and model probes would interpret differently.
fn normalize_provider_settings_payload(payload: Value) -> Result<Value, String> {
    let items = payload
        .as_array()
        .ok_or_else(|| "provider settings payload must be an array".to_string())?;
    if items.len() > MAX_PROVIDER_IMPORT_RECORDS {
        return Err("provider settings contain too many records".to_string());
    }

    let mut ids = HashSet::with_capacity(items.len());
    let mut normalized = Vec::with_capacity(items.len());
    for item in items {
        let provider = normalize_provider_connection_draft(item.clone())?;
        let id = provider
            .get("id")
            .and_then(Value::as_str)
            .expect("normalized provider has a valid id");
        if !ids.insert(id.to_string()) {
            return Err("provider settings contain duplicate ids".to_string());
        }
        normalized.push(provider);
    }
    Ok(Value::Array(normalized))
}

/// Settings saves normally contain the full list rendered by the WebView. A
/// concurrent native import can add a record after that list was loaded, so an
/// omitted ID is preserved unless the UI explicitly confirmed its deletion.
fn merge_provider_settings_preserving_unmentioned(
    existing: Option<&Value>,
    incoming: Value,
    removed_ids: &HashSet<String>,
) -> Value {
    let mut merged = merge_provider_settings(existing, incoming);
    let (Some(existing_items), Some(output)) =
        (existing.and_then(Value::as_array), merged.as_array_mut())
    else {
        return merged;
    };
    let present_ids = output
        .iter()
        .filter_map(provider_id)
        .map(str::to_string)
        .collect::<HashSet<_>>();
    for record in existing_items {
        let Some(id) = provider_id(record) else {
            continue;
        };
        if !present_ids.contains(id) && !removed_ids.contains(id) {
            output.push(record.clone());
        }
    }
    merged
}

#[derive(Debug, Clone)]
struct ParsedProviderImport {
    provider: Value,
    has_credential: bool,
}

fn provider_import_string<'a>(record: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        record
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}

fn provider_import_text(value: &str, max_chars: usize) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        return None;
    }
    let result: String = value.chars().take(max_chars.saturating_add(1)).collect();
    (result.chars().count() <= max_chars).then_some(result)
}

fn provider_import_id(record: &Map<String, Value>, fallback: Option<&str>) -> Option<String> {
    let value = provider_import_string(record, &["id", "providerId", "provider_id"])
        .or(fallback)
        .map(str::trim)?;
    if value.is_empty()
        || value.len() > MAX_PROVIDER_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return None;
    }
    Some(value.to_string())
}

fn provider_import_model_id(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_PROVIDER_MODEL_ID_BYTES
        || value.chars().any(char::is_control)
    {
        return None;
    }
    Some(value.to_string())
}

fn append_provider_import_model(value: &Value, models: &mut Vec<String>) {
    let candidate = value.as_str().or_else(|| {
        value.as_object().and_then(|object| {
            provider_import_string(object, &["id", "modelId", "model_id", "name"])
        })
    });
    let Some(candidate) = candidate.and_then(provider_import_model_id) else {
        return;
    };
    if !models.contains(&candidate) && models.len() < MAX_PROVIDER_IMPORT_MODELS {
        models.push(candidate);
    }
}

fn provider_import_models(record: &Map<String, Value>) -> Vec<String> {
    let mut models = Vec::new();
    for key in [
        "models",
        "modelList",
        "model_list",
        "activeModels",
        "active_models",
    ] {
        if let Some(items) = record.get(key).and_then(Value::as_array) {
            for item in items {
                append_provider_import_model(item, &mut models);
            }
        }
    }
    for key in [
        "defaultModel",
        "default_model",
        "model",
        "modelId",
        "model_id",
    ] {
        if let Some(value) = record.get(key) {
            append_provider_import_model(value, &mut models);
        }
    }
    models
}

fn provider_import_safe_api_path(path: &str) -> bool {
    if path.contains('%') || path.contains("//") {
        return false;
    }
    let path = path.trim_end_matches('/').to_ascii_lowercase();
    PROVIDER_IMPORT_SAFE_API_PATHS
        .iter()
        .any(|allowed| path == *allowed)
}

/// Return only the reviewed public API root for renderer display. The caller
/// supplies a base URL that was already normalized by `provider_import_base_url`;
/// this second allowlist check keeps the preview fail-closed if that invariant
/// is ever weakened.
fn provider_import_api_root(base_url: &str) -> Option<String> {
    let parsed = Url::parse(base_url).ok()?;
    let path = parsed.path();
    if !provider_import_safe_api_path(path) {
        return None;
    }
    if path == "/" {
        Some("/".to_string())
    } else {
        Some(path.trim_end_matches('/').to_string())
    }
}

fn provider_import_base_url(record: &Map<String, Value>) -> Option<String> {
    let value = provider_import_string(
        record,
        &[
            "baseUrl", "base_url", "apiHost", "api_host", "endpoint", "url",
        ],
    )?;
    if value.len() > MAX_PROVIDER_IMPORT_URL_BYTES {
        return None;
    }
    let parsed = Url::parse(value).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !provider_import_safe_api_path(parsed.path())
    {
        return None;
    }
    let normalized = parsed.to_string();
    (normalized.len() <= MAX_PROVIDER_IMPORT_URL_BYTES).then_some(normalized)
}

fn provider_import_protocol(record: &Map<String, Value>) -> (&'static str, &'static str) {
    let hint = [
        "type",
        "providerType",
        "provider_type",
        "protocol",
        "api",
        "requestFormat",
        "request_format",
    ]
    .iter()
    .filter_map(|key| record.get(*key).and_then(Value::as_str))
    .collect::<Vec<_>>()
    .join(" ")
    .to_ascii_lowercase();
    if hint.contains("anthropic") || hint.contains("claude") {
        return ("claude_code", "anthropic-messages");
    }
    if hint.contains("gemini") || hint.contains("google") {
        return ("gemini", "google-generative-ai");
    }
    if hint.contains("completion") || hint.contains("chat/completion") {
        return ("codex", "openai-completions");
    }
    ("codex", "openai-responses")
}

fn provider_import_has_credential(value: &Value, in_headers: bool) -> bool {
    match value {
        Value::Array(items) => items
            .iter()
            .any(|item| provider_import_has_credential(item, in_headers)),
        Value::Object(object) => {
            let header_entry = in_headers
                && (object.contains_key("key") || object.contains_key("name"))
                && object.contains_key("value");
            object.iter().any(|(key, item)| {
                let lowered = key.replace(['-', '_', '.'], "").to_ascii_lowercase();
                let header_value = in_headers
                    && ((header_entry && lowered == "value")
                        || (!header_entry
                            && !matches!(lowered.as_str(), "key" | "name")
                            && item.is_string()));
                if header_value {
                    return item.as_str().is_some_and(|value| !value.trim().is_empty());
                }
                if !in_headers && secret_setting_key(&lowered) {
                    return item.as_str().is_some_and(|value| !value.trim().is_empty());
                }
                provider_import_has_credential(
                    item,
                    in_headers || matches!(lowered.as_str(), "headers" | "customheaders"),
                )
            })
        }
        _ => false,
    }
}

fn normalize_provider_import_record(
    record: &Map<String, Value>,
    fallback_id: Option<&str>,
) -> Option<ParsedProviderImport> {
    let id = provider_import_id(record, fallback_id)?;
    let name = provider_import_string(record, &["name", "label", "displayName", "display_name"])
        .and_then(|value| provider_import_text(value, MAX_PROVIDER_IMPORT_NAME_CHARS))
        .unwrap_or_else(|| id.clone());
    let base_url = provider_import_base_url(record)?;
    let models = provider_import_models(record);
    if models.is_empty() {
        return None;
    }
    let (provider_type, protocol) = provider_import_protocol(record);
    let has_credential = provider_import_has_credential(&Value::Object(record.clone()), false);
    let mut output = Map::new();
    output.insert("id".to_string(), Value::String(id));
    output.insert("name".to_string(), Value::String(name));
    output.insert("type".to_string(), Value::String(provider_type.to_string()));
    output.insert("protocol".to_string(), Value::String(protocol.to_string()));
    if provider_type == "codex" {
        output.insert(
            "requestFormat".to_string(),
            Value::String(protocol.to_string()),
        );
    }
    output.insert("baseUrl".to_string(), Value::String(base_url));
    output.insert(
        "models".to_string(),
        Value::Array(models.iter().cloned().map(Value::String).collect()),
    );
    output.insert(
        "activeModels".to_string(),
        Value::Array(
            models
                .first()
                .cloned()
                .map(Value::String)
                .into_iter()
                .collect(),
        ),
    );
    output.insert(
        "defaultModel".to_string(),
        Value::String(models.first().cloned().unwrap_or_default()),
    );
    Some(ParsedProviderImport {
        provider: Value::Object(output),
        has_credential,
    })
}

fn provider_import_source(root: &Value) -> Option<&Value> {
    if root.is_array() {
        return Some(root);
    }
    let object = root.as_object()?;
    if object.contains_key("id")
        || object.contains_key("providerId")
        || object.contains_key("provider_id")
    {
        return Some(root);
    }
    for key in [
        "providers",
        "customProviders",
        "custom_providers",
        "providerConfigs",
        "provider_configs",
    ] {
        if let Some(value) = object.get(key) {
            return Some(value);
        }
    }
    for wrapper in ["data", "config", "settings"] {
        let Some(nested) = object.get(wrapper).and_then(Value::as_object) else {
            continue;
        };
        for key in [
            "providers",
            "customProviders",
            "custom_providers",
            "providerConfigs",
            "provider_configs",
        ] {
            if let Some(value) = nested.get(key) {
                return Some(value);
            }
        }
    }
    None
}

fn provider_import_source_records(source: &Value) -> Option<Vec<(&Value, Option<&str>)>> {
    if let Some(items) = source.as_array() {
        return Some(items.iter().map(|item| (item, None)).collect());
    }
    let object = source.as_object()?;
    if object.contains_key("id")
        || object.contains_key("providerId")
        || object.contains_key("provider_id")
    {
        return Some(vec![(source, None)]);
    }
    Some(
        object
            .iter()
            .filter_map(|(id, value)| value.as_object().map(|_| (value, Some(id.as_str()))))
            .collect(),
    )
}

fn parse_provider_import_export(text: &str) -> Result<(Vec<ParsedProviderImport>, usize), String> {
    let root: Value = serde_json::from_str(text)
        .map_err(|_| "the selected export is not valid JSON".to_string())?;
    let source = provider_import_source(&root).ok_or_else(|| {
        "the selected JSON does not contain a supported providers export".to_string()
    })?;
    let records = provider_import_source_records(source).ok_or_else(|| {
        "the selected JSON does not contain a supported providers export".to_string()
    })?;
    if records.len() > MAX_PROVIDER_IMPORT_RECORDS {
        return Err(format!(
            "the selected export contains more than {MAX_PROVIDER_IMPORT_RECORDS} providers"
        ));
    }
    let mut output = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut skipped = 0;
    for (value, fallback_id) in records {
        let Some(object) = value.as_object() else {
            skipped += 1;
            continue;
        };
        let Some(candidate) = normalize_provider_import_record(object, fallback_id) else {
            skipped += 1;
            continue;
        };
        let Some(id) = provider_id(&candidate.provider) else {
            skipped += 1;
            continue;
        };
        if !seen_ids.insert(id.to_string()) {
            skipped += 1;
            continue;
        }
        output.push(candidate);
    }
    if output.is_empty() {
        return Err("the selected export has no compatible, non-empty providers".to_string());
    }
    Ok((output, skipped))
}

fn provider_import_path_is_json(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

fn provider_import_file_is_regular(metadata: &fs::Metadata) -> bool {
    !metadata.file_type().is_symlink()
        && !has_reparse_point(metadata)
        && metadata.file_type().is_file()
}

fn provider_import_file_metadata(path: &Path) -> Result<fs::Metadata, String> {
    fs::symlink_metadata(path)
        .map_err(|_| "could not inspect the selected provider export".to_string())
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProviderImportFileIdentity {
    volume_serial_number: u32,
    file_index: u64,
}

#[cfg(windows)]
fn provider_import_file_identity(file: &fs::File) -> Result<ProviderImportFileIdentity, String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    // `MetadataExt::{volume_serial_number, file_index}` is unstable in the
    // standard library. Use the stable Win32 handle query instead, so this
    // compares the actual opened object rather than only a pathname stat.
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let success =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut information) };
    if success == 0 {
        return Err("could not identify the selected provider export".to_string());
    }
    Ok(ProviderImportFileIdentity {
        volume_serial_number: information.dwVolumeSerialNumber,
        file_index: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
    })
}

#[cfg(windows)]
fn provider_import_file_identity_matches(
    first: &fs::File,
    second: &fs::File,
) -> Result<bool, String> {
    Ok(provider_import_file_identity(first)? == provider_import_file_identity(second)?)
}

#[cfg(unix)]
fn provider_import_file_identity_matches(
    first: &fs::File,
    second: &fs::File,
) -> Result<bool, String> {
    use std::os::unix::fs::MetadataExt;

    let first_metadata = first
        .metadata()
        .map_err(|_| "could not identify the selected provider export".to_string())?;
    let second_metadata = second
        .metadata()
        .map_err(|_| "could not identify the selected provider export".to_string())?;
    Ok(first_metadata.dev() == second_metadata.dev()
        && first_metadata.ino() == second_metadata.ino())
}

#[cfg(all(not(windows), not(unix)))]
fn provider_import_file_identity_matches(
    _first: &fs::File,
    _second: &fs::File,
) -> Result<bool, String> {
    // This target has no stable file identity exposed by std. The repeated
    // regular-file checks below still reject direct link/reparse replacement.
    Ok(true)
}

#[cfg(windows)]
fn open_provider_import_file_no_follow(path: &Path) -> Result<fs::File, String> {
    use std::os::windows::fs::OpenOptionsExt;

    // Ask CreateFileW for the reparse-point object itself. The opened handle is
    // then checked before any bytes are read, so a link swap between pathname
    // inspection and open cannot redirect this import to a link target.
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| "could not read the selected provider export".to_string())
}

#[cfg(unix)]
fn open_provider_import_file_no_follow(path: &Path) -> Result<fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| "could not read the selected provider export".to_string())
}

#[cfg(all(not(windows), not(unix)))]
fn open_provider_import_file_no_follow(path: &Path) -> Result<fs::File, String> {
    fs::File::open(path).map_err(|_| "could not read the selected provider export".to_string())
}

fn read_provider_import_file(path: &Path) -> Result<String, String> {
    if !provider_import_path_is_json(path) {
        return Err("select one JSON export file".to_string());
    }
    let metadata = provider_import_file_metadata(path)?;
    if !provider_import_file_is_regular(&metadata) {
        return Err("select one regular JSON export file, not a link or directory".to_string());
    }
    if metadata.len() > MAX_PROVIDER_IMPORT_FILE_BYTES {
        return Err(format!(
            "the selected export exceeds the {MAX_PROVIDER_IMPORT_FILE_BYTES}-byte limit"
        ));
    }

    // On Windows this opens the reparse-point object instead of following it;
    // check the handle and then the pathname again. The final pathname check
    // narrows the remaining cross-platform TOCTOU window without treating a
    // pre-open metadata result as proof of what the handle actually reads.
    let file = open_provider_import_file_no_follow(path)?;
    let opened_metadata = file
        .metadata()
        .map_err(|_| "could not inspect the selected provider export".to_string())?;
    if !provider_import_file_is_regular(&opened_metadata) {
        return Err("select one regular JSON export file, not a link or directory".to_string());
    }
    if opened_metadata.len() > MAX_PROVIDER_IMPORT_FILE_BYTES {
        return Err(format!(
            "the selected export exceeds the {MAX_PROVIDER_IMPORT_FILE_BYTES}-byte limit"
        ));
    }
    let post_open_metadata = provider_import_file_metadata(path)?;
    if !provider_import_file_is_regular(&post_open_metadata) {
        return Err("select one regular JSON export file, not a link or directory".to_string());
    }
    if post_open_metadata.len() > MAX_PROVIDER_IMPORT_FILE_BYTES {
        return Err(format!(
            "the selected export exceeds the {MAX_PROVIDER_IMPORT_FILE_BYTES}-byte limit"
        ));
    }
    let post_open_file = open_provider_import_file_no_follow(path)?;
    let post_open_handle_metadata = post_open_file
        .metadata()
        .map_err(|_| "could not inspect the selected provider export".to_string())?;
    if !provider_import_file_is_regular(&post_open_handle_metadata) {
        return Err("select one regular JSON export file, not a link or directory".to_string());
    }
    if !provider_import_file_identity_matches(&file, &post_open_file)? {
        return Err(
            "the selected provider export changed while opening; choose it again".to_string(),
        );
    }

    let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
    file.take(MAX_PROVIDER_IMPORT_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "could not read the selected provider export".to_string())?;
    if bytes.len() as u64 > MAX_PROVIDER_IMPORT_FILE_BYTES {
        return Err(format!(
            "the selected export exceeds the {MAX_PROVIDER_IMPORT_FILE_BYTES}-byte limit"
        ));
    }
    String::from_utf8(bytes).map_err(|_| "the selected export must be UTF-8 JSON".to_string())
}

fn provider_record_for_id<'a>(value: &'a Value, id: &str) -> Option<&'a Value> {
    match value {
        Value::Array(items) => items.iter().find(|item| provider_id(item) == Some(id)),
        Value::Object(object) => {
            if provider_id(value) == Some(id) {
                return Some(value);
            }
            if let Some(nested) = object.get("providers") {
                if let Some(found) = provider_record_for_id(nested, id) {
                    return Some(found);
                }
            }
            object
                .values()
                .find(|candidate| provider_id(candidate) == Some(id))
        }
        _ => None,
    }
}

fn prune_provider_import_previews(previews: &mut HashMap<String, ProviderImportDraft>) {
    let now = Instant::now();
    previews.retain(|_, draft| now.duration_since(draft.created_at) < PROVIDER_IMPORT_PREVIEW_TTL);
    while previews.len() >= MAX_PROVIDER_IMPORT_PREVIEWS {
        let oldest = previews
            .iter()
            .min_by_key(|(_, draft)| draft.created_at)
            .map(|(token, _)| token.clone());
        let Some(oldest) = oldest else {
            break;
        };
        previews.remove(&oldest);
    }
}

fn provider_import_preview_for_candidates(
    state: &AppState,
    candidates: Vec<ParsedProviderImport>,
    skipped: usize,
) -> ProviderImportPreview {
    let existing = state
        .settings
        .lock()
        .get("providers")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let import_token = format!("provider-import-{}", Uuid::new_v4());
    let mut items = Vec::with_capacity(candidates.len());
    let mut providers = Vec::with_capacity(candidates.len());
    let mut existing_public = HashMap::with_capacity(candidates.len());
    for candidate in candidates {
        let id = provider_id(&candidate.provider)
            .unwrap_or_default()
            .to_string();
        let existing_record = provider_record_for_id(&existing, &id);
        let current = existing_record.map(redact_settings_secrets);
        let object = candidate
            .provider
            .as_object()
            .expect("normalized provider is an object");
        let host = object
            .get("baseUrl")
            .and_then(Value::as_str)
            .and_then(|value| Url::parse(value).ok())
            .and_then(|url| {
                url.host_str().map(|host| match url.port() {
                    Some(port) => format!("{host}:{port}"),
                    None => host.to_string(),
                })
            })
            .unwrap_or_default();
        let api_root = object
            .get("baseUrl")
            .and_then(Value::as_str)
            .and_then(provider_import_api_root)
            .unwrap_or_default();
        let requires_credential_reentry =
            existing_record
                .and_then(Value::as_object)
                .is_some_and(|existing| {
                    provider_record_has_secret_values(existing)
                        && !provider_secret_context_matches(existing, object)
                });
        items.push(ProviderImportPreviewItem {
            id: id.clone(),
            name: object
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            host,
            api_root,
            protocol: object
                .get("protocol")
                .and_then(Value::as_str)
                .unwrap_or("openai-responses")
                .to_string(),
            model_count: object
                .get("models")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
            conflict: current.is_some(),
            has_credential: candidate.has_credential,
            requires_credential_reentry,
        });
        existing_public.insert(id, current);
        providers.push(candidate.provider);
    }
    let mut previews = state.provider_import_previews.lock();
    prune_provider_import_previews(&mut previews);
    previews.insert(
        import_token.clone(),
        ProviderImportDraft {
            providers,
            existing_public,
            created_at: Instant::now(),
        },
    );
    ProviderImportPreview {
        import_token,
        providers: items,
        skipped,
    }
}

fn provider_import_selected_ids(provider_ids: Vec<String>) -> Result<HashSet<String>, String> {
    if provider_ids.is_empty() || provider_ids.len() > MAX_PROVIDER_IMPORT_RECORDS {
        return Err("select one or more providers from the preview".to_string());
    }
    let mut selected = HashSet::with_capacity(provider_ids.len());
    for id in provider_ids {
        let value = id.trim();
        if value.is_empty()
            || value.len() > MAX_PROVIDER_ID_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            || !selected.insert(value.to_string())
        {
            return Err("the selected provider identifiers are invalid".to_string());
        }
    }
    Ok(selected)
}

fn merge_imported_provider_record(
    existing: Option<&Map<String, Value>>,
    imported: &Map<String, Value>,
) -> Value {
    let mut incoming = existing.cloned().unwrap_or_default();
    for key in [
        "apiKey",
        "api_key",
        "key",
        "token",
        "accessToken",
        "access_token",
        "secret",
        "password",
        "privateKey",
        "private_key",
        "headers",
        "customHeaders",
        "custom_headers",
        PROVIDER_CREDENTIAL_BINDING_KEY,
    ] {
        incoming.remove(key);
    }
    for (key, value) in imported {
        incoming.insert(key.clone(), value.clone());
    }
    if let Some(existing) = existing {
        // Preserve native-only custom headers only if the import still targets
        // the exact same endpoint and auth family. Otherwise they could move a
        // credential to a different gateway path or third-party endpoint.
        if provider_secret_context_matches(existing, &incoming) {
            for key in ["headers", "customHeaders", "custom_headers"] {
                if let Some(value) = existing.get(key) {
                    incoming.insert(key.to_string(), value.clone());
                }
            }
        }
    }
    merge_provider_record(existing, &incoming)
}

fn merge_provider_import_selection(
    existing: Option<&Value>,
    imported: &[Value],
) -> Result<Value, String> {
    let mut output = match existing {
        None => Vec::new(),
        Some(Value::Array(items)) => items.clone(),
        Some(_) => {
            return Err(
                "provider import requires the current provider collection to be an array; refresh and save a provider first"
                    .to_string(),
            )
        }
    };
    for candidate in imported {
        let id = provider_id(candidate)
            .ok_or_else(|| "the imported provider identifier is invalid".to_string())?;
        let imported_object = candidate
            .as_object()
            .ok_or_else(|| "the imported provider is invalid".to_string())?;
        if let Some(index) = output
            .iter()
            .position(|current| provider_id(current) == Some(id))
        {
            output[index] =
                merge_imported_provider_record(output[index].as_object(), imported_object);
        } else {
            output.push(merge_imported_provider_record(None, imported_object));
        }
    }
    Ok(Value::Array(output))
}

fn apply_provider_import_draft(
    state: &AppState,
    draft: &ProviderImportDraft,
    selected: &HashSet<String>,
) -> Result<ProviderImportApplyResult, String> {
    state.require_persistence_ready()?;
    let _persist_guard = state.persist_lock.lock();
    let existing = state.settings.lock().get("providers").cloned();
    let mut selected_records = Vec::with_capacity(selected.len());
    let mut added = 0;
    let mut updated = 0;
    for candidate in &draft.providers {
        let id = provider_id(candidate)
            .ok_or_else(|| "the imported provider identifier is invalid".to_string())?;
        if !selected.contains(id) {
            continue;
        }
        let expected = draft
            .existing_public
            .get(id)
            .ok_or_else(|| "the import preview is invalid; choose the export again".to_string())?;
        let current = existing
            .as_ref()
            .and_then(|settings| provider_record_for_id(settings, id))
            .map(redact_settings_secrets);
        if &current != expected {
            return Err(
                "provider settings changed since this preview; choose the export again before merging"
                    .to_string(),
            );
        }
        if current.is_some() {
            updated += 1;
        } else {
            added += 1;
        }
        selected_records.push(candidate.clone());
    }
    if selected_records.len() != selected.len() {
        return Err("the selected provider is no longer available in this preview".to_string());
    }
    let merged = merge_provider_import_selection(existing.as_ref(), &selected_records)?;
    let previous = state
        .settings
        .lock()
        .insert("providers".to_string(), merged);
    if let Err(error) = state.persist_settings_locked() {
        let mut settings = state.settings.lock();
        if let Some(previous) = previous {
            settings.insert("providers".to_string(), previous);
        } else {
            settings.remove("providers");
        }
        return Err(error);
    }
    Ok(ProviderImportApplyResult { added, updated })
}

fn record_identity(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    ["id", "providerId", "provider_id", "key", "name", "host"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

fn merge_redacted_settings(existing: Option<&Value>, incoming: &Value) -> Value {
    match incoming {
        Value::Array(items) => {
            let existing_items = existing.and_then(Value::as_array);
            Value::Array(
                items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        let old = record_identity(item)
                            .as_deref()
                            .and_then(|identity| {
                                existing_items.into_iter().flatten().find(|candidate| {
                                    record_identity(candidate).as_deref() == Some(identity)
                                })
                            })
                            .or_else(|| {
                                // Only anonymous records may inherit by
                                // position. A newly inserted/reordered named
                                // record must never receive another item's
                                // credential just because its index matches.
                                (record_identity(item).is_none())
                                    .then(|| existing_items.and_then(|items| items.get(index)))
                                    .flatten()
                            });
                        merge_redacted_settings(old, item)
                    })
                    .collect(),
            )
        }
        Value::Object(object) => {
            let existing_object = existing.and_then(Value::as_object);
            let mut output = Map::new();
            for (key, item) in object {
                if let Some(base_key) = key.strip_suffix("Configured") {
                    if item.as_bool() == Some(true) {
                        if let Some(old) = existing_object.and_then(|values| values.get(base_key)) {
                            output.insert(base_key.to_string(), old.clone());
                        }
                    }
                    continue;
                }
                if item.as_str() == Some("[configured]") {
                    if let Some(old) = existing_object.and_then(|values| values.get(key)) {
                        output.insert(key.clone(), old.clone());
                    }
                    continue;
                }
                if secret_setting_key(key)
                    && item.as_str().is_some_and(|value| value.trim().is_empty())
                {
                    continue;
                }
                output.insert(
                    key.clone(),
                    merge_redacted_settings(
                        existing_object.and_then(|values| values.get(key)),
                        item,
                    ),
                );
            }
            Value::Object(output)
        }
        other => other.clone(),
    }
}

fn save_setting(state: &AppState, scope: &str, payload: Value) -> Result<(), String> {
    state.require_persistence_ready()?;
    let _persist_guard = state.persist_lock.lock();
    let previous = state.settings.lock().insert(scope.to_string(), payload);
    if let Err(error) = state.persist_settings_locked() {
        let mut settings = state.settings.lock();
        if let Some(previous) = previous {
            settings.insert(scope.to_string(), previous);
        } else {
            settings.remove(scope);
        }
        return Err(error);
    }
    Ok(())
}

/// Merge a system-settings patch while holding the same persistence lock as the
/// protected-settings write.  Appearance controls and the permission picker
/// own disjoint fields and can save concurrently, so replacing the full
/// renderer snapshot here would let one stale payload discard another
/// component's newer fields.
fn save_system_settings(state: &AppState, payload: Value) -> Result<(), String> {
    state.require_persistence_ready()?;
    let patch = payload
        .as_object()
        .ok_or_else(|| "system settings payload must be an object".to_string())?;
    let _persist_guard = state.persist_lock.lock();
    let mut merged = state
        .settings
        .lock()
        .get("system")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (key, value) in patch {
        merged.insert(key.clone(), value.clone());
    }
    let normalized = normalize_system_settings_payload(Value::Object(merged))?;
    let previous = state
        .settings
        .lock()
        .insert("system".to_string(), normalized);
    if let Err(error) = state.persist_settings_locked() {
        let mut settings = state.settings.lock();
        if let Some(previous) = previous {
            settings.insert("system".to_string(), previous);
        } else {
            settings.remove("system");
        }
        return Err(error);
    }
    Ok(())
}

/// Save the renderer's provider list as one transaction. In contrast with the
/// generic settings helper, read/merge/persist all share `persist_lock` so an
/// import cannot be silently overwritten by a save that captured an older list.
fn save_provider_settings(
    state: &AppState,
    payload: Value,
    removed_provider_ids: Option<Vec<String>>,
) -> Result<(), String> {
    state.require_persistence_ready()?;
    let payload = normalize_provider_settings_payload(payload)?;
    let removed_ids = provider_removal_ids(removed_provider_ids)?;
    let _persist_guard = state.persist_lock.lock();
    let existing = state.settings.lock().get("providers").cloned();
    let merged =
        merge_provider_settings_preserving_unmentioned(existing.as_ref(), payload, &removed_ids);
    let previous = state
        .settings
        .lock()
        .insert("providers".to_string(), merged);
    if let Err(error) = state.persist_settings_locked() {
        let mut settings = state.settings.lock();
        if let Some(previous) = previous {
            settings.insert("providers".to_string(), previous);
        } else {
            settings.remove("providers");
        }
        return Err(error);
    }
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub fn settings_save_providers(
    state: State<'_, Arc<AppState>>,
    payload: Value,
    removed_provider_ids: Option<Vec<String>>,
) -> Result<(), String> {
    save_provider_settings(&state, payload, removed_provider_ids)
}

/// Open the operating-system picker for one provider export chosen by the
/// user. This command has no path argument and never scans application data,
/// home directories, or any other privacy-sensitive location. The selected
/// JSON is normalized natively; credentials are represented only by a boolean
/// in the preview and never cross into the WebView.
#[tauri::command]
pub async fn provider_import_preview(
    state: State<'_, Arc<AppState>>,
) -> Result<Option<ProviderImportPreview>, String> {
    let selected = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("Import provider export JSON")
            .add_filter("JSON", &["json"])
            .pick_file()
    })
    .await
    .map_err(|_| "could not open the provider import picker".to_string())?;

    let Some(selected) = selected else {
        return Ok(None);
    };
    let text = read_provider_import_file(&selected)?;
    let (candidates, skipped) = parse_provider_import_export(&text)?;
    Ok(Some(provider_import_preview_for_candidates(
        &state, candidates, skipped,
    )))
}

/// Merge only the provider IDs explicitly selected from a short-lived native
/// preview. The import token is consumed before persistence so it cannot be
/// replayed, and the native merge retains existing credentials only when their
/// endpoint/auth-family binding still matches.
#[tauri::command(rename_all = "camelCase")]
pub fn provider_import_apply(
    state: State<'_, Arc<AppState>>,
    import_token: String,
    provider_ids: Vec<String>,
) -> Result<ProviderImportApplyResult, String> {
    if import_token.len() > 128 || !import_token.starts_with("provider-import-") {
        return Err("the import preview token is invalid".to_string());
    }
    let selected = provider_import_selected_ids(provider_ids)?;
    let draft = {
        let mut previews = state.provider_import_previews.lock();
        prune_provider_import_previews(&mut previews);
        previews
            .remove(import_token.trim())
            .ok_or_else(|| "the import preview expired; choose the export again".to_string())?
    };
    apply_provider_import_draft(&state, &draft, &selected)
}

#[tauri::command]
pub fn settings_save_system(state: State<'_, Arc<AppState>>, payload: Value) -> Result<(), String> {
    save_system_settings(&state, payload)
}

#[tauri::command]
pub fn settings_save_projects(
    state: State<'_, Arc<AppState>>,
    payload: Value,
) -> Result<Value, String> {
    state.require_persistence_ready()?;
    let _persist_guard = state.persist_lock.lock();
    let previous = state.settings.lock().get("projects").cloned();
    reject_direct_legacy_project_path_changes(previous.as_ref(), &payload)?;
    let normalized = normalize_projects_settings_payload(&state, payload)?;
    reject_direct_stable_project_path_changes(previous.as_ref(), &normalized)?;
    let previous_keys = project_path_keys_from_value(previous.as_ref());
    let next_keys = project_path_keys_from_value(Some(&normalized));
    let removed_keys = previous_keys
        .difference(&next_keys)
        .cloned()
        .collect::<HashSet<_>>();
    let affected_session_ids = state
        .sessions
        .lock()
        .iter()
        .filter_map(|(id, record)| {
            workspace_path_key(&record.summary.cwd)
                .filter(|key| removed_keys.contains(key))
                .map(|_| id.clone())
        })
        .collect::<HashSet<_>>();
    if state
        .active_runs
        .lock()
        .values()
        .any(|run| affected_session_ids.contains(&run.session_id))
    {
        return Err(
            "cannot remove a project while one of its sessions has an active agent run".to_string(),
        );
    }

    state
        .settings
        .lock()
        .insert("projects".to_string(), normalized.clone());
    if let Err(error) = state.persist_settings_locked() {
        let mut settings = state.settings.lock();
        if let Some(previous) = previous {
            settings.insert("projects".to_string(), previous);
        } else {
            settings.remove("projects");
        }
        return Err(error);
    }
    refresh_approved_workdirs_after_project_save(&state);
    if !affected_session_ids.is_empty() {
        revoke_session_owned_access(&state, &affected_session_ids);
    }
    Ok(normalized)
}

#[tauri::command]
pub async fn settings_save_mcp(
    state: State<'_, Arc<AppState>>,
    runtime: State<'_, Arc<McpRuntimeManager>>,
    payload: Value,
) -> Result<(), String> {
    // Keep configuration persistence and client eviction in the same runtime
    // gate as every launch/use path. A request that captured an old trusted
    // config must not create its client after this save revokes it.
    let _execution_guard = runtime.lock_execution_gate().await;
    save_mcp_settings_with_native_consent(&state, payload).map_err(str::to_string)?;
    // Existing clients retain the last validated config, including native-only
    // headers/environment. Drop every live client after a successful save so
    // a removed or rotated server cannot keep using stale credentials.
    runtime.shutdown_all().await;
    Ok(())
}

#[tauri::command]
pub fn settings_save_agents(state: State<'_, Arc<AppState>>, payload: Value) -> Result<(), String> {
    save_setting(&state, "agents", payload)
}

#[tauri::command]
pub fn settings_save_ssh(state: State<'_, Arc<AppState>>, payload: Value) -> Result<(), String> {
    let existing = state.settings.lock().get("ssh").cloned();
    save_setting(
        &state,
        "ssh",
        merge_redacted_settings(existing.as_ref(), &payload),
    )
}

#[tauri::command]
pub fn settings_save_remote(state: State<'_, Arc<AppState>>, payload: Value) -> Result<(), String> {
    let existing = state.settings.lock().get("remote").cloned();
    save_setting(
        &state,
        "remote",
        merge_redacted_settings(existing.as_ref(), &payload),
    )
}

#[tauri::command]
pub fn settings_save_memory(state: State<'_, Arc<AppState>>, payload: Value) -> Result<(), String> {
    save_setting(&state, "memory", payload)
}

// ---------------------------------------------------------------------------
// Native MCP configuration and runtime boundary

/// Keep the renderer's selector bounded before looking it up in native
/// settings.  The actual config is never accepted as an IPC argument.
fn normalized_mcp_server_id(raw: &str) -> Result<String, String> {
    let id = raw.trim();
    if id.is_empty() || id.len() > MAX_MCP_SERVER_ID_BYTES || id.chars().any(char::is_control) {
        return Err("MCP server id is invalid".to_string());
    }
    Ok(id.to_string())
}

const MCP_SETTINGS_COLLECTION_KEYS: [&str; 4] = ["servers", "items", "mcpServers", "mcp_servers"];
// This native-only setting is deliberately not part of SettingsLoadResponse.
// It records an OS confirmation for one exact stdio execution configuration;
// the renderer cannot create or edit it through any settings command.
const MCP_STDIO_TRUST_SETTINGS_KEY: &str = "mcp_stdio_native_trust";
const MCP_STDIO_TRUST_SCHEMA_VERSION: u64 = 1;
const MCP_STDIO_NATIVE_CONFIRMATION_REQUIRED: &str = "mcp_stdio_native_confirmation_required";
#[cfg(all(not(windows), not(test)))]
const MCP_STDIO_NATIVE_CONFIRMATION_UNAVAILABLE: &str = "mcp_stdio_native_confirmation_unavailable";
#[cfg(all(windows, not(test)))]
const MCP_STDIO_NATIVE_CONFIRMATION_DENIED: &str = "mcp_stdio_native_confirmation_denied";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct McpStdioExecutionBinding<'a> {
    server_id: &'a str,
    command: &'a str,
    args: &'a [String],
    env: Option<&'a BTreeMap<String, String>>,
    cwd: Option<&'a str>,
    stdio_framing: Option<&'a str>,
}

fn has_mcp_settings_collection(object: &Map<String, Value>) -> bool {
    MCP_SETTINGS_COLLECTION_KEYS
        .iter()
        .any(|key| object.contains_key(*key))
}

/// Read one id from any supported spelling and reject ambiguous records before
/// a keyed legacy collection can be normalised into the canonical array.
fn configured_mcp_server_id(object: &Map<String, Value>) -> Result<Option<String>, String> {
    let mut configured = None;
    for key in ["id", "serverId", "server_id"] {
        let Some(value) = object.get(key) else {
            continue;
        };
        let value = value
            .as_str()
            .ok_or_else(|| "MCP server id is invalid".to_string())?;
        let value = normalized_mcp_server_id(value)?;
        if let Some(existing) = configured.as_ref() {
            if existing != &value {
                return Err("MCP server id aliases disagree".to_string());
            }
        } else {
            configured = Some(value);
        }
    }
    Ok(configured)
}

/// Turn one record into an id-bearing value. Keyed `mcpServers` maps may omit
/// the id from the value, but an explicit id may never disagree with its key.
fn mcp_settings_record(
    value: &Value,
    fallback_id: Option<&str>,
) -> Result<(String, Value), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "MCP server configuration is invalid".to_string())?;
    let explicit_id = configured_mcp_server_id(object)?;
    if explicit_id.is_some() && has_mcp_settings_collection(object) {
        return Err("MCP settings cannot mix a server record with a server collection".to_string());
    }
    let fallback_id = fallback_id.map(normalized_mcp_server_id).transpose()?;
    let id = match (explicit_id, fallback_id) {
        (Some(explicit), Some(fallback)) if explicit != fallback => {
            return Err("MCP server id does not match its collection key".to_string())
        }
        (Some(explicit), _) => explicit,
        (None, Some(fallback)) => fallback,
        (None, None) => return Err("MCP server id is invalid".to_string()),
    };
    let mut record = object.clone();
    record.insert("id".to_string(), Value::String(id.clone()));
    Ok((id, Value::Object(record)))
}

fn collect_mcp_settings_collection(
    value: &Value,
    records: &mut Vec<(String, Value)>,
) -> Result<(), String> {
    match value {
        Value::Array(values) => {
            for value in values {
                records.push(mcp_settings_record(value, None)?);
            }
        }
        Value::Object(object) => {
            if configured_mcp_server_id(object)?.is_some() {
                records.push(mcp_settings_record(value, None)?);
            } else {
                for (fallback_id, value) in object {
                    records.push(mcp_settings_record(value, Some(fallback_id))?);
                }
            }
        }
        _ => return Err("MCP settings have an invalid server collection".to_string()),
    }
    Ok(())
}

/// Accept the historical array/keyed-map/container spellings once at the save
/// boundary. The persisted form has one collection, so a later runtime lookup
/// cannot discover a second hidden collection or an alias collision.
fn mcp_settings_records(value: &Value) -> Result<Vec<(String, Value)>, String> {
    let mut records = Vec::new();
    match value {
        Value::Array(_) => collect_mcp_settings_collection(value, &mut records)?,
        Value::Object(object) => {
            if configured_mcp_server_id(object)?.is_some() {
                records.push(mcp_settings_record(value, None)?);
            } else {
                let mut found_collection = false;
                for key in MCP_SETTINGS_COLLECTION_KEYS {
                    if let Some(collection) = object.get(key) {
                        found_collection = true;
                        collect_mcp_settings_collection(collection, &mut records)?;
                    }
                }
                if !found_collection {
                    collect_mcp_settings_collection(value, &mut records)?;
                }
            }
        }
        _ => return Err("MCP settings have an invalid server collection".to_string()),
    }
    Ok(records)
}

fn normalised_mcp_server_label(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    ["label", "name", "title"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalise_mcp_settings_record(value: Value, requested_id: &str) -> Result<Value, String> {
    let label = normalised_mcp_server_label(&value);
    let value = normalise_mcp_server_config(value, requested_id)?;
    let config = serde_json::from_value::<McpServerConfig>(value)
        .map_err(|_| "MCP server configuration is invalid".to_string())?
        .normalised_for_settings()?;
    let mut value = serde_json::to_value(config)
        .map_err(|_| "MCP server configuration is invalid".to_string())?;
    if let Some(label) = label {
        value
            .as_object_mut()
            .ok_or_else(|| "MCP server configuration is invalid".to_string())?
            .insert("label".to_string(), Value::String(label));
    }
    Ok(value)
}

/// Bind a redacted MCP credential to the native execution target without
/// copying header or environment values into the binding.  The names of those
/// credential slots still matter: moving a retained token from one header or
/// environment variable to another can change how it is sent or consumed.
fn mcp_secret_execution_binding(value: &Value) -> Result<Value, String> {
    let config = serde_json::from_value::<McpServerConfig>(value.clone())
        .map_err(|_| "MCP server configuration is invalid".to_string())?;
    let header_names = config
        .headers
        .as_ref()
        .map(|headers| headers.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let environment_names = config
        .env
        .as_ref()
        .map(|environment| environment.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let credential_slots = json!({
        "headerNames": header_names,
        "environmentNames": environment_names,
    });
    let transport = config.transport.clone();

    match transport.as_deref() {
        Some("stdio") => Ok(json!({
            "transport": "stdio",
            "command": config.command,
            "args": config.args,
            "cwd": config.cwd,
            "stdioFraming": config.stdio_framing,
            "credentialSlots": credential_slots,
        })),
        Some("http") => Ok(json!({
            "transport": "http",
            "url": config.url,
            "allowRemote": config.allow_remote,
            "credentialSlots": credential_slots,
        })),
        Some("sse") => Ok(json!({
            "transport": "sse",
            "url": config.url,
            "messageUrl": config.message_url,
            "allowRemote": config.allow_remote,
            "credentialSlots": credential_slots,
        })),
        _ => Err("MCP server configuration is invalid".to_string()),
    }
}

/// Merge redacted native secrets by canonical server id, validate each native
/// runtime configuration, and emit the one persisted MCP settings shape. This
/// function is deliberately pure: its caller owns the persistence lock and
/// must not mutate settings until this succeeds.
fn merge_validate_and_canonicalize_mcp_settings(
    existing: Option<&Value>,
    payload: Value,
) -> Result<Value, String> {
    let records = mcp_settings_records(&payload)?;
    let mut existing_by_id = HashMap::new();
    if let Some(existing) = existing {
        if let Ok(existing_records) = mcp_settings_records(existing) {
            let mut seen = HashSet::with_capacity(existing_records.len());
            for (id, record) in existing_records {
                if !seen.insert(id.clone()) {
                    existing_by_id.clear();
                    break;
                }
                existing_by_id.insert(id, record);
            }
        }
    }

    let mut ids = HashSet::with_capacity(records.len());
    let mut normalised = Vec::with_capacity(records.len());
    for (id, record) in records {
        if !ids.insert(id.clone()) {
            return Err("MCP settings contain duplicate server ids".to_string());
        }
        // Fully canonicalize both sides before retaining a redacted secret.
        // Header field names are case-insensitive, so merging raw JSON here
        // could otherwise lose a retained value when only its spelling changed.
        let record = normalise_mcp_settings_record(record, &id)?;
        let existing = existing_by_id
            .get(&id)
            .and_then(|existing| normalise_mcp_settings_record(existing.clone(), &id).ok());
        let incoming_binding = mcp_secret_execution_binding(&record)?;
        let existing_for_secret = match existing.as_ref() {
            Some(existing) if mcp_secret_execution_binding(existing)? == incoming_binding => {
                Some(existing)
            }
            _ => None,
        };
        // `[configured]` only retains a secret from a server with the same
        // canonical id *and* execution binding. A renamed server or a changed
        // HTTP/SSE endpoint, transport, stdio command, args, cwd, or framing
        // must require the credential to be entered again.
        let record = merge_redacted_settings(existing_for_secret, &record);
        normalised.push(normalise_mcp_settings_record(record, &id)?);
    }
    Ok(json!({"schemaVersion": 1, "servers": normalised}))
}

const MCP_SETTINGS_SAVE_INVALID: &str = "mcp_settings_invalid";
const MCP_SETTINGS_SAVE_INVALID_COMMAND: &str = "mcp_settings_invalid_command";
const MCP_SETTINGS_SAVE_INVALID_ENVIRONMENT: &str = "mcp_settings_invalid_environment";
const MCP_SETTINGS_SAVE_INVALID_HEADERS: &str = "mcp_settings_invalid_headers";
const MCP_SETTINGS_SAVE_INVALID_ID: &str = "mcp_settings_invalid_id";
const MCP_SETTINGS_SAVE_INVALID_TIMEOUT: &str = "mcp_settings_invalid_timeout";
const MCP_SETTINGS_SAVE_INVALID_TRANSPORT: &str = "mcp_settings_invalid_transport";
const MCP_SETTINGS_SAVE_INVALID_URL: &str = "mcp_settings_invalid_url";
const MCP_SETTINGS_SAVE_UNAVAILABLE: &str = "mcp_settings_unavailable";

/// Project internal validation errors onto a closed set of renderer-visible
/// codes. Native error text can contain storage diagnostics or renderer-supplied
/// values, so it must not cross the MCP settings save boundary.
fn mcp_settings_validation_error_code(error: &str) -> &'static str {
    if error.contains("server id") || error.contains("duplicate server ids") {
        MCP_SETTINGS_SAVE_INVALID_ID
    } else if error.contains("url") || error.contains("messageUrl") {
        MCP_SETTINGS_SAVE_INVALID_URL
    } else if error.contains("transport") {
        MCP_SETTINGS_SAVE_INVALID_TRANSPORT
    } else if error.contains("header") {
        MCP_SETTINGS_SAVE_INVALID_HEADERS
    } else if error.contains("timeout") {
        MCP_SETTINGS_SAVE_INVALID_TIMEOUT
    } else if error.contains("command") || error.contains("argument") || error.contains("cwd") {
        MCP_SETTINGS_SAVE_INVALID_COMMAND
    } else if error.contains("environment") {
        MCP_SETTINGS_SAVE_INVALID_ENVIRONMENT
    } else {
        MCP_SETTINGS_SAVE_INVALID
    }
}

/// Commit a complete MCP configuration atomically. Reading the old settings,
/// restoring configured secrets, validating the candidate, snapshotting the
/// new value, and rolling it back on failure all share `persist_lock`; another
/// settings mutation therefore cannot observe or overwrite an intermediate
/// MCP configuration.
#[cfg(test)]
fn save_mcp_settings(state: &AppState, payload: Value) -> Result<(), &'static str> {
    let _persist_guard = state.persist_lock.lock();
    state
        .require_durable_mutation()
        .map_err(|_| MCP_SETTINGS_SAVE_UNAVAILABLE)?;
    let existing = state.settings.lock().get("mcp").cloned();
    let canonical = merge_validate_and_canonicalize_mcp_settings(existing.as_ref(), payload)
        .map_err(|error| mcp_settings_validation_error_code(&error))?;
    // This lower-level helper is retained for non-IPC tests. It never mints
    // stdio trust, so a future internal caller cannot accidentally turn a
    // direct settings write into executable authorization.
    let (previous, previous_trust) = {
        let mut settings = state.settings.lock();
        (
            settings.insert("mcp".to_string(), canonical),
            settings.remove(MCP_STDIO_TRUST_SETTINGS_KEY),
        )
    };
    if state.persist_settings_locked().is_err() {
        let mut settings = state.settings.lock();
        if let Some(previous) = previous {
            settings.insert("mcp".to_string(), previous);
        } else {
            settings.remove("mcp");
        }
        if let Some(previous) = previous_trust {
            settings.insert(MCP_STDIO_TRUST_SETTINGS_KEY.to_string(), previous);
        }
        return Err(MCP_SETTINGS_SAVE_UNAVAILABLE);
    }
    Ok(())
}

/// Persist MCP settings only after every newly enabled stdio process has an
/// OS-native approval bound to its full execution configuration. This holds
/// `persist_lock` across the dialog so the candidate shown to the user is the
/// exact candidate that is later committed.
fn save_mcp_settings_with_native_consent(
    state: &AppState,
    payload: Value,
) -> Result<(), &'static str> {
    let _persist_guard = state.persist_lock.lock();
    state
        .require_durable_mutation()
        .map_err(|_| MCP_SETTINGS_SAVE_UNAVAILABLE)?;
    let (existing, prior_trust) = {
        let settings = state.settings.lock();
        (
            settings.get("mcp").cloned(),
            mcp_stdio_trust_entries(&settings),
        )
    };
    let canonical = merge_validate_and_canonicalize_mcp_settings(existing.as_ref(), payload)
        .map_err(|error| mcp_settings_validation_error_code(&error))?;
    let configs =
        canonical_mcp_server_configs(&canonical).map_err(|_| MCP_SETTINGS_SAVE_INVALID_COMMAND)?;

    let mut next_trust = HashMap::new();
    let mut confirmation_required = Vec::new();
    for config in configs {
        if config.transport.as_deref() != Some("stdio") {
            continue;
        }
        let fingerprint = mcp_stdio_execution_fingerprint(&config)
            .map_err(|_| MCP_SETTINGS_SAVE_INVALID_COMMAND)?;
        if prior_trust.get(&config.id) == Some(&fingerprint) {
            // Preserve a trust record for an unchanged disabled server. If
            // the user later re-enables the exact same configuration it does
            // not silently become a different executable.
            next_trust.insert(config.id.clone(), fingerprint);
        } else if config.enabled {
            confirmation_required.push(config.clone());
            next_trust.insert(config.id.clone(), fingerprint);
        }
    }
    if !confirmation_required.is_empty() {
        request_native_mcp_stdio_confirmation(&confirmation_required)?;
    }

    let (previous_mcp, previous_trust) = {
        let mut settings = state.settings.lock();
        let previous_mcp = settings.insert("mcp".to_string(), canonical);
        let previous_trust = if next_trust.is_empty() {
            settings.remove(MCP_STDIO_TRUST_SETTINGS_KEY)
        } else {
            settings.insert(
                MCP_STDIO_TRUST_SETTINGS_KEY.to_string(),
                mcp_stdio_trust_value(&next_trust),
            )
        };
        (previous_mcp, previous_trust)
    };
    if state.persist_settings_locked().is_err() {
        let mut settings = state.settings.lock();
        if let Some(previous) = previous_mcp {
            settings.insert("mcp".to_string(), previous);
        } else {
            settings.remove("mcp");
        }
        if let Some(previous) = previous_trust {
            settings.insert(MCP_STDIO_TRUST_SETTINGS_KEY.to_string(), previous);
        } else {
            settings.remove(MCP_STDIO_TRUST_SETTINGS_KEY);
        }
        return Err(MCP_SETTINGS_SAVE_UNAVAILABLE);
    }
    Ok(())
}

fn mcp_record_value(value: &Value, requested_id: &str, fallback_id: Option<&str>) -> Option<Value> {
    let object = value.as_object()?;
    let configured_id = object
        .get("id")
        .or_else(|| object.get("serverId"))
        .or_else(|| object.get("server_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .or(fallback_id);
    if configured_id != Some(requested_id) {
        return None;
    }
    let mut record = object.clone();
    // `McpServerConfig` intentionally has a single canonical id field. A
    // keyed `mcpServers` map may omit it, but its native map key is the id.
    record.insert("id".to_string(), Value::String(requested_id.to_string()));
    Some(Value::Object(record))
}

fn mcp_record_from_collection(
    collection: &Value,
    requested_id: &str,
) -> Result<Option<Value>, String> {
    let mut matches = Vec::new();
    match collection {
        Value::Array(items) => {
            for item in items {
                if let Some(record) = mcp_record_value(item, requested_id, None) {
                    matches.push(record);
                }
            }
        }
        Value::Object(object) => {
            // An object with its own id is a single server record. Otherwise
            // it is treated as a key -> config map, as used by `mcpServers`.
            if let Some(record) = mcp_record_value(collection, requested_id, None) {
                matches.push(record);
            } else {
                for (key, item) in object {
                    if let Some(record) = mcp_record_value(item, requested_id, Some(key)) {
                        matches.push(record);
                    }
                }
            }
        }
        _ => return Err("MCP settings have an invalid server collection".to_string()),
    }
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        _ => Err("MCP settings contain duplicate server ids".to_string()),
    }
}

fn raw_mcp_server_config(state: &AppState, server_id: &str) -> Result<Value, String> {
    let requested_id = normalized_mcp_server_id(server_id)?;
    let settings = state.settings.lock();
    let mcp = settings
        .get("mcp")
        .ok_or_else(|| "MCP server is not configured".to_string())?;

    let candidate = match mcp {
        Value::Object(object) => {
            let containers = MCP_SETTINGS_COLLECTION_KEYS;
            let mut nested = Vec::new();
            for key in containers {
                if let Some(collection) = object.get(key) {
                    if let Some(record) = mcp_record_from_collection(collection, &requested_id)? {
                        nested.push(record);
                    }
                }
            }
            match nested.len() {
                0 => mcp_record_from_collection(mcp, &requested_id)?,
                1 => Some(nested.remove(0)),
                _ => return Err("MCP settings contain duplicate server ids".to_string()),
            }
        }
        _ => mcp_record_from_collection(mcp, &requested_id)?,
    };
    candidate.ok_or_else(|| "MCP server is not configured".to_string())
}

fn normalise_mcp_header_entries(value: &Value) -> Result<Value, String> {
    match value {
        Value::Object(_) => Ok(value.clone()),
        Value::Array(entries) => {
            let mut headers = Map::new();
            for entry in entries {
                let object = entry
                    .as_object()
                    .ok_or_else(|| "MCP headers are invalid".to_string())?;
                let header_name = |key: &str| -> Result<Option<String>, String> {
                    let Some(value) = object.get(key) else {
                        return Ok(None);
                    };
                    let value = value
                        .as_str()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| "MCP headers are invalid".to_string())?;
                    Ok(Some(value.to_string()))
                };
                let key = header_name("key")?;
                let name = header_name("name")?;
                let name = match (key, name) {
                    (Some(key), Some(name)) if !key.eq_ignore_ascii_case(&name) => {
                        return Err("MCP header aliases disagree".to_string())
                    }
                    (Some(key), _) => key,
                    (_, Some(name)) => name,
                    (None, None) => return Err("MCP headers are invalid".to_string()),
                };
                let header_value = object
                    .get("value")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "MCP headers are invalid".to_string())?;
                if headers
                    .insert(name, Value::String(header_value.to_string()))
                    .is_some()
                {
                    return Err("MCP headers contain duplicate names".to_string());
                }
            }
            Ok(Value::Object(headers))
        }
        _ => Err("MCP headers are invalid".to_string()),
    }
}

fn normalised_mcp_transport(value: &Value) -> Result<String, String> {
    let value = value
        .as_str()
        .ok_or_else(|| "unsupported MCP transport".to_string())?;
    match value.trim().to_ascii_lowercase().as_str() {
        "stdio" | "command" => Ok("stdio".to_string()),
        "http" | "streamable-http" | "streamablehttp" => Ok("http".to_string()),
        "sse" | "legacy-sse" | "legacy_sse" => Ok("sse".to_string()),
        _ => Err("unsupported MCP transport".to_string()),
    }
}

fn normalised_mcp_transport_alias(object: &Map<String, Value>) -> Result<Option<String>, String> {
    let mut transport = None;
    for key in ["transport", "connectionType", "connection_type", "type"] {
        let Some(value) = object.get(key) else {
            continue;
        };
        let value = normalised_mcp_transport(value)?;
        if let Some(existing) = transport.as_ref() {
            if existing != &value {
                return Err("MCP transport aliases disagree".to_string());
            }
        } else {
            transport = Some(value);
        }
    }
    Ok(transport)
}

fn normalised_mcp_string_alias(
    object: &Map<String, Value>,
    keys: &[&str],
    label: &str,
) -> Result<Option<String>, String> {
    let mut normalised = None;
    for key in keys {
        let Some(value) = object.get(*key) else {
            continue;
        };
        // Canonical serde output represents absent optional strings as null.
        // Treat null like an omitted legacy alias so a persisted record can
        // be validated and merged again without relaxing non-null conflicts.
        if value.is_null() {
            continue;
        }
        let value = value
            .as_str()
            .ok_or_else(|| format!("MCP {label} is invalid"))?
            .trim()
            .to_string();
        if let Some(existing) = normalised.as_ref() {
            if existing != &value {
                return Err(format!("MCP {label} aliases disagree"));
            }
        } else {
            normalised = Some(value);
        }
    }
    Ok(normalised)
}

fn normalised_mcp_value_alias(
    object: &Map<String, Value>,
    keys: &[&str],
    label: &str,
) -> Result<Option<Value>, String> {
    let mut normalised = None;
    for key in keys {
        let Some(value) = object.get(*key) else {
            continue;
        };
        if let Some(existing) = normalised.as_ref() {
            if existing != value {
                return Err(format!("MCP {label} aliases disagree"));
            }
        } else {
            normalised = Some(value.clone());
        }
    }
    Ok(normalised)
}

fn normalised_mcp_header_alias(object: &Map<String, Value>) -> Result<Option<Value>, String> {
    let mut normalised = None;
    for key in ["headers", "customHeaders"] {
        let Some(value) = object.get(key) else {
            continue;
        };
        // `headers: null` is the canonical representation of no headers.
        // It is equivalent to omitting the optional legacy field.
        if value.is_null() {
            continue;
        }
        let value = normalise_mcp_header_entries(value)?;
        if let Some(existing) = normalised.as_ref() {
            if existing != &value {
                return Err("MCP header aliases disagree".to_string());
            }
        } else {
            normalised = Some(value);
        }
    }
    Ok(normalised)
}

/// Normalize legacy/UI spelling while retaining only the native raw value.
/// This stays in native code and never returns the configuration to the
/// renderer.
fn normalise_mcp_server_config(value: Value, requested_id: &str) -> Result<Value, String> {
    let mut object = value
        .as_object()
        .cloned()
        .ok_or_else(|| "MCP server configuration is invalid".to_string())?;
    let configured_id = configured_mcp_server_id(&object)?
        .ok_or_else(|| "MCP server configuration is invalid".to_string())?;
    if configured_id != requested_id {
        return Err("MCP server selection does not match native settings".to_string());
    }
    object.insert("id".to_string(), Value::String(configured_id));
    object
        .entry("enabled".to_string())
        .or_insert(Value::Bool(true));

    if let Some(transport) = normalised_mcp_transport_alias(&object)? {
        object.insert("transport".to_string(), Value::String(transport));
    }
    if let Some(url) =
        normalised_mcp_string_alias(&object, &["url", "endpoint", "baseUrl", "base_url"], "url")?
    {
        object.insert("url".to_string(), Value::String(url));
    }
    if let Some(env) = normalised_mcp_value_alias(
        &object,
        &[
            "env",
            "environment",
            "environmentVariables",
            "environmentVars",
        ],
        "environment",
    )? {
        object.insert("env".to_string(), env);
    }
    if let Some(timeout_ms) =
        normalised_mcp_value_alias(&object, &["timeoutMs", "timeout_ms", "timeout"], "timeout")?
    {
        object.insert("timeoutMs".to_string(), timeout_ms);
    }
    if let Some(message_url) =
        normalised_mcp_string_alias(&object, &["messageUrl", "message_url"], "messageUrl")?
    {
        object.insert("messageUrl".to_string(), Value::String(message_url));
    }
    if let Some(allow_remote) =
        normalised_mcp_value_alias(&object, &["allowRemote", "allow_remote"], "allowRemote")?
    {
        object.insert("allowRemote".to_string(), allow_remote);
    }
    if let Some(stdio_framing) =
        normalised_mcp_string_alias(&object, &["stdioFraming", "stdio_framing"], "stdio framing")?
    {
        object.insert("stdioFraming".to_string(), Value::String(stdio_framing));
    }
    if let Some(headers) = normalised_mcp_header_alias(&object)? {
        object.insert("headers".to_string(), headers);
    }
    for key in [
        "serverId",
        "server_id",
        "connectionType",
        "connection_type",
        "type",
        "endpoint",
        "baseUrl",
        "base_url",
        "environment",
        "environmentVariables",
        "environmentVars",
        "timeout_ms",
        "timeout",
        "message_url",
        "allow_remote",
        "stdio_framing",
        "customHeaders",
    ] {
        object.remove(key);
    }
    Ok(Value::Object(object))
}

fn canonical_mcp_server_configs(value: &Value) -> Result<Vec<McpServerConfig>, String> {
    mcp_settings_records(value)?
        .into_iter()
        .map(|(id, record)| {
            let record = normalise_mcp_server_config(record, &id)?;
            let config = serde_json::from_value::<McpServerConfig>(record)
                .map_err(|_| "MCP server configuration is invalid".to_string())?;
            config.normalised_for_settings()
        })
        .collect()
}

fn normalized_mcp_stdio_execution_config(
    config: &McpServerConfig,
) -> Result<McpServerConfig, String> {
    let normalized = config.normalised_for_settings()?;
    if normalized.transport.as_deref() != Some("stdio") {
        return Err("MCP server is not a stdio configuration".to_string());
    }
    Ok(normalized)
}

/// A cryptographic, secret-preserving binding for every stdio field which can
/// affect the launched process. The digest is persisted instead of any new
/// renderer-visible configuration or approval boolean.
fn mcp_stdio_execution_fingerprint(config: &McpServerConfig) -> Result<String, String> {
    let config = normalized_mcp_stdio_execution_config(config)?;
    let binding = McpStdioExecutionBinding {
        server_id: &config.id,
        command: &config.command,
        args: &config.args,
        env: config.env.as_ref(),
        cwd: config.cwd.as_deref(),
        stdio_framing: config.stdio_framing.as_deref(),
    };
    let encoded = serde_json::to_vec(&binding)
        .map_err(|_| "MCP stdio configuration could not be bound".to_string())?;
    let digest: [u8; 32] = Sha256::digest(encoded).into();
    Ok(full_permission_prompt_fingerprint(&digest))
}

fn is_mcp_stdio_fingerprint(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn mcp_stdio_trust_entries(settings: &HashMap<String, Value>) -> HashMap<String, String> {
    let Some(object) = settings
        .get(MCP_STDIO_TRUST_SETTINGS_KEY)
        .and_then(Value::as_object)
    else {
        return HashMap::new();
    };
    if object.get("schemaVersion").and_then(Value::as_u64) != Some(MCP_STDIO_TRUST_SCHEMA_VERSION) {
        return HashMap::new();
    }
    object
        .get("servers")
        .and_then(Value::as_object)
        .map(|servers| {
            servers
                .iter()
                .filter_map(|(server_id, fingerprint)| {
                    let server_id = normalized_mcp_server_id(server_id).ok()?;
                    let fingerprint = fingerprint.as_str()?;
                    is_mcp_stdio_fingerprint(fingerprint)
                        .then(|| (server_id, fingerprint.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn mcp_stdio_trust_value(entries: &HashMap<String, String>) -> Value {
    let mut servers = Map::new();
    let mut sorted = entries.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.0.cmp(right.0));
    for (server_id, fingerprint) in sorted {
        servers.insert(server_id.clone(), Value::String(fingerprint.clone()));
    }
    json!({
        "schemaVersion": MCP_STDIO_TRUST_SCHEMA_VERSION,
        "servers": servers,
    })
}

fn mcp_stdio_confirmation_description(configs: &[McpServerConfig]) -> Result<String, String> {
    const DISPLAY_LIMIT: usize = 1_200;
    let mut description = String::from(
        "Allow NovaVei to run these exact local MCP stdio servers?\n\n\
Each server runs with your user account. This native approval is bound to its command, arguments, working directory, environment, and framing. Environment values are not shown because they can contain secrets. Displayed fields may be abbreviated; the SHA-256 binding covers the complete configuration.\n",
    );
    for config in configs {
        let config = normalized_mcp_stdio_execution_config(config)?;
        let fingerprint = mcp_stdio_execution_fingerprint(&config)?;
        let command = truncate_mcp_confirmation_text(&config.command, DISPLAY_LIMIT);
        let args = config
            .args
            .iter()
            .map(|arg| truncate_mcp_confirmation_text(arg, DISPLAY_LIMIT))
            .collect::<Vec<_>>()
            .join("\n  ");
        let env_names = config
            .env
            .as_ref()
            .map(|entries| entries.keys().cloned().collect::<Vec<_>>().join(", "))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "(none)".to_string());
        let cwd = config
            .cwd
            .as_deref()
            .unwrap_or("(inherit application directory)");
        let args_display = if args.is_empty() {
            " (none)".to_string()
        } else {
            format!("\n  {args}")
        };
        let server_id = truncate_mcp_confirmation_text(&config.id, DISPLAY_LIMIT);
        description.push_str(&format!(
            "\nServer: {}\nCommand: {}\nArguments:{}\nWorking directory: {}\nEnvironment keys: {}\nConfiguration SHA-256: {}\n",
            server_id,
            command,
            args_display,
            truncate_mcp_confirmation_text(cwd, DISPLAY_LIMIT),
            truncate_mcp_confirmation_text(&env_names, DISPLAY_LIMIT),
            fingerprint,
        ));
    }
    Ok(description)
}

fn truncate_mcp_confirmation_text(value: &str, max: usize) -> String {
    // These strings originate at the renderer boundary. Keep them as visible
    // literal text in the native dialog: line/control or bidi format
    // characters could otherwise make one approved execution appear to be
    // another. The full unmodified configuration remains covered by the
    // displayed SHA-256 binding.
    let mut displayed = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control()
            || matches!(
                character,
                '\u{202a}'
                    | '\u{202b}'
                    | '\u{202c}'
                    | '\u{202d}'
                    | '\u{202e}'
                    | '\u{2066}'
                    | '\u{2067}'
                    | '\u{2068}'
                    | '\u{2069}'
            )
        {
            use std::fmt::Write as _;
            let _ = write!(displayed, "\\u{{{:04x}}}", character as u32);
        } else {
            displayed.push(character);
        }
    }
    if displayed.len() <= max {
        return displayed;
    }
    let mut end = max;
    while end > 0 && !displayed.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [abbreviated]", &displayed[..end])
}

/// A native dialog is the authorization boundary for renderer-configured
/// executable paths. Builds without a native dialog fail closed.
fn request_native_mcp_stdio_confirmation(configs: &[McpServerConfig]) -> Result<(), &'static str> {
    let description = mcp_stdio_confirmation_description(configs)
        .map_err(|_| MCP_STDIO_NATIVE_CONFIRMATION_REQUIRED)?;
    #[cfg(all(windows, not(test)))]
    {
        let response = rfd::MessageDialog::new()
            .set_title("Allow NovaVei local MCP server")
            .set_description(description)
            .set_buttons(rfd::MessageButtons::OkCancel)
            .show();
        if response == rfd::MessageDialogResult::Ok {
            Ok(())
        } else {
            Err(MCP_STDIO_NATIVE_CONFIRMATION_DENIED)
        }
    }
    #[cfg(test)]
    {
        let _ = description;
        Ok(())
    }
    #[cfg(all(not(windows), not(test)))]
    {
        let _ = description;
        Err(MCP_STDIO_NATIVE_CONFIRMATION_UNAVAILABLE)
    }
}

fn require_native_mcp_stdio_trust(
    state: &AppState,
    config: &McpServerConfig,
) -> Result<(), String> {
    let config = normalized_mcp_stdio_execution_config(config)?;
    if !config.enabled {
        return Ok(());
    }
    // A persisted fingerprint records an approval made by the Windows native
    // dialog. Never treat that record as portable authorization on a build
    // where the dialog cannot be displayed.
    #[cfg(all(not(windows), not(test)))]
    {
        let _ = state;
        return Err(MCP_STDIO_NATIVE_CONFIRMATION_UNAVAILABLE.to_string());
    }
    let fingerprint = mcp_stdio_execution_fingerprint(&config)?;
    let trusted = mcp_stdio_trust_entries(&state.settings.lock());
    if trusted.get(&config.id) == Some(&fingerprint) {
        Ok(())
    } else {
        Err(MCP_STDIO_NATIVE_CONFIRMATION_REQUIRED.to_string())
    }
}

fn require_native_mcp_execution_trust(
    state: &AppState,
    config: &McpServerConfig,
) -> Result<(), String> {
    let normalized = config.normalised_for_settings()?;
    if normalized.transport.as_deref() == Some("stdio") {
        require_native_mcp_stdio_trust(state, &normalized)?;
    }
    Ok(())
}

/// Resolve a complete server configuration strictly from the protected native
/// settings snapshot. IPC callers can only select by id.
fn native_mcp_server_config(state: &AppState, server_id: &str) -> Result<McpServerConfig, String> {
    let requested_id = normalized_mcp_server_id(server_id)?;
    let value = raw_mcp_server_config(state, &requested_id)?;
    let value = normalise_mcp_server_config(value, &requested_id)?;
    serde_json::from_value::<McpServerConfig>(value)
        .map_err(|_| "MCP server configuration is invalid".to_string())
}

fn mcp_secret_values(config: &McpServerConfig) -> Vec<String> {
    let mut values = config
        .env
        .as_ref()
        .into_iter()
        .flat_map(|entries| entries.values())
        .chain(
            config
                .headers
                .as_ref()
                .into_iter()
                .flat_map(|entries| entries.values()),
        )
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    values.dedup();
    values
}

fn redact_mcp_secret_text(value: &str, secrets: &[String]) -> String {
    let mut redacted = value.to_string();
    for secret in secrets {
        redacted = redacted.replace(secret, "[redacted]");
    }
    redacted
}

fn redact_mcp_secret_value(value: Value, secrets: &[String]) -> Value {
    match value {
        Value::String(value) => Value::String(redact_mcp_secret_text(&value, secrets)),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| redact_mcp_secret_value(value, secrets))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| {
                    if event_secret_key(&key) {
                        (key, Value::String("[redacted]".to_string()))
                    } else {
                        (key, redact_mcp_secret_value(value, secrets))
                    }
                })
                .collect(),
        ),
        value => value,
    }
}

fn redact_mcp_error(error: String, secrets: &[String]) -> String {
    redact_mcp_secret_text(&error, secrets)
}

fn redact_mcp_tool_infos(tools: &mut [McpToolInfo], secrets: &[String]) {
    for tool in tools {
        tool.description = redact_mcp_secret_text(&tool.description, secrets);
        tool.input_schema = redact_mcp_secret_value(tool.input_schema.clone(), secrets);
    }
}

fn redact_mcp_call_response(
    mut response: McpCallToolResponse,
    secrets: &[String],
) -> McpCallToolResponse {
    for content in &mut response.content {
        let fields = Value::Object(std::mem::take(&mut content.fields));
        content.fields = redact_mcp_secret_value(fields, secrets)
            .as_object()
            .cloned()
            .unwrap_or_default();
    }
    response.details = redact_mcp_secret_value(response.details, secrets);
    response
}

fn redact_mcp_status(mut status: McpRuntimeStatus, secrets: &[String]) -> McpRuntimeStatus {
    status.last_error = status
        .last_error
        .as_deref()
        .map(|value| redact_mcp_secret_text(value, secrets));
    status
}

fn redact_mcp_test_response(
    mut response: McpRuntimeTestResponse,
    secrets: &[String],
) -> McpRuntimeTestResponse {
    for tool in &mut response.tools {
        tool.description = redact_mcp_secret_text(&tool.description, secrets);
        tool.input_schema = tool
            .input_schema
            .take()
            .map(|value| redact_mcp_secret_value(value, secrets));
    }
    response.error = response
        .error
        .as_deref()
        .map(|value| redact_mcp_secret_text(value, secrets));
    // Child-process stderr is intentionally never surfaced to the renderer.
    // It is not a trusted diagnostic channel and string redaction cannot
    // reliably identify arbitrary inherited or runtime credentials.
    response.stderr_tail = None;
    response
}

fn mcp_permission_tool_name(server_id: &str, name: &str) -> Result<String, String> {
    let server_id = normalized_mcp_server_id(server_id)?;
    let name = name.trim();
    if name.is_empty()
        || name.len() > MAX_MCP_PERMISSION_TOOL_NAME_BYTES
        || name.chars().any(char::is_control)
    {
        return Err("MCP tool name is invalid".to_string());
    }
    let permission_name = format!("mcp__{server_id}__{name}");
    if permission_name.len() > MAX_MCP_PERMISSION_TOOL_NAME_BYTES {
        return Err("MCP permission tool name is oversized".to_string());
    }
    Ok(permission_name)
}

/// Discover a configured server's tool descriptors. The config, headers and
/// environment are resolved only in native code and never serialized back.
#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_list_tools(
    state: State<'_, Arc<AppState>>,
    runtime: State<'_, Arc<McpRuntimeManager>>,
    server_id: String,
) -> Result<Vec<McpToolInfo>, String> {
    let _execution_guard = runtime.lock_execution_gate().await;
    let config = native_mcp_server_config(&state, &server_id)?;
    require_native_mcp_execution_trust(&state, &config)?;
    let secrets = mcp_secret_values(&config);
    let mut tools = runtime
        .list_tools(config)
        .await
        .map_err(|error| redact_mcp_error(error, &secrets))?;
    redact_mcp_tool_infos(&mut tools, &secrets);
    Ok(tools)
}

/// Invoke a configured MCP tool during an active Pi turn. The one-time native
/// approval is bound to `mcp__{serverId}__{name}`, not just the generic MCP
/// action, so renderer code cannot replay it against another tool.
#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub async fn mcp_call_tool(
    state: State<'_, Arc<AppState>>,
    runtime: State<'_, Arc<McpRuntimeManager>>,
    server_id: String,
    name: String,
    arguments: Option<Value>,
    capability_token: String,
    tool_call_id: Option<String>,
    workdir: String,
) -> Result<McpCallToolResponse, String> {
    let _execution_guard = runtime.lock_execution_gate().await;
    let config = native_mcp_server_config(&state, &server_id)?;
    require_native_mcp_execution_trust(&state, &config)?;
    let permission_name = mcp_permission_tool_name(&config.id, &name)?;
    let canonical_parent_workdir = require_capability_for_named(
        &state,
        Some(&capability_token),
        &workdir,
        ToolAction::Mcp,
        tool_call_id.as_deref(),
        Some(&permission_name),
    )?;
    let secrets = mcp_secret_values(&config);
    let response = runtime
        .call_tool(
            config,
            McpCallToolRequest {
                name,
                arguments: arguments.unwrap_or_else(|| json!({})),
            },
        )
        .await
        .map_err(|error| redact_mcp_error(error, &secrets))?;
    // The tool effect may already have occurred, but a cancellation racing the
    // response must not re-enter the Pi context with a post-cancel result.
    recheck_mutation_capability(
        &state,
        Some(&capability_token),
        &canonical_parent_workdir,
        ToolAction::Mcp,
    )?;
    Ok(redact_mcp_call_response(response, &secrets))
}

/// Read only runtime state for a still-configured server. No renderer-supplied
/// config is used, which also prevents probing arbitrary retained clients.
#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_runtime_status(
    state: State<'_, Arc<AppState>>,
    runtime: State<'_, Arc<McpRuntimeManager>>,
    server_id: String,
) -> Result<McpRuntimeStatus, String> {
    let _execution_guard = runtime.lock_execution_gate().await;
    let config = native_mcp_server_config(&state, &server_id)?;
    let secrets = mcp_secret_values(&config);
    Ok(redact_mcp_status(
        runtime.runtime_status(&config.id).await,
        &secrets,
    ))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_test_server(
    state: State<'_, Arc<AppState>>,
    runtime: State<'_, Arc<McpRuntimeManager>>,
    server_id: String,
) -> Result<McpRuntimeTestResponse, String> {
    let _execution_guard = runtime.lock_execution_gate().await;
    let config = native_mcp_server_config(&state, &server_id)?;
    require_native_mcp_execution_trust(&state, &config)?;
    let secrets = mcp_secret_values(&config);
    Ok(redact_mcp_test_response(
        runtime.test_server(config).await,
        &secrets,
    ))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_stop_server(
    _state: State<'_, Arc<AppState>>,
    runtime: State<'_, Arc<McpRuntimeManager>>,
    server_id: String,
) -> Result<McpStopServerResponse, String> {
    let _execution_guard = runtime.lock_execution_gate().await;
    // A settings save can evict a server from native configuration before a
    // stale client has been stopped. Keep this cleanup path available by
    // validating only the selector rather than re-reading the config.
    let server_id = normalized_mcp_server_id(&server_id)?;
    Ok(runtime.stop_server(&server_id).await)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_restart_server(
    state: State<'_, Arc<AppState>>,
    runtime: State<'_, Arc<McpRuntimeManager>>,
    server_id: String,
) -> Result<McpRuntimeStatus, String> {
    let _execution_guard = runtime.lock_execution_gate().await;
    let config = native_mcp_server_config(&state, &server_id)?;
    require_native_mcp_execution_trust(&state, &config)?;
    let secrets = mcp_secret_values(&config);
    let status = runtime
        .restart_server(config)
        .await
        .map_err(|error| redact_mcp_error(error, &secrets))?;
    Ok(redact_mcp_status(status, &secrets))
}

/// Save a Pi-created durable memory only while the originating turn remains
/// active. Project scope is pinned to the capability's canonical workspace;
/// a model cannot nominate a different project path in its tool arguments.
#[tauri::command(rename_all = "camelCase")]
pub fn memory_agent_create(
    state: State<'_, Arc<AppState>>,
    services: State<'_, Arc<LocalServices>>,
    mut input: MemoryCreateInput,
    capability_token: String,
    tool_call_id: Option<String>,
    workdir: String,
) -> Result<MemoryEntry, String> {
    state.require_persistence_ready()?;
    let canonical_workdir = require_capability_for(
        &state,
        Some(&capability_token),
        &workdir,
        ToolAction::MemoryWrite,
        tool_call_id.as_deref(),
    )?;
    match input.scope.trim() {
        "project" => input.workdir = Some(path_for_display(&canonical_workdir)),
        "global" => input.workdir = None,
        _ => return Err("memory scope must be global or project".to_string()),
    }
    let entry = services.memory_create(input)?;
    // Do not return a result to Pi after cancellation raced this small SQLite
    // write; the user sees the cancellation rather than a stale continuation.
    recheck_mutation_capability(
        &state,
        Some(&capability_token),
        &canonical_workdir,
        ToolAction::MemoryWrite,
    )?;
    Ok(entry)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentTaskStartResponse {
    pub task: StoredSubagentTask,
    pub capability_token: String,
    pub proxy_request_id: String,
    pub agent_id: String,
    pub private_context: Option<String>,
}

fn subagent_proxy_request_id(mode: SubagentCapabilityMode, task_id: &str) -> String {
    match mode {
        SubagentCapabilityMode::Readonly => format!("subagent-run-{task_id}"),
        SubagentCapabilityMode::Worktree => format!("worktree-run-{task_id}"),
    }
}

/// Create one read-only delegated task bound to the currently active parent
/// turn. The task statement never reaches native persistence; only its bounded
/// user-visible title and lifecycle metadata are retained.
#[cfg(feature = "desktop")]
#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub fn subagent_task_start(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    title: String,
    task: String,
    workdir: String,
    capability_token: String,
    tool_call_id: String,
    allow_global_read: Option<bool>,
    agent_id: Option<String>,
    resume: Option<bool>,
) -> Result<SubagentTaskStartResponse, String> {
    state.require_persistence_ready()?;
    let task = task.trim();
    if task.is_empty()
        || task.chars().count() > MAX_SUBAGENT_TASK_CHARS
        || task.chars().any(char::is_control)
    {
        return Err("subagent task statement is invalid".to_string());
    }
    let allow_global_read = allow_global_read.unwrap_or(false);
    let canonical_parent_workdir = require_capability_for_named_with_constraints(
        &state,
        Some(&capability_token),
        &workdir,
        ToolAction::Subagent,
        Some(&tool_call_id),
        Some("DelegateReadOnly"),
        allow_global_read,
        allow_global_read,
    )?;
    let parent_grant = state
        .capabilities
        .lock()
        .get(capability_token.trim())
        .cloned()
        .ok_or_else(|| "parent agent capability is no longer active".to_string())?;
    if state
        .subagent_capabilities
        .lock()
        .values()
        .filter(|grant| {
            grant.parent_request_id == parent_grant.request_id
                && !grant.cancelled.load(Ordering::SeqCst)
        })
        .count()
        >= MAX_CONCURRENT_SUBAGENTS_PER_PARENT
    {
        return Err("subagent concurrency limit reached for this parent turn".to_string());
    }

    let task_id = format!("subtask-{}", Uuid::new_v4());
    let created_at = now_ms();
    let agent_id = agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("agent-{}", Uuid::new_v4().simple()));
    let private_context = state.subagent_tasks.prepare_agent(
        &parent_grant.session_id,
        &agent_id,
        "readonly",
        resume.unwrap_or(false),
        created_at,
    )?;
    let task = state.subagent_tasks.create_task(NewSubagentTask {
        id: task_id.clone(),
        session_id: parent_grant.session_id.clone(),
        agent_id: agent_id.clone(),
        parent_turn_id: parent_grant.turn_id.clone(),
        parent_request_id: parent_grant.request_id.clone(),
        title,
        created_at,
    })?;
    if let Err(error) = state.subagent_tasks.mark_running(&task_id, created_at) {
        let _ = state.subagent_tasks.finish_task(
            &task_id,
            SubagentTaskStatus::Failed,
            None,
            Some("initialization_failed".to_string()),
            now_ms(),
        );
        return Err(error);
    }
    // A parent cancellation can race the small durable setup above. Do not
    // issue a child capability after its originating parent has stopped.
    if let Err(error) = recheck_mutation_capability(
        &state,
        Some(&capability_token),
        &canonical_parent_workdir,
        ToolAction::Subagent,
    ) {
        let _ = state.subagent_tasks.finish_task(
            &task_id,
            SubagentTaskStatus::Cancelled,
            None,
            None,
            now_ms(),
        );
        return Err(error);
    }

    let child_capability_token = format!("subcap-{}", Uuid::new_v4());
    let proxy_request_id = subagent_proxy_request_id(SubagentCapabilityMode::Readonly, &task_id);
    state.subagent_capabilities.lock().insert(
        child_capability_token.clone(),
        SubagentCapabilityGrant {
            task_id: task_id.clone(),
            session_id: parent_grant.session_id,
            parent_turn_id: parent_grant.turn_id,
            parent_request_id: parent_grant.request_id,
            proxy_request_id: proxy_request_id.clone(),
            workdir: canonical_parent_workdir,
            mode: SubagentCapabilityMode::Readonly,
            allow_global_read,
            expires_at: Instant::now() + Duration::from_secs(10 * 60),
            cancelled: Arc::new(AtomicBool::new(false)),
        },
    );
    let task = state
        .subagent_tasks
        .get_task(&task.session_id, &task_id)?
        .ok_or_else(|| "started subagent task is unavailable".to_string())?;
    emit_subagent_task_update(&app, &task)?;
    Ok(SubagentTaskStartResponse {
        task,
        capability_token: child_capability_token,
        proxy_request_id,
        agent_id,
        private_context,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn subagent_tasks_list(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    limit: Option<usize>,
) -> Result<Vec<StoredSubagentTask>, String> {
    let session_id = session_id.trim();
    if !state.sessions.lock().contains_key(session_id) {
        return Err("session not found".to_string());
    }
    state
        .subagent_tasks
        .list_tasks(session_id, limit.unwrap_or(30))
}

#[tauri::command(rename_all = "camelCase")]
pub fn subagent_task_get(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    task_id: String,
) -> Result<Option<StoredSubagentTask>, String> {
    let session_id = session_id.trim();
    if !state.sessions.lock().contains_key(session_id) {
        return Err("session not found".to_string());
    }
    state.subagent_tasks.get_task(session_id, task_id.trim())
}

/// Finish a child task without accepting its raw output. The parent Agent
/// receives its bounded summary as the tool result in-memory; SQLite stores
/// only the lifecycle outcome and a stable failure category.
#[cfg(feature = "desktop")]
#[tauri::command(rename_all = "camelCase")]
pub fn subagent_task_finish(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    task_id: String,
    capability_token: String,
    outcome: String,
) -> Result<StoredSubagentTask, String> {
    state.require_persistence_ready()?;
    let grant = require_subagent_capability(&state, &task_id, &capability_token)?;
    let outcome = outcome.trim();
    let cleanup_disposition = match outcome {
        "completed" if grant.mode == SubagentCapabilityMode::Worktree => {
            return Err("worktree subagent completion must collect a patch for review".to_string())
        }
        "completed" => None,
        "failed" if grant.mode == SubagentCapabilityMode::Worktree => {
            Some(WorktreeCleanupDisposition::ChildFailed)
        }
        "cancelled" if grant.mode == SubagentCapabilityMode::Worktree => {
            Some(WorktreeCleanupDisposition::ChildCancelled)
        }
        "failed" | "cancelled" => None,
        _ => return Err("subagent task outcome is invalid".to_string()),
    };
    // Fail closed before publishing the terminal task status. Otherwise a
    // concurrent proxy handshake could observe the old Running state between
    // the durable transition and revocation below.
    grant.cancelled.store(true, Ordering::SeqCst);
    let finished_at = now_ms();
    if let Some(disposition) = cleanup_disposition {
        state
            .subagent_tasks
            .mark_cleanup_pending(&grant.task_id, disposition, finished_at)?;
    } else {
        let (status, failure_code) = match outcome {
            "completed" => (SubagentTaskStatus::Completed, None),
            "failed" => (
                SubagentTaskStatus::Failed,
                Some("runtime_failed".to_string()),
            ),
            "cancelled" => (SubagentTaskStatus::Cancelled, None),
            _ => return Err("subagent task outcome is invalid".to_string()),
        };
        state.subagent_tasks.finish_task(
            &grant.task_id,
            status,
            None,
            failure_code,
            finished_at,
        )?;
    }
    // A completed child must not retain a previously issued proxy token while
    // its capability record is being removed.
    state
        .subagent_capabilities
        .lock()
        .remove(capability_token.trim());
    let task = state
        .subagent_tasks
        .get_task(&grant.session_id, &grant.task_id)?
        .ok_or_else(|| "finished subagent task is unavailable".to_string())?;
    emit_subagent_task_update(&app, &task)?;
    Ok(task)
}

/// Save the child's own bounded final report as its private continuation
/// checkpoint. The task list never returns this text; a later child receives
/// it only when the parent explicitly requests `resume` for the same agent id.
#[cfg(feature = "desktop")]
#[tauri::command(rename_all = "camelCase")]
pub fn subagent_private_context_save(
    state: State<'_, Arc<AppState>>,
    task_id: String,
    capability_token: String,
    report: String,
) -> Result<(), String> {
    state.require_persistence_ready()?;
    let grant = require_subagent_capability(&state, &task_id, &capability_token)?;
    let task = state
        .subagent_tasks
        .get_task(&grant.session_id, &grant.task_id)?
        .ok_or_else(|| "subagent task is unavailable".to_string())?;
    state.subagent_tasks.save_agent_private_context(
        &grant.session_id,
        &task.agent_id,
        &report,
        now_ms(),
    )
}

/// Route a bounded coordination note through native storage. Child messages
/// may target their parent or the session broadcast channel; a parent may
/// address a stable child identity or broadcast. The database stores only the
/// transcript-encrypted body and the renderer sees it through a session feed.
#[cfg(feature = "desktop")]
#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub fn subagent_message_send(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    task_id: Option<String>,
    capability_token: String,
    workdir: String,
    recipient: String,
    channel: Option<String>,
    content: String,
) -> Result<StoredSubagentMessage, String> {
    state.require_persistence_ready()?;
    let recipient = recipient.trim().to_string();
    let channel = channel.unwrap_or_else(|| "status".to_string());
    let (session_id, sender_agent_id) = if let Some(task_id) = task_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let grant = require_subagent_capability(&state, task_id, &capability_token)?;
        if recipient != "parent" && recipient != "*" {
            return Err("a child may message only parent or broadcast".to_string());
        }
        let task = state
            .subagent_tasks
            .get_task(&grant.session_id, &grant.task_id)?
            .ok_or_else(|| "subagent task is unavailable".to_string())?;
        (grant.session_id, task.agent_id)
    } else {
        // A parent run can send to a stable child identity or broadcast, but
        // cannot impersonate a child or route a message back to itself.
        let _ = require_capability(&state, Some(&capability_token), &workdir)?;
        let grant = state
            .capabilities
            .lock()
            .get(capability_token.trim())
            .cloned()
            .ok_or_else(|| "parent agent capability is no longer active".to_string())?;
        if recipient == "parent" {
            return Err("a parent message requires a child agent id or broadcast".to_string());
        }
        if recipient != "*"
            && !state
                .subagent_tasks
                .agent_exists(&grant.session_id, &recipient)?
        {
            return Err("subagent message recipient is unavailable".to_string());
        }
        (grant.session_id, "parent".to_string())
    };
    let message = state.subagent_tasks.save_message(
        &format!("submsg-{}", Uuid::new_v4()),
        &session_id,
        &sender_agent_id,
        &recipient,
        &channel,
        &content,
        now_ms(),
    )?;
    emit_subagent_message(&app, &message)?;
    Ok(message)
}

#[tauri::command(rename_all = "camelCase")]
pub fn subagent_messages_list(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    limit: Option<usize>,
) -> Result<Vec<StoredSubagentMessage>, String> {
    let session_id = session_id.trim();
    if !state.sessions.lock().contains_key(session_id) {
        return Err("session not found".to_string());
    }
    state
        .subagent_tasks
        .list_messages(session_id, limit.unwrap_or(30))
}

#[cfg(feature = "desktop")]
#[tauri::command(rename_all = "camelCase")]
pub fn subagent_task_cancel(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    task_id: String,
    capability_token: String,
) -> Result<StoredSubagentTask, String> {
    state.require_persistence_ready()?;
    let grant = require_subagent_capability(&state, &task_id, &capability_token)?;
    grant.cancelled.store(true, Ordering::SeqCst);
    if grant.mode == SubagentCapabilityMode::Worktree {
        state.subagent_tasks.mark_cleanup_pending(
            &grant.task_id,
            WorktreeCleanupDisposition::ChildCancelled,
            now_ms(),
        )?;
    } else {
        state.subagent_tasks.cancel_task(&grant.task_id, now_ms())?;
    }
    state
        .subagent_capabilities
        .lock()
        .remove(capability_token.trim());
    let task = state
        .subagent_tasks
        .get_task(&grant.session_id, &grant.task_id)?
        .ok_or_else(|| "cancelled subagent task is unavailable".to_string())?;
    emit_subagent_task_update(&app, &task)?;
    Ok(task)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeTaskStartResponse {
    pub task: StoredSubagentTask,
    pub capability_token: String,
    pub proxy_request_id: String,
    pub workdir: String,
    pub base_commit: String,
    pub agent_id: String,
    pub private_context: Option<String>,
}

/// Provision a detached worktree only after the parent turn received a
/// one-time `DelegateWorktree` approval. Non-Git workdirs are rejected; this
/// never initializes a repository on the user's behalf.
#[cfg(feature = "desktop")]
#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub fn worktree_task_start(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    title: String,
    task: String,
    workdir: String,
    capability_token: String,
    tool_call_id: String,
    allow_global_read: Option<bool>,
    agent_id: Option<String>,
    resume: Option<bool>,
) -> Result<WorktreeTaskStartResponse, String> {
    state.require_persistence_ready()?;
    let task = task.trim();
    if task.is_empty()
        || task.chars().count() > MAX_SUBAGENT_TASK_CHARS
        || task.chars().any(char::is_control)
    {
        return Err("worktree task statement is invalid".to_string());
    }
    let allow_global_read = allow_global_read.unwrap_or(false);
    let canonical_parent_workdir = require_capability_for_named_with_constraints(
        &state,
        Some(&capability_token),
        &workdir,
        ToolAction::Worktree,
        Some(&tool_call_id),
        Some("DelegateWorktree"),
        allow_global_read,
        allow_global_read,
    )?;
    let parent_grant = state
        .capabilities
        .lock()
        .get(capability_token.trim())
        .cloned()
        .ok_or_else(|| "parent agent capability is no longer active".to_string())?;
    if state
        .subagent_capabilities
        .lock()
        .values()
        .filter(|grant| {
            grant.parent_request_id == parent_grant.request_id
                && !grant.cancelled.load(Ordering::SeqCst)
        })
        .count()
        >= MAX_CONCURRENT_SUBAGENTS_PER_PARENT
    {
        return Err("subagent concurrency limit reached for this parent turn".to_string());
    }
    let task_id = format!("worktree-{}", Uuid::new_v4());
    let created_at = now_ms();
    let agent_id = agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("agent-{}", Uuid::new_v4().simple()));
    let private_context = state.subagent_tasks.prepare_agent(
        &parent_grant.session_id,
        &agent_id,
        "worktree",
        resume.unwrap_or(false),
        created_at,
    )?;
    let task_record = state.subagent_tasks.create_task(NewSubagentTask {
        id: task_id.clone(),
        session_id: parent_grant.session_id.clone(),
        agent_id: agent_id.clone(),
        parent_turn_id: parent_grant.turn_id.clone(),
        parent_request_id: parent_grant.request_id.clone(),
        title,
        created_at,
    })?;
    let storage_root = worktree_runtime::managed_storage_root();
    let lease = match worktree_runtime::provision_isolated_worktree(
        &storage_root,
        &task_id,
        &canonical_parent_workdir,
    ) {
        Ok(lease) => lease,
        Err(error) => {
            let _ = state.subagent_tasks.finish_task(
                &task_id,
                SubagentTaskStatus::Failed,
                None,
                Some("worktree_provision_failed".to_string()),
                now_ms(),
            );
            return Err(error);
        }
    };
    if let Err(error) = state.subagent_tasks.save_worktree(
        &task_id,
        &lease.repository_root,
        &lease.worktree_path,
        &lease.base_commit,
        now_ms(),
    ) {
        let _ = worktree_runtime::discard_isolated_worktree(&storage_root, &lease);
        return Err(error);
    }
    if let Err(error) = state.subagent_tasks.mark_running(&task_id, now_ms()) {
        let _ = worktree_runtime::discard_isolated_worktree(&storage_root, &lease);
        return Err(error);
    }
    let child_workdir = canonical_workdir(&lease.worktree_path)?;
    let child_capability_token = format!("subcap-{}", Uuid::new_v4());
    let proxy_request_id = subagent_proxy_request_id(SubagentCapabilityMode::Worktree, &task_id);
    state.subagent_capabilities.lock().insert(
        child_capability_token.clone(),
        SubagentCapabilityGrant {
            task_id: task_id.clone(),
            session_id: parent_grant.session_id,
            parent_turn_id: parent_grant.turn_id,
            parent_request_id: parent_grant.request_id,
            proxy_request_id: proxy_request_id.clone(),
            workdir: child_workdir,
            mode: SubagentCapabilityMode::Worktree,
            allow_global_read,
            expires_at: Instant::now() + Duration::from_secs(20 * 60),
            cancelled: Arc::new(AtomicBool::new(false)),
        },
    );
    let task = state
        .subagent_tasks
        .get_task(&task_record.session_id, &task_id)?
        .ok_or_else(|| "started worktree task is unavailable".to_string())?;
    emit_subagent_task_update(&app, &task)?;
    Ok(WorktreeTaskStartResponse {
        task,
        capability_token: child_capability_token,
        proxy_request_id,
        workdir: lease.worktree_path,
        base_commit: lease.base_commit,
        agent_id,
        private_context,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeTaskReviewResponse {
    pub task: StoredSubagentTask,
    pub digest: String,
    pub changed_paths: Vec<String>,
}

#[cfg(feature = "desktop")]
#[tauri::command(rename_all = "camelCase")]
pub fn worktree_task_finish(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    task_id: String,
    capability_token: String,
) -> Result<WorktreeTaskReviewResponse, String> {
    state.require_persistence_ready()?;
    let grant = require_subagent_capability(&state, &task_id, &capability_token)?;
    if grant.mode != SubagentCapabilityMode::Worktree {
        return Err("subagent task does not own an isolated worktree".to_string());
    }
    let stored_worktree = state
        .subagent_tasks
        .get_worktree(&grant.task_id)?
        .ok_or_else(|| "subagent worktree metadata is unavailable".to_string())?;
    let lease = WorktreeLease {
        task_id: grant.task_id.clone(),
        repository_root: stored_worktree.repository_root,
        worktree_path: stored_worktree.worktree_path,
        base_commit: stored_worktree.base_commit,
    };
    let patch =
        worktree_runtime::collect_review_patch(&worktree_runtime::managed_storage_root(), &lease)?;
    // Review-ready is terminal for the child model. Revoke before committing
    // that state so an issued loopback token cannot race the transition.
    grant.cancelled.store(true, Ordering::SeqCst);
    state.subagent_tasks.mark_review_ready(
        &grant.task_id,
        &patch.digest,
        &patch.changed_paths,
        now_ms(),
    )?;
    // Review-ready is terminal for the child model; remove its scoped
    // capability after the already-revoked durable transition.
    state
        .subagent_capabilities
        .lock()
        .remove(capability_token.trim());
    let task = state
        .subagent_tasks
        .get_task(&grant.session_id, &grant.task_id)?
        .ok_or_else(|| "review-ready worktree task is unavailable".to_string())?;
    emit_subagent_task_update(&app, &task)?;
    Ok(WorktreeTaskReviewResponse {
        task,
        digest: patch.digest,
        changed_paths: patch.changed_paths,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn worktree_task_review_get(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    task_id: String,
) -> Result<WorktreePatch, String> {
    let task = state
        .subagent_tasks
        .get_task(session_id.trim(), task_id.trim())?
        .ok_or_else(|| "subagent task not found".to_string())?;
    if task.status != SubagentTaskStatus::ReviewReady {
        return Err("subagent task is not awaiting patch review".to_string());
    }
    let worktree = state
        .subagent_tasks
        .get_worktree(&task.id)?
        .ok_or_else(|| "subagent worktree metadata is unavailable".to_string())?;
    let digest = worktree
        .patch_digest
        .ok_or_else(|| "subagent patch has not been collected".to_string())?;
    worktree_runtime::load_review_patch(
        &worktree_runtime::managed_storage_root(),
        &task.id,
        &digest,
        &worktree.base_commit,
    )
}

#[cfg(feature = "desktop")]
#[tauri::command(rename_all = "camelCase")]
pub fn worktree_task_apply(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    session_id: String,
    task_id: String,
    digest: String,
) -> Result<StoredSubagentTask, String> {
    state.require_persistence_ready()?;
    let task = state
        .subagent_tasks
        .get_task(session_id.trim(), task_id.trim())?
        .ok_or_else(|| "subagent task not found".to_string())?;
    if task.status != SubagentTaskStatus::ReviewReady {
        return Err("subagent task is not awaiting patch application".to_string());
    }
    let worktree = state
        .subagent_tasks
        .get_worktree(&task.id)?
        .ok_or_else(|| "subagent worktree metadata is unavailable".to_string())?;
    worktree_runtime::apply_reviewed_patch(
        &worktree_runtime::managed_storage_root(),
        &task.id,
        Path::new(&worktree.repository_root),
        &worktree.base_commit,
        &digest,
    )?;
    // Applying a patch never force-removes its detached checkout. The user
    // must explicitly clean it up after the native apply confirmation.
    state.subagent_tasks.mark_cleanup_pending(
        &task.id,
        WorktreeCleanupDisposition::PatchApplied,
        now_ms(),
    )?;
    let applied = state
        .subagent_tasks
        .get_task(&task.session_id, &task.id)?
        .ok_or_else(|| "applied worktree cleanup state is unavailable".to_string())?;
    emit_subagent_task_update(&app, &applied)?;
    Ok(applied)
}

/// Explicitly discard a reviewed worktree, or clean up a worktree after its
/// reviewed patch was applied. Native metadata determines every filesystem
/// path; renderer input only identifies the stored task.
#[cfg(feature = "desktop")]
#[tauri::command(rename_all = "camelCase")]
pub fn worktree_task_discard(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    session_id: String,
    task_id: String,
) -> Result<StoredSubagentTask, String> {
    state.require_persistence_ready()?;
    let task = state
        .subagent_tasks
        .get_task(session_id.trim(), task_id.trim())?
        .ok_or_else(|| "subagent task not found".to_string())?;
    let terminal_status = match task.status {
        SubagentTaskStatus::ReviewReady => SubagentTaskStatus::Cancelled,
        SubagentTaskStatus::CleanupPending => {
            worktree_cleanup_terminal_status(task.failure_code.as_deref())
        }
        _ => return Err("subagent task is not awaiting explicit worktree cleanup".to_string()),
    };
    let worktree = state
        .subagent_tasks
        .get_worktree(&task.id)?
        .ok_or_else(|| "subagent worktree metadata is unavailable".to_string())?;
    let lease = WorktreeLease {
        task_id: task.id.clone(),
        repository_root: worktree.repository_root,
        worktree_path: worktree.worktree_path,
        base_commit: worktree.base_commit,
    };
    if let Err(error) = worktree_runtime::discard_reviewed_worktree(
        &worktree_runtime::managed_storage_root(),
        &lease,
    ) {
        // Declining the native confirmation leaves a review-ready task
        // reviewable. Filesystem cleanup failures become explicitly
        // retryable, but never claim that the worktree was removed.
        if error != "user_cancelled" && task.status == SubagentTaskStatus::ReviewReady {
            let _ = state.subagent_tasks.mark_cleanup_pending(
                &task.id,
                WorktreeCleanupDisposition::ChildCancelled,
                now_ms(),
            );
        }
        if let Some(pending) = state.subagent_tasks.get_task(&task.session_id, &task.id)? {
            let _ = emit_subagent_task_update(&app, &pending);
        }
        return Err(error);
    }
    state.subagent_tasks.remove_worktree(&task.id)?;
    let terminal_failure_code = if terminal_status == SubagentTaskStatus::Failed {
        Some("runtime_failed".to_string())
    } else {
        None
    };
    state.subagent_tasks.finish_task(
        &task.id,
        terminal_status,
        None,
        terminal_failure_code,
        now_ms(),
    )?;
    let completed = state
        .subagent_tasks
        .get_task(&task.session_id, &task.id)?
        .ok_or_else(|| "cleaned worktree task is unavailable".to_string())?;
    emit_subagent_task_update(&app, &completed)?;
    Ok(completed)
}

fn emit_subagent_task_update<R: tauri::Runtime>(
    app: &AppHandle<R>,
    task: &StoredSubagentTask,
) -> Result<(), String> {
    app.emit("subagent:task-update", task)
        .map_err(|error| format!("emit subagent task update: {error}"))
}

fn emit_subagent_message<R: tauri::Runtime>(
    app: &AppHandle<R>,
    message: &StoredSubagentMessage,
) -> Result<(), String> {
    app.emit("subagent:message", message)
        .map_err(|error| format!("emit subagent message: {error}"))
}

fn require_subagent_capability(
    state: &AppState,
    task_id: &str,
    capability_token: &str,
) -> Result<SubagentCapabilityGrant, String> {
    let token = capability_token.trim();
    let task_id = task_id.trim();
    if token.is_empty() || task_id.is_empty() {
        return Err("subagent task capability is required".to_string());
    }
    let grant = state
        .subagent_capabilities
        .lock()
        .get(token)
        .cloned()
        .ok_or_else(|| "subagent task capability is invalid or expired".to_string())?;
    if grant.task_id != task_id
        || grant.expires_at <= Instant::now()
        || grant.cancelled.load(Ordering::SeqCst)
    {
        return Err("subagent task capability no longer authorizes this operation".to_string());
    }
    let parent_active = state
        .active_runs
        .lock()
        .get(&grant.parent_request_id)
        .is_some_and(|parent| {
            parent.session_id == grant.session_id && parent.turn_id == grant.parent_turn_id
        });
    if !parent_active {
        return Err("subagent parent run is no longer active".to_string());
    }
    Ok(grant)
}

fn cancel_subagent_tasks_for_parent<R: tauri::Runtime>(
    state: &AppState,
    app: &AppHandle<R>,
    parent_request_id: &str,
    cancelled_at: i64,
) {
    let matching = state
        .subagent_capabilities
        .lock()
        .iter()
        .filter(|(_, grant)| grant.parent_request_id == parent_request_id)
        .map(|(token, grant)| (token.clone(), grant.clone()))
        .collect::<Vec<_>>();
    for (token, grant) in matching {
        grant.cancelled.store(true, Ordering::SeqCst);
        let task_cancelled = if grant.mode == SubagentCapabilityMode::Worktree {
            state
                .subagent_tasks
                .mark_cleanup_pending(
                    &grant.task_id,
                    WorktreeCleanupDisposition::ChildCancelled,
                    cancelled_at,
                )
                .is_ok()
        } else {
            state
                .subagent_tasks
                .cancel_task(&grant.task_id, cancelled_at)
                .is_ok()
        };
        state.subagent_capabilities.lock().remove(&token);
        if task_cancelled {
            if let Ok(Some(task)) = state
                .subagent_tasks
                .get_task(&grant.session_id, &grant.task_id)
            {
                let _ = emit_subagent_task_update(app, &task);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Workspace/file commands

fn canonical_workdir(raw: &str) -> Result<PathBuf, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("workdir is required".to_string());
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!("workdir must be absolute: {value}"));
    }
    let canonical = fs::canonicalize(&path)
        .map_err(|error| format!("workdir is not accessible: {value}: {error}"))?;
    if !canonical.is_dir() {
        return Err(format!("workdir is not a directory: {value}"));
    }
    Ok(canonical)
}

fn unavailable_workspace_path_status(path: String) -> WorkspacePathStatus {
    WorkspacePathStatus {
        path,
        accessible: false,
        reason: Some("unavailable".to_string()),
    }
}

/// Probe a path already present in local project/history metadata. This is
/// intentionally separate from `canonical_workdir`: the whole point is to
/// classify paths which can no longer be canonicalized.
fn workspace_path_status(raw: &str) -> WorkspacePathStatus {
    let path = path_string_for_display(raw);
    if historical_workspace_path_display(raw).is_err() {
        return unavailable_workspace_path_status(path);
    }
    // Match the authority path used by `canonical_workdir`: metadata alone can
    // still succeed for a stale mount or a directory the process can no longer
    // traverse. Probe canonicalization first so usable paths do not pay for a
    // redundant metadata syscall; only classify the failed probe afterward.
    match fs::canonicalize(Path::new(raw.trim())) {
        Ok(canonical) if canonical.is_dir() => WorkspacePathStatus {
            path,
            accessible: true,
            reason: None,
        },
        Ok(_) => WorkspacePathStatus {
            path,
            accessible: false,
            reason: Some("not_directory".to_string()),
        },
        Err(_) => match fs::metadata(Path::new(raw.trim())) {
            Ok(metadata) if metadata.is_dir() => unavailable_workspace_path_status(path),
            Ok(_) => WorkspacePathStatus {
                path,
                accessible: false,
                reason: Some("not_directory".to_string()),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => WorkspacePathStatus {
                path,
                accessible: false,
                reason: Some("missing".to_string()),
            },
            Err(_) => unavailable_workspace_path_status(path),
        },
    }
}

/// Return only paths that are already persisted in local history/projects.
/// This keeps the status API from becoming a general filesystem-existence
/// oracle for arbitrary renderer-provided paths.
fn known_workspace_paths(state: &AppState) -> HashMap<String, String> {
    let mut paths = state
        .sessions
        .lock()
        .values()
        .filter_map(|record| {
            workspace_path_key(&record.summary.cwd).map(|key| (key, record.summary.cwd.clone()))
        })
        .collect::<HashMap<_, _>>();
    let projects = project_metadata_from_value(state.settings.lock().get("projects"));
    for project in projects {
        if let Some(key) = workspace_path_key(&project.path) {
            paths.entry(key).or_insert(project.path);
        }
    }
    paths
}

/// Check the current filesystem state of known historical workspaces. The
/// response has no raw platform errors, so a disconnected drive is visibly
/// distinguishable without leaking user or machine-specific failure details.
#[tauri::command(rename_all = "camelCase")]
pub fn workspace_paths_status(
    state: State<'_, Arc<AppState>>,
    paths: Vec<String>,
) -> Result<WorkspacePathsStatusResponse, String> {
    if paths.len() > MAX_WORKSPACE_STATUS_PATHS {
        return Err(format!(
            "check at most {MAX_WORKSPACE_STATUS_PATHS} workspace paths at a time"
        ));
    }
    let known = known_workspace_paths(&state);
    let mut cache = HashMap::<String, WorkspacePathStatus>::new();
    let paths = paths
        .into_iter()
        .map(|requested| {
            let requested_display = path_string_for_display(&requested);
            let Some(key) = workspace_path_key(&requested) else {
                return WorkspacePathStatus {
                    path: requested_display,
                    accessible: false,
                    reason: Some("unavailable".to_string()),
                };
            };
            let Some(known_path) = known.get(&key) else {
                // Do not probe a path the renderer invented. It may still
                // render as unavailable, which is safe and useful for a
                // stale client cache while history reloads.
                return WorkspacePathStatus {
                    path: requested_display,
                    accessible: false,
                    reason: Some("unavailable".to_string()),
                };
            };
            let mut status = cache
                .entry(key)
                .or_insert_with(|| workspace_path_status(known_path))
                .clone();
            status.path = requested_display;
            status
        })
        .collect();
    Ok(WorkspacePathsStatusResponse { paths })
}

fn workdir_is_approved(approved: &HashSet<PathBuf>, workdir: &Path) -> bool {
    approved.contains(workdir)
}

fn require_approved_workdir(state: &AppState, workdir: &Path) -> Result<(), String> {
    if workdir_is_approved(&state.approved_workdirs.lock(), workdir) {
        Ok(())
    } else {
        Err("workdir has not been approved with the native folder picker".to_string())
    }
}

/// Check that a relocation picker was launched for a path already present in
/// local history/projects. This remains a metadata lookup only; it cannot
/// make an arbitrary renderer string eligible for a native authority grant.
fn known_relocation_source_key(state: &AppState, raw: &str) -> Result<String, String> {
    let source = historical_workspace_path_display(raw)?;
    let key = workspace_path_key(&source)
        .ok_or_else(|| "source workspace path is invalid".to_string())?;
    if !known_workspace_paths(state).contains_key(&key) {
        return Err("source workspace is not known to local history or projects".to_string());
    }
    Ok(key)
}

fn issue_relocation_picker_grant(state: &AppState, from_key: String, to_workdir: PathBuf) {
    // A new native selection supersedes any unresolved confirmation for the
    // same source, even if it happens to select the same destination again.
    state
        .relocation_conflict_grants
        .lock()
        .retain(|_, grant| grant.from_key != from_key);
    let mut grants = state.relocation_picker_grants.lock();
    let now = Instant::now();
    grants.retain(|_, grant| grant.expires_at > now);
    // One source has at most one active picker result. A newer explicit
    // selection supersedes an earlier one rather than allowing ambiguity.
    grants.insert(
        from_key,
        RelocationPickerGrant {
            to_workdir,
            expires_at: now + RELOCATION_PICKER_GRANT_TTL,
        },
    );
}

/// Verify a source-bound picker result without consuming it. Conflict UI must
/// be able to round-trip through the renderer while the native selection stays
/// available for the eventual successful mutation.
fn require_relocation_picker_grant(
    state: &AppState,
    from_key: &str,
    to_workdir: &Path,
) -> Result<(), String> {
    let mut grants = state.relocation_picker_grants.lock();
    let now = Instant::now();
    grants.retain(|_, grant| grant.expires_at > now);
    let grant = grants.get(from_key).ok_or_else(|| {
        "relocation requires a fresh native folder selection for this source workspace".to_string()
    })?;
    if grant.to_workdir != to_workdir {
        return Err(
            "the selected replacement folder does not match this relocation request".to_string(),
        );
    }
    Ok(())
}

/// Consume the source-bound native picker result. This is intentionally
/// separate from `approved_workdirs`: a previously approved target is never
/// enough to relocate a durable historical binding.
fn consume_relocation_picker_grant(
    state: &AppState,
    from_key: &str,
    to_workdir: &Path,
) -> Result<(), String> {
    let mut grants = state.relocation_picker_grants.lock();
    let now = Instant::now();
    grants.retain(|_, grant| grant.expires_at > now);
    let grant = grants.remove(from_key).ok_or_else(|| {
        "relocation requires a fresh native folder selection for this source workspace".to_string()
    })?;
    if grant.to_workdir != to_workdir {
        return Err(
            "the selected replacement folder does not match this relocation request".to_string(),
        );
    }
    Ok(())
}

fn issue_relocation_conflict_grant(
    state: &AppState,
    from_key: String,
    to_workdir: PathBuf,
    conflict: &WorkspaceRelocationConflict,
) -> Result<String, String> {
    let now = Instant::now();
    let mut grants = state.relocation_conflict_grants.lock();
    grants.retain(|_, grant| grant.expires_at > now && grant.from_key != from_key);
    if grants.len() >= MAX_PENDING_RELOCATION_CONFLICT_GRANTS {
        return Err("too many pending workspace relocation confirmations".to_string());
    }
    let token = format!("relocation-conflict-{}", Uuid::new_v4());
    grants.insert(
        token.clone(),
        RelocationConflictGrant {
            from_key,
            to_workdir,
            conflict: conflict.clone(),
            expires_at: now + RELOCATION_CONFLICT_GRANT_TTL,
        },
    );
    Ok(token)
}

fn revoke_matching_relocation_picker_grant(state: &AppState, from_key: &str, to_workdir: &Path) {
    let mut picker_grants = state.relocation_picker_grants.lock();
    let matches = picker_grants
        .get(from_key)
        .is_some_and(|grant| grant.to_workdir == to_workdir);
    if matches {
        picker_grants.remove(from_key);
    }
}

fn require_relocation_conflict_grant(
    state: &AppState,
    token: &str,
    from_key: &str,
    to_workdir: &Path,
    current_conflict: &WorkspaceRelocationConflict,
) -> Result<(), String> {
    let token = token.trim();
    if token.is_empty() || token.len() > 128 || !token.starts_with("relocation-conflict-") {
        return Err("workspace relocation conflict confirmation is invalid or expired".to_string());
    }
    let now = Instant::now();
    let mut grants = state.relocation_conflict_grants.lock();
    grants.retain(|_, grant| grant.expires_at > now);
    let grant = grants.get(token).ok_or_else(|| {
        "workspace relocation conflict confirmation is invalid or expired".to_string()
    })?;
    if grant.from_key != from_key
        || grant.to_workdir != to_workdir
        || grant.conflict != *current_conflict
    {
        return Err(
            "workspace relocation conflict changed; choose the replacement folder again"
                .to_string(),
        );
    }
    Ok(())
}

fn revoke_relocation_conflict_grant(state: &AppState, token: &str) {
    let now = Instant::now();
    let mut grants = state.relocation_conflict_grants.lock();
    let cancelled = grants.remove(token);
    grants.retain(|_, grant| grant.expires_at > now);
    drop(grants);
    if let Some(cancelled) = cancelled {
        revoke_matching_relocation_picker_grant(state, &cancelled.from_key, &cancelled.to_workdir);
    }
}

/// Consume one pending-conflict proof before any relocation mutation. A failed
/// comparison consumes the proof too, so changed project settings and replayed
/// renderer messages always fail closed.
fn consume_relocation_conflict_grant(
    state: &AppState,
    token: &str,
    from_key: &str,
    to_workdir: &Path,
    current_conflict: Option<&WorkspaceRelocationConflict>,
) -> Result<(), String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("workspace relocation conflict confirmation is required".to_string());
    }
    if token.len() > 128 || !token.starts_with("relocation-conflict-") {
        return Err("workspace relocation conflict confirmation is invalid or expired".to_string());
    }
    let now = Instant::now();
    let mut grants = state.relocation_conflict_grants.lock();
    grants.retain(|_, grant| grant.expires_at > now);
    let grant = grants.remove(token);
    drop(grants);
    let Some(grant) = grant else {
        revoke_matching_relocation_picker_grant(state, from_key, to_workdir);
        return Err("workspace relocation conflict confirmation is invalid or expired".to_string());
    };
    let conflict_changed = grant.from_key != from_key
        || grant.to_workdir != to_workdir
        || current_conflict != Some(&grant.conflict);
    if conflict_changed {
        // A stale confirmation must not leave its older native picker proof
        // available for an immediate retry that claims the user reselected it.
        revoke_matching_relocation_picker_grant(state, &grant.from_key, &grant.to_workdir);
        return Err(
            "workspace relocation conflict changed; choose the replacement folder again"
                .to_string(),
        );
    }
    Ok(())
}

/// Explicitly revoke both the pending conflict proof and its still-pending
/// picker proof when a choice dialog is dismissed. Cancellation is idempotent
/// and intentionally does not require a writable persistence store because it
/// only removes process-local authority.
#[tauri::command(rename_all = "camelCase")]
pub fn sessions_relocate_workspace_cancel(
    state: State<'_, Arc<AppState>>,
    conflict_token: String,
) -> Result<(), String> {
    let token = conflict_token.trim();
    if token.len() > 128 || !token.starts_with("relocation-conflict-") {
        return Err("workspace relocation conflict confirmation is invalid or expired".to_string());
    }
    revoke_relocation_conflict_grant(&state, token);
    Ok(())
}

/// Resolve a project root for a user-triggered file-manager reveal.  This is
/// deliberately an exact approved-root check: the renderer cannot turn this
/// convenience action into a general path-opening capability.
fn approved_workspace_reveal_target(
    state: &AppState,
    raw: &str,
) -> Result<(PathBuf, String), String> {
    let canonical = canonical_workdir(raw)?;
    require_registered_project_workdir(state, &canonical)?;
    Ok((canonical.clone(), path_for_display(&canonical)))
}

fn reveal_workspace_in_file_manager(workdir: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer.exe")
            .arg(path_for_display(workdir))
            .spawn()
            .map(|_| ())
            // Do not reflect a platform/path error through the WebView. The
            // caller only needs one safe recovery action: try again after the
            // project's location and local file-manager availability are fixed.
            .map_err(|_| "could not open the project folder in the system file manager".to_string())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(workdir)
            .spawn()
            .map(|_| ())
            .map_err(|_| "could not open the project folder in the system file manager".to_string())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(workdir)
            .spawn()
            .map(|_| ())
            .map_err(|_| "could not open the project folder in the system file manager".to_string())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    {
        let _ = workdir;
        Err("opening the system file manager is not supported on this platform".to_string())
    }
}

/// Open an already-approved project root in the operating-system file manager.
/// This does not grant a new filesystem capability and never accepts a
/// renderer-invented path.
#[tauri::command(rename_all = "camelCase")]
pub fn workspace_reveal(
    state: State<'_, Arc<AppState>>,
    workdir: String,
) -> Result<String, String> {
    let (canonical, displayed) = approved_workspace_reveal_target(&state, &workdir)?;
    reveal_workspace_in_file_manager(&canonical)?;
    Ok(displayed)
}

/// Open the operating-system folder picker and register the exact selected
/// directory as a workspace root. The only supported policy is an exact
/// project root; renderer-provided paths never grant access, and a picked
/// folder later becomes a separate session's root rather than an extra path
/// on an existing capability. When `relocation_from` is supplied, the same
/// native selection also mints a short-lived, source-bound relocation proof;
/// callers must still submit it to `sessions_relocate_workspace` immediately.
#[tauri::command(rename_all = "camelCase")]
pub async fn workspace_pick(
    state: State<'_, Arc<AppState>>,
    start_dir: Option<String>,
    relocation_from: Option<String>,
) -> Result<Option<String>, String> {
    require_project_root_workdir_policy(&state)?;
    let relocation_source_key = relocation_from
        .as_deref()
        .map(|source| known_relocation_source_key(&state, source))
        .transpose()?;
    let start_dir = start_dir
        .as_deref()
        .and_then(|value| canonical_workdir(value).ok())
        .filter(|path| state.approved_workdirs.lock().contains(path));

    let selected = tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new().set_title("Open project folder");
        if let Some(start_dir) = start_dir {
            dialog = dialog.set_directory(start_dir);
        }
        dialog.pick_folder()
    })
    .await
    .map_err(|error| format!("open workspace picker: {error}"))?;

    let Some(selected) = selected else {
        return Ok(None);
    };
    let canonical = canonical_workdir(&selected.display().to_string())?;
    // Settings may have changed while the native dialog was open. Re-check
    // before turning a user selection into a process-local approved root.
    require_project_root_workdir_policy(&state)?;
    state.picker_workdirs.lock().insert(canonical.clone());
    state.approved_workdirs.lock().insert(canonical.clone());
    if let Some(from_key) = relocation_source_key {
        issue_relocation_picker_grant(&state, from_key, canonical.clone());
    }
    Ok(Some(path_for_display(&canonical)))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsRelocateWorkspaceResponse {
    pub status: WorkspaceRelocationStatus,
    pub from_workdir: String,
    pub to_workdir: String,
    pub updated_session_ids: Vec<String>,
    pub updated_sessions: Vec<SessionSummary>,
    pub updated_project_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict: Option<WorkspaceRelocationConflict>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_token: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRelocationStatus {
    Conflict,
    Relocated,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRelocationConflictResolution {
    /// Preserve the historical/source project's stable metadata and remove the
    /// destination project record before binding that root to the source.
    KeepSource,
    /// Preserve the already-registered destination project and move the
    /// source workspace's sessions into it.
    MergeIntoTarget,
}

fn native_relocation_conflict_confirmation_description(
    from_workdir: &str,
    to_workdir: &str,
    conflict: &WorkspaceRelocationConflict,
    resolution: WorkspaceRelocationConflictResolution,
) -> Result<String, String> {
    match resolution {
        WorkspaceRelocationConflictResolution::KeepSource => {
            let source = conflict.source_project.as_ref().ok_or_else(|| {
                "cannot keep the source project because the historical workspace is not registered"
                    .to_string()
            })?;
            Ok(format!(
                "Keep the historical NovaVei project and replace the existing destination project?\n\nHistorical project: {}\nHistorical path: {}\n\nDestination project to remove: {}\nDestination path: {}\n\nThe historical sessions will be rebound to:\n{}\n\nChoose OK only to keep the historical project's name and settings. The destination project's registration will be removed; transcript history is not deleted.",
                source.name, from_workdir, conflict.target_project.name, to_workdir, to_workdir
            ))
        }
        WorkspaceRelocationConflictResolution::MergeIntoTarget => Ok(format!(
            "Merge the historical NovaVei sessions into the existing destination project?\n\nHistorical path: {}\n\nDestination project to keep: {}\nDestination path: {}\n\nThe historical sessions will be rebound to this destination. {} Transcript history is not deleted.",
            from_workdir,
            conflict.target_project.name,
            to_workdir,
            conflict
                .source_project
                .as_ref()
                .map(|source| format!(
                    "The historical project registration \"{}\" will be removed.",
                    source.name
                ))
                .unwrap_or_default(),
        )),
    }
}

/// The renderer can request a resolution but cannot authorize it. The operating
/// system dialog repeats the exact destructive effect immediately before the
/// token is consumed. Unit tests deliberately bypass the platform UI so the
/// snapshot/replay tests can exercise the native state machine.
fn request_native_relocation_conflict_confirmation(
    from_workdir: &str,
    to_workdir: &str,
    conflict: &WorkspaceRelocationConflict,
    resolution: WorkspaceRelocationConflictResolution,
) -> Result<(), String> {
    let description = native_relocation_conflict_confirmation_description(
        from_workdir,
        to_workdir,
        conflict,
        resolution,
    )?;
    #[cfg(all(windows, not(test)))]
    {
        let response = rfd::MessageDialog::new()
            .set_title("Confirm NovaVei workspace relocation")
            .set_description(description)
            .set_buttons(rfd::MessageButtons::OkCancel)
            .show();
        if response == rfd::MessageDialogResult::Ok {
            Ok(())
        } else {
            Err("user_cancelled".to_string())
        }
    }
    #[cfg(test)]
    {
        let _ = description;
        Ok(())
    }
    #[cfg(all(not(windows), not(test)))]
    {
        let _ = description;
        Err("native workspace relocation confirmation is unavailable on this platform".to_string())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRelocationConflictProject {
    pub id: String,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRelocationConflict {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_project: Option<WorkspaceRelocationConflictProject>,
    pub target_project: WorkspaceRelocationConflictProject,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceProjectRegistrationResponse {
    pub created: bool,
    pub project: Value,
}

fn session_summary_value(record: &SessionRecord) -> SessionSummary {
    SessionSummary {
        message_count: record.message_count,
        is_pinned: record.pinned_at.is_some(),
        is_archived: record.archived_at.is_some(),
        ..record.summary.clone()
    }
}

struct ProjectRelocationPreparation {
    projects_payload: Option<Value>,
    updated_project_ids: Vec<String>,
    conflict: Option<WorkspaceRelocationConflict>,
}

fn relocation_conflict_project(
    entry: &Map<String, Value>,
) -> Result<WorkspaceRelocationConflictProject, String> {
    let id = entry
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| valid_project_id(id))
        .ok_or_else(|| "project id is invalid".to_string())?;
    let name = entry
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| bounded_project_text(name, 200))
        .ok_or_else(|| "project name is invalid".to_string())?;
    let path = entry
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "project path is required".to_string())?;
    Ok(WorkspaceRelocationConflictProject {
        id: id.to_string(),
        name: name.to_string(),
        path: historical_workspace_path_display(path)?,
    })
}

fn project_settings_payload_for_relocation(
    state: &AppState,
    from_key: &str,
    to_workdir: &str,
    to_key: &str,
    conflict_resolution: Option<WorkspaceRelocationConflictResolution>,
) -> Result<ProjectRelocationPreparation, String> {
    let current = state
        .settings
        .lock()
        .get("projects")
        .cloned()
        .unwrap_or_else(|| default_setting("projects", &current_workdir()));
    let object = current
        .as_object()
        .ok_or_else(|| "projects settings are invalid".to_string())?;
    let entries = object
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| "projects settings are invalid".to_string())?;

    let mut source_index = None;
    let mut target_index = None;
    let mut parsed_entries = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let entry = entry
            .as_object()
            .cloned()
            .ok_or_else(|| "project entry is invalid".to_string())?;
        let raw_path = entry
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| "project path is required".to_string())?;
        let path_key =
            workspace_path_key(raw_path).ok_or_else(|| "project path is invalid".to_string())?;
        if path_key == from_key {
            source_index = Some(index);
        } else if path_key == to_key {
            target_index = Some(index);
        }
        parsed_entries.push(entry);
    }

    if let Some(target_index) = target_index {
        let conflict = WorkspaceRelocationConflict {
            source_project: source_index
                .map(|index| relocation_conflict_project(&parsed_entries[index]))
                .transpose()?,
            target_project: relocation_conflict_project(&parsed_entries[target_index])?,
        };
        if conflict_resolution.is_none() {
            return Ok(ProjectRelocationPreparation {
                projects_payload: None,
                updated_project_ids: Vec::new(),
                conflict: Some(conflict),
            });
        }
    }

    if conflict_resolution == Some(WorkspaceRelocationConflictResolution::KeepSource)
        && source_index.is_none()
    {
        return Err(
            "cannot keep the source project because the historical workspace is not registered"
                .to_string(),
        );
    }
    if conflict_resolution == Some(WorkspaceRelocationConflictResolution::MergeIntoTarget)
        && target_index.is_none()
    {
        return Err("the destination project changed; choose the relocation again".to_string());
    }

    let mut moved_project_ids = Vec::new();
    let mut updated_entries = Vec::with_capacity(parsed_entries.len());
    for (index, mut entry) in parsed_entries.into_iter().enumerate() {
        if target_index == Some(index)
            && conflict_resolution == Some(WorkspaceRelocationConflictResolution::KeepSource)
        {
            continue;
        }
        if source_index == Some(index)
            && conflict_resolution == Some(WorkspaceRelocationConflictResolution::MergeIntoTarget)
        {
            continue;
        }
        if source_index == Some(index) {
            let id = entry
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| valid_project_id(id))
                .ok_or_else(|| "project id is invalid".to_string())?
                .to_string();
            entry.insert("path".to_string(), Value::String(to_workdir.to_string()));
            moved_project_ids.push(id);
        } else if target_index == Some(index)
            && conflict_resolution == Some(WorkspaceRelocationConflictResolution::MergeIntoTarget)
        {
            let id = entry
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| valid_project_id(id))
                .ok_or_else(|| "project id is invalid".to_string())?
                .to_string();
            moved_project_ids.push(id);
        }
        updated_entries.push(Value::Object(entry));
    }
    Ok(ProjectRelocationPreparation {
        projects_payload: Some(json!({
            "version": PROJECT_SETTINGS_VERSION,
            "initialized": object
                .get("initialized")
                .and_then(Value::as_bool)
                .unwrap_or(!updated_entries.is_empty()),
            "entries": updated_entries,
        })),
        updated_project_ids: moved_project_ids,
        conflict: None,
    })
}

/// Rebind every historical session using one old workspace string to a folder
/// the user just selected through the native picker. Transcript rows keep
/// their IDs and contents; only the durable workspace metadata changes.
#[tauri::command(rename_all = "camelCase")]
pub fn sessions_relocate_workspace(
    state: State<'_, Arc<AppState>>,
    from_workdir: String,
    to_workdir: String,
    conflict_resolution: Option<WorkspaceRelocationConflictResolution>,
    conflict_token: Option<String>,
) -> Result<SessionsRelocateWorkspaceResponse, String> {
    state.require_persistence_ready()?;
    let from_workdir = historical_workspace_path_display(&from_workdir)?;
    let from_key = workspace_path_key(&from_workdir)
        .ok_or_else(|| "source workspace path is invalid".to_string())?;
    let to_canonical = canonical_workdir(&to_workdir)?;
    require_approved_workdir(&state, &to_canonical)?;
    let to_workdir = path_for_display(&to_canonical);
    let to_key = workspace_path_key(&to_workdir)
        .ok_or_else(|| "replacement workspace path is invalid".to_string())?;
    if from_key == to_key {
        return Err(
            "the replacement workspace is the same as the historical workspace".to_string(),
        );
    }

    // A DOM choice is useful for explaining the two outcomes, but it cannot
    // grant a destructive project mutation. Before consuming the renderer-held
    // pending-conflict token, prove that it still matches the native snapshot
    // and ask the operating system to confirm the exact requested outcome.
    // Keep the persistence lock out of the modal lifetime, then re-check and
    // consume the same snapshot under the lock below.
    if let (Some(resolution), Some(token)) = (conflict_resolution, conflict_token.as_deref()) {
        let pending_conflict = {
            let _persist_guard = state.persist_lock.lock();
            project_settings_payload_for_relocation(&state, &from_key, &to_workdir, &to_key, None)?
                .conflict
        };
        if let Some(conflict) = pending_conflict {
            require_relocation_conflict_grant(&state, token, &from_key, &to_canonical, &conflict)?;
            if let Err(error) = request_native_relocation_conflict_confirmation(
                &from_workdir,
                &to_workdir,
                &conflict,
                resolution,
            ) {
                // A rejected or unavailable native dialog must leave neither
                // the conflict proof nor its picker proof available for a
                // renderer-only retry.
                revoke_relocation_conflict_grant(&state, token.trim());
                return Err(error);
            }
        }
    }

    let _persist_guard = state.persist_lock.lock();
    let initial_preparation =
        project_settings_payload_for_relocation(&state, &from_key, &to_workdir, &to_key, None)?;
    let preparation = match initial_preparation.conflict {
        Some(conflict) => match (conflict_resolution, conflict_token.as_deref()) {
            (None, None) => {
                require_relocation_picker_grant(&state, &from_key, &to_canonical)?;
                let conflict_token = issue_relocation_conflict_grant(
                    &state,
                    from_key.clone(),
                    to_canonical.clone(),
                    &conflict,
                )?;
                return Ok(SessionsRelocateWorkspaceResponse {
                    status: WorkspaceRelocationStatus::Conflict,
                    from_workdir,
                    to_workdir,
                    updated_session_ids: Vec::new(),
                    updated_sessions: Vec::new(),
                    updated_project_ids: Vec::new(),
                    conflict: Some(conflict),
                    conflict_token: Some(conflict_token),
                });
            }
            (Some(resolution), Some(token)) => {
                consume_relocation_conflict_grant(
                    &state,
                    token,
                    &from_key,
                    &to_canonical,
                    Some(&conflict),
                )?;
                project_settings_payload_for_relocation(
                    &state,
                    &from_key,
                    &to_workdir,
                    &to_key,
                    Some(resolution),
                )?
            }
            (Some(_), None) => {
                return Err("workspace relocation conflict confirmation is required".to_string());
            }
            (None, Some(token)) => {
                consume_relocation_conflict_grant(
                    &state,
                    token,
                    &from_key,
                    &to_canonical,
                    Some(&conflict),
                )?;
                return Err("workspace relocation conflict resolution is required".to_string());
            }
        },
        None => {
            if let Some(token) = conflict_token.as_deref() {
                consume_relocation_conflict_grant(&state, token, &from_key, &to_canonical, None)?;
                return Err("workspace relocation conflict no longer exists".to_string());
            }
            if conflict_resolution.is_some() {
                return Err("workspace relocation conflict confirmation is required".to_string());
            }
            initial_preparation
        }
    };
    let projects_payload = preparation
        .projects_payload
        .ok_or_else(|| "workspace relocation plan is incomplete".to_string())?;
    let updated_project_ids = preparation.updated_project_ids;
    let previous_sessions = state
        .sessions
        .lock()
        .iter()
        .filter(|(_, record)| {
            workspace_path_key(&record.summary.cwd).as_deref() == Some(from_key.as_str())
        })
        .map(|(id, record)| (id.clone(), record.clone()))
        .collect::<Vec<_>>();
    if previous_sessions.is_empty() && updated_project_ids.is_empty() {
        return Err("no historical session or project uses the selected old workspace".to_string());
    }
    let affected_ids = previous_sessions
        .iter()
        .map(|(id, _)| id.clone())
        .collect::<HashSet<_>>();
    if state
        .active_runs
        .lock()
        .values()
        .any(|run| affected_ids.contains(&run.session_id))
    {
        return Err(
            "cannot relocate a workspace while one of its sessions has an active agent run"
                .to_string(),
        );
    }

    // Ambient approval merely permits future workspace access. Rebinding
    // durable historical records also requires a one-use native picker result
    // tied to this exact old path and canonical destination.
    consume_relocation_picker_grant(&state, &from_key, &to_canonical)?;

    let mutation_time = now_ms();
    {
        let mut sessions = state.sessions.lock();
        for (id, _) in &previous_sessions {
            let record = sessions
                .get_mut(id)
                .expect("validated relocation session must remain available");
            record.summary.cwd = to_workdir.clone();
            record.summary.updated_at =
                mutation_time.max(record.summary.updated_at.saturating_add(1));
        }
    }
    let normalized_projects = match normalize_projects_settings_payload(&state, projects_payload) {
        Ok(value) => value,
        Err(error) => {
            restore_history_records(&state, &previous_sessions);
            return Err(error);
        }
    };
    // The projects normalizer upgrades legacy path-derived IDs to stable
    // UUIDs. Report the post-normalization IDs, not the stale pre-migration
    // values captured while preparing the relocation payload.
    let updated_project_ids = if updated_project_ids.is_empty() {
        Vec::new()
    } else {
        project_metadata_from_value(Some(&normalized_projects))
            .into_iter()
            .filter(|project| workspace_path_key(&project.path).as_deref() == Some(to_key.as_str()))
            .map(|project| project.id)
            .collect()
    };
    let previous_projects = state
        .settings
        .lock()
        .insert("projects".to_string(), normalized_projects);
    let persistence = (|| -> Result<(), String> {
        let sessions = state.sessions.lock();
        let changed_sessions = previous_sessions
            .iter()
            .filter_map(|(id, _)| sessions.get(id).map(stored_session_from_record))
            .collect::<Vec<_>>();
        drop(sessions);
        let projects = state
            .settings
            .lock()
            .get("projects")
            .cloned()
            .ok_or_else(|| "projects settings are missing".to_string())?;
        let protected_projects =
            protect_settings(&HashMap::from([("projects".to_string(), projects)]))?;
        state
            .history
            .upsert_session_metadata_batch_and_settings(&changed_sessions, &protected_projects)
    })();
    if let Err(error) = persistence {
        restore_history_records(&state, &previous_sessions);
        let mut settings = state.settings.lock();
        if let Some(previous_projects) = previous_projects {
            settings.insert("projects".to_string(), previous_projects);
        } else {
            settings.remove("projects");
        }
        return Err(error);
    }

    refresh_approved_workdirs_after_project_save(&state);
    // A renamed/moved root must not leave stale File Dock or agent grants
    // usable for the old path. Active runs were rejected above, so this only
    // clears stale capabilities after the new durable binding succeeds.
    revoke_session_owned_access(&state, &affected_ids);
    let updated_sessions = {
        let sessions = state.sessions.lock();
        previous_sessions
            .iter()
            .filter_map(|(id, _)| sessions.get(id).map(session_summary_value))
            .collect::<Vec<_>>()
    };
    Ok(SessionsRelocateWorkspaceResponse {
        status: WorkspaceRelocationStatus::Relocated,
        from_workdir,
        to_workdir,
        updated_session_ids: previous_sessions.into_iter().map(|(id, _)| id).collect(),
        updated_sessions,
        updated_project_ids,
        conflict: None,
        conflict_token: None,
    })
}

fn default_project_name(workdir: &Path) -> String {
    workdir
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| bounded_project_text(name, 200))
        .map(str::to_string)
        .unwrap_or_else(|| "Project".to_string())
}

/// Turn a known workspace group into a durable project entry without asking
/// the user to select the same directory again. An existing picker/project
/// grant is accepted, or this explicit action may promote one currently
/// accessible historical path already known to local history. A renderer
/// cannot use it to register an arbitrary directory.
#[tauri::command(rename_all = "camelCase")]
pub fn workspace_register_project(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    name: Option<String>,
) -> Result<WorkspaceProjectRegistrationResponse, String> {
    state.require_persistence_ready()?;
    let canonical = canonical_workdir(&workdir)?;
    let workdir = path_for_display(&canonical);
    let workdir_key =
        workspace_path_key(&workdir).ok_or_else(|| "workspace path is invalid".to_string())?;
    let requested_name = match name {
        Some(name) => {
            let name = name.trim();
            if !bounded_project_text(name, 200) {
                return Err("project name is invalid".to_string());
            }
            name.to_string()
        }
        None => default_project_name(&canonical),
    };

    let _persist_guard = state.persist_lock.lock();
    // Recheck under the persistence gate so an erased/relocated historical
    // record cannot race this explicit promotion into a new authority root.
    let was_approved = workdir_is_approved(&state.approved_workdirs.lock(), &canonical);
    let known_historical = known_workspace_paths(&state).contains_key(&workdir_key);
    if !was_approved && !known_historical {
        return Err(
            "workspace is not a known historical path; select it with the native folder picker first"
                .to_string(),
        );
    }
    let existing = state
        .settings
        .lock()
        .get("projects")
        .cloned()
        .unwrap_or_else(|| default_setting("projects", &current_workdir()));
    let normalized_existing = normalize_projects_settings_payload_with_extra_approved(
        &state,
        existing,
        (!was_approved && known_historical).then_some(canonical.as_path()),
    )?;
    if let Some(project) = project_metadata_from_value(Some(&normalized_existing))
        .into_iter()
        .find(|project| workspace_path_key(&project.path).as_deref() == Some(workdir_key.as_str()))
    {
        // A durable project already exists. The explicit registration action
        // is also enough to restore its process-local access after a drive
        // was unavailable during startup.
        refresh_approved_workdirs_after_project_save(&state);
        return Ok(WorkspaceProjectRegistrationResponse {
            created: false,
            project: json!({
                "id": project.id,
                "name": project.name,
                "path": project.path,
                "lastSessionId": project.last_session_id,
                "pinned": project.pinned,
            }),
        });
    }

    let last_session_id = state
        .sessions
        .lock()
        .values()
        .filter(|record| {
            workspace_path_key(&record.summary.cwd).as_deref() == Some(workdir_key.as_str())
        })
        .max_by(|left, right| left.summary.updated_at.cmp(&right.summary.updated_at))
        .map(|record| record.summary.id.clone());
    let mut entries = normalized_existing
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| "projects settings are invalid".to_string())?;
    let entry = json!({
        "id": new_stable_project_id(),
        "name": requested_name,
        "path": workdir,
        "lastSessionId": last_session_id,
        "pinned": false,
    });
    entries.push(entry.clone());
    let normalized = normalize_projects_settings_payload_with_extra_approved(
        &state,
        json!({
            "version": PROJECT_SETTINGS_VERSION,
            "initialized": true,
            "entries": entries
        }),
        (!was_approved && known_historical).then_some(canonical.as_path()),
    )?;
    let project = normalized
        .get("entries")
        .and_then(Value::as_array)
        .and_then(|entries| entries.last())
        .cloned()
        .ok_or_else(|| "project registration failed".to_string())?;
    let previous = state
        .settings
        .lock()
        .insert("projects".to_string(), normalized);
    if let Err(error) = state.persist_settings_locked() {
        let mut settings = state.settings.lock();
        if let Some(previous) = previous {
            settings.insert("projects".to_string(), previous);
        } else {
            settings.remove("projects");
        }
        return Err(error);
    }
    // Do not publish the new authority until its project entry is durable.
    // This also consumes any process-local picker approval for the same root.
    refresh_approved_workdirs_after_project_save(&state);
    Ok(WorkspaceProjectRegistrationResponse {
        created: true,
        project,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePickedFile {
    pub path: String,
    pub name: String,
}

fn validate_picked_workspace_files(
    workdir: &Path,
    selected: Vec<PathBuf>,
) -> Result<Vec<WorkspacePickedFile>, String> {
    if selected.len() > MAX_PICKED_FILES {
        return Err(format!("select at most {MAX_PICKED_FILES} files at a time"));
    }
    let mut total_bytes = 0_u64;
    let mut output = Vec::with_capacity(selected.len());
    for selected_path in selected {
        let (path, bytes) = read_bounded_regular_file(
            &selected_path,
            MAX_PICKED_FILE_BYTES,
            true,
            Some(workdir),
            "selected file",
        )?;
        let relative = workspace_relative_path(&path, workdir, "selected file")?;
        let logical = relative
            .to_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "selected file path is not valid UTF-8".to_string())?
            .replace('\\', "/");
        total_bytes = total_bytes.saturating_add(bytes.len() as u64);
        if total_bytes > MAX_PICKED_TOTAL_BYTES {
            return Err(format!(
                "selected files exceed the {} byte total limit",
                MAX_PICKED_TOTAL_BYTES
            ));
        }
        String::from_utf8(bytes)
            .map_err(|_| format!("selected file is not UTF-8 text: {}", path.display()))?;
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "selected file name is not valid UTF-8".to_string())?
            .to_string();
        output.push(WorkspacePickedFile {
            path: logical,
            name,
        });
    }
    Ok(output)
}

/// Let the user explicitly select attachment files for one existing session.
/// Selection is not a general filesystem grant: every returned file is
/// revalidated against that session's canonical workspace root.
#[tauri::command(rename_all = "camelCase")]
pub async fn workspace_pick_files(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    session_id: String,
) -> Result<Vec<WorkspacePickedFile>, String> {
    require_project_root_workdir_policy(&state)?;
    let canonical = canonical_workdir(&workdir)?;
    // A historical session remains readable metadata, but it cannot turn a
    // file-picker gesture into workspace access until the root is registered
    // or selected again through the native project picker.
    require_registered_project_workdir(&state, &canonical)?;
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err("session id is required for file selection".to_string());
    }
    let session_workdir = state
        .sessions
        .lock()
        .get(session_id)
        .and_then(|record| canonical_workdir(&record.summary.cwd).ok())
        .ok_or_else(|| "file picker session is not available".to_string())?;
    if session_workdir != canonical {
        return Err("file picker workdir does not match the session workspace".to_string());
    }

    let selected = {
        let picker_root = canonical.clone();
        tauri::async_runtime::spawn_blocking(move || {
            rfd::FileDialog::new()
                .set_title("Attach workspace files")
                .set_directory(picker_root)
                .pick_files()
        })
        .await
        .map_err(|error| format!("open workspace file picker: {error}"))?
    };

    validate_picked_workspace_files(&canonical, selected.unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Composer media attachments

/// The renderer receives opaque attachment ids and never receives a native
/// filesystem path.  Images can be supplied to Pi as typed image blocks;
/// audio/video remain local preview attachments instead of being coerced into
/// model prompt text.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ComposerMediaKind {
    Image,
    Audio,
    Video,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposerMediaAttachment {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub kind: ComposerMediaKind,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposerPickedAttachment {
    /// `text` remains constrained to the active workspace.  `media` has been
    /// copied to native session storage and is addressed only by `id`.
    #[serde(rename = "type")]
    pub attachment_type: String,
    pub path: Option<String>,
    pub name: String,
    pub id: Option<String>,
    pub mime: Option<String>,
    pub media_kind: Option<ComposerMediaKind>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposerMediaLoad {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub kind: ComposerMediaKind,
    pub size_bytes: u64,
}

/// The JSON header prepended to the raw image bytes for an explicit clipboard
/// paste. The fields remain subject to the same session/workspace checks as
/// the former JSON command arguments; only the image body bypasses JSON.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ComposerPastedImageIpcHeader {
    workdir: String,
    session_id: String,
    name: String,
    mime: Option<String>,
}

fn validate_composer_pasted_image_ipc_header(
    header: &ComposerPastedImageIpcHeader,
) -> Result<(), String> {
    if header.workdir.contains('\0')
        || header.session_id.contains('\0')
        || header.name.contains('\0')
        || header
            .mime
            .as_deref()
            .is_some_and(|mime| mime.contains('\0'))
    {
        return Err("pasted image IPC header contains a NUL byte".to_string());
    }
    Ok(())
}

fn validate_composer_media_session_id(value: &str) -> Result<String, String> {
    let id = value.trim();
    if id.is_empty()
        || id.len() > MAX_SESSION_GOAL_SESSION_ID_BYTES
        || id.len() != value.len()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("composer attachment session id is invalid".to_string());
    }
    Ok(id.to_string())
}

fn validate_composer_media_id(value: &str) -> Result<String, String> {
    let id = value.trim();
    if id.is_empty() || id.len() != value.len() {
        return Err("composer attachment id is invalid".to_string());
    }
    Uuid::parse_str(id)
        .map(|parsed| parsed.to_string())
        .map_err(|_| "composer attachment id is invalid".to_string())
}

fn composer_attachment_session(
    state: &AppState,
    workdir: String,
    session_id: String,
) -> Result<(String, PathBuf), String> {
    require_project_root_workdir_policy(state)?;
    let canonical = canonical_workdir(&workdir)?;
    require_registered_project_workdir(state, &canonical)?;
    let session_id = validate_composer_media_session_id(&session_id)?;
    let stored_workdir = state
        .sessions
        .lock()
        .get(&session_id)
        .map(|record| record.summary.cwd.clone())
        .ok_or_else(|| "composer attachment session is not available".to_string())?;
    let stored_workdir = canonical_workdir(&stored_workdir)?;
    if stored_workdir != canonical {
        return Err("composer attachment workdir does not match the session workspace".to_string());
    }
    Ok((session_id, canonical))
}

fn composer_attachment_session_exists(
    state: &AppState,
    session_id: String,
) -> Result<String, String> {
    let session_id = validate_composer_media_session_id(&session_id)?;
    if !state.sessions.lock().contains_key(&session_id) {
        return Err("composer attachment session is not available".to_string());
    }
    Ok(session_id)
}

fn ensure_composer_media_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.file_type().is_symlink()
                || has_reparse_point(&metadata)
                || !metadata.is_dir() =>
        {
            return Err("composer attachment storage directory is invalid".to_string())
        }
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("inspect composer attachment storage: {error}")),
    }
    fs::create_dir_all(path)
        .map_err(|error| format!("create composer attachment storage: {error}"))?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect composer attachment storage: {error}"))?;
    if metadata.file_type().is_symlink() || has_reparse_point(&metadata) || !metadata.is_dir() {
        return Err("composer attachment storage directory is invalid".to_string());
    }
    Ok(())
}

fn composer_media_session_root(state: &AppState, session_id: &str) -> Result<PathBuf, String> {
    let data_dir = state
        .data_file
        .parent()
        .ok_or_else(|| "composer attachment storage is unavailable".to_string())?;
    let canonical_data_dir = fs::canonicalize(data_dir)
        .map_err(|error| format!("resolve composer attachment data directory: {error}"))?;
    let root = data_dir.join(COMPOSER_MEDIA_DIRECTORY);
    ensure_composer_media_directory(&root)?;
    let root = fs::canonicalize(&root)
        .map_err(|error| format!("resolve composer attachment storage: {error}"))?;
    if root.parent() != Some(canonical_data_dir.as_path()) {
        return Err("composer attachment storage escaped its data directory".to_string());
    }
    let session_root = root.join(session_id);
    ensure_composer_media_directory(&session_root)?;
    let session_root = fs::canonicalize(&session_root)
        .map_err(|error| format!("resolve composer session storage: {error}"))?;
    if session_root.parent() != Some(root.as_path()) {
        return Err("composer attachment storage escaped its session root".to_string());
    }
    Ok(session_root)
}

fn safe_composer_media_name(value: &str) -> String {
    let leaf = value.rsplit(['/', '\\']).next().unwrap_or(value).trim();
    let name = leaf
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_COMPOSER_MEDIA_NAME_CHARS)
        .collect::<String>();
    if name.is_empty() {
        "attachment".to_string()
    } else {
        name
    }
}

fn composer_media_extension(name: &str) -> Option<String> {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .filter(|extension| !extension.is_empty() && extension.len() <= 12)
}

fn detect_composer_media(bytes: &[u8], name: &str) -> Option<(ComposerMediaKind, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((ComposerMediaKind::Image, "image/png"));
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some((ComposerMediaKind::Image, "image/jpeg"));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some((ComposerMediaKind::Image, "image/gif"));
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
        return Some((ComposerMediaKind::Image, "image/webp"));
    }
    if bytes.starts_with(b"OggS") {
        return Some((ComposerMediaKind::Audio, "audio/ogg"));
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE".as_slice()) {
        return Some((ComposerMediaKind::Audio, "audio/wav"));
    }
    if bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0)
    {
        return Some((ComposerMediaKind::Audio, "audio/mpeg"));
    }
    if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return Some((ComposerMediaKind::Video, "video/webm"));
    }
    if bytes.len() >= 12 && bytes.get(4..8) == Some(b"ftyp".as_slice()) {
        return match composer_media_extension(name).as_deref() {
            Some("m4a") | Some("m4b") => Some((ComposerMediaKind::Audio, "audio/mp4")),
            _ => Some((ComposerMediaKind::Video, "video/mp4")),
        };
    }
    None
}

fn read_be_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_le_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn validate_composer_image_bounds(width: u32, height: u32, frames: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > MAX_COMPOSER_IMAGE_DIMENSION
        || height > MAX_COMPOSER_IMAGE_DIMENSION
    {
        return Err("composer image dimensions exceed the safe limit".to_string());
    }
    if frames == 0 || frames > MAX_COMPOSER_IMAGE_FRAMES {
        return Err("composer image animation has too many frames".to_string());
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| "composer image dimensions are invalid".to_string())?;
    if pixels > MAX_COMPOSER_IMAGE_PIXELS
        || pixels.saturating_mul(u64::from(frames)) > MAX_COMPOSER_IMAGE_DECODED_PIXELS
    {
        return Err("composer image decoded pixel count exceeds the safe limit".to_string());
    }
    Ok(())
}

fn validate_png_image(bytes: &[u8]) -> Result<(u32, u32, u32), String> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("composer PNG signature is invalid".to_string());
    }
    let mut offset = 8_usize;
    let mut dimensions = None;
    let mut declared_frames = None;
    let mut frame_controls = 0_u32;
    let mut saw_iend = false;
    while offset < bytes.len() {
        let length = read_be_u32(bytes, offset)
            .ok_or_else(|| "composer PNG chunk is truncated".to_string())?
            as usize;
        let chunk_type = bytes
            .get(offset + 4..offset + 8)
            .ok_or_else(|| "composer PNG chunk is truncated".to_string())?;
        let data_start = offset
            .checked_add(8)
            .ok_or_else(|| "composer PNG chunk is invalid".to_string())?;
        let data_end = data_start
            .checked_add(length)
            .ok_or_else(|| "composer PNG chunk is invalid".to_string())?;
        let chunk_end = data_end
            .checked_add(4)
            .ok_or_else(|| "composer PNG chunk is invalid".to_string())?;
        let data = bytes
            .get(data_start..data_end)
            .ok_or_else(|| "composer PNG chunk is truncated".to_string())?;
        if dimensions.is_none() && chunk_type != b"IHDR" {
            return Err("composer PNG must begin with IHDR".to_string());
        }
        match chunk_type {
            b"IHDR" => {
                if dimensions.is_some() || data.len() != 13 {
                    return Err("composer PNG IHDR is invalid".to_string());
                }
                dimensions = Some((
                    read_be_u32(data, 0).unwrap_or_default(),
                    read_be_u32(data, 4).unwrap_or_default(),
                ));
            }
            b"acTL" => {
                if declared_frames.is_some() || data.len() != 8 {
                    return Err("composer APNG animation header is invalid".to_string());
                }
                declared_frames = Some(read_be_u32(data, 0).unwrap_or_default());
            }
            b"fcTL" => {
                if data.len() != 26 {
                    return Err("composer APNG frame header is invalid".to_string());
                }
                frame_controls = frame_controls.saturating_add(1);
            }
            b"IEND" => {
                if !data.is_empty() || chunk_end != bytes.len() {
                    return Err("composer PNG trailer is invalid".to_string());
                }
                saw_iend = true;
            }
            _ => {}
        }
        offset = chunk_end;
        if saw_iend {
            break;
        }
    }
    let (width, height) = dimensions.ok_or_else(|| "composer PNG has no dimensions".to_string())?;
    if !saw_iend {
        return Err("composer PNG is incomplete".to_string());
    }
    let frames = declared_frames.unwrap_or(1);
    if declared_frames.is_some() {
        if frames == 0 || frame_controls != frames {
            return Err("composer APNG frame count is invalid".to_string());
        }
    } else if frame_controls != 0 {
        return Err("composer APNG frame controls have no animation header".to_string());
    }
    Ok((width, height, frames))
}

fn validate_jpeg_image(bytes: &[u8]) -> Result<(u32, u32, u32), String> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return Err("composer JPEG signature is invalid".to_string());
    }
    let mut offset = 2_usize;
    while offset < bytes.len() {
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *bytes
            .get(offset)
            .ok_or_else(|| "composer JPEG marker is truncated".to_string())?;
        offset += 1;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let segment_length = u16::from_be_bytes(
            bytes
                .get(offset..offset + 2)
                .ok_or_else(|| "composer JPEG segment is truncated".to_string())?
                .try_into()
                .map_err(|_| "composer JPEG segment is invalid".to_string())?,
        ) as usize;
        if segment_length < 2 {
            return Err("composer JPEG segment is invalid".to_string());
        }
        let segment_end = offset
            .checked_add(segment_length)
            .ok_or_else(|| "composer JPEG segment is invalid".to_string())?;
        let segment = bytes
            .get(offset + 2..segment_end)
            .ok_or_else(|| "composer JPEG segment is truncated".to_string())?;
        let is_start_of_frame = matches!(
            marker,
            0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf
        );
        if is_start_of_frame {
            if segment.len() < 6 {
                return Err("composer JPEG frame header is invalid".to_string());
            }
            let height = u16::from_be_bytes([segment[1], segment[2]]) as u32;
            let width = u16::from_be_bytes([segment[3], segment[4]]) as u32;
            return Ok((width, height, 1));
        }
        offset = segment_end;
    }
    Err("composer JPEG has no decoded dimensions".to_string())
}

fn skip_gif_sub_blocks(bytes: &[u8], offset: &mut usize) -> Result<(), String> {
    loop {
        let length = *bytes
            .get(*offset)
            .ok_or_else(|| "composer GIF data block is truncated".to_string())?
            as usize;
        *offset += 1;
        if length == 0 {
            return Ok(());
        }
        *offset = offset
            .checked_add(length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| "composer GIF data block is truncated".to_string())?;
    }
}

fn validate_gif_image(bytes: &[u8]) -> Result<(u32, u32, u32), String> {
    if bytes.len() < 13 || !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return Err("composer GIF header is invalid".to_string());
    }
    let width = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
    let height = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
    let packed = bytes[10];
    let mut offset = 13_usize;
    if packed & 0x80 != 0 {
        let table_bytes = 3_usize << ((packed & 0x07) as usize + 1);
        offset = offset
            .checked_add(table_bytes)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| "composer GIF color table is truncated".to_string())?;
    }
    let mut frames = 0_u32;
    let mut saw_trailer = false;
    while offset < bytes.len() {
        let introducer = bytes[offset];
        offset += 1;
        match introducer {
            0x2c => {
                let descriptor = bytes
                    .get(offset..offset + 9)
                    .ok_or_else(|| "composer GIF frame is truncated".to_string())?;
                let frame_width = u16::from_le_bytes([descriptor[4], descriptor[5]]) as u32;
                let frame_height = u16::from_le_bytes([descriptor[6], descriptor[7]]) as u32;
                validate_composer_image_bounds(frame_width, frame_height, 1)?;
                offset += 9;
                if descriptor[8] & 0x80 != 0 {
                    let table_bytes = 3_usize << ((descriptor[8] & 0x07) as usize + 1);
                    offset = offset
                        .checked_add(table_bytes)
                        .filter(|end| *end <= bytes.len())
                        .ok_or_else(|| "composer GIF frame color table is truncated".to_string())?;
                }
                if bytes.get(offset).is_none() {
                    return Err("composer GIF LZW header is truncated".to_string());
                }
                offset += 1;
                skip_gif_sub_blocks(bytes, &mut offset)?;
                frames = frames.saturating_add(1);
            }
            0x21 => {
                if bytes.get(offset).is_none() {
                    return Err("composer GIF extension is truncated".to_string());
                }
                offset += 1;
                skip_gif_sub_blocks(bytes, &mut offset)?;
            }
            0x3b => {
                if offset != bytes.len() {
                    return Err("composer GIF trailer is invalid".to_string());
                }
                saw_trailer = true;
                break;
            }
            _ => return Err("composer GIF block structure is invalid".to_string()),
        }
    }
    if !saw_trailer || frames == 0 {
        return Err("composer GIF is incomplete".to_string());
    }
    Ok((width, height, frames))
}

fn read_le_u24(bytes: &[u8], offset: usize) -> Option<u32> {
    let value = bytes.get(offset..offset + 3)?;
    Some(u32::from(value[0]) | (u32::from(value[1]) << 8) | (u32::from(value[2]) << 16))
}

fn validate_webp_image(bytes: &[u8]) -> Result<(u32, u32, u32), String> {
    if bytes.len() < 20
        || !bytes.starts_with(b"RIFF")
        || bytes.get(8..12) != Some(b"WEBP".as_slice())
        || read_le_u32(bytes, 4)
            .and_then(|length| usize::try_from(length).ok())
            .and_then(|length| length.checked_add(8))
            != Some(bytes.len())
    {
        return Err("composer WebP container is invalid".to_string());
    }
    let mut offset = 12_usize;
    let mut dimensions = None;
    let mut animated = false;
    let mut frames = 0_u32;
    while offset < bytes.len() {
        let chunk_type = bytes
            .get(offset..offset + 4)
            .ok_or_else(|| "composer WebP chunk is truncated".to_string())?;
        let length = read_le_u32(bytes, offset + 4)
            .ok_or_else(|| "composer WebP chunk is truncated".to_string())?
            as usize;
        let data_start = offset
            .checked_add(8)
            .ok_or_else(|| "composer WebP chunk is invalid".to_string())?;
        let data_end = data_start
            .checked_add(length)
            .ok_or_else(|| "composer WebP chunk is invalid".to_string())?;
        let data = bytes
            .get(data_start..data_end)
            .ok_or_else(|| "composer WebP chunk is truncated".to_string())?;
        match chunk_type {
            b"VP8X" => {
                if data.len() != 10 || dimensions.is_some() {
                    return Err("composer WebP extended header is invalid".to_string());
                }
                animated = data[0] & 0x02 != 0;
                dimensions = Some((
                    read_le_u24(data, 4).unwrap_or_default() + 1,
                    read_le_u24(data, 7).unwrap_or_default() + 1,
                ));
            }
            b"VP8 " if dimensions.is_none() => {
                if data.len() < 10 || data.get(3..6) != Some([0x9d, 0x01, 0x2a].as_slice()) {
                    return Err("composer WebP VP8 header is invalid".to_string());
                }
                dimensions = Some((
                    u16::from_le_bytes([data[6], data[7]]) as u32 & 0x3fff,
                    u16::from_le_bytes([data[8], data[9]]) as u32 & 0x3fff,
                ));
            }
            b"VP8L" if dimensions.is_none() => {
                if data.len() < 5 || data[0] != 0x2f {
                    return Err("composer WebP lossless header is invalid".to_string());
                }
                let bits = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
                dimensions = Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1));
            }
            b"ANMF" => {
                if data.len() < 16 {
                    return Err("composer WebP animation frame is invalid".to_string());
                }
                let frame_width = read_le_u24(data, 6).unwrap_or_default() + 1;
                let frame_height = read_le_u24(data, 9).unwrap_or_default() + 1;
                validate_composer_image_bounds(frame_width, frame_height, 1)?;
                frames = frames.saturating_add(1);
            }
            _ => {}
        }
        offset = data_end
            .checked_add(length & 1)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| "composer WebP chunk padding is invalid".to_string())?;
    }
    let (width, height) =
        dimensions.ok_or_else(|| "composer WebP has no dimensions".to_string())?;
    if animated && frames == 0 {
        return Err("composer WebP animation has no frames".to_string());
    }
    if !animated && frames != 0 {
        return Err("composer WebP frames have no animation flag".to_string());
    }
    Ok((width, height, frames.max(1)))
}

fn validate_composer_image_bytes(bytes: &[u8], mime: &str) -> Result<(), String> {
    let (width, height, frames) = match mime {
        "image/png" => validate_png_image(bytes)?,
        "image/jpeg" => validate_jpeg_image(bytes)?,
        "image/gif" => validate_gif_image(bytes)?,
        "image/webp" => validate_webp_image(bytes)?,
        _ => return Err("composer image type is not supported".to_string()),
    };
    validate_composer_image_bounds(width, height, frames)
}

fn write_new_composer_media_file(path: &Path, bytes: &[u8], label: &str) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create composer {label}: {error}"))?;
    file.write_all(bytes)
        .map_err(|error| format!("write composer {label}: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("sync composer {label}: {error}"))
}

fn stage_composer_media_bytes(
    state: &AppState,
    session_id: &str,
    name: &str,
    bytes: &[u8],
    media: (ComposerMediaKind, &'static str),
) -> Result<ComposerMediaAttachment, String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_COMPOSER_MEDIA_BYTES {
        return Err(format!(
            "composer media must contain 1 to {MAX_COMPOSER_MEDIA_BYTES} bytes"
        ));
    }
    if media.0 == ComposerMediaKind::Image {
        validate_composer_image_bytes(bytes, media.1)?;
    }
    let id = Uuid::new_v4().to_string();
    let descriptor = ComposerMediaAttachment {
        id: id.clone(),
        name: safe_composer_media_name(name),
        mime: media.1.to_string(),
        kind: media.0,
        size_bytes: bytes.len() as u64,
    };
    let root = composer_media_session_root(state, session_id)?;
    let data_path = root.join(format!("{id}.bin"));
    let metadata_path = root.join(format!("{id}.json"));
    write_new_composer_media_file(&data_path, bytes, "media data")?;
    let metadata = serde_json::to_vec(&descriptor)
        .map_err(|error| format!("serialize composer media metadata: {error}"));
    match metadata.and_then(|metadata| {
        if metadata.len() as u64 > MAX_COMPOSER_MEDIA_METADATA_BYTES {
            return Err("composer media metadata exceeds its limit".to_string());
        }
        write_new_composer_media_file(&metadata_path, &metadata, "media metadata")
    }) {
        Ok(()) => Ok(descriptor),
        Err(error) => {
            let _ = fs::remove_file(&data_path);
            Err(error)
        }
    }
}

#[cfg(windows)]
fn open_regular_file_no_follow(path: &Path, label: &str) -> Result<fs::File, String> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| format!("open {label}: {error}"))
}

#[cfg(unix)]
fn open_regular_file_no_follow(path: &Path, label: &str) -> Result<fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| format!("open {label}: {error}"))
}

#[cfg(all(not(windows), not(unix)))]
fn open_regular_file_no_follow(path: &Path, label: &str) -> Result<fs::File, String> {
    // This target has no portable std no-follow open flag. Callers still
    // inspect the opened handle and bound its stream before accepting bytes.
    fs::File::open(path).map_err(|error| format!("open {label}: {error}"))
}

/// Reject a symlink or reparse point in a lexical workspace path before it is
/// opened. `O_NOFOLLOW` / `FILE_FLAG_OPEN_REPARSE_POINT` protects the final
/// component, while this catches an already-present linked parent directory.
/// The later handle checks remain necessary because a hostile process can
/// still change a path after this inspection.
fn reject_workspace_path_links(path: &Path, workdir: &Path, label: &str) -> Result<(), String> {
    let relative = workspace_relative_path(path, workdir, label)?;
    let mut component = workdir.to_path_buf();
    for part in relative.components() {
        match part {
            std::path::Component::Normal(part) => component.push(part),
            std::path::Component::CurDir => continue,
            _ => return Err(format!("{label} is outside the session workspace")),
        }
        let metadata = fs::symlink_metadata(&component)
            .map_err(|error| format!("inspect {label}: {error}"))?;
        if metadata.file_type().is_symlink() || has_reparse_point(&metadata) {
            return Err(format!(
                "{label} must not traverse a symlink or reparse point"
            ));
        }
    }
    Ok(())
}

/// Return a component-aware workspace-relative path after reconciling the
/// equivalent Windows spellings that `canonicalize` and native file pickers
/// can produce. The fallback is comparison-only: callers still perform their
/// no-follow, reparse-point, opened-handle, and post-open checks using the
/// original paths.
fn workspace_relative_path(path: &Path, workdir: &Path, label: &str) -> Result<PathBuf, String> {
    if let Ok(relative) = path.strip_prefix(workdir) {
        return Ok(relative.to_path_buf());
    }

    #[cfg(windows)]
    {
        // Windows filesystems compare ordinary drive and UNC paths without
        // case sensitivity, while `Path::strip_prefix` can observe a raw
        // canonical root beside an ordinary picker path. Normalize only for
        // comparison and retain component boundaries to prevent sibling roots
        // such as `C:\workspace-other` from matching `C:\workspace`.
        let comparable_path = PathBuf::from(normalize_windows_workspace_path_key(
            &path_for_display(path),
        ));
        let comparable_workdir = PathBuf::from(normalize_windows_workspace_path_key(
            &path_for_display(workdir),
        ));
        comparable_path
            .strip_prefix(&comparable_workdir)
            .map(Path::to_path_buf)
            .map_err(|_| format!("{label} is outside the session workspace"))
    }

    #[cfg(not(windows))]
    Err(format!("{label} is outside the session workspace"))
}

/// Open a regular file from a checked handle. Both the lexical path and the
/// opened handle are checked so a FIFO, device, symlink, or reparse point is
/// rejected before any potentially blocking read or unbounded allocation.
fn open_checked_regular_file(
    path: &Path,
    required_root: Option<&Path>,
    label: &str,
) -> Result<(PathBuf, fs::File, fs::Metadata), String> {
    if let Some(root) = required_root {
        reject_workspace_path_links(path, root, label)?;
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("inspect {label}: {error}"))?;
    if metadata.file_type().is_symlink() || has_reparse_point(&metadata) || !metadata.is_file() {
        return Err(format!(
            "{label} must be a regular file, not a symlink or reparse point"
        ));
    }
    let canonical = fs::canonicalize(path).map_err(|error| format!("resolve {label}: {error}"))?;
    if let Some(root) = required_root {
        workspace_relative_path(&canonical, root, label)?;
    }

    // Open the original lexical name rather than its resolved spelling. This
    // lets the platform no-follow flag reject a final-component swap that
    // happens between canonicalization and open.
    let file = open_regular_file_no_follow(path, label)?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("inspect opened {label}: {error}"))?;
    if opened_metadata.file_type().is_symlink()
        || has_reparse_point(&opened_metadata)
        || !opened_metadata.is_file()
        || opened_metadata.len() != metadata.len()
    {
        return Err(format!("{label} changed while opening; choose it again"));
    }

    // Re-resolve after opening to catch the common parent-directory swap.
    // The opened descriptor is never read until the root check has passed.
    let resolved_after_open =
        fs::canonicalize(path).map_err(|error| format!("resolve opened {label}: {error}"))?;
    if let Some(root) = required_root {
        workspace_relative_path(&resolved_after_open, root, label)?;
    }
    Ok((resolved_after_open, file, opened_metadata))
}

/// Open and read a user-selected regular file from one checked handle. The
/// path can change while a native picker is open, so pathname metadata alone
/// must never determine the allocation or the bytes later trusted by the UI.
fn read_bounded_regular_file(
    path: &Path,
    limit: u64,
    allow_empty: bool,
    required_root: Option<&Path>,
    label: &str,
) -> Result<(PathBuf, Vec<u8>), String> {
    let (canonical, file, opened_metadata) = open_checked_regular_file(path, required_root, label)?;
    if (!allow_empty && opened_metadata.len() == 0) || opened_metadata.len() > limit {
        return Err(format!("{label} must contain an allowed number of bytes"));
    }
    let capacity = usize::try_from(opened_metadata.len().min(limit)).unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {label}: {error}"))?;
    let read_size = bytes.len() as u64;
    if read_size > limit {
        return Err(format!("{label} exceeds the allowed size"));
    }
    if read_size != opened_metadata.len() || (!allow_empty && read_size == 0) {
        return Err(format!("{label} changed while reading; choose it again"));
    }
    Ok((canonical, bytes))
}

fn read_composer_media_file(
    root: &Path,
    path: &Path,
    limit: u64,
    expected_size: Option<u64>,
    label: &str,
) -> Result<Vec<u8>, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("inspect composer {label}: {error}"))?;
    if metadata.file_type().is_symlink()
        || has_reparse_point(&metadata)
        || !metadata.is_file()
        || metadata.len() > limit
        || expected_size.is_some_and(|size| metadata.len() != size)
    {
        return Err(format!("composer {label} is invalid"));
    }
    let resolved =
        fs::canonicalize(path).map_err(|error| format!("resolve composer {label}: {error}"))?;
    if resolved.parent() != Some(root) {
        return Err(format!("composer {label} escaped its session root"));
    }
    // Read from the same checked handle and cap the stream itself. This keeps
    // a concurrently replaced or growing file from bypassing the pre-open
    // metadata limit and allocating an unbounded IPC payload.
    let file = open_regular_file_no_follow(&resolved, label)?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("inspect opened composer {label}: {error}"))?;
    if opened_metadata.file_type().is_symlink()
        || has_reparse_point(&opened_metadata)
        || !opened_metadata.is_file()
        || opened_metadata.len() > limit
        || opened_metadata.len() != metadata.len()
        || expected_size.is_some_and(|size| opened_metadata.len() != size)
    {
        return Err(format!("composer {label} is invalid"));
    }
    let capacity = usize::try_from(opened_metadata.len().min(limit)).unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read composer {label}: {error}"))?;
    let read_size = bytes.len() as u64;
    if read_size > limit {
        return Err(format!("composer {label} exceeds its limit"));
    }
    if read_size != opened_metadata.len() || expected_size.is_some_and(|size| read_size != size) {
        return Err(format!("composer {label} changed while reading"));
    }
    Ok(bytes)
}

fn load_composer_media_attachment(
    state: &AppState,
    session_id: &str,
    attachment_id: &str,
) -> Result<(ComposerMediaAttachment, Vec<u8>), String> {
    let root = composer_media_session_root(state, session_id)?;
    let metadata_path = root.join(format!("{attachment_id}.json"));
    let metadata = read_composer_media_file(
        &root,
        &metadata_path,
        MAX_COMPOSER_MEDIA_METADATA_BYTES,
        None,
        "media metadata",
    )?;
    let descriptor = serde_json::from_slice::<ComposerMediaAttachment>(&metadata)
        .map_err(|_| "composer media metadata is invalid".to_string())?;
    if descriptor.id != attachment_id
        || descriptor.name != safe_composer_media_name(&descriptor.name)
        || descriptor.size_bytes == 0
        || descriptor.size_bytes > MAX_COMPOSER_MEDIA_BYTES
    {
        return Err("composer media metadata is invalid".to_string());
    }
    let data_path = root.join(format!("{attachment_id}.bin"));
    let bytes = read_composer_media_file(
        &root,
        &data_path,
        MAX_COMPOSER_MEDIA_BYTES,
        Some(descriptor.size_bytes),
        "media data",
    )?;
    let matches_descriptor = matches!(
        detect_composer_media(&bytes, &descriptor.name),
        Some((kind, mime)) if kind == descriptor.kind && mime == descriptor.mime.as_str()
    );
    if bytes.len() as u64 != descriptor.size_bytes || !matches_descriptor {
        return Err("composer media data does not match its metadata".to_string());
    }
    if descriptor.kind == ComposerMediaKind::Image {
        validate_composer_image_bytes(&bytes, &descriptor.mime)?;
    }
    Ok((descriptor, bytes))
}

/// Compose the small, authoritative attachment descriptor and the media bytes
/// into one binary IPC payload. Keeping the descriptor in the envelope lets
/// the renderer verify its transcript marker without serializing each byte as
/// a JSON number.
fn composer_media_ipc_payload(
    descriptor: ComposerMediaAttachment,
    mut bytes: Vec<u8>,
) -> Result<Vec<u8>, String> {
    let header = serde_json::to_vec(&ComposerMediaLoad {
        id: descriptor.id,
        name: descriptor.name,
        mime: descriptor.mime,
        kind: descriptor.kind,
        size_bytes: descriptor.size_bytes,
    })
    .map_err(|error| format!("serialize composer media response header: {error}"))?;
    if header.is_empty() || header.len() > MAX_COMPOSER_MEDIA_IPC_HEADER_BYTES {
        return Err("composer media response metadata is invalid".to_string());
    }
    let mut payload = Vec::with_capacity(4 + header.len() + bytes.len());
    payload.extend_from_slice(&(header.len() as u32).to_be_bytes());
    payload.extend_from_slice(&header);
    payload.append(&mut bytes);
    Ok(payload)
}

fn composer_media_ipc_response(
    descriptor: ComposerMediaAttachment,
    bytes: Vec<u8>,
) -> Result<tauri::ipc::Response, String> {
    Ok(tauri::ipc::Response::new(composer_media_ipc_payload(
        descriptor, bytes,
    )?))
}

fn composer_marker_hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn decode_composer_media_marker(value: &str) -> Option<Value> {
    if value.is_empty() || value.len() > MAX_COMPOSER_MEDIA_MARKER_BYTES {
        return None;
    }
    let source = value.as_bytes();
    let mut decoded = Vec::with_capacity(source.len());
    let mut index = 0_usize;
    while index < source.len() {
        if source[index] == b'%' {
            let high = composer_marker_hex(*source.get(index + 1)?)?;
            let low = composer_marker_hex(*source.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(source[index]);
            index += 1;
        }
    }
    serde_json::from_slice(&decoded).ok()
}

/// Extract only the typed `media[].id` values from the opaque, URL-encoded
/// renderer marker. Display names that happen to look like UUIDs cannot become
/// attachment references or make an otherwise valid branch fail.
fn composer_media_ids_in_content(content: &str) -> HashSet<String> {
    let mut ids = HashSet::new();
    let mut remainder = content;
    while let Some(prefix_index) = remainder.find(COMPOSER_MEDIA_MARKER_PREFIX) {
        let marker = &remainder[prefix_index + COMPOSER_MEDIA_MARKER_PREFIX.len()..];
        let Some(end_index) = marker.find(']') else {
            break;
        };
        if let Some(payload) = decode_composer_media_marker(&marker[..end_index]) {
            let valid_version = payload
                .get("version")
                .and_then(Value::as_u64)
                .is_some_and(|version| version == 1);
            if valid_version {
                if let Some(media) = payload.get("media").and_then(Value::as_array) {
                    for item in media {
                        let Some(candidate) = item.get("id").and_then(Value::as_str) else {
                            continue;
                        };
                        if let Ok(id) = Uuid::parse_str(candidate) {
                            ids.insert(id.hyphenated().to_string());
                        }
                    }
                }
            }
        }
        remainder = &marker[end_index + 1..];
    }
    ids
}

fn copy_composer_media_for_branch<'a>(
    state: &AppState,
    source_session_id: &str,
    destination_session_id: &str,
    attachment_ids: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let attachment_ids = attachment_ids.into_iter().collect::<Vec<_>>();
    if attachment_ids.is_empty() {
        return Ok(());
    }
    let source_session_id = validate_composer_media_session_id(source_session_id)?;
    let destination_session_id = validate_composer_media_session_id(destination_session_id)?;
    let destination_root = composer_media_session_root(state, &destination_session_id)?;
    for attachment_id in attachment_ids {
        let attachment_id = validate_composer_media_id(attachment_id)?;
        let (descriptor, bytes) =
            load_composer_media_attachment(state, &source_session_id, &attachment_id)?;
        let data_path = destination_root.join(format!("{attachment_id}.bin"));
        let metadata_path = destination_root.join(format!("{attachment_id}.json"));
        write_new_composer_media_file(&data_path, &bytes, "branched media data")?;
        let metadata = serde_json::to_vec(&descriptor)
            .map_err(|error| format!("serialize branched composer media metadata: {error}"));
        let copied = metadata.and_then(|metadata| {
            if metadata.len() as u64 > MAX_COMPOSER_MEDIA_METADATA_BYTES {
                return Err("branched composer media metadata exceeds its limit".to_string());
            }
            write_new_composer_media_file(&metadata_path, &metadata, "branched media metadata")
        });
        if let Err(error) = copied {
            let _ = fs::remove_file(&data_path);
            return Err(error);
        }
    }
    Ok(())
}

/// Explicitly pick UTF-8 workspace files and safe image/audio/video files.
/// Native media is copied immediately; a file outside the workspace cannot be
/// retained as a text reference or exposed by its original path.
#[tauri::command(rename_all = "camelCase")]
pub async fn composer_pick_attachments(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    session_id: String,
) -> Result<Vec<ComposerPickedAttachment>, String> {
    let (session_id, canonical_workdir) = composer_attachment_session(&state, workdir, session_id)?;
    let picker_root = canonical_workdir.clone();
    let selected = tauri::async_runtime::spawn_blocking(move || {
        rfd::FileDialog::new()
            .set_title("Attach files or media")
            .set_directory(picker_root)
            .pick_files()
    })
    .await
    .map_err(|error| format!("open composer attachment picker: {error}"))?
    .unwrap_or_default();
    if selected.len() > MAX_PICKED_FILES {
        return Err(format!(
            "select at most {MAX_PICKED_FILES} attachments at a time"
        ));
    }

    // Media is staged as each selected path is inspected. Treat the picker
    // batch as all-or-nothing so a later unsupported file, size failure, or
    // workspace-boundary error cannot leave native-only orphan media behind.
    let mut staged_ids = Vec::new();
    let result = (|| {
        let mut total_bytes = 0_u64;
        let mut output = Vec::with_capacity(selected.len());
        for selected_path in selected {
            let (path, bytes) = read_bounded_regular_file(
                &selected_path,
                MAX_COMPOSER_MEDIA_BYTES,
                false,
                None,
                "selected attachment",
            )?;
            let size_bytes = bytes.len() as u64;
            total_bytes = total_bytes.saturating_add(size_bytes);
            if total_bytes > MAX_COMPOSER_MEDIA_TOTAL_BYTES {
                return Err(format!(
                    "selected attachments exceed the {MAX_COMPOSER_MEDIA_TOTAL_BYTES} byte total limit"
                ));
            }
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .map(safe_composer_media_name)
                .ok_or_else(|| "selected attachment file name is not valid UTF-8".to_string())?;
            if let Some(media) = detect_composer_media(&bytes, &name) {
                let descriptor =
                    stage_composer_media_bytes(&state, &session_id, &name, &bytes, media)?;
                staged_ids.push(descriptor.id.clone());
                output.push(ComposerPickedAttachment {
                    attachment_type: "media".to_string(),
                    path: None,
                    name: descriptor.name,
                    id: Some(descriptor.id),
                    mime: Some(descriptor.mime),
                    media_kind: Some(descriptor.kind),
                    size_bytes: Some(descriptor.size_bytes),
                });
                continue;
            }
            if size_bytes > MAX_PICKED_FILE_BYTES {
                return Err(format!(
                    "selected text attachment exceeds the {MAX_PICKED_FILE_BYTES} byte limit: {}",
                    path.display()
                ));
            }
            String::from_utf8(bytes).map_err(|_| {
                "only UTF-8 workspace files and safe image/audio/video attachments are supported"
                    .to_string()
            })?;
            let relative = workspace_relative_path(&path, &canonical_workdir, "text attachment")
                .map_err(|_| {
                    "text attachments must stay inside the session workspace".to_string()
                })?;
            let logical = relative
                .to_str()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "selected text attachment path is not valid UTF-8".to_string())?
                .replace('\\', "/");
            output.push(ComposerPickedAttachment {
                attachment_type: "text".to_string(),
                path: Some(logical),
                name,
                id: None,
                mime: None,
                media_kind: None,
                size_bytes: Some(size_bytes),
            });
        }
        Ok(output)
    })();
    if result.is_err() {
        // All ids below were generated locally by `stage_composer_media_bytes`
        // and `root` is session-scoped/canonical. Best-effort removal keeps
        // the original picker failure authoritative while avoiding orphaned
        // files if a subsequent selection entry was rejected.
        if let Ok(root) = composer_media_session_root(&state, &session_id) {
            for id in staged_ids {
                let _ = fs::remove_file(root.join(format!("{id}.json")));
                let _ = fs::remove_file(root.join(format!("{id}.bin")));
            }
        }
    }
    result
}

/// Decode the small JSON header embedded before a raw pasted-image body.
///
/// Tauri's raw invoke body is intentionally used here instead of a `Vec<u8>`
/// command argument: serde_json represents the latter as one number per byte,
/// making multi-megabyte clipboard images needlessly large and expensive.
fn decode_composer_pasted_image_ipc_payload(
    payload: &[u8],
) -> Result<(ComposerPastedImageIpcHeader, &[u8]), String> {
    const HEADER_LENGTH_OFFSET: usize = 5;
    let max_payload_bytes = COMPOSER_PASTED_IMAGE_IPC_PREFIX_BYTES
        .checked_add(MAX_COMPOSER_PASTED_IMAGE_IPC_HEADER_BYTES)
        .and_then(|size| size.checked_add(MAX_COMPOSER_MEDIA_BYTES as usize))
        .ok_or_else(|| "pasted image IPC limits are invalid".to_string())?;
    if payload.len() > max_payload_bytes {
        return Err("pasted image IPC payload exceeds its limit".to_string());
    }
    if payload.len() < COMPOSER_PASTED_IMAGE_IPC_PREFIX_BYTES {
        return Err("pasted image IPC payload is truncated".to_string());
    }
    if &payload[..COMPOSER_PASTED_IMAGE_IPC_MAGIC.len()] != COMPOSER_PASTED_IMAGE_IPC_MAGIC {
        return Err("pasted image IPC magic is invalid".to_string());
    }
    if payload[COMPOSER_PASTED_IMAGE_IPC_MAGIC.len()] != COMPOSER_PASTED_IMAGE_IPC_VERSION {
        return Err("pasted image IPC version is not supported".to_string());
    }
    let header_len = u32::from_be_bytes(
        payload[HEADER_LENGTH_OFFSET..COMPOSER_PASTED_IMAGE_IPC_PREFIX_BYTES]
            .try_into()
            .map_err(|_| "pasted image IPC header is invalid".to_string())?,
    ) as usize;
    if header_len == 0 || header_len > MAX_COMPOSER_PASTED_IMAGE_IPC_HEADER_BYTES {
        return Err("pasted image IPC header exceeds its limit".to_string());
    }
    let body_start = COMPOSER_PASTED_IMAGE_IPC_PREFIX_BYTES
        .checked_add(header_len)
        .ok_or_else(|| "pasted image IPC header is invalid".to_string())?;
    if body_start > payload.len() {
        return Err("pasted image IPC payload is truncated".to_string());
    }
    let body = &payload[body_start..];
    if body.is_empty() || body.len() as u64 > MAX_COMPOSER_MEDIA_BYTES {
        return Err(format!(
            "pasted image IPC body must contain 1 to {MAX_COMPOSER_MEDIA_BYTES} bytes"
        ));
    }
    let header =
        serde_json::from_slice(&payload[COMPOSER_PASTED_IMAGE_IPC_PREFIX_BYTES..body_start])
            .map_err(|_| "pasted image IPC header is invalid".to_string())?;
    validate_composer_pasted_image_ipc_header(&header)?;
    Ok((header, body))
}

/// Stage an image copied from the composer clipboard. The renderer sends a
/// bounded metadata header followed by the image as Tauri raw IPC bytes. The
/// session/workspace binding, media signature, declared MIME, and image-bounds
/// checks remain native and apply before any file is written.
#[tauri::command(rename_all = "camelCase")]
pub fn composer_stage_pasted_image(
    state: State<'_, Arc<AppState>>,
    request: tauri::ipc::Request<'_>,
) -> Result<ComposerMediaAttachment, String> {
    let payload = match request.body() {
        tauri::ipc::InvokeBody::Raw(payload) => payload.as_slice(),
        tauri::ipc::InvokeBody::Json(_) => {
            return Err("pasted image must use raw binary IPC".to_string())
        }
    };
    let (header, bytes) = decode_composer_pasted_image_ipc_payload(payload)?;
    let (session_id, _) = composer_attachment_session(&state, header.workdir, header.session_id)?;
    let safe_name = safe_composer_media_name(&header.name);
    let media = detect_composer_media(bytes, &safe_name)
        .ok_or_else(|| "pasted content is not a supported image".to_string())?;
    if media.0 != ComposerMediaKind::Image {
        return Err("only pasted images are supported".to_string());
    }
    let declared = header.mime.unwrap_or_default().trim().to_ascii_lowercase();
    if !declared.is_empty() && declared != media.1 {
        return Err("pasted image type does not match its data".to_string());
    }
    stage_composer_media_bytes(&state, &session_id, &safe_name, bytes, media)
}

/// Resolve one trusted attachment into bytes for a Blob URL.  No filesystem
/// URL is returned, so the WebView cannot directly open arbitrary local paths.
#[tauri::command(rename_all = "camelCase")]
pub fn composer_media_load(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    attachment_id: String,
) -> Result<tauri::ipc::Response, String> {
    let session_id = composer_attachment_session_exists(&state, session_id)?;
    let attachment_id = validate_composer_media_id(&attachment_id)?;
    let (descriptor, bytes) = load_composer_media_attachment(&state, &session_id, &attachment_id)?;
    composer_media_ipc_response(descriptor, bytes)
}

/// Discard an unsent attachment after the user removes it or leaves a draft.
/// Sent attachments are intentionally retained so historical transcript cards
/// can be resolved through the same bounded native command.
#[tauri::command(rename_all = "camelCase")]
pub fn composer_media_discard(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    attachment_id: String,
) -> Result<(), String> {
    state.require_persistence_ready()?;
    // Serialize the durable-reference check with agent_run. Without this
    // guard, a session switch could inspect an old transcript and unlink the
    // media pair immediately before the user marker is committed.
    let _persist_guard = state.persist_lock.lock();
    let session_id = composer_attachment_session_exists(&state, session_id)?;
    let attachment_id = validate_composer_media_id(&attachment_id)?;
    if state
        .history
        .load_messages(&session_id)?
        .iter()
        .any(|message| composer_media_ids_in_content(&message.content).contains(&attachment_id))
    {
        return Err("sent composer attachments cannot be discarded".to_string());
    }
    let root = composer_media_session_root(&state, &session_id)?;
    let paths = [
        root.join(format!("{attachment_id}.json")),
        root.join(format!("{attachment_id}.bin")),
    ];
    // An earlier partially completed discard may have removed only one side
    // of the pair, so do not require metadata to be loadable here. The UUID
    // is validated and every path remains under the canonical session root.
    // Inspect both first to refuse symlinks/directories, then delete either
    // regular file that remains; a retry can clean up a transient failure.
    let mut resolved_paths = Vec::with_capacity(paths.len());
    for path in &paths {
        match fs::symlink_metadata(path) {
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    || has_reparse_point(&metadata)
                    || !metadata.is_file() =>
            {
                return Err("composer attachment storage entry is invalid".to_string())
            }
            Ok(_) => {
                let resolved = fs::canonicalize(path)
                    .map_err(|error| format!("resolve composer attachment: {error}"))?;
                if resolved.parent() != Some(root.as_path()) {
                    return Err("composer attachment escaped its session root".to_string());
                }
                resolved_paths.push(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                resolved_paths.push(path.clone())
            }
            Err(error) => return Err(format!("inspect composer attachment: {error}")),
        }
    }
    for path in &resolved_paths {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("discard composer attachment: {error}")),
        }
    }
    Ok(())
}

/// Validate the capability minted by `agent_run`.  The token is bound to the
/// turn's canonical workspace and cancellation flag, so a renderer cannot
/// reuse a token for another project or continue after cancellation.
fn require_capability(
    state: &AppState,
    capability_token: Option<&str>,
    workdir: &str,
) -> Result<PathBuf, String> {
    require_capability_for(state, capability_token, workdir, ToolAction::Read, None)
}

fn validate_workspace_view_grant(
    state: &AppState,
    token: &str,
    canonical: &Path,
    grant: &WorkspaceCapabilityGrant,
) -> Result<(), String> {
    if grant.expires_at <= Instant::now() {
        state.workspace_capabilities.lock().remove(token);
        return Err("workspace capability token is expired".to_string());
    }
    if grant.workdir != canonical {
        return Err("workspace capability token is bound to a different workdir".to_string());
    }
    let session_matches = state
        .sessions
        .lock()
        .get(&grant.session_id)
        .and_then(|record| canonical_workdir(&record.summary.cwd).ok())
        .is_some_and(|stored| stored == canonical);
    if !session_matches {
        state.workspace_capabilities.lock().remove(token);
        return Err("workspace capability session is no longer available".to_string());
    }
    if require_registered_project_workdir(state, canonical).is_err() {
        state.workspace_capabilities.lock().remove(token);
        return Err("workspace capability project is no longer registered".to_string());
    }
    Ok(())
}

fn require_workspace_view_capability(
    state: &AppState,
    capability_token: Option<&str>,
    workdir: &str,
) -> Result<(PathBuf, WorkspaceCapabilityGrant), String> {
    let token = capability_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "workspace capability token is required".to_string())?;
    let canonical = canonical_workdir(workdir)?;
    let grant = state
        .workspace_capabilities
        .lock()
        .get(token)
        .cloned()
        .ok_or_else(|| "workspace capability token is invalid or expired".to_string())?;
    validate_workspace_view_grant(state, token, &canonical, &grant)?;
    Ok((canonical, grant))
}

fn require_read_capability(
    state: &AppState,
    capability_token: Option<&str>,
    workdir: &str,
) -> Result<PathBuf, String> {
    let token = capability_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "capability token is required for workspace reads".to_string())?;
    let canonical = canonical_workdir(workdir)?;
    if let Some(grant) = state.subagent_capabilities.lock().get(token).cloned() {
        if grant.expires_at <= Instant::now() {
            state.subagent_capabilities.lock().remove(token);
            return Err("subagent task capability is expired".to_string());
        }
        if grant.cancelled.load(Ordering::SeqCst) {
            return Err("subagent task has been cancelled".to_string());
        }
        if grant.workdir != canonical {
            return Err("subagent task capability is bound to a different workdir".to_string());
        }
        let parent_active = state
            .active_runs
            .lock()
            .get(&grant.parent_request_id)
            .is_some_and(|parent| {
                parent.session_id == grant.session_id && parent.turn_id == grant.parent_turn_id
            });
        if !parent_active {
            return Err("subagent parent run is no longer active".to_string());
        }
        let task_running = state
            .subagent_tasks
            .get_task(&grant.session_id, &grant.task_id)?
            .is_some_and(|task| task.status == SubagentTaskStatus::Running);
        if !task_running {
            return Err("subagent task is no longer running".to_string());
        }
        return Ok(canonical);
    }
    let Some(grant) = state.workspace_capabilities.lock().get(token).cloned() else {
        return require_capability(state, Some(token), workdir);
    };
    validate_workspace_view_grant(state, token, &canonical, &grant)?;
    Ok(canonical)
}

/// Global filesystem reads are available to a parent Agent capability. A child
/// capability is accepted only when the delegation approval explicitly minted
/// it with `allow_global_read`; workspace-viewer tokens never satisfy this.
fn require_global_read_capability(
    state: &AppState,
    capability_token: Option<&str>,
    workdir: &str,
) -> Result<PathBuf, String> {
    let token = capability_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "capability token is required for GlobalRead".to_string())?;
    if let Some(child) = state.subagent_capabilities.lock().get(token).cloned() {
        if !child.allow_global_read {
            return Err(
                "GlobalRead was not approved for this subagent task; request a new delegated task with global read enabled"
                    .to_string(),
            );
        }
        return require_read_capability(state, Some(token), workdir);
    }
    require_capability_for(state, Some(token), workdir, ToolAction::Read, None)
}

/// A worktree child inherits its parent's one-time `DelegateWorktree`
/// authorization, but only for mutations inside the detached checkout that
/// native code provisioned for that task. It cannot consume a parent approval
/// or use this path for shell, MCP, Memory, or arbitrary workspaces.
fn require_worktree_child_mutation_capability(
    state: &AppState,
    capability_token: &str,
    workdir: &Path,
    action: ToolAction,
) -> Result<bool, String> {
    if !matches!(
        action,
        ToolAction::Write | ToolAction::Edit | ToolAction::Delete
    ) {
        return Ok(false);
    }
    let token = capability_token.trim();
    let Some(grant) = state.subagent_capabilities.lock().get(token).cloned() else {
        return Ok(false);
    };
    if grant.mode != SubagentCapabilityMode::Worktree {
        return Err("read-only subagent capability cannot mutate files".to_string());
    }
    if grant.expires_at <= Instant::now() {
        state.subagent_capabilities.lock().remove(token);
        return Err("worktree subagent capability is expired".to_string());
    }
    if grant.cancelled.load(Ordering::SeqCst) {
        return Err("worktree subagent task has been cancelled".to_string());
    }
    if grant.workdir != workdir {
        return Err("worktree subagent capability is bound to a different workdir".to_string());
    }
    let parent_active = state
        .active_runs
        .lock()
        .get(&grant.parent_request_id)
        .is_some_and(|parent| {
            parent.session_id == grant.session_id && parent.turn_id == grant.parent_turn_id
        });
    if !parent_active {
        return Err("worktree subagent parent run is no longer active".to_string());
    }
    let task_running = state
        .subagent_tasks
        .get_task(&grant.session_id, &grant.task_id)?
        .is_some_and(|task| task.status == SubagentTaskStatus::Running);
    if !task_running {
        return Err("worktree subagent task is no longer running".to_string());
    }
    Ok(true)
}

/// Accept either a normal parent capability or a still-running isolated
/// worktree child capability for a workspace mutation.
fn require_workspace_mutation_capability(
    state: &AppState,
    capability_token: Option<&str>,
    workdir: &str,
    action: ToolAction,
    tool_call_id: Option<&str>,
) -> Result<PathBuf, String> {
    let token = capability_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "capability token is required for agent tools".to_string())?;
    let canonical = canonical_workdir(workdir)?;
    if require_worktree_child_mutation_capability(state, token, &canonical, action)? {
        return Ok(canonical);
    }
    require_capability_for(state, Some(token), workdir, action, tool_call_id)
}

fn require_capability_for(
    state: &AppState,
    capability_token: Option<&str>,
    workdir: &str,
    action: ToolAction,
    tool_call_id: Option<&str>,
) -> Result<PathBuf, String> {
    require_capability_for_named(state, capability_token, workdir, action, tool_call_id, None)
}

/// Like [`require_capability_for`], but binds an approval to the exact tool
/// name. Dynamic MCP tools use this so a one-time approval cannot be replayed
/// for a different native MCP server/tool pair.
fn require_capability_for_named(
    state: &AppState,
    capability_token: Option<&str>,
    workdir: &str,
    action: ToolAction,
    tool_call_id: Option<&str>,
    expected_tool_name: Option<&str>,
) -> Result<PathBuf, String> {
    require_capability_for_named_with_constraints(
        state,
        capability_token,
        workdir,
        action,
        tool_call_id,
        expected_tool_name,
        false,
        false,
    )
}

/// Validate a named tool capability and optionally require a visible one-use
/// approval even in Full mode. Global child reads use this path so the
/// delegation confirmation cannot be silently bypassed by a broad parent mode.
#[allow(clippy::too_many_arguments)]
fn require_capability_for_named_with_constraints(
    state: &AppState,
    capability_token: Option<&str>,
    workdir: &str,
    action: ToolAction,
    tool_call_id: Option<&str>,
    expected_tool_name: Option<&str>,
    require_explicit_approval: bool,
    require_subagent_global_read: bool,
) -> Result<PathBuf, String> {
    let token = capability_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "capability token is required for agent tools".to_string())?;
    let canonical = canonical_workdir(workdir)?;
    let grant = state
        .capabilities
        .lock()
        .get(token)
        .cloned()
        .ok_or_else(|| "capability token is invalid or expired".to_string())?;
    if grant.expires_at <= Instant::now() {
        state.capabilities.lock().remove(token);
        return Err("capability token is expired".to_string());
    }
    if grant.cancelled.load(Ordering::SeqCst) {
        return Err("agent run has been cancelled".to_string());
    }
    if grant.workdir != canonical {
        return Err("capability token is bound to a different workdir".to_string());
    }
    // Ensure the token still belongs to the active native run. This rejects
    // stale tokens after a terminal event or app-side cleanup.
    let active = state.active_runs.lock();
    let matches = active.values().any(|run| {
        run.capability_token == token
            && run.session_id == grant.session_id
            && run.conversation_id == grant.conversation_id
            && run.turn_id == grant.turn_id
            && run.request_id == grant.request_id
    });
    if !matches {
        return Err("capability token is no longer active".to_string());
    }
    drop(active);
    let permission = permission_requirement(grant.permission_mode, action);
    if permission == PermissionRequirement::Deny {
        return Err("current capability is read-only for this tool".to_string());
    }
    let approval_required =
        permission == PermissionRequirement::Approval || require_explicit_approval;
    if approval_required {
        let tool_call_id = tool_call_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "native tool approval is required".to_string())?;
        let approval = state
            .tool_approvals
            .lock()
            .remove(tool_call_id)
            .ok_or_else(|| "native tool approval is missing or already used".to_string())?;
        if approval.capability_token != token
            || approval.action != action
            || expected_tool_name.is_some_and(|expected| approval.tool_name != expected)
            || approval.subagent_global_read != require_subagent_global_read
            || approval.expires_at <= Instant::now()
        {
            return Err("native tool approval does not match this operation".to_string());
        }
    }
    Ok(canonical)
}

fn recheck_mutation_capability(
    state: &AppState,
    capability_token: Option<&str>,
    workdir: &Path,
    action: ToolAction,
) -> Result<(), String> {
    let token = capability_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "capability token is required for agent tools".to_string())?;
    if require_worktree_child_mutation_capability(state, token, workdir, action)? {
        return Ok(());
    }
    let grant = state
        .capabilities
        .lock()
        .get(token)
        .cloned()
        .ok_or_else(|| "capability token is invalid or expired".to_string())?;
    if grant.expires_at <= Instant::now() {
        state.capabilities.lock().remove(token);
        return Err("capability token is expired".to_string());
    }
    if grant.cancelled.load(Ordering::SeqCst) {
        return Err("agent run has been cancelled".to_string());
    }
    if grant.workdir != workdir {
        return Err("capability token is bound to a different workdir".to_string());
    }
    let active = state.active_runs.lock();
    let still_active = active.values().any(|run| {
        run.capability_token == token
            && run.session_id == grant.session_id
            && run.conversation_id == grant.conversation_id
            && run.turn_id == grant.turn_id
            && run.request_id == grant.request_id
    });
    if !still_active {
        return Err("capability token is no longer active".to_string());
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCapabilityResponse {
    pub capability_token: String,
    pub workdir: String,
    pub session_id: String,
}

/// Mint a read-only token for the visible file dock. It is intentionally not
/// accepted by write, edit, delete, or shell commands.
#[tauri::command(rename_all = "camelCase")]
pub fn workspace_capability_issue(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    session_id: String,
) -> Result<WorkspaceCapabilityResponse, String> {
    // File Dock data is only meaningful when the session snapshot is durable.
    // Do not mint a new renderer capability while recovery is required.
    state.require_persistence_ready()?;
    // Serialize issuance with project removal/relocation. A token must either
    // exist before the mutation (and be revoked by it) or observe the new
    // durable project registry and fail closed.
    let _persist_guard = state.persist_lock.lock();
    require_project_root_workdir_policy(&state)?;
    let canonical = canonical_workdir(&workdir)?;
    require_registered_project_workdir(&state, &canonical)?;
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Err("session id is required for workspace access".to_string());
    }
    let stored = state
        .sessions
        .lock()
        .get(&session_id)
        .cloned()
        .ok_or_else(|| "session not found".to_string())?;
    if canonical_workdir(&stored.summary.cwd)? != canonical {
        return Err("workspace workdir does not match the session workspace".to_string());
    }
    let token = format!("workspace-cap-{}", Uuid::new_v4());
    let mut grants = state.workspace_capabilities.lock();
    grants.retain(|_, grant| grant.expires_at > Instant::now());
    grants.insert(
        token.clone(),
        WorkspaceCapabilityGrant {
            session_id: session_id.clone(),
            workdir: canonical.clone(),
            expires_at: Instant::now() + Duration::from_secs(8 * 60 * 60),
        },
    );
    Ok(WorkspaceCapabilityResponse {
        capability_token: token,
        workdir: path_for_display(&canonical),
        session_id,
    })
}

fn relative_workspace_path(raw: &str) -> Result<PathBuf, String> {
    let value = raw.trim().replace('\\', "/");
    if value.is_empty() || value == "." {
        return Ok(PathBuf::new());
    }
    let path = Path::new(&value);
    if path.is_absolute() {
        return Err(format!("workspace path must be relative: {raw}"));
    }
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(segment) => {
                if segment.to_string_lossy().contains(':') {
                    return Err(format!("invalid workspace path: {raw}"));
                }
                result.push(segment);
            }
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(format!("workspace path escapes workdir: {raw}"));
            }
        }
    }
    Ok(result)
}

/// A linked worktree exposes its Git metadata through a top-level `.git`
/// gitfile. Worktree child capabilities must not inspect or replace that file:
/// doing so could redirect later native Git operations to the parent index.
fn is_top_level_git_metadata_path(relative: &Path) -> bool {
    matches!(
        relative.components().next(),
        Some(std::path::Component::Normal(component))
            if component.to_string_lossy().eq_ignore_ascii_case(".git")
    )
}

fn is_worktree_child_for_workdir(
    state: &AppState,
    capability_token: Option<&str>,
    workdir: &Path,
) -> bool {
    let Some(token) = capability_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    state
        .subagent_capabilities
        .lock()
        .get(token)
        .is_some_and(|grant| {
            grant.mode == SubagentCapabilityMode::Worktree && grant.workdir == workdir
        })
}

fn reject_worktree_child_git_metadata_access(
    state: &AppState,
    capability_token: Option<&str>,
    workdir: &Path,
    relative: &Path,
) -> Result<(), String> {
    if is_top_level_git_metadata_path(relative)
        && is_worktree_child_for_workdir(state, capability_token, workdir)
    {
        return Err("worktree subagents cannot access top-level Git metadata".to_string());
    }
    Ok(())
}

fn is_sensitive_workspace_path(path: &Path) -> bool {
    let path_components = path
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(component) => component.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>();
    let Some(file_name) = path_components.last() else {
        return false;
    };
    let file_name = file_name.to_ascii_lowercase();
    let contains_sensitive_directory = path_components.iter().any(|component| {
        matches!(
            component.to_ascii_lowercase().as_str(),
            ".ssh"
                | ".aws"
                | ".azure"
                | ".codex"
                | ".claude"
                | ".config"
                | ".docker"
                | ".gnupg"
                | ".grok"
                | ".kube"
                | "gcloud"
        )
    });
    contains_sensitive_directory
        || file_name == ".env"
        || file_name.starts_with(".env.")
        || file_name == ".git-credentials"
        || file_name == ".netrc"
        || file_name == ".npmrc"
        || file_name.ends_with(".pem")
        || file_name.ends_with(".key")
        || file_name.ends_with(".p12")
        || file_name.ends_with(".pfx")
        || file_name.starts_with("id_rsa")
        || (file_name.starts_with("credentials") && file_name.ends_with(".json"))
        || (file_name.starts_with("service-account") && file_name.ends_with(".json"))
}

fn sensitive_read_permission_requirement(mode: PermissionMode) -> PermissionRequirement {
    match mode {
        PermissionMode::Readonly => PermissionRequirement::Deny,
        PermissionMode::Full => PermissionRequirement::Allow,
        PermissionMode::Ask => PermissionRequirement::Approval,
    }
}

/// Sensitive workspace reads require an active Agent capability, never the
/// renderer's long-lived workspace token. Ask and auto modes consume the
/// normal, one-use native approval after Pi has requested it explicitly.
fn require_sensitive_read_capability(
    state: &AppState,
    capability_token: Option<&str>,
    workdir: &str,
    expected_tool_name: &str,
    tool_call_id: Option<&str>,
) -> Result<(), String> {
    let token = capability_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "capability token is required for sensitive workspace reads".to_string())?;
    let grant = state
        .capabilities
        .lock()
        .get(token)
        .cloned()
        .ok_or_else(|| {
            "sensitive workspace reads require an active Agent capability".to_string()
        })?;
    require_capability_for(state, Some(token), workdir, ToolAction::Read, None)?;
    match sensitive_read_permission_requirement(grant.permission_mode) {
        PermissionRequirement::Allow => Ok(()),
        PermissionRequirement::Deny => Err(
            "current capability is read-only and cannot read sensitive workspace files".to_string(),
        ),
        PermissionRequirement::Approval => {
            // Generic "always allow Read" must not cover sensitive files: those
            // remain one-use approvals bound to an explicit tool_call_id and
            // the exact Read/List/Grep tool that requested them.
            let tool_call_id = tool_call_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    "explicit approval is required for sensitive workspace reads".to_string()
                })?;
            let approval = state
                .tool_approvals
                .lock()
                .remove(tool_call_id)
                .ok_or_else(|| {
                    "sensitive workspace read approval is missing or already used".to_string()
                })?;
            if approval.capability_token != token
                || approval.action != ToolAction::Read
                || approval.tool_name != expected_tool_name
                || approval.expires_at <= Instant::now()
            {
                return Err(
                    "sensitive workspace read approval does not match this operation".to_string(),
                );
            }
            Ok(())
        }
    }
}

/// Resolve one absolute regular-file path for GlobalRead. The capability check
/// remains required, but the read does not need a separate user confirmation.
fn canonical_global_read_target(raw_path: &str) -> Result<PathBuf, String> {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() {
        return Err("GlobalRead requires an absolute file path".to_string());
    }
    let path = PathBuf::from(raw_path);
    if !path.is_absolute() {
        return Err("GlobalRead requires an absolute file path".to_string());
    }
    let canonical = fs::canonicalize(&path).map_err(|_| {
        "GlobalRead path is not accessible; provide an existing file path".to_string()
    })?;
    let metadata = fs::symlink_metadata(&canonical).map_err(|_| {
        "GlobalRead path is not accessible; provide an existing file path".to_string()
    })?;
    if metadata.file_type().is_symlink() || has_reparse_point(&metadata) || !metadata.is_file() {
        return Err("GlobalRead path must identify a regular file".to_string());
    }
    Ok(canonical)
}

fn existing_target(workdir: &Path, raw_path: &str) -> Result<(PathBuf, String), String> {
    let relative = relative_workspace_path(raw_path)?;
    let target = workdir.join(&relative);
    if relative.as_os_str().is_empty() {
        return Err("a file path is required".to_string());
    }
    reject_workspace_path_links(&target, workdir, "workspace file")?;
    let metadata = fs::symlink_metadata(&target)
        .map_err(|error| format!("path is not accessible: {}: {error}", raw_path))?;
    if metadata.file_type().is_symlink() || has_reparse_point(&metadata) {
        return Err(format!(
            "path must not be a symlink or reparse point: {raw_path}"
        ));
    }
    let canonical = fs::canonicalize(&target)
        .map_err(|error| format!("path is not accessible: {}: {error}", raw_path))?;
    if !canonical.starts_with(workdir) {
        return Err(format!("path is outside workdir: {raw_path}"));
    }
    let logical = relative.to_string_lossy().replace('\\', "/");
    // Keep the lexical path for the eventual no-follow open. Returning the
    // resolved target here would hide a symlink replacement that occurs after
    // this check from `open_checked_regular_file`.
    Ok((target, logical))
}

#[cfg(windows)]
fn has_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn has_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn reject_delete_links(
    workdir: &Path,
    relative: &Path,
    target: &Path,
) -> Result<fs::Metadata, String> {
    let mut component = workdir.to_path_buf();
    for part in relative.components() {
        component.push(part);
        let metadata = fs::symlink_metadata(&component)
            .map_err(|error| format!("stat delete target: {error}"))?;
        if metadata.file_type().is_symlink() || has_reparse_point(&metadata) {
            return Err("refusing to delete a symlink or reparse point".to_string());
        }
    }
    let metadata =
        fs::symlink_metadata(target).map_err(|error| format!("stat delete target: {error}"))?;
    if metadata.file_type().is_symlink() || has_reparse_point(&metadata) {
        return Err("refusing to delete a symlink or reparse point".to_string());
    }
    let canonical =
        fs::canonicalize(target).map_err(|error| format!("resolve delete target: {error}"))?;
    if !canonical.starts_with(workdir) {
        return Err("delete target is outside workdir".to_string());
    }
    if metadata.is_dir() {
        for entry in WalkDir::new(target).follow_links(false) {
            let entry = entry.map_err(|error| format!("scan delete target: {error}"))?;
            let entry_metadata = entry
                .metadata()
                .map_err(|error| format!("stat delete entry: {error}"))?;
            if entry_metadata.file_type().is_symlink() || has_reparse_point(&entry_metadata) {
                return Err(
                    "refusing to delete a tree containing a symlink or reparse point".to_string(),
                );
            }
            let entry_canonical = fs::canonicalize(entry.path())
                .map_err(|error| format!("resolve delete entry: {error}"))?;
            if !entry_canonical.starts_with(workdir) {
                return Err("delete tree contains a path outside workdir".to_string());
            }
        }
    }
    Ok(metadata)
}

/// Best-effort cleanup for media belonging to a session whose durable history
/// projection has already been deleted. This intentionally does not use
/// `composer_media_session_root`: that helper creates missing directories and
/// requires the session to still exist. Every component is independently
/// validated before the recursive removal is allowed.
fn cleanup_deleted_session_composer_media(
    state: &AppState,
    session_id: &str,
) -> Result<(), String> {
    let session_id = validate_composer_media_session_id(session_id)?;
    let data_dir = state
        .data_file
        .parent()
        .ok_or_else(|| "composer attachment storage is unavailable".to_string())?;
    let canonical_data_dir = fs::canonicalize(data_dir)
        .map_err(|_| "composer attachment storage is unavailable".to_string())?;
    let root_candidate = data_dir.join(COMPOSER_MEDIA_DIRECTORY);
    let root_metadata = match fs::symlink_metadata(&root_candidate) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("inspect composer attachment storage".to_string()),
    };
    if root_metadata.file_type().is_symlink()
        || has_reparse_point(&root_metadata)
        || !root_metadata.is_dir()
    {
        return Err("composer attachment storage directory is invalid".to_string());
    }
    let root = fs::canonicalize(&root_candidate)
        .map_err(|_| "resolve composer attachment storage".to_string())?;
    if root.parent() != Some(canonical_data_dir.as_path()) {
        return Err("composer attachment storage escaped its data directory".to_string());
    }

    let session_candidate = root.join(&session_id);
    let session_metadata = match fs::symlink_metadata(&session_candidate) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("inspect deleted session media".to_string()),
    };
    if session_metadata.file_type().is_symlink()
        || has_reparse_point(&session_metadata)
        || !session_metadata.is_dir()
    {
        return Err("deleted session media directory is invalid".to_string());
    }
    let session_root = fs::canonicalize(&session_candidate)
        .map_err(|_| "resolve deleted session media".to_string())?;
    if session_root.parent() != Some(root.as_path()) {
        return Err("deleted session media escaped its storage root".to_string());
    }

    // The existing tree validator rejects every symlink/reparse point and
    // confirms each canonical descendant remains inside `root` before the
    // final recursive deletion. A future retry is safe if this step fails.
    let _ = reject_delete_links(&root, Path::new(session_id.as_str()), &session_root)?;
    fs::remove_dir_all(&session_root).map_err(|_| "remove deleted session media".to_string())
}

fn cleanup_deleted_sessions_composer_media(state: &AppState, session_ids: &[String]) {
    for session_id in session_ids {
        if let Err(error) = cleanup_deleted_session_composer_media(state, session_id) {
            // History deletion is already durable at this point. Do not report
            // an overall delete failure or try to restore history after media
            // cleanup fails; leave only this bounded orphan for later/manual
            // recovery and retain a privacy-safe diagnostic signal.
            let _ = error;
            diagnostics::record_event(
                "attachments",
                "deleted_session_media_cleanup_failed",
                "failure",
                None,
            );
        }
    }
}

/// Validate a workspace write path without following a link/reparse-point
/// parent. Canonical containment alone is insufficient here: a junction that
/// happens to resolve back into the workspace still gives an attacker a name
/// they can replace between validation and the eventual create/rename.
fn reject_workspace_write_target_links(
    target: &Path,
    workdir: &Path,
    raw_path: &str,
) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| "invalid file path".to_string())?;
    reject_workspace_path_links(parent, workdir, "workspace write parent")?;
    let parent_canonical = fs::canonicalize(parent)
        .map_err(|error| format!("parent directory is not accessible: {error}"))?;
    if !parent_canonical.starts_with(workdir) {
        return Err(format!("path is outside workdir: {raw_path}"));
    }

    match fs::symlink_metadata(target) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || has_reparse_point(&metadata) {
                return Err(format!(
                    "refusing to write through a symlink or reparse point: {raw_path}"
                ));
            }
            if !metadata.is_file() {
                return Err(format!("write target must be a regular file: {raw_path}"));
            }
            let canonical_target = fs::canonicalize(target)
                .map_err(|error| format!("path is not accessible: {raw_path}: {error}"))?;
            if !canonical_target.starts_with(workdir) {
                return Err(format!("path is outside workdir: {raw_path}"));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("inspect write target {raw_path}: {error}")),
    }
    Ok(())
}

fn writable_target(workdir: &Path, raw_path: &str) -> Result<(PathBuf, String), String> {
    let relative = relative_workspace_path(raw_path)?;
    if relative.as_os_str().is_empty() {
        return Err("a file path is required".to_string());
    }
    let target = workdir.join(&relative);
    reject_workspace_write_target_links(&target, workdir, raw_path)?;
    let logical = relative.to_string_lossy().replace('\\', "/");
    Ok((target, logical))
}

fn metadata_mtime_ms(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Replace a text file through a same-directory temporary file. This keeps a
/// crash or disk-full failure from leaving a truncated target behind.
fn atomic_replace_file(target: &Path, content: &[u8]) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| "target has no parent directory".to_string())?;
    let temporary = parent.join(format!(".novavei-write-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("create temporary file: {error}"))?;
        file.write_all(content)
            .map_err(|error| format!("write temporary file: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("flush temporary file: {error}"))?;
        drop(file);

        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{
                MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            };
            let source = temporary
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>();
            let destination = target
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>();
            let moved = unsafe {
                MoveFileExW(
                    source.as_ptr(),
                    destination.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            };
            if moved == 0 {
                return Err(format!(
                    "atomically replace file: {}",
                    io::Error::last_os_error()
                ));
            }
        }
        #[cfg(not(windows))]
        {
            fs::rename(&temporary, target)
                .map_err(|error| format!("atomically replace file: {error}"))?;
            if let Ok(directory) = fs::File::open(parent) {
                let _ = directory.sync_all();
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Keep the final lexical link/reparse validation immediately next to the
/// pathname-based temporary-file and rename operations. This substantially
/// narrows normal swaps; fully closing a hostile concurrent parent-directory
/// replacement still requires handle-relative mutation APIs.
fn atomic_replace_workspace_file(
    target: &Path,
    workdir: &Path,
    raw_path: &str,
    content: &[u8],
) -> Result<(), String> {
    reject_workspace_write_target_links(target, workdir, raw_path)?;
    atomic_replace_file(target, content)
}

fn line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsReadResponse {
    pub kind: String,
    /// `project` for a workspace-relative read and `global` for an absolute
    /// path. The Agent response renders this field alongside the tool name, so
    /// the source scope stays visible in history.
    pub scope: String,
    pub path: String,
    pub content: Option<String>,
    pub truncated: bool,
    pub start_line: Option<usize>,
    pub num_lines: Option<usize>,
    pub total_lines: Option<usize>,
    pub mtime_ms: u64,
    pub content_hash: String,
    pub size_bytes: usize,
}

fn read_opened_text_impl(
    logical: String,
    scope: &str,
    file: fs::File,
    opened_metadata: fs::Metadata,
    start_line: Option<usize>,
    limit: Option<usize>,
) -> Result<FsReadResponse, String> {
    let mut bytes = Vec::new();
    file.take((MAX_READ_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {logical}: {error}"))?;
    let exceeded_read_limit = bytes.len() > MAX_READ_BYTES;
    if exceeded_read_limit {
        bytes.truncate(MAX_READ_BYTES);
    }
    let content_hash = hash_bytes(&bytes);
    let mut text =
        String::from_utf8(bytes).map_err(|_| format!("file is not UTF-8 text: {logical}"))?;
    let total_lines = line_count(&text);
    let requested_start = start_line.unwrap_or(1).max(1);
    let requested_limit = limit.unwrap_or(2000).clamp(1, 50_000);
    let lines = text.lines().map(str::to_string).collect::<Vec<_>>();
    let start_index = requested_start.saturating_sub(1).min(lines.len());
    let end_index = (start_index + requested_limit).min(lines.len());
    let is_partial = start_index > 0 || end_index < lines.len();
    if is_partial {
        text = lines[start_index..end_index].join("\n");
        if end_index < lines.len() {
            text.push('\n');
        }
    }
    let truncated = exceeded_read_limit || opened_metadata.len() as usize > MAX_READ_BYTES;
    Ok(FsReadResponse {
        kind: "text".to_string(),
        scope: scope.to_string(),
        path: logical,
        content: Some(text.clone()),
        truncated: truncated || is_partial,
        start_line: Some(requested_start),
        num_lines: Some(line_count(&text)),
        total_lines: Some(total_lines),
        mtime_ms: metadata_mtime_ms(&opened_metadata),
        content_hash,
        size_bytes: opened_metadata.len() as usize,
    })
}

fn read_text_impl(
    workdir: String,
    path: String,
    start_line: Option<usize>,
    limit: Option<usize>,
) -> Result<FsReadResponse, String> {
    let workdir = canonical_workdir(&workdir)?;
    let (target, logical) = existing_target(&workdir, &path)?;
    let (_, file, opened_metadata) = open_checked_regular_file(&target, Some(&workdir), &path)?;
    read_opened_text_impl(logical, "project", file, opened_metadata, start_line, limit)
}

#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)]
pub fn fs_read_text(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    path: String,
    start_line: Option<usize>,
    limit: Option<usize>,
    _page_start: Option<usize>,
    _page_limit: Option<usize>,
    _cell_start: Option<usize>,
    _cell_limit: Option<usize>,
    capability_token: Option<String>,
    tool_call_id: Option<String>,
) -> Result<FsReadResponse, String> {
    require_read_capability(&state, capability_token.as_deref(), &workdir)?;
    let canonical_workdir = canonical_workdir(&workdir)?;
    let requested_path = relative_workspace_path(&path)?;
    reject_worktree_child_git_metadata_access(
        &state,
        capability_token.as_deref(),
        &canonical_workdir,
        &requested_path,
    )?;
    let (target, logical) = existing_target(&canonical_workdir, &path)?;
    let normalized_path = target
        .strip_prefix(&canonical_workdir)
        .unwrap_or_else(|_| Path::new(&logical));
    if is_sensitive_workspace_path(normalized_path) {
        require_sensitive_read_capability(
            &state,
            capability_token.as_deref(),
            &workdir,
            "Read",
            tool_call_id.as_deref(),
        )?;
    }
    read_text_impl(workdir, path, start_line, limit)
}

/// Read an absolute file path outside the current project. The active run
/// supplies its project workdir solely to bind this read to the running Agent
/// capability; GlobalRead never grants write access or directory enumeration.
#[tauri::command(rename_all = "snake_case")]
pub fn fs_read_global_text(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    path: String,
    start_line: Option<usize>,
    limit: Option<usize>,
    capability_token: Option<String>,
    _tool_call_id: Option<String>,
) -> Result<FsReadResponse, String> {
    // Verify the active Agent capability before resolving any arbitrary path,
    // so this command is not a renderer-controlled filesystem oracle.
    require_global_read_capability(&state, capability_token.as_deref(), &workdir)?;
    let target = canonical_global_read_target(&path)?;
    let (opened_target, file, opened_metadata) =
        open_checked_regular_file(&target, None, "GlobalRead file")?;
    if opened_target != target {
        return Err("GlobalRead file changed while opening; retry the read".to_string());
    }
    read_opened_text_impl(
        path_for_display(&target),
        "global",
        file,
        opened_metadata,
        start_line,
        limit,
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsEditableTextResponse {
    pub path: String,
    pub content: String,
    pub mtime_ms: u64,
    pub content_hash: String,
    pub size_bytes: usize,
    pub total_lines: usize,
}

#[tauri::command(rename_all = "snake_case")]
pub fn fs_read_editable_text(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    path: String,
    capability_token: Option<String>,
    tool_call_id: Option<String>,
) -> Result<FsEditableTextResponse, String> {
    // Worktree children use the same constrained read boundary as Read,
    // List, and Grep. Their top-level `.git` metadata is separately blocked,
    // and sensitive files still require their separate approval below.
    require_read_capability(&state, capability_token.as_deref(), &workdir)?;
    let workdir = canonical_workdir(&workdir)?;
    let requested_path = relative_workspace_path(&path)?;
    reject_worktree_child_git_metadata_access(
        &state,
        capability_token.as_deref(),
        &workdir,
        &requested_path,
    )?;
    let (target, logical) = existing_target(&workdir, &path)?;
    let normalized_path = target
        .strip_prefix(&workdir)
        .unwrap_or_else(|_| Path::new(&logical));
    if is_sensitive_workspace_path(normalized_path) {
        require_sensitive_read_capability(
            &state,
            capability_token.as_deref(),
            &path_for_display(&workdir),
            "Read",
            tool_call_id.as_deref(),
        )?;
    }
    let (checked_target, bytes) = read_bounded_regular_file(
        &target,
        MAX_WRITE_BYTES as u64,
        true,
        Some(&workdir),
        "editable file",
    )?;
    let metadata =
        fs::metadata(&checked_target).map_err(|error| format!("stat {path}: {error}"))?;
    let content =
        String::from_utf8(bytes.clone()).map_err(|_| format!("file is not UTF-8 text: {path}"))?;
    Ok(FsEditableTextResponse {
        path: logical,
        content: content.clone(),
        mtime_ms: metadata_mtime_ms(&metadata),
        content_hash: hash_bytes(&bytes),
        size_bytes: bytes.len(),
        total_lines: line_count(&content),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsPathStatusResponse {
    pub path: String,
    pub exists: bool,
    pub kind: Option<String>,
    pub size_bytes: Option<u64>,
    pub mtime_ms: Option<u64>,
}

#[tauri::command(rename_all = "snake_case")]
pub fn fs_path_status(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    path: String,
    capability_token: Option<String>,
) -> Result<FsPathStatusResponse, String> {
    require_read_capability(&state, capability_token.as_deref(), &workdir)?;
    let workdir = canonical_workdir(&workdir)?;
    let relative = relative_workspace_path(&path)?;
    reject_worktree_child_git_metadata_access(
        &state,
        capability_token.as_deref(),
        &workdir,
        &relative,
    )?;
    let target = workdir.join(&relative);
    let logical = relative.to_string_lossy().replace('\\', "/");
    match fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                let canonical = fs::canonicalize(&target)
                    .map_err(|error| format!("resolve symlink {path}: {error}"))?;
                if !canonical.starts_with(&workdir) {
                    return Err(format!("path is outside workdir: {path}"));
                }
            }
            let kind = if metadata.is_dir() {
                "dir"
            } else if metadata.is_file() {
                "file"
            } else {
                "other"
            };
            Ok(FsPathStatusResponse {
                path: logical,
                exists: true,
                kind: Some(kind.to_string()),
                size_bytes: Some(metadata.len()),
                mtime_ms: Some(metadata_mtime_ms(&metadata)),
            })
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(FsPathStatusResponse {
            path: logical,
            exists: false,
            kind: None,
            size_bytes: None,
            mtime_ms: None,
        }),
        Err(error) => Err(format!("stat {path}: {error}")),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsWriteResponse {
    pub path: String,
    pub mode: String,
    pub existed_before: bool,
    pub bytes_written: usize,
    pub mtime_ms: u64,
    pub content_hash: String,
    pub total_lines: usize,
}

#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)]
pub fn fs_write_text(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    path: String,
    content: String,
    mode: String,
    expected_mtime_ms: Option<u64>,
    expected_content_hash: Option<String>,
    capability_token: Option<String>,
    tool_call_id: Option<String>,
) -> Result<FsWriteResponse, String> {
    if content.len() > MAX_WRITE_BYTES {
        return Err("content exceeds the native write limit".to_string());
    }
    if !mode.is_empty() && mode != "rewrite" && mode != "replace" {
        return Err("write mode must be rewrite".to_string());
    }
    require_workspace_mutation_capability(
        &state,
        capability_token.as_deref(),
        &workdir,
        ToolAction::Write,
        tool_call_id.as_deref(),
    )?;
    let workdir = canonical_workdir(&workdir)?;
    let relative = relative_workspace_path(&path)?;
    reject_worktree_child_git_metadata_access(
        &state,
        capability_token.as_deref(),
        &workdir,
        &relative,
    )?;
    let (target, logical) = writable_target(&workdir, &path)?;
    let existed_before = match fs::symlink_metadata(&target) {
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("inspect write target {path}: {error}")),
    };
    if existed_before {
        let (_, existing) = read_bounded_regular_file(
            &target,
            MAX_WRITE_BYTES as u64,
            true,
            Some(&workdir),
            "existing editable file",
        )?;
        if let Some(expected) = expected_content_hash.as_deref() {
            if expected != hash_bytes(&existing) {
                return Err(format!("content changed since last read: {path}"));
            }
        }
        if let Some(expected) = expected_mtime_ms {
            let metadata =
                fs::metadata(&target).map_err(|error| format!("stat {path}: {error}"))?;
            if expected != metadata_mtime_ms(&metadata) {
                return Err(format!("file changed since last read: {path}"));
            }
        }
    }
    recheck_mutation_capability(
        &state,
        capability_token.as_deref(),
        &workdir,
        ToolAction::Write,
    )?;
    atomic_replace_workspace_file(&target, &workdir, &path, content.as_bytes())
        .map_err(|error| format!("write {path}: {error}"))?;
    let metadata = fs::metadata(&target).map_err(|error| format!("stat {path}: {error}"))?;
    Ok(FsWriteResponse {
        path: logical,
        mode: "rewrite".to_string(),
        existed_before,
        bytes_written: content.len(),
        mtime_ms: metadata_mtime_ms(&metadata),
        content_hash: hash_bytes(content.as_bytes()),
        total_lines: line_count(&content),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsEditResponse {
    pub path: String,
    pub replacements: usize,
    pub replace_all: bool,
    pub match_strategy: String,
    pub mtime_ms: u64,
    pub content_hash: String,
    pub total_lines: usize,
}

#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)]
pub fn fs_edit_text(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    path: String,
    old_string: String,
    new_string: String,
    expected_replacements: Option<usize>,
    replace_all: Option<bool>,
    expected_mtime_ms: Option<u64>,
    expected_content_hash: Option<String>,
    capability_token: Option<String>,
    tool_call_id: Option<String>,
) -> Result<FsEditResponse, String> {
    if old_string.is_empty() {
        return Err("old_string must not be empty".to_string());
    }
    require_workspace_mutation_capability(
        &state,
        capability_token.as_deref(),
        &workdir,
        ToolAction::Edit,
        tool_call_id.as_deref(),
    )?;
    let workdir = canonical_workdir(&workdir)?;
    let relative = relative_workspace_path(&path)?;
    reject_worktree_child_git_metadata_access(
        &state,
        capability_token.as_deref(),
        &workdir,
        &relative,
    )?;
    let (target, logical) = existing_target(&workdir, &path)?;
    let (checked_target, current_bytes) = read_bounded_regular_file(
        &target,
        MAX_WRITE_BYTES as u64,
        true,
        Some(&workdir),
        "editable file",
    )?;
    if let Some(expected) = expected_content_hash.as_deref() {
        if expected != hash_bytes(&current_bytes) {
            return Err(format!("content changed since last read: {path}"));
        }
    }
    if let Some(expected) = expected_mtime_ms {
        let metadata =
            fs::metadata(&checked_target).map_err(|error| format!("stat {path}: {error}"))?;
        if expected != metadata_mtime_ms(&metadata) {
            return Err(format!("file changed since last read: {path}"));
        }
    }
    let current =
        String::from_utf8(current_bytes).map_err(|_| format!("file is not UTF-8 text: {path}"))?;
    let count = current.matches(&old_string).count();
    if count == 0 {
        return Err(format!("old_string was not found in {path}"));
    }
    let all = replace_all.unwrap_or(false);
    if !all && count > 1 {
        return Err(format!(
            "old_string matched {count} locations; set replace_all to continue"
        ));
    }
    let actual = if all { count } else { 1 };
    if let Some(expected) = expected_replacements {
        if expected != actual {
            return Err(format!("expected {expected} replacements, found {actual}"));
        }
    }
    let next = if all {
        current.replace(&old_string, &new_string)
    } else {
        current.replacen(&old_string, &new_string, 1)
    };
    if next.len() > MAX_WRITE_BYTES {
        return Err(format!(
            "edited content exceeds the native write limit: {path}"
        ));
    }
    recheck_mutation_capability(
        &state,
        capability_token.as_deref(),
        &workdir,
        ToolAction::Edit,
    )?;
    atomic_replace_workspace_file(&target, &workdir, &path, next.as_bytes())
        .map_err(|error| format!("write {path}: {error}"))?;
    let metadata = fs::metadata(&target).map_err(|error| format!("stat {path}: {error}"))?;
    Ok(FsEditResponse {
        path: logical,
        replacements: actual,
        replace_all: all,
        match_strategy: "exact".to_string(),
        mtime_ms: metadata_mtime_ms(&metadata),
        content_hash: hash_bytes(next.as_bytes()),
        total_lines: line_count(&next),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsDeleteResponse {
    pub path: String,
    pub kind: String,
}

#[tauri::command(rename_all = "snake_case")]
pub fn fs_delete(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    path: String,
    capability_token: Option<String>,
    tool_call_id: Option<String>,
) -> Result<FsDeleteResponse, String> {
    require_workspace_mutation_capability(
        &state,
        capability_token.as_deref(),
        &workdir,
        ToolAction::Delete,
        tool_call_id.as_deref(),
    )?;
    let workdir = canonical_workdir(&workdir)?;
    let relative = relative_workspace_path(&path)?;
    if relative.as_os_str().is_empty() {
        return Err("refusing to delete workdir".to_string());
    }
    reject_worktree_child_git_metadata_access(
        &state,
        capability_token.as_deref(),
        &workdir,
        &relative,
    )?;
    let target = workdir.join(&relative);
    reject_delete_links(&workdir, &relative, &target)?;
    // Recheck the logical target immediately before the destructive call.
    // This does not eliminate a hostile local race, but prevents a normal
    // symlink/junction swap between the preflight and deletion.
    let final_metadata = reject_delete_links(&workdir, &relative, &target)?;
    recheck_mutation_capability(
        &state,
        capability_token.as_deref(),
        &workdir,
        ToolAction::Delete,
    )?;
    let kind = if final_metadata.is_dir() {
        fs::remove_dir_all(&target).map_err(|error| format!("delete {path}: {error}"))?;
        "dir"
    } else {
        fs::remove_file(&target).map_err(|error| format!("delete {path}: {error}"))?;
        "file"
    };
    Ok(FsDeleteResponse {
        path: relative.to_string_lossy().replace('\\', "/"),
        kind: kind.to_string(),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsEntry {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub size_bytes: u64,
    pub mtime_ms: u64,
}

#[tauri::command(rename_all = "snake_case")]
pub fn fs_list(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    path: Option<String>,
    include_hidden: Option<bool>,
    capability_token: Option<String>,
    tool_call_id: Option<String>,
) -> Result<Vec<FsEntry>, String> {
    require_read_capability(&state, capability_token.as_deref(), &workdir)?;
    let workdir = canonical_workdir(&workdir)?;
    let relative = relative_workspace_path(path.as_deref().unwrap_or("."))?;
    reject_worktree_child_git_metadata_access(
        &state,
        capability_token.as_deref(),
        &workdir,
        &relative,
    )?;
    let hide_worktree_git_metadata =
        is_worktree_child_for_workdir(&state, capability_token.as_deref(), &workdir);
    if is_sensitive_workspace_path(&relative) {
        require_sensitive_read_capability(
            &state,
            capability_token.as_deref(),
            &path_for_display(&workdir),
            "List",
            tool_call_id.as_deref(),
        )?;
    }
    let directory = workdir.join(&relative);
    let directory = fs::canonicalize(&directory)
        .map_err(|error| format!("list target is not accessible: {error}"))?;
    if !directory.starts_with(&workdir) {
        return Err("list target is outside workdir".to_string());
    }
    if !directory.is_dir() {
        return Err("list target is not a directory".to_string());
    }
    let include_hidden = include_hidden.unwrap_or(false);
    let mut entries = Vec::new();
    for entry in fs::read_dir(&directory).map_err(|error| format!("list directory: {error}"))? {
        let entry = entry.map_err(|error| format!("read directory entry: {error}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !include_hidden && name.starts_with('.') {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|error| format!("stat directory entry: {error}"))?;
        let child_relative = relative.join(&name);
        if hide_worktree_git_metadata && is_top_level_git_metadata_path(&child_relative) {
            continue;
        }
        if let Ok(canonical_child) = fs::canonicalize(entry.path()) {
            if !canonical_child.starts_with(&workdir) {
                continue;
            }
            let normalized_child = canonical_child
                .strip_prefix(&workdir)
                .unwrap_or(child_relative.as_path());
            if !is_sensitive_workspace_path(&relative)
                && is_sensitive_workspace_path(normalized_child)
            {
                continue;
            }
        } else {
            continue;
        }
        entries.push(FsEntry {
            path: child_relative.to_string_lossy().replace('\\', "/"),
            name,
            kind: if metadata.is_dir() { "dir" } else { "file" }.to_string(),
            size_bytes: metadata.len(),
            mtime_ms: metadata_mtime_ms(&metadata),
        });
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrepMatch {
    pub path: String,
    pub line: usize,
    pub text: String,
    pub before: Vec<String>,
    pub after: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrepFileSummary {
    pub path: String,
    pub count: usize,
    pub first_line: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrepResponse {
    pub path: Option<String>,
    pub pattern: String,
    pub file_pattern: Option<String>,
    pub ignore_case: bool,
    pub output_mode: String,
    pub head_limit: usize,
    pub offset: usize,
    pub context: usize,
    pub multiline: bool,
    pub match_count: usize,
    pub file_count: usize,
    pub has_more: bool,
    pub matches: Vec<GrepMatch>,
    pub files: Vec<GrepFileSummary>,
    pub target_kind: Option<String>,
}

fn grep_context(lines: &[&str], line: usize, context: usize) -> (Vec<String>, Vec<String>) {
    let index = line.saturating_sub(1);
    let before_start = index.saturating_sub(context);
    let before = lines[before_start..index]
        .iter()
        .map(|value| (*value).to_string())
        .collect();
    let after_end = (index + context + 1).min(lines.len());
    let after = lines[index + 1..after_end]
        .iter()
        .map(|value| (*value).to_string())
        .collect();
    (before, after)
}

fn should_skip_grep_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | ".hg" | ".svn" | "node_modules" | "target" | "dist" | "build" | ".next"
    )
}

#[allow(clippy::too_many_arguments)]
fn fs_grep_sync(
    workdir: String,
    path: Option<String>,
    pattern: String,
    file_pattern: Option<String>,
    ignore_case: Option<bool>,
    output_mode: Option<String>,
    head_limit: Option<usize>,
    offset: Option<usize>,
    context: Option<usize>,
    multiline: Option<bool>,
    include_hidden: Option<bool>,
    include_sensitive_target: bool,
    exclude_worktree_git_metadata: bool,
) -> Result<GrepResponse, String> {
    let workdir = canonical_workdir(&workdir)?;
    let relative = relative_workspace_path(path.as_deref().unwrap_or("."))?;
    let base = workdir.join(&relative);
    if !relative.as_os_str().is_empty() {
        reject_workspace_path_links(&base, &workdir, "grep target")?;
    }
    let base_link_metadata =
        fs::symlink_metadata(&base).map_err(|error| format!("grep target: {error}"))?;
    if base_link_metadata.file_type().is_symlink() || has_reparse_point(&base_link_metadata) {
        return Err("grep target must not be a symlink or reparse point".to_string());
    }
    let base_canonical =
        fs::canonicalize(&base).map_err(|error| format!("grep target: {error}"))?;
    if !base_canonical.starts_with(&workdir) {
        return Err("grep target is outside workdir".to_string());
    }
    let metadata =
        fs::metadata(&base_canonical).map_err(|error| format!("grep target: {error}"))?;
    let target_kind = if metadata.is_file() {
        "file"
    } else if metadata.is_dir() {
        "dir"
    } else {
        return Err("grep target is not a file or directory".to_string());
    };
    let pattern = pattern.trim().to_string();
    if pattern.is_empty() {
        return Err("grep pattern cannot be empty".to_string());
    }
    let ignore_case = ignore_case.unwrap_or(true);
    let output_mode = output_mode.unwrap_or_else(|| "content".to_string());
    if !matches!(output_mode.as_str(), "content" | "files" | "count") {
        return Err("grep output_mode must be content, files, or count".to_string());
    }
    let head_limit = head_limit.unwrap_or(100).clamp(1, 1000);
    let offset = offset.unwrap_or(0);
    let context = context.unwrap_or(0).min(20);
    let multiline = multiline.unwrap_or(false);
    // Keep existing tool callers compatible: historically grep included normal
    // dot files. UI surfaces that mirror the Files dock opt out explicitly.
    let include_hidden = include_hidden.unwrap_or(true);
    let regex = regex::RegexBuilder::new(&pattern)
        .case_insensitive(ignore_case)
        .multi_line(multiline)
        .dot_matches_new_line(multiline)
        .build()
        .map_err(|error| format!("invalid grep pattern: {error}"))?;
    let globset = file_pattern
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            let mut builder = globset::GlobSetBuilder::new();
            for part in value
                .split('|')
                .map(str::trim)
                .filter(|part| !part.is_empty())
            {
                builder.add(
                    globset::Glob::new(part)
                        .map_err(|error| format!("invalid file pattern: {error}"))?,
                );
            }
            builder
                .build()
                .map_err(|error| format!("invalid file pattern: {error}"))
        })
        .transpose()?;

    let mut match_count = 0usize;
    let mut file_summaries = BTreeMap::<String, GrepFileSummary>::new();
    let mut matches = Vec::new();
    let candidates = if target_kind == "file" {
        vec![base_canonical.clone()]
    } else {
        WalkDir::new(&base_canonical)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                let name = entry.file_name().to_string_lossy();
                let is_explicit_base = entry.path() == base_canonical.as_path();
                (include_hidden || is_explicit_base || !name.starts_with('.'))
                    && (!entry.file_type().is_dir() || !should_skip_grep_dir(&name))
            })
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| entry.into_path())
            .collect::<Vec<_>>()
    };

    for candidate in candidates {
        let (canonical, bytes) = match read_bounded_regular_file(
            &candidate,
            MAX_READ_BYTES as u64,
            true,
            Some(&workdir),
            "workspace search candidate",
        ) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let relative_path = canonical
            .strip_prefix(&workdir)
            .unwrap_or(&canonical)
            .to_string_lossy()
            .replace('\\', "/");
        if exclude_worktree_git_metadata
            && is_top_level_git_metadata_path(Path::new(&relative_path))
        {
            continue;
        }
        if is_sensitive_workspace_path(Path::new(&relative_path)) && !include_sensitive_target {
            continue;
        }
        if let Some(globset) = globset.as_ref() {
            let file_name = canonical
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if !globset.is_match(&relative_path) && !globset.is_match(file_name) {
                continue;
            }
        }
        if bytes.contains(&0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let lines = text.lines().collect::<Vec<_>>();
        let mut file_count = 0usize;
        let mut first_line = None;
        if multiline {
            for found in regex.find_iter(&text) {
                let line = text[..found.start()]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count()
                    + 1;
                file_count += 1;
                first_line.get_or_insert(line);
                match_count += 1;
                if output_mode == "content" && match_count > offset && matches.len() < head_limit {
                    let (before, after) = grep_context(&lines, line, context);
                    matches.push(GrepMatch {
                        path: relative_path.clone(),
                        line,
                        text: lines.get(line.saturating_sub(1)).unwrap_or(&"").to_string(),
                        before,
                        after,
                    });
                }
            }
        } else {
            for (index, line_text) in lines.iter().enumerate() {
                if !regex.is_match(line_text) {
                    continue;
                }
                let line = index + 1;
                file_count += 1;
                first_line.get_or_insert(line);
                match_count += 1;
                if output_mode == "content" && match_count > offset && matches.len() < head_limit {
                    let (before, after) = grep_context(&lines, line, context);
                    matches.push(GrepMatch {
                        path: relative_path.clone(),
                        line,
                        text: (*line_text).to_string(),
                        before,
                        after,
                    });
                }
            }
        }
        if file_count > 0 {
            file_summaries.insert(
                relative_path.clone(),
                GrepFileSummary {
                    path: relative_path,
                    count: file_count,
                    first_line,
                },
            );
        }
    }

    let file_count = file_summaries.len();
    let mut files = if output_mode == "files" {
        file_summaries.values().cloned().collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    files.sort_by(|left, right| left.path.cmp(&right.path));
    if output_mode == "files" {
        files = files.into_iter().skip(offset).take(head_limit).collect();
    }
    matches.sort_by(|left, right| left.path.cmp(&right.path).then(left.line.cmp(&right.line)));
    let has_more = match output_mode.as_str() {
        "files" => offset.saturating_add(head_limit) < file_count,
        "count" => false,
        _ => offset.saturating_add(head_limit) < match_count,
    };
    Ok(GrepResponse {
        path: Some(relative.to_string_lossy().replace('\\', "/")),
        pattern,
        file_pattern,
        ignore_case,
        output_mode,
        head_limit,
        offset,
        context,
        multiline,
        match_count,
        file_count,
        has_more,
        matches,
        files,
        target_kind: Some(target_kind.to_string()),
    })
}

#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)]
pub async fn fs_grep(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    path: Option<String>,
    pattern: String,
    file_pattern: Option<String>,
    ignore_case: Option<bool>,
    output_mode: Option<String>,
    head_limit: Option<usize>,
    offset: Option<usize>,
    context: Option<usize>,
    multiline: Option<bool>,
    include_hidden: Option<bool>,
    capability_token: Option<String>,
    tool_call_id: Option<String>,
) -> Result<GrepResponse, String> {
    require_read_capability(&state, capability_token.as_deref(), &workdir)?;
    let canonical_workdir = canonical_workdir(&workdir)?;
    let requested_path = relative_workspace_path(path.as_deref().unwrap_or("."))?;
    reject_worktree_child_git_metadata_access(
        &state,
        capability_token.as_deref(),
        &canonical_workdir,
        &requested_path,
    )?;
    let exclude_worktree_git_metadata =
        is_worktree_child_for_workdir(&state, capability_token.as_deref(), &canonical_workdir);
    let canonical_target = fs::canonicalize(canonical_workdir.join(requested_path))
        .map_err(|error| format!("grep target: {error}"))?;
    if !canonical_target.starts_with(&canonical_workdir) {
        return Err("grep target is outside workdir".to_string());
    }
    let normalized_target = canonical_target
        .strip_prefix(&canonical_workdir)
        .unwrap_or_else(|_| Path::new(""));
    let include_sensitive_target = is_sensitive_workspace_path(normalized_target);
    if include_sensitive_target {
        require_sensitive_read_capability(
            &state,
            capability_token.as_deref(),
            &workdir,
            "Grep",
            tool_call_id.as_deref(),
        )?;
    }
    tauri::async_runtime::spawn_blocking(move || {
        fs_grep_sync(
            workdir,
            path,
            pattern,
            file_pattern,
            ignore_case,
            output_mode,
            head_limit,
            offset,
            context,
            multiline,
            include_hidden,
            include_sensitive_target,
            exclude_worktree_git_metadata,
        )
    })
    .await
    .map_err(|error| format!("fs_grep join failed: {error}"))?
}

#[tauri::command(rename_all = "snake_case")]
pub fn fs_roots(
    state: State<'_, Arc<AppState>>,
    capability_token: Option<String>,
    workdir: String,
) -> Result<Value, String> {
    let canonical = require_read_capability(&state, capability_token.as_deref(), &workdir)?;
    Ok(json!({
        "roots": [{"path": path_for_display(&canonical), "kind": "workspace"}]
    }))
}

// ---------------------------------------------------------------------------
// Shell execution

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellRunResponse {
    pub exit_code: i32,
    pub shell: String,
    pub platform: String,
    pub profile: String,
    pub shell_family: String,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdio_open_after_exit: bool,
    pub effective_timeout_ms: u64,
    pub duration_ms: u128,
}

#[derive(Debug, Default, Clone)]
struct ShellOutputSnapshot {
    bytes: Vec<u8>,
    truncated: bool,
}

struct ShellOutputCapture {
    snapshot: Arc<Mutex<ShellOutputSnapshot>>,
    reader: std::thread::JoinHandle<()>,
}

fn bounded_output(mut reader: impl Read + Send + 'static) -> ShellOutputCapture {
    let snapshot = Arc::new(Mutex::new(ShellOutputSnapshot::default()));
    let reader_snapshot = snapshot.clone();
    let reader = std::thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    let mut output = reader_snapshot.lock();
                    if output.bytes.len() < MAX_COMMAND_OUTPUT_BYTES {
                        let remaining = MAX_COMMAND_OUTPUT_BYTES - output.bytes.len();
                        output
                            .bytes
                            .extend_from_slice(&buffer[..count.min(remaining)]);
                    }
                    if output.bytes.len() >= MAX_COMMAND_OUTPUT_BYTES
                        || count > MAX_COMMAND_OUTPUT_BYTES
                    {
                        output.truncated = true;
                    }
                }
                Err(_) => break,
            }
        }
    });
    ShellOutputCapture { snapshot, reader }
}

/// Collect the output that has arrived without allowing a copied child pipe
/// handle to block the shell response indefinitely. If the reader remains
/// open, its thread is intentionally detached and the partial snapshot is
/// returned with an explicit incomplete-output signal.
fn collect_bounded_output(
    capture: Option<ShellOutputCapture>,
    deadline: Instant,
) -> (Vec<u8>, bool, bool) {
    let Some(ShellOutputCapture { snapshot, reader }) = capture else {
        return (Vec::new(), false, false);
    };
    while !reader.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if reader.is_finished() {
        let reader_panicked = reader.join().is_err();
        let snapshot = snapshot.lock().clone();
        return (snapshot.bytes, snapshot.truncated || reader_panicked, false);
    }

    let snapshot = snapshot.lock().clone();
    // Dropping an unfinished JoinHandle detaches it. That reader owns the pipe
    // and can finish naturally if the last inherited handle is eventually
    // closed, while the caller receives a bounded response now.
    (snapshot.bytes, true, true)
}

/// Hide console-subsystem helpers (PowerShell, cmd) on Windows so Dock terminal
/// and Agent shell tools do not flash a black console window.
#[cfg(windows)]
fn configure_hidden_console_process(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_hidden_console_process(_command: &mut Command) {}

/// Scoped registration for a cancellable shell run. Keeping this guard alive
/// from before process creation through output collection makes cancellation
/// visible before a shell can start and guarantees cleanup on every return.
struct ShellRunRegistrationGuard<'a> {
    state: &'a AppState,
    run_id: String,
    cancelled: Arc<AtomicBool>,
}

impl ShellRunRegistrationGuard<'_> {
    fn cancellation_token(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }
}

impl Drop for ShellRunRegistrationGuard<'_> {
    fn drop(&mut self) {
        let mut shell_runs = self.state.shell_runs.lock();
        let is_current = shell_runs
            .get(&self.run_id)
            .is_some_and(|run| Arc::ptr_eq(&run.cancelled, &self.cancelled));
        if is_current {
            shell_runs.remove(&self.run_id);
        }
    }
}

fn register_shell_run<'a>(
    state: &'a AppState,
    run_id: Option<&str>,
    capability_token: Option<&str>,
) -> Result<Option<ShellRunRegistrationGuard<'a>>, String> {
    let Some(run_id) = run_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let Some(capability_token) = capability_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };

    let cancelled = Arc::new(AtomicBool::new(false));
    let mut shell_runs = state.shell_runs.lock();
    if shell_runs.contains_key(run_id) {
        return Err("shell run id is already active".to_string());
    }
    shell_runs.insert(
        run_id.to_string(),
        ShellRunRegistration {
            capability_token: capability_token.to_string(),
            cancelled: cancelled.clone(),
        },
    );
    Ok(Some(ShellRunRegistrationGuard {
        state,
        run_id: run_id.to_string(),
        cancelled,
    }))
}

fn cancel_shell_run(state: &AppState, run_id: &str, capability_token: Option<&str>) -> bool {
    let run_id = run_id.trim();
    let Some(capability_token) = capability_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    if run_id.is_empty() {
        return false;
    }
    let shell_runs = state.shell_runs.lock();
    let Some(run) = shell_runs.get(run_id) else {
        return false;
    };
    if run.capability_token != capability_token {
        return false;
    }
    run.cancelled.store(true, Ordering::SeqCst);
    true
}

/// Stop the shell and every process it spawned on Windows.
///
/// `Child::kill` only terminates the directly launched PowerShell process.
/// PowerShell commands such as `Start-Process` can otherwise leave build
/// servers, package managers, or other descendants running after a terminal
/// cancellation or timeout. Use the system `taskkill` tree mode first, then
/// retain the direct kill as a fallback if the helper cannot be started.
#[cfg(windows)]
fn terminate_shell_process_tree(child: &mut std::process::Child) {
    let taskkill = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join("taskkill.exe");
    if taskkill.is_file() {
        let pid = child.id().to_string();
        let mut killer = Command::new(taskkill);
        killer
            .args(["/PID", &pid, "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_hidden_console_process(&mut killer);
        if let Ok(mut process) = killer.spawn() {
            let deadline = Instant::now() + Duration::from_millis(SHELL_REAP_TIMEOUT_MS);
            loop {
                match process.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(20));
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
    let _ = child.kill();
}

#[cfg(not(windows))]
fn terminate_shell_process_tree(child: &mut std::process::Child) {
    let _ = child.kill();
}

fn wait_for_terminated_shell(
    child: &mut std::process::Child,
    operation: &str,
) -> Result<std::process::ExitStatus, String> {
    let deadline = Instant::now() + Duration::from_millis(SHELL_REAP_TIMEOUT_MS);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                return Err(format!(
                    "{operation} shell did not exit after process-tree termination"
                ));
            }
            Err(error) => return Err(format!("reap {operation} shell: {error}")),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_shell_sync(
    state: &AppState,
    workdir: String,
    command: String,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
    max_timeout_ms: Option<u64>,
    run_id: Option<String>,
    capability_token: Option<String>,
    tool_call_id: Option<String>,
) -> Result<ShellRunResponse, String> {
    let workdir_path = require_capability_for(
        state,
        capability_token.as_deref(),
        &workdir,
        ToolAction::Shell,
        tool_call_id.as_deref(),
    )?;
    run_shell_sync_authorized(
        state,
        workdir,
        command,
        cwd,
        timeout_ms,
        max_timeout_ms,
        run_id,
        capability_token,
        workdir_path,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_shell_sync_authorized(
    state: &AppState,
    _workdir: String,
    command: String,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
    max_timeout_ms: Option<u64>,
    run_id: Option<String>,
    capability_token: Option<String>,
    workdir_path: PathBuf,
    recheck_agent_capability: bool,
) -> Result<ShellRunResponse, String> {
    let command = command.trim().to_string();
    if command.is_empty() {
        return Err("command cannot be empty".to_string());
    }
    let actual_cwd = match cwd.filter(|value| !value.trim().is_empty()) {
        None => workdir_path.clone(),
        Some(value) if Path::new(&value).is_absolute() => {
            let canonical = canonical_workdir(&value)?;
            if !canonical.starts_with(&workdir_path) {
                return Err("cwd must be a directory inside workdir".to_string());
            }
            canonical
        }
        Some(value) => {
            let relative = relative_workspace_path(&value)?;
            let candidate = workdir_path.join(relative);
            let canonical = fs::canonicalize(&candidate)
                .map_err(|error| format!("cwd is not accessible: {error}"))?;
            if !canonical.starts_with(&workdir_path) || !canonical.is_dir() {
                return Err("cwd must be a directory inside workdir".to_string());
            }
            canonical
        }
    };
    let mut timeout = timeout_ms.unwrap_or(DEFAULT_SHELL_TIMEOUT_MS);
    if let Some(maximum) = max_timeout_ms {
        timeout = timeout.min(maximum.max(1));
    }
    timeout = timeout.clamp(1, MAX_SHELL_TIMEOUT_MS);

    let (program, args, shell, profile, family) = if cfg!(windows) {
        (
            "powershell.exe",
            vec![
                "-NoLogo".to_string(),
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-Command".to_string(),
                command,
            ],
            "powershell",
            "windows-powershell",
            "powershell",
        )
    } else {
        (
            "sh",
            vec!["-lc".to_string(), command],
            "sh",
            "posix-sh",
            "posix",
        )
    };

    let started = Instant::now();
    let mut process = Command::new(program);
    process
        .args(args)
        .current_dir(actual_cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // On Windows, PowerShell is a console subsystem binary. Without this flag a
    // brief black console window flashes for every tool/terminal shell run.
    configure_hidden_console_process(&mut process);
    // Publish a single, capability-bound cancellation record before starting
    // PowerShell/sh.  Agent cancellation can then find this run even if it
    // races process creation; the scoped guard also removes it on any error.
    let shell_registration =
        register_shell_run(state, run_id.as_deref(), capability_token.as_deref())?;
    let cancel_token = shell_registration
        .as_ref()
        .map(ShellRunRegistrationGuard::cancellation_token);
    if recheck_agent_capability {
        // The original authorization can race a whole-agent cancellation while
        // cwd/shell setup is in progress. Do not launch after that revocation.
        if let Err(error) = recheck_mutation_capability(
            state,
            capability_token.as_deref(),
            &workdir_path,
            ToolAction::Shell,
        ) {
            if !cancel_token
                .as_ref()
                .is_some_and(|token| token.load(Ordering::SeqCst))
            {
                return Err(error);
            }
        }
    }
    if cancel_token
        .as_ref()
        .is_some_and(|token| token.load(Ordering::SeqCst))
    {
        return Ok(ShellRunResponse {
            exit_code: -1,
            shell: shell.to_string(),
            platform: if cfg!(windows) { "windows" } else { "linux" }.to_string(),
            profile: profile.to_string(),
            shell_family: family.to_string(),
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            timed_out: false,
            cancelled: true,
            stdio_open_after_exit: false,
            effective_timeout_ms: timeout,
            duration_ms: started.elapsed().as_millis(),
        });
    }
    let mut child = process
        .spawn()
        .map_err(|error| format!("start {shell}: {error}"))?;
    let stdout_capture = child.stdout.take().map(bounded_output);
    let stderr_capture = child.stderr.take().map(bounded_output);

    let mut timed_out = false;
    let mut cancelled = false;
    let status = loop {
        if cancel_token
            .as_ref()
            .is_some_and(|token| token.load(Ordering::SeqCst))
        {
            cancelled = true;
            terminate_shell_process_tree(&mut child);
            break wait_for_terminated_shell(&mut child, "cancelled")?;
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                terminate_shell_process_tree(&mut child);
                let _ = wait_for_terminated_shell(&mut child, "failed");
                return Err(format!("wait shell: {error}"));
            }
        }
        if started.elapsed() >= Duration::from_millis(timeout) {
            timed_out = true;
            terminate_shell_process_tree(&mut child);
            break wait_for_terminated_shell(&mut child, "timed-out")?;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let stdio_deadline = Instant::now() + Duration::from_millis(SHELL_STDIO_DRAIN_TIMEOUT_MS);
    let (stdout, stdout_truncated, stdout_open) =
        collect_bounded_output(stdout_capture, stdio_deadline);
    let (stderr, stderr_truncated, stderr_open) =
        collect_bounded_output(stderr_capture, stdio_deadline);
    Ok(ShellRunResponse {
        exit_code: status.code().unwrap_or(-1),
        shell: shell.to_string(),
        platform: if cfg!(windows) { "windows" } else { "linux" }.to_string(),
        profile: profile.to_string(),
        shell_family: family.to_string(),
        stdout: String::from_utf8_lossy(&stdout).to_string(),
        stderr: String::from_utf8_lossy(&stderr).to_string(),
        stdout_truncated,
        stderr_truncated,
        timed_out,
        cancelled,
        stdio_open_after_exit: stdout_open || stderr_open,
        effective_timeout_ms: timeout,
        duration_ms: started.elapsed().as_millis(),
    })
}

#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)]
pub async fn shell_run(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    command: String,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
    max_timeout_ms: Option<u64>,
    _provider_id: Option<String>,
    run_id: Option<String>,
    capability_token: Option<String>,
    tool_call_id: Option<String>,
) -> Result<ShellRunResponse, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        run_shell_sync(
            &state,
            workdir,
            command,
            cwd,
            timeout_ms,
            max_timeout_ms,
            run_id,
            capability_token,
            tool_call_id,
        )
    })
    .await
    .map_err(|error| format!("shell_run join failed: {error}"))?
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellCancelResponse {
    pub cancelled: bool,
}

#[tauri::command(rename_all = "snake_case")]
pub fn shell_cancel(
    state: State<'_, Arc<AppState>>,
    run_id: String,
    capability_token: Option<String>,
) -> ShellCancelResponse {
    ShellCancelResponse {
        cancelled: cancel_shell_run(&state, &run_id, capability_token.as_deref()),
    }
}

// ---------------------------------------------------------------------------
// Git Review

/// The Dock deliberately exposes a small, deterministic Git surface instead
/// of a terminal. The renderer can review one approved workspace and create a
/// confirmed commit from files that the user has already staged; it never
/// receives an arbitrary command or a repository path outside that workspace.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusEntry {
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusResponse {
    pub is_repository: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub entries: Vec<GitStatusEntry>,
    pub staged_count: usize,
    pub unstaged_count: usize,
    pub untracked_count: usize,
    pub clean: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

impl GitStatusResponse {
    fn unavailable(reason: &str) -> Self {
        Self {
            is_repository: false,
            repository_root: None,
            branch: None,
            ahead: 0,
            behind: 0,
            entries: Vec::new(),
            staged_count: 0,
            unstaged_count: 0,
            untracked_count: 0,
            clean: true,
            unavailable_reason: Some(reason.to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_id: Option<String>,
    pub committed_files: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitCapabilityResponse {
    pub grant_token: String,
    pub workdir: String,
    pub staged_count: usize,
    pub expires_at_ms: i64,
}

#[derive(Debug)]
struct GitCommandOutput {
    exit_code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_truncated: bool,
    stderr_truncated: bool,
    timed_out: bool,
}

fn run_git_command(
    repository_root: &Path,
    arguments: &[String],
    timeout_ms: u64,
) -> Result<GitCommandOutput, String> {
    let mut command = Command::new("git");
    command
        .args(arguments)
        .current_dir(repository_root)
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        // Keep diagnostics parseable and never let a repository-provided
        // fsmonitor helper run merely because the user opened Git Review.
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_hidden_console_process(&mut command);

    let started = Instant::now();
    let mut child = command.spawn().map_err(|_| {
        "Git executable is unavailable. Install Git and restart NovaVei to enable Git Review."
            .to_string()
    })?;
    let stdout_capture = child.stdout.take().map(bounded_output);
    let stderr_capture = child.stderr.take().map(bounded_output);
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < Duration::from_millis(timeout_ms) => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                timed_out = true;
                terminate_shell_process_tree(&mut child);
                break wait_for_terminated_shell(&mut child, "timed-out Git")?;
            }
            Err(error) => {
                terminate_shell_process_tree(&mut child);
                let _ = wait_for_terminated_shell(&mut child, "failed Git");
                return Err(format!("wait for Git: {error}"));
            }
        }
    };
    let drain_deadline = Instant::now() + Duration::from_millis(SHELL_STDIO_DRAIN_TIMEOUT_MS);
    let (stdout, stdout_truncated, _) = collect_bounded_output(stdout_capture, drain_deadline);
    let (stderr, stderr_truncated, _) = collect_bounded_output(stderr_capture, drain_deadline);
    Ok(GitCommandOutput {
        exit_code: status.code().unwrap_or(-1),
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
        timed_out,
    })
}

fn git_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn git_command_failed(output: &GitCommandOutput) -> bool {
    output.timed_out || output.exit_code != 0
}

fn git_not_repository(output: &GitCommandOutput) -> bool {
    git_text(&output.stderr)
        .to_ascii_lowercase()
        .contains("not a git repository")
}

fn git_args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn parse_git_branch_header(header: &str) -> (Option<String>, usize, usize) {
    let header = header.trim();
    if let Some(branch) = header.strip_prefix("No commits yet on ") {
        return (Some(branch.trim().to_string()), 0, 0);
    }
    let (branch, tracking) = header.split_once("...").unwrap_or((header, ""));
    let branch = match branch.trim() {
        "" => None,
        "HEAD (no branch)" => Some("detached HEAD".to_string()),
        value => Some(value.to_string()),
    };
    let mut ahead = 0;
    let mut behind = 0;
    if let Some((_, progress)) = tracking.split_once('[') {
        let progress = progress.trim_end_matches(']');
        for item in progress.split(',').map(str::trim) {
            if let Some(value) = item.strip_prefix("ahead ") {
                ahead = value.parse().unwrap_or(0);
            } else if let Some(value) = item.strip_prefix("behind ") {
                behind = value.parse().unwrap_or(0);
            }
        }
    }
    (branch, ahead, behind)
}

fn parse_git_status(
    output: &[u8],
) -> Result<(Option<String>, usize, usize, Vec<GitStatusEntry>), String> {
    let mut branch = None;
    let mut ahead = 0;
    let mut behind = 0;
    let mut entries = Vec::new();
    let mut records = output.split(|byte| *byte == b'\0');
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        if record.starts_with(b"## ") {
            let (parsed_branch, parsed_ahead, parsed_behind) =
                parse_git_branch_header(&String::from_utf8_lossy(&record[3..]));
            branch = parsed_branch;
            ahead = parsed_ahead;
            behind = parsed_behind;
            continue;
        }
        if record.len() < 4 || record[2] != b' ' {
            return Err("Git returned an unsupported status record".to_string());
        }
        let index_status = record[0] as char;
        let worktree_status = record[1] as char;
        let path = git_text(&record[3..]);
        if path.trim().is_empty() {
            return Err("Git returned a status record without a file path".to_string());
        }
        if (index_status == 'R' || index_status == 'C') && records.next().is_none() {
            return Err("Git returned an incomplete rename status record".to_string());
        }
        entries.push(GitStatusEntry {
            path,
            index_status: index_status.to_string(),
            worktree_status: worktree_status.to_string(),
        });
        if entries.len() > MAX_GIT_STATUS_ENTRIES {
            return Err("Git status has too many changed paths to display safely".to_string());
        }
    }
    Ok((branch, ahead, behind, entries))
}

fn git_status_for_workspace(workdir: &Path) -> Result<GitStatusResponse, String> {
    let root = run_git_command(
        workdir,
        &git_args(&["-c", "core.fsmonitor=false", "rev-parse", "--show-toplevel"]),
        GIT_STATUS_TIMEOUT_MS,
    )?;
    if root.timed_out {
        return Err("Git repository discovery timed out".to_string());
    }
    if root.exit_code != 0 {
        if git_not_repository(&root) {
            return Ok(GitStatusResponse::unavailable("not_repository"));
        }
        return Err("Git could not inspect the current project".to_string());
    }
    if root.stdout_truncated || root.stderr_truncated {
        return Err("Git repository discovery returned too much output".to_string());
    }
    let root_text = String::from_utf8_lossy(&root.stdout).trim().to_string();
    let repository_root = fs::canonicalize(&root_text)
        .map_err(|_| "Git returned an inaccessible repository root".to_string())?;
    if repository_root != workdir {
        // Git may discover a parent checkout when a user opens only one of its
        // subfolders. Do not expose or commit sibling changes outside NovaVei's
        // exact approved project root.
        return Ok(GitStatusResponse::unavailable(
            "repository_outside_workspace",
        ));
    }
    let status = run_git_command(
        &repository_root,
        &git_args(&[
            "-c",
            "color.ui=false",
            "-c",
            "core.pager=cat",
            "-c",
            "core.fsmonitor=false",
            "status",
            "--porcelain=v1",
            "-z",
            "-b",
            "--untracked-files=all",
        ]),
        GIT_STATUS_TIMEOUT_MS,
    )?;
    if status.timed_out {
        return Err("Git status timed out".to_string());
    }
    if status.exit_code != 0 {
        return Err("Git could not read the current project status".to_string());
    }
    if status.stdout_truncated || status.stderr_truncated {
        return Err("Git status is too large to display safely".to_string());
    }
    let (branch, ahead, behind, entries) = parse_git_status(&status.stdout)?;
    let staged_count = entries
        .iter()
        .filter(|entry| entry.index_status != " " && entry.index_status != "?")
        .count();
    let unstaged_count = entries
        .iter()
        .filter(|entry| entry.worktree_status != " ")
        .count();
    let untracked_count = entries
        .iter()
        .filter(|entry| entry.index_status == "?" && entry.worktree_status == "?")
        .count();
    Ok(GitStatusResponse {
        is_repository: true,
        repository_root: Some(path_for_display(&repository_root)),
        branch,
        ahead,
        behind,
        clean: entries.is_empty(),
        entries,
        staged_count,
        unstaged_count,
        untracked_count,
        unavailable_reason: None,
    })
}

fn normalized_git_commit_message(message: &str) -> Result<String, String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("a Git commit message is required".to_string());
    }
    if message.chars().count() > MAX_GIT_COMMIT_MESSAGE_CHARS {
        return Err(format!(
            "Git commit messages must be at most {MAX_GIT_COMMIT_MESSAGE_CHARS} characters"
        ));
    }
    if message.contains('\0') {
        return Err("Git commit messages cannot contain NUL characters".to_string());
    }
    Ok(message.to_string())
}

fn git_commit_message_digest(message: &str) -> [u8; 32] {
    Sha256::digest(message.as_bytes()).into()
}

fn git_staged_snapshot_digest(workdir: &Path) -> Result<[u8; 32], String> {
    let output = run_git_command(
        workdir,
        &git_args(&[
            "diff",
            "--cached",
            "--raw",
            "-z",
            "--full-index",
            "--no-ext-diff",
            "--no-renames",
            "--",
        ]),
        GIT_STATUS_TIMEOUT_MS,
    )?;
    if git_command_failed(&output) {
        return Err("Git staged snapshot is unavailable".to_string());
    }
    if output.stdout_truncated || output.stderr_truncated {
        return Err("Git staged snapshot is too large to confirm safely".to_string());
    }
    let mut hasher = Sha256::new();
    hasher.update(b"git-staged-raw-v1\0");
    hasher.update(&output.stdout);
    Ok(hasher.finalize().into())
}

fn prune_git_commit_capabilities(grants: &mut HashMap<String, GitCommitCapabilityGrant>) {
    let now = Instant::now();
    grants.retain(|_, grant| grant.expires_at > now);
}

fn request_native_git_commit_confirmation(
    workdir: &Path,
    message: &str,
    staged_count: usize,
) -> Result<(), String> {
    let description = format!(
        "NovaVei will create a Git commit in:\n{}\n\nStaged files: {}\n\nMessage:\n{}\n\nProject Git hooks may run during this commit.",
        truncate_mcp_confirmation_text(&path_for_display(workdir), 240),
        staged_count,
        truncate_mcp_confirmation_text(message, 800)
    );
    #[cfg(all(windows, not(test)))]
    {
        let response = rfd::MessageDialog::new()
            .set_title("Confirm NovaVei Git commit")
            .set_description(description)
            .set_buttons(rfd::MessageButtons::OkCancel)
            .show();
        if response == rfd::MessageDialogResult::Ok {
            Ok(())
        } else {
            Err("native Git commit confirmation was denied".to_string())
        }
    }
    #[cfg(test)]
    {
        let _ = description;
        Ok(())
    }
    #[cfg(all(not(windows), not(test)))]
    {
        let _ = description;
        Err("native Git commit confirmation is unavailable on this platform".to_string())
    }
}

fn git_commit_status_or_error(workdir: &Path) -> Result<GitStatusResponse, String> {
    let status = git_status_for_workspace(workdir)?;
    if !status.is_repository {
        return Err("the current project is not an eligible Git repository".to_string());
    }
    if status.staged_count == 0 {
        return Err("there are no staged changes to commit".to_string());
    }
    Ok(status)
}

fn issue_git_commit_capability(
    state: &AppState,
    workdir: &str,
    message: &str,
    capability_token: Option<&str>,
) -> Result<GitCommitCapabilityResponse, String> {
    state.require_persistence_ready()?;
    let (workdir, workspace_grant) =
        require_workspace_view_capability(state, capability_token, workdir)?;
    let message = normalized_git_commit_message(message)?;
    let status = git_commit_status_or_error(&workdir)?;
    let staged_digest = git_staged_snapshot_digest(&workdir)?;
    request_native_git_commit_confirmation(&workdir, &message, status.staged_count)?;

    let current_status = git_commit_status_or_error(&workdir)?;
    if current_status.staged_count != status.staged_count
        || git_staged_snapshot_digest(&workdir)? != staged_digest
    {
        return Err("Git staged changes changed while confirming the commit".to_string());
    }

    let grant_token = format!("git-commit-{}", Uuid::new_v4());
    let expires_at = Instant::now() + GIT_COMMIT_GRANT_TTL;
    {
        let mut grants = state.git_commit_capabilities.lock();
        prune_git_commit_capabilities(&mut grants);
        if grants.len() >= MAX_PENDING_GIT_COMMIT_GRANTS {
            return Err(
                "too many pending Git commit confirmations; wait for one to expire".to_string(),
            );
        }
        grants.insert(
            grant_token.clone(),
            GitCommitCapabilityGrant {
                session_id: workspace_grant.session_id,
                workdir: workdir.clone(),
                message_digest: git_commit_message_digest(&message),
                staged_digest,
                staged_count: status.staged_count,
                expires_at,
            },
        );
    }

    Ok(GitCommitCapabilityResponse {
        grant_token,
        workdir: path_for_display(&workdir),
        staged_count: status.staged_count,
        expires_at_ms: now_ms() + GIT_COMMIT_GRANT_TTL.as_millis() as i64,
    })
}

fn consume_git_commit_capability(
    state: &AppState,
    commit_token: Option<&str>,
    workdir: &str,
    message: &str,
) -> Result<PathBuf, String> {
    let token = commit_token
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 128 && value.starts_with("git-commit-"))
        .ok_or_else(|| "Git commit requires a current native confirmation".to_string())?;
    let workdir = canonical_workdir(workdir)?;
    let message = normalized_git_commit_message(message)?;
    let grant = {
        let mut grants = state.git_commit_capabilities.lock();
        prune_git_commit_capabilities(&mut grants);
        grants
            .remove(token)
            .ok_or_else(|| "Git commit confirmation is invalid or expired".to_string())?
    };
    if grant.expires_at <= Instant::now()
        || grant.workdir != workdir
        || grant.message_digest != git_commit_message_digest(&message)
    {
        return Err("Git commit confirmation does not match this operation".to_string());
    }
    let session_matches = state
        .sessions
        .lock()
        .get(&grant.session_id)
        .and_then(|record| canonical_workdir(&record.summary.cwd).ok())
        .is_some_and(|stored| stored == workdir);
    if !session_matches {
        return Err("Git commit confirmation session is no longer available".to_string());
    }
    require_registered_project_workdir(state, &workdir)?;
    let status = git_commit_status_or_error(&workdir)?;
    if status.staged_count != grant.staged_count
        || git_staged_snapshot_digest(&workdir)? != grant.staged_digest
    {
        return Err("Git staged changes changed after commit confirmation".to_string());
    }
    Ok(workdir)
}

fn git_commit_failure(output: &GitCommandOutput) -> String {
    if output.timed_out {
        return "Git commit timed out; NovaVei did not assume whether a commit was created"
            .to_string();
    }
    let detail = git_text(&output.stderr).to_ascii_lowercase();
    if detail.contains("please tell me who you are")
        || detail.contains("unable to auto-detect email")
    {
        return "Git needs user.name and user.email before it can create a commit".to_string();
    }
    if detail.contains("nothing to commit") {
        return "There are no staged changes to commit".to_string();
    }
    if detail.contains("hook") {
        return "A project Git hook rejected or interrupted the commit".to_string();
    }
    "Git commit failed; review the staged changes and repository settings".to_string()
}

fn git_commit_for_workspace(workdir: &Path, message: &str) -> Result<GitCommitResponse, String> {
    let message = normalized_git_commit_message(message)?;
    let status = git_status_for_workspace(workdir)?;
    if !status.is_repository {
        return Err("the current project is not an eligible Git repository".to_string());
    }
    if status.staged_count == 0 {
        return Err("there are no staged changes to commit".to_string());
    }
    let output = run_git_command(
        workdir,
        &[
            "-c".to_string(),
            "color.ui=false".to_string(),
            "-c".to_string(),
            "core.pager=cat".to_string(),
            "commit".to_string(),
            "-m".to_string(),
            message,
        ],
        GIT_COMMIT_TIMEOUT_MS,
    )?;
    if git_command_failed(&output) {
        return Err(git_commit_failure(&output));
    }
    let revision = run_git_command(
        workdir,
        &git_args(&["rev-parse", "--short=12", "HEAD"]),
        GIT_STATUS_TIMEOUT_MS,
    )?;
    let commit_id = (!git_command_failed(&revision))
        .then(|| String::from_utf8_lossy(&revision.stdout).trim().to_string())
        .filter(|value| !value.is_empty());
    Ok(GitCommitResponse {
        commit_id,
        committed_files: status.staged_count,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn git_status(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    capability_token: Option<String>,
) -> Result<GitStatusResponse, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let workdir = require_read_capability(&state, capability_token.as_deref(), &workdir)?;
        git_status_for_workspace(&workdir)
    })
    .await
    .map_err(|error| format!("git_status join failed: {error}"))?
}

/// Issue a short-lived, one-use native grant for one exact Git commit. The
/// grant is bound to the visible workspace session, commit message, and staged
/// file snapshot; the read-only workspace capability is never accepted by the
/// commit command itself.
#[tauri::command(rename_all = "snake_case")]
pub async fn git_commit_capability_issue(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    message: String,
    capability_token: Option<String>,
) -> Result<GitCommitCapabilityResponse, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        issue_git_commit_capability(&state, &workdir, &message, capability_token.as_deref())
    })
    .await
    .map_err(|error| format!("git_commit_capability_issue join failed: {error}"))?
}

/// Commit only already-staged paths. The app UI obtains the message and shows
/// explicit confirmation before a native grant is minted; no shell command,
/// ref name, or repository path is renderer-controlled.
#[tauri::command(rename_all = "snake_case")]
pub async fn git_commit(
    state: State<'_, Arc<AppState>>,
    workdir: String,
    message: String,
    commit_token: Option<String>,
) -> Result<GitCommitResponse, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let workdir =
            consume_git_commit_capability(&state, commit_token.as_deref(), &workdir, &message)?;
        git_commit_for_workspace(&workdir, &message)
    })
    .await
    .map_err(|error| format!("git_commit join failed: {error}"))?
}

// ---------------------------------------------------------------------------
// History compatibility surface

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatHistorySummary {
    pub id: String,
    pub title: String,
    pub provider_id: String,
    pub model: String,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub selected_model_json: Option<String>,
    pub message_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub is_pinned: bool,
    pub pinned_at: Option<i64>,
    pub is_archived: bool,
    pub archived_at: Option<i64>,
    pub is_shared: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatHistoryListResponse {
    pub items: Vec<ChatHistorySummary>,
    pub total_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatHistoryBatchMutationResult {
    pub affected_ids: Vec<String>,
}

/// The renderer identifies a historical trace through the exact assistant
/// message it rendered, not merely a globally reusable turn id. Keeping the
/// message and session in this DTO prevents a stale DOM node from reading a
/// later turn or a different conversation's trace.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatHistoryTraceInput {
    pub session_id: String,
    pub message_id: String,
    pub turn_id: String,
}

/// A safe trace row for the WebView. Do not add tool arguments, results,
/// errors, event payloads, request ids, or tool ids here: historical tool
/// content remains native-only even if an event was already redacted at
/// ingestion time.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatHistoryTraceTool {
    pub name: String,
    pub status: String,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatHistoryTraceResponse {
    pub session_id: String,
    pub turn_id: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub tools: Vec<ChatHistoryTraceTool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatHistoryConversationInput {
    pub id: String,
    pub title: String,
    pub provider_id: String,
    pub model: String,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub selected_model_json: Option<String>,
    pub context_meta_json: String,
    pub active_segment_index: i64,
    pub total_segment_count: i64,
    pub total_message_count: i64,
    pub created_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatHistorySegmentInput {
    pub segment_index: i64,
    pub segment_id: String,
    pub summary_json: Option<String>,
    pub messages_json: String,
    pub message_count: i64,
    pub start_message_id: Option<String>,
    pub end_message_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatHistorySegmentMutationInput {
    pub conversation: ChatHistoryConversationInput,
    pub segment: ChatHistorySegmentInput,
}

fn nonempty_history_value(value: &str, field: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        Err(format!("{field} is required"))
    } else {
        Ok(value.to_string())
    }
}

fn validate_json_object(raw: &str, field: &str) -> Result<(), String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|error| format!("{field} is invalid JSON: {error}"))?;
    if !value.is_object() {
        return Err(format!("{field} must contain a JSON object"));
    }
    Ok(())
}

fn stored_segment_mutation(
    input: &ChatHistorySegmentMutationInput,
) -> Result<(StoredHistoryHeader, StoredHistorySegment), String> {
    let conversation_id = nonempty_history_value(&input.conversation.id, "conversation id")?;
    nonempty_history_value(&input.conversation.title, "conversation title")?;
    nonempty_history_value(&input.conversation.provider_id, "providerId")?;
    nonempty_history_value(&input.conversation.model, "model")?;
    validate_json_object(&input.conversation.context_meta_json, "contextMetaJson")?;
    if let Some(selected) = input.conversation.selected_model_json.as_deref() {
        validate_json_object(selected, "selectedModelJson")?;
    }
    if input.conversation.total_segment_count <= 0
        || input.conversation.active_segment_index != input.conversation.total_segment_count - 1
        || input.conversation.total_message_count < 0
    {
        return Err("conversation segment counts are inconsistent".to_string());
    }
    if input.segment.segment_index != input.conversation.active_segment_index
        || input.segment.segment_index < 0
        || input.segment.message_count < 0
    {
        return Err("active segment metadata is inconsistent".to_string());
    }
    let segment_id = nonempty_history_value(&input.segment.segment_id, "segmentId")?;
    let messages: Value = serde_json::from_str(&input.segment.messages_json)
        .map_err(|error| format!("messagesJson is invalid JSON: {error}"))?;
    let messages = messages
        .as_array()
        .ok_or_else(|| "messagesJson must contain a JSON array".to_string())?;
    if messages.len() as i64 != input.segment.message_count {
        return Err("messageCount does not match messagesJson.length".to_string());
    }
    if let Some(summary) = input.segment.summary_json.as_deref() {
        validate_json_object(summary, "summaryJson")?;
    }
    let created_at = input.conversation.created_at.unwrap_or_else(now_ms);
    let updated_at = if input.conversation.updated_at > 0 {
        input.conversation.updated_at
    } else {
        now_ms()
    };
    Ok((
        StoredHistoryHeader {
            conversation_id,
            session_id: input
                .conversation
                .session_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            context_meta_json: input.conversation.context_meta_json.clone(),
            active_segment_index: input.conversation.active_segment_index,
            total_segment_count: input.conversation.total_segment_count,
            total_message_count: input.conversation.total_message_count,
            created_at,
            updated_at,
        },
        StoredHistorySegment {
            segment_index: input.segment.segment_index,
            segment_id,
            summary_json: input.segment.summary_json.clone(),
            messages_json: input.segment.messages_json.clone(),
            message_count: input.segment.message_count,
            start_message_id: input.segment.start_message_id.clone(),
            end_message_id: input.segment.end_message_id.clone(),
            created_at: input.segment.created_at,
            updated_at: input.segment.updated_at,
        },
    ))
}

fn history_summary(record: &SessionRecord) -> ChatHistorySummary {
    ChatHistorySummary {
        id: record.summary.id.clone(),
        title: record.summary.title.clone(),
        provider_id: record.provider_id.clone(),
        model: record.model.clone(),
        session_id: Some(record.summary.id.clone()),
        cwd: Some(record.summary.cwd.clone()),
        selected_model_json: record.selected_model_json.clone(),
        message_count: record.message_count,
        created_at: record
            .messages
            .first()
            .map(|message| message.created_at)
            .unwrap_or(record.summary.updated_at),
        updated_at: record.summary.updated_at,
        is_pinned: record.pinned_at.is_some(),
        pinned_at: record.pinned_at,
        is_archived: record.archived_at.is_some(),
        archived_at: record.archived_at,
        is_shared: record.share_enabled,
    }
}

fn history_segment_value(segment: &StoredHistorySegment) -> Value {
    json!({
        "segmentIndex": segment.segment_index,
        "segmentId": segment.segment_id,
        "summaryJson": segment.summary_json,
        "messagesJson": segment.messages_json,
        "messageCount": segment.message_count,
        "startMessageId": segment.start_message_id,
        "endMessageId": segment.end_message_id,
        "createdAt": segment.created_at,
        "updatedAt": segment.updated_at,
    })
}

fn segmented_history_value(record: &SessionRecord, history: &StoredSegmentedHistory) -> Value {
    let summary = history_summary(record);
    let segments = history
        .segments
        .iter()
        .map(history_segment_value)
        .collect::<Vec<_>>();
    json!({
        "id": summary.id,
        "title": summary.title,
        "providerId": summary.provider_id,
        "model": summary.model,
        "sessionId": history.header.session_id,
        "cwd": summary.cwd,
        "selectedModelJson": summary.selected_model_json,
        "contextMetaJson": history.header.context_meta_json,
        "activeSegmentIndex": history.header.active_segment_index,
        "totalSegmentCount": history.header.total_segment_count,
        "totalMessageCount": history.header.total_message_count,
        "segments": segments,
        "createdAt": history.header.created_at,
        "updatedAt": history.header.updated_at,
        "isPinned": summary.is_pinned,
        "pinnedAt": summary.pinned_at,
        "isArchived": summary.is_archived,
        "archivedAt": summary.archived_at,
        "isShared": summary.is_shared,
        "redactToolContent": record.redact_tool_content,
    })
}

fn bounded_history_trace_identity(
    value: String,
    field: &str,
    max_bytes: usize,
) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{field} is required"));
    }
    if value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(format!("{field} is invalid"));
    }
    Ok(value.to_string())
}

fn history_trace_turn_status(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "starting" => "starting".to_string(),
        "running" => "running".to_string(),
        "waiting_permission" => "waiting_permission".to_string(),
        "completed" | "success" | "done" => "completed".to_string(),
        "cancelled" | "canceled" => "cancelled".to_string(),
        "error" | "failed" => "failed".to_string(),
        "interrupted" => "interrupted".to_string(),
        _ => "unknown".to_string(),
    }
}

fn history_trace_tool_name(value: &str) -> String {
    let value = value.trim();
    let allowed = value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':' | '/' | '@')
    });
    if value.is_empty() || value.chars().count() > MAX_HISTORY_TRACE_TOOL_NAME_CHARS || !allowed {
        "工具".to_string()
    } else {
        value.to_string()
    }
}

fn history_trace_tool_status(value: &str) -> String {
    let status = history_trace_turn_status(value);
    if status.len() > MAX_HISTORY_TRACE_TOOL_STATUS_CHARS {
        "unknown".to_string()
    } else {
        status
    }
}

fn history_trace_tool_value(tool: StoredTurnTraceTool) -> ChatHistoryTraceTool {
    ChatHistoryTraceTool {
        name: history_trace_tool_name(&tool.name),
        status: history_trace_tool_status(&tool.status),
        started_at: tool.started_at,
        finished_at: tool.finished_at,
    }
}

fn history_trace_value(trace: StoredTurnTrace) -> ChatHistoryTraceResponse {
    ChatHistoryTraceResponse {
        session_id: trace.session_id,
        turn_id: trace.turn_id,
        status: history_trace_turn_status(&trace.status),
        started_at: trace.started_at,
        finished_at: trace.finished_at,
        tools: trace
            .tools
            .into_iter()
            .map(history_trace_tool_value)
            .collect(),
    }
}

fn load_chat_history_trace(
    state: &AppState,
    input: ChatHistoryTraceInput,
) -> Result<ChatHistoryTraceResponse, String> {
    let session_id = bounded_history_trace_identity(
        input.session_id,
        "history trace session id",
        MAX_HISTORY_TRACE_SESSION_ID_BYTES,
    )?;
    let message_id = bounded_history_trace_identity(
        input.message_id,
        "history trace message id",
        MAX_HISTORY_TRACE_MESSAGE_ID_BYTES,
    )?;
    let turn_id = bounded_history_trace_identity(
        input.turn_id,
        "history trace turn id",
        MAX_HISTORY_TRACE_TURN_ID_BYTES,
    )?;
    let message_matches_turn = {
        let sessions = state.sessions.lock();
        let Some(record) = sessions.get(&session_id) else {
            return Err("history conversation not found".to_string());
        };
        let in_cache = record.messages.iter().any(|message| {
            message.id == message_id
                && message.role.eq_ignore_ascii_case("assistant")
                && message.turn_id.as_deref() == Some(turn_id.as_str())
        });
        if in_cache {
            true
        } else {
            // Partial caches omit older rows; fall back to durable membership.
            drop(sessions);
            state
                .history
                .message_matches_assistant_turn(&session_id, &message_id, &turn_id)?
        }
    };
    if !message_matches_turn {
        return Err("the historical reply has no verifiable saved trace".to_string());
    }

    state
        .history
        .load_turn_trace(&session_id, &turn_id)?
        .map(history_trace_value)
        .ok_or_else(|| "the historical reply has no saved trace".to_string())
}

/// Return metadata only for the exact assistant message currently displayed by
/// the renderer. Raw tool content and event payloads intentionally stay in
/// native storage.
#[tauri::command(rename_all = "camelCase")]
pub fn chat_history_trace_get(
    state: State<'_, Arc<AppState>>,
    input: ChatHistoryTraceInput,
) -> Result<ChatHistoryTraceResponse, String> {
    load_chat_history_trace(&state, input)
}

#[tauri::command(rename_all = "camelCase")]
pub fn chat_history_list(
    state: State<'_, Arc<AppState>>,
    page: Option<i64>,
    page_size: Option<i64>,
    cwd: Option<String>,
    cwd_empty: Option<bool>,
) -> ChatHistoryListResponse {
    let page = page.unwrap_or(1).max(1);
    let page_size = page_size.unwrap_or(50).clamp(1, 500);
    let requested_cwd = cwd.as_deref().map(str::trim);
    let requested_cwd_key = requested_cwd
        .filter(|value| !value.is_empty())
        .and_then(workspace_path_key);
    let mut items = state
        .sessions
        .lock()
        .values()
        .filter(|record| {
            if cwd_empty.unwrap_or(false) {
                record.summary.cwd.trim().is_empty()
            } else {
                match requested_cwd {
                    None | Some("") => true,
                    Some(_) => requested_cwd_key.as_deref().is_some_and(|requested| {
                        workspace_path_key(&record.summary.cwd).as_deref() == Some(requested)
                    }),
                }
            }
        })
        .map(history_summary)
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        left.is_archived
            .cmp(&right.is_archived)
            .then_with(|| right.is_pinned.cmp(&left.is_pinned))
            .then_with(|| right.pinned_at.cmp(&left.pinned_at))
            .then_with(|| right.updated_at.cmp(&left.updated_at))
    });
    let total_count = items.len() as i64;
    let start = ((page - 1) * page_size) as usize;
    let end = (start + page_size as usize).min(items.len());
    let items = if start < items.len() {
        items[start..end].to_vec()
    } else {
        Vec::new()
    };
    ChatHistoryListResponse { items, total_count }
}

#[tauri::command]
pub fn chat_history_workdirs(state: State<'_, Arc<AppState>>) -> Value {
    // Group by the same durable path identity used by projects, relocation,
    // and metadata search. Keep a display value separately so a slash/case
    // variant from old history cannot create a second workspace group.
    let mut counts: HashMap<String, (String, i64, i64)> = HashMap::new();
    for record in state.sessions.lock().values() {
        let display = historical_workspace_path_display(&record.summary.cwd)
            .unwrap_or_else(|_| path_string_for_display(&record.summary.cwd));
        let key = workspace_path_key(&record.summary.cwd)
            .unwrap_or_else(|| format!("invalid-workspace:{display}"));
        let entry = counts
            .entry(key)
            .or_insert((display, 0, record.summary.updated_at));
        entry.1 += 1;
        entry.2 = entry.2.max(record.summary.updated_at);
    }
    let projects = project_metadata_from_value(state.settings.lock().get("projects"));
    let projects_by_path = projects
        .iter()
        .filter_map(|project| workspace_path_key(&project.path).map(|key| (key, project)))
        .collect::<HashMap<_, _>>();
    let mut workdirs = counts
        .into_iter()
        .map(|(_, (path, conversation_count, updated_at))| {
            let status = workspace_path_status(&path);
            let project =
                workspace_path_key(&path).and_then(|key| projects_by_path.get(&key).copied());
            json!({
                "path": path,
                "conversationCount": conversation_count,
                "updatedAt": updated_at,
                "accessible": status.accessible,
                "reason": status.reason,
                "registered": project.is_some(),
                "projectId": project.map(|project| project.id.clone()),
                "projectName": project.map(|project| project.name.clone()),
                "projectPinned": project.map(|project| project.pinned),
            })
        })
        .collect::<Vec<_>>();
    workdirs.sort_by(|left, right| {
        right["updatedAt"]
            .as_i64()
            .cmp(&left["updatedAt"].as_i64())
            .then_with(|| left["path"].as_str().cmp(&right["path"].as_str()))
    });
    json!({"workdirs": workdirs})
}

#[tauri::command]
pub fn chat_history_get(state: State<'_, Arc<AppState>>, id: String) -> Result<Value, String> {
    let id = id.trim().to_string();
    let mut record = state
        .sessions
        .lock()
        .get(&id)
        .cloned()
        .ok_or_else(|| "history conversation not found".to_string())?;
    if let Some(history) = state.history.load_segmented_history(&id)? {
        return Ok(segmented_history_value(&record, &history));
    }
    // Export/copy needs the full transcript once; do not force it into AppState
    // when the working cache is only a partial page.
    if !record.messages_complete {
        let durable = state.history.load_messages(&id)?;
        record.messages = durable
            .into_iter()
            .map(message_record_from_stored)
            .collect();
        record.message_count = record.messages.len() as i64;
        record.messages_complete = true;
        record.messages_loaded = true;
    }
    let summary = history_summary(&record);
    Ok(json!({
        "id": summary.id,
        "title": summary.title,
        "providerId": summary.provider_id,
        "model": summary.model,
        "sessionId": summary.session_id,
        "cwd": summary.cwd,
        "selectedModelJson": summary.selected_model_json,
        "contextMetaJson": "{}",
        "activeSegmentIndex": 0,
        "totalSegmentCount": 1,
        "totalMessageCount": summary.message_count,
        "segments": [{
            "segmentIndex": 0,
            "segmentId": format!("{}-segment-0", summary.id),
            "summaryJson": null,
            "messagesJson": serde_json::to_string(&record.messages).unwrap_or_else(|_| "[]".to_string()),
            "messageCount": summary.message_count,
            "startMessageId": record.messages.first().map(|message| message.id.clone()),
            "endMessageId": record.messages.last().map(|message| message.id.clone()),
            "createdAt": summary.created_at,
            "updatedAt": summary.updated_at
        }],
        "createdAt": summary.created_at,
        "updatedAt": summary.updated_at,
        "isPinned": summary.is_pinned,
        "pinnedAt": summary.pinned_at,
        "isArchived": summary.is_archived,
        "archivedAt": summary.archived_at,
        "isShared": summary.is_shared,
        "redactToolContent": record.redact_tool_content
    }))
}

#[tauri::command]
pub fn chat_history_get_active_segment(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<Value, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("history conversation id is required".to_string());
    }
    let record = state
        .sessions
        .lock()
        .get(id)
        .cloned()
        .ok_or_else(|| "history conversation not found".to_string())?;
    let history = state
        .history
        .load_segmented_history(id)?
        .ok_or_else(|| "history conversation does not have segmented data".to_string())?;
    let active = history
        .segments
        .iter()
        .find(|segment| segment.segment_index == history.header.active_segment_index)
        .ok_or_else(|| "active history segment is missing".to_string())?;
    let summary = history_summary(&record);
    Ok(json!({
        "id": summary.id,
        "title": summary.title,
        "providerId": summary.provider_id,
        "model": summary.model,
        "sessionId": history.header.session_id,
        "cwd": summary.cwd,
        "selectedModelJson": summary.selected_model_json,
        "contextMetaJson": history.header.context_meta_json,
        "activeSegmentIndex": history.header.active_segment_index,
        "totalSegmentCount": history.header.total_segment_count,
        "totalMessageCount": history.header.total_message_count,
        "activeSegment": history_segment_value(active),
        "createdAt": history.header.created_at,
        "updatedAt": history.header.updated_at,
        "isPinned": summary.is_pinned,
        "pinnedAt": summary.pinned_at,
        "isShared": summary.is_shared,
    }))
}

fn history_search_normalize(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    // This helper receives renderer-controlled query text as well as durable
    // history content. Bound the scan itself, not only the resulting string:
    // collecting/splitting an arbitrarily large query before truncating it
    // would make the apparent 256-character IPC limit ineffective.
    let scan_limit = max_chars.saturating_mul(8).max(max_chars);
    let mut normalized = String::with_capacity(max_chars.min(value.len()));
    let mut output_chars = 0usize;
    let mut pending_space = false;

    for character in value.chars().take(scan_limit) {
        if character.is_whitespace() {
            pending_space |= output_chars > 0;
            continue;
        }
        if pending_space {
            if output_chars >= max_chars {
                break;
            }
            normalized.push(' ');
            output_chars += 1;
            pending_space = false;
        }
        if output_chars >= max_chars {
            break;
        }
        normalized.push(character);
        output_chars += 1;
    }
    normalized
}

fn history_search_preview(value: &str, query: &str) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let characters = normalized.chars().collect::<Vec<_>>();
    if characters.len() <= MAX_HISTORY_SEARCH_PREVIEW_CHARS {
        return normalized;
    }

    let lowered = normalized.to_lowercase();
    let match_index = lowered
        .find(query)
        .map(|byte_index| lowered[..byte_index].chars().count())
        .unwrap_or(0);
    let window = MAX_HISTORY_SEARCH_PREVIEW_CHARS;
    let start = match_index
        .saturating_sub(window / 3)
        .min(characters.len().saturating_sub(window));
    let end = (start + window).min(characters.len());
    let mut preview = characters[start..end].iter().collect::<String>();
    if start > 0 {
        preview.insert(0, '…');
    }
    if end < characters.len() {
        preview.push('…');
    }
    preview
}

fn history_search_grams(value: &str) -> Vec<String> {
    let characters = value.chars().collect::<Vec<_>>();
    if characters.len() < HISTORY_SEARCH_INDEX_GRAM_CHARS {
        return Vec::new();
    }
    let mut unique = HashSet::new();
    for window in characters.windows(HISTORY_SEARCH_INDEX_GRAM_CHARS) {
        unique.insert(window.iter().collect::<String>());
    }
    unique.into_iter().collect()
}

/// This deliberately hashes only cache-shape metadata, never writes it, and
/// never sends it over IPC. Session updates already advance `updated_at`; the
/// loaded-count/complete bits also catch lazy transcript page changes.
fn history_search_source_stamp(sessions: &HashMap<String, SessionRecord>) -> [u8; 32] {
    let mut records = sessions.values().collect::<Vec<_>>();
    records.sort_by(|left, right| left.summary.id.cmp(&right.summary.id));
    let mut hasher = Sha256::new();
    for record in records {
        hasher.update(record.summary.id.as_bytes());
        hasher.update(record.summary.title.as_bytes());
        hasher.update(record.summary.updated_at.to_le_bytes());
        hasher.update(record.message_count.to_le_bytes());
        hasher.update((record.messages.len() as u64).to_le_bytes());
        hasher.update([
            u8::from(record.messages_loaded),
            u8::from(record.messages_complete),
        ]);
        for message in &record.messages {
            hasher.update(message.id.as_bytes());
            hasher.update(message.role.as_bytes());
            hasher.update(message.created_at.to_le_bytes());
            hasher.update(message.content.len().to_le_bytes());
            hasher.update(message.content.as_bytes());
        }
    }
    hasher.finalize().into()
}

fn history_search_matches(
    index: &mut InMemoryHistorySearchIndex,
    sessions: &HashMap<String, SessionRecord>,
    raw_query: &str,
    requested_limit: Option<u64>,
    requested_offset: Option<u64>,
) -> Vec<Value> {
    let query = history_search_normalize(raw_query, MAX_HISTORY_SEARCH_QUERY_CHARS).to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let limit = requested_limit
        .unwrap_or(12)
        .clamp(1, MAX_HISTORY_SEARCH_RESULTS as u64) as usize;
    let offset = requested_offset.unwrap_or(0).min(10_000) as usize;
    index.search(sessions, &query, offset, limit)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetadataSearchMatch {
    /// `project` rows make an empty project discoverable by Ctrl/Cmd+K;
    /// `session` rows retain a directly openable conversation id.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    /// A bounded native projection for the result's durable workspace path.
    /// It is metadata only, never a filesystem capability or raw OS error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_status: Option<WorkspacePathStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// A historical cwd is searchable even when it is not a durable project.
    /// The renderer uses this bit together with path status to label the result
    /// read-only before it is opened.
    pub registered: bool,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetadataSearchResponse {
    pub matches: Vec<SessionMetadataSearchMatch>,
}

#[derive(Debug, Clone)]
struct SessionMetadataSearchRecord {
    summary: SessionSummary,
    model: String,
    provider_id: String,
}

fn session_metadata_search_records(
    sessions: &HashMap<String, SessionRecord>,
) -> Vec<SessionMetadataSearchRecord> {
    sessions
        .values()
        .map(|record| SessionMetadataSearchRecord {
            summary: session_summary_value(record),
            model: record.model.clone(),
            provider_id: record.provider_id.clone(),
        })
        .collect()
}

/// Normalize searchable metadata while treating Windows and POSIX separator
/// spellings as equivalent. This applies to every metadata field so a path
/// query can use either `foo/bar` or `foo\\bar` without changing title/model
/// matching semantics.
fn metadata_search_normalize(value: &str, max_chars: usize) -> String {
    let normalized = history_search_normalize(value, max_chars);
    let mut separator_pending = false;
    let mut output = String::with_capacity(normalized.len());
    for character in normalized.chars() {
        if matches!(character, '/' | '\\') {
            if !separator_pending {
                output.push('/');
                separator_pending = true;
            }
        } else {
            output.push(character);
            separator_pending = false;
        }
    }
    output.to_lowercase()
}

fn metadata_search_score(value: &str, query: &str, weight: i64, max_chars: usize) -> i64 {
    let value = metadata_search_normalize(value, max_chars);
    if value.is_empty() || !value.contains(query) {
        return 0;
    }
    if value == query {
        weight + 40
    } else if value.starts_with(query) {
        weight + 20
    } else {
        weight
    }
}

/// Search only durable session/project metadata. It deliberately does not
/// read transcript text, tool payloads, or provider secrets, so it can power
/// the command palette without widening the existing full-history search
/// boundary.
fn session_metadata_search_matches_from_records(
    sessions: &[SessionMetadataSearchRecord],
    projects: &[ProjectMetadata],
    raw_query: &str,
    requested_limit: Option<u64>,
) -> Vec<SessionMetadataSearchMatch> {
    let query = metadata_search_normalize(raw_query, MAX_HISTORY_SEARCH_QUERY_CHARS);
    if query.is_empty() || query.chars().any(char::is_control) {
        return Vec::new();
    }
    let limit = requested_limit
        .unwrap_or(20)
        .clamp(1, MAX_SESSION_METADATA_SEARCH_RESULTS as u64) as usize;
    let projects_by_path = projects
        .iter()
        .filter_map(|project| workspace_path_key(&project.path).map(|key| (key, project)))
        .collect::<HashMap<_, _>>();
    let mut project_updated_at = HashMap::<String, i64>::new();
    let mut matches = Vec::<(i64, i64, String, SessionMetadataSearchMatch)>::new();

    for record in sessions {
        let summary = &record.summary;
        let workdir = historical_workspace_path_display(&summary.cwd).ok();
        let project = workdir
            .as_deref()
            .and_then(workspace_path_key)
            .and_then(|key| projects_by_path.get(&key).copied());
        if let Some(project) = project {
            project_updated_at
                .entry(project.id.clone())
                .and_modify(|updated_at| *updated_at = (*updated_at).max(summary.updated_at))
                .or_insert(summary.updated_at);
        }

        let mut score =
            metadata_search_score(&summary.title, &query, 500, MAX_HISTORY_SEARCH_TITLE_CHARS);
        if let Some(workdir) = workdir.as_deref() {
            score = score.max(metadata_search_score(
                workdir,
                &query,
                350,
                MAX_WORKSPACE_PATH_CHARS,
            ));
        }
        if let Some(project) = project {
            score = score.max(metadata_search_score(&project.name, &query, 425, 200));
        }
        if !record.model.trim().is_empty() {
            score = score.max(metadata_search_score(
                &record.model,
                &query,
                300,
                MAX_PROVIDER_MODEL_ID_BYTES,
            ));
        }
        if !record.provider_id.trim().is_empty() {
            score = score.max(metadata_search_score(
                &record.provider_id,
                &query,
                200,
                MAX_PROVIDER_ID_BYTES,
            ));
        }
        if score == 0 {
            continue;
        }
        let conversation_id = summary.id.trim();
        if conversation_id.is_empty()
            || conversation_id.len() > MAX_HISTORY_TRACE_SESSION_ID_BYTES
            || conversation_id.chars().any(char::is_control)
        {
            continue;
        }
        matches.push((
            score,
            summary.updated_at,
            format!("session:{conversation_id}"),
            SessionMetadataSearchMatch {
                kind: "session".to_string(),
                conversation_id: Some(conversation_id.to_string()),
                conversation_title: Some(history_search_normalize(
                    &summary.title,
                    MAX_HISTORY_SEARCH_TITLE_CHARS,
                )),
                project_id: project.map(|project| project.id.clone()),
                project_name: project.map(|project| project.name.clone()),
                workspace_status: None,
                workdir,
                model: (!record.model.trim().is_empty()).then(|| record.model.clone()),
                registered: project.is_some(),
                updated_at: summary.updated_at,
            },
        ));
    }

    for project in projects {
        let score = metadata_search_score(&project.name, &query, 450, 200).max(
            metadata_search_score(&project.path, &query, 400, MAX_WORKSPACE_PATH_CHARS),
        );
        if score == 0 {
            continue;
        }
        matches.push((
            score,
            project_updated_at.get(&project.id).copied().unwrap_or(0),
            format!("project:{}", project.id),
            SessionMetadataSearchMatch {
                kind: "project".to_string(),
                conversation_id: None,
                conversation_title: None,
                project_id: Some(project.id.clone()),
                project_name: Some(project.name.clone()),
                workspace_status: None,
                workdir: Some(project.path.clone()),
                model: None,
                registered: true,
                updated_at: project_updated_at.get(&project.id).copied().unwrap_or(0),
            },
        ));
    }

    matches.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    let mut selected = matches
        .into_iter()
        .take(limit)
        .map(|(_, _, _, value)| value)
        .collect::<Vec<_>>();
    // Filesystem metadata can block noticeably for disconnected drives and
    // network roots. Rank first, then probe only the bounded results that will
    // actually cross IPC instead of every matching historical cwd.
    let mut workspace_statuses = HashMap::<String, WorkspacePathStatus>::new();
    for result in &mut selected {
        let Some(workdir) = result.workdir.as_deref() else {
            continue;
        };
        let key = workspace_path_key(workdir).unwrap_or_else(|| workdir.to_string());
        let status = workspace_statuses
            .entry(key)
            .or_insert_with(|| workspace_path_status(workdir))
            .clone();
        result.workspace_status = Some(status);
    }
    selected
}

#[cfg(test)]
fn session_metadata_search_matches(
    sessions: &HashMap<String, SessionRecord>,
    projects: &[ProjectMetadata],
    raw_query: &str,
    requested_limit: Option<u64>,
) -> Vec<SessionMetadataSearchMatch> {
    let records = session_metadata_search_records(sessions);
    session_metadata_search_matches_from_records(&records, projects, raw_query, requested_limit)
}

/// `args` intentionally mirrors the legacy `chat_history_search` input
/// shape: invoke with `{ args: { query, limit } }`.
#[tauri::command]
pub fn session_metadata_search(
    state: State<'_, Arc<AppState>>,
    args: Value,
) -> SessionMetadataSearchResponse {
    let raw_query = args.get("query").and_then(Value::as_str).unwrap_or("");
    let requested_limit = args.get("limit").and_then(Value::as_u64);
    let projects = project_metadata_from_value(state.settings.lock().get("projects"));
    // Copy only searchable metadata, then release the session lock before any
    // potentially slow filesystem probe for disconnected or remote roots.
    let sessions = session_metadata_search_records(&state.sessions.lock());
    SessionMetadataSearchResponse {
        matches: session_metadata_search_matches_from_records(
            &sessions,
            &projects,
            raw_query,
            requested_limit,
        ),
    }
}

#[tauri::command]
pub fn chat_history_search(state: State<'_, Arc<AppState>>, args: Value) -> Value {
    let raw_query = args.get("query").and_then(Value::as_str).unwrap_or("");
    let requested_limit = args.get("limit").and_then(Value::as_u64);
    let requested_offset = args.get("offset").and_then(Value::as_u64);
    let sessions = state.sessions.lock();
    let matches = history_search_matches(
        &mut state.history_search_index.lock(),
        &sessions,
        raw_query,
        requested_limit,
        requested_offset,
    );
    json!({"matches": matches})
}

fn update_history_payload(
    state: &AppState,
    payload: &Value,
) -> Result<(SessionSummary, Option<SessionRecord>), String> {
    state.require_persistence_ready()?;
    let object = payload
        .as_object()
        .ok_or_else(|| "history payload must be an object".to_string())?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let title = object
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("新建对话")
        .to_string();
    let requested_cwd = object
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            state
                .sessions
                .lock()
                .get(&id)
                .map(|record| record.summary.cwd.clone())
        })
        .unwrap_or_else(current_workdir);
    let canonical_cwd = canonical_workdir(&requested_cwd)?;
    require_approved_workdir(state, &canonical_cwd)?;
    let cwd = path_for_display(&canonical_cwd);
    let updated_at = object
        .get("updatedAt")
        .or_else(|| object.get("updated_at"))
        .and_then(Value::as_i64)
        .unwrap_or_else(now_ms);
    let provider_id = object
        .get("providerId")
        .or_else(|| object.get("provider_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let model = object
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let selected_model_json = object
        .get("selectedModelJson")
        .or_else(|| object.get("selected_model_json"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut sessions = state.sessions.lock();
    if let Some(existing) = sessions.get(&id) {
        let existing_cwd = canonical_workdir(&existing.summary.cwd)?;
        if existing_cwd != canonical_cwd {
            return Err("history session workspace cannot be changed".to_string());
        }
    }
    let previous = sessions.get(&id).cloned();
    let record = sessions.entry(id.clone()).or_insert_with(|| SessionRecord {
        summary: SessionSummary {
            id: id.clone(),
            title: title.clone(),
            cwd: cwd.clone(),
            updated_at,
            message_count: 0,
            is_pinned: false,
            is_archived: false,
            last_run_status: None,
            last_run_finished_at: None,
        },
        messages: Vec::new(),
        message_count: 0,
        messages_loaded: true,
        messages_complete: true,
        provider_id: default_provider_id(),
        model: String::new(),
        selected_model_json: None,
        pinned_at: None,
        archived_at: None,
        share_enabled: false,
        share_token: None,
        share_created_at: None,
        share_updated_at: None,
        redact_tool_content: true,
        goal: None,
    });
    record.summary.title = title;
    record.summary.cwd = cwd;
    record.summary.updated_at = updated_at;
    if let Some(provider_id) = provider_id {
        record.provider_id = provider_id;
    }
    if let Some(model) = model {
        record.model = model;
    }
    if selected_model_json.is_some() {
        record.selected_model_json = selected_model_json;
    }
    Ok((session_summary_value(record), previous))
}

#[tauri::command]
pub fn chat_history_upsert(
    state: State<'_, Arc<AppState>>,
    input: Value,
) -> Result<ChatHistorySummary, String> {
    let _persist_guard = state.persist_lock.lock();
    let (summary, previous) = update_history_payload(&state, &input)?;
    if let Err(error) = state.persist_session_locked(&summary.id) {
        restore_session_snapshot(&state, &summary.id, previous.as_ref());
        return Err(error);
    }
    let record = state
        .sessions
        .lock()
        .get(&summary.id)
        .cloned()
        .ok_or_else(|| "history upsert failed".to_string())?;
    Ok(history_summary(&record))
}

fn mutate_segmented_history(
    state: &AppState,
    input: ChatHistorySegmentMutationInput,
    append: bool,
) -> Result<ChatHistorySummary, String> {
    state.require_persistence_ready()?;
    let (header, segment) = stored_segment_mutation(&input)?;
    let id = header.conversation_id.clone();
    let _persist_guard = state.persist_lock.lock();
    let previous = state.sessions.lock().get(&id).cloned();
    let canonical_cwd = match previous.as_ref() {
        Some(record) => {
            let stored = canonical_workdir(&record.summary.cwd)?;
            if let Some(requested) = input
                .conversation
                .cwd
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if canonical_workdir(requested)? != stored {
                    return Err("history session workspace cannot be changed".to_string());
                }
            }
            stored
        }
        None => {
            let requested = input
                .conversation
                .cwd
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .unwrap_or_else(current_workdir);
            canonical_workdir(&requested)?
        }
    };
    require_approved_workdir(state, &canonical_cwd)?;
    let cwd = path_for_display(&canonical_cwd);
    let mut record = previous.clone().unwrap_or_else(|| SessionRecord {
        summary: SessionSummary {
            id: id.clone(),
            title: input.conversation.title.trim().to_string(),
            cwd: cwd.clone(),
            updated_at: header.updated_at,
            message_count: 0,
            is_pinned: false,
            is_archived: false,
            last_run_status: None,
            last_run_finished_at: None,
        },
        messages: Vec::new(),
        message_count: 0,
        messages_loaded: true,
        messages_complete: true,
        provider_id: default_provider_id(),
        model: String::new(),
        selected_model_json: None,
        pinned_at: None,
        archived_at: None,
        share_enabled: false,
        share_token: None,
        share_created_at: None,
        share_updated_at: None,
        redact_tool_content: true,
        goal: None,
    });
    record.summary.title = input.conversation.title.trim().to_string();
    record.summary.cwd = cwd;
    record.summary.updated_at = header.updated_at;
    record.provider_id = input.conversation.provider_id.trim().to_string();
    record.model = input.conversation.model.trim().to_string();
    if let Some(selected) = input.conversation.selected_model_json.as_ref() {
        record.selected_model_json = Some(selected.clone());
    }
    state.sessions.lock().insert(id.clone(), record);
    // Prefer session-scoped projection so other conversations are not rewritten.
    let session_snapshot = match state.sessions.lock().get(&id).cloned() {
        Some(record) => record,
        None => {
            restore_session_snapshot(state, &id, previous.as_ref());
            return Err("history segment mutation lost its session".to_string());
        }
    };
    let stored_session = StoredSession {
        id: session_snapshot.summary.id.clone(),
        title: session_snapshot.summary.title.clone(),
        cwd: session_snapshot.summary.cwd.clone(),
        updated_at: session_snapshot.summary.updated_at,
        provider_id: session_snapshot.provider_id.clone(),
        model: session_snapshot.model.clone(),
        selected_model_json: session_snapshot.selected_model_json.clone(),
        pinned_at: session_snapshot.pinned_at,
        archived_at: session_snapshot.archived_at,
        share_enabled: session_snapshot.share_enabled,
        share_token: session_snapshot.share_token.clone(),
        share_created_at: session_snapshot.share_created_at,
        share_updated_at: session_snapshot.share_updated_at,
        redact_tool_content: session_snapshot.redact_tool_content,
    };
    let stored_messages = session_snapshot
        .messages
        .iter()
        .map(|message| StoredMessage {
            id: message.id.clone(),
            session_id: session_snapshot.summary.id.clone(),
            role: message.role.clone(),
            content: message.content.clone(),
            created_at: message.created_at,
            turn_id: message.turn_id.clone(),
        })
        .collect::<Vec<_>>();
    if let Err(error) = state.history.upsert_session_and_mutate_segment(
        &stored_session,
        &stored_messages,
        &header,
        &segment,
        append,
    ) {
        restore_session_snapshot(state, &id, previous.as_ref());
        return Err(error);
    }
    let record = state
        .sessions
        .lock()
        .get(&id)
        .cloned()
        .ok_or_else(|| "history segment mutation lost its session".to_string())?;
    let stored = state
        .history
        .load_segmented_history(&id)?
        .ok_or_else(|| "history segment mutation was not persisted".to_string())?;
    let mut summary = history_summary(&record);
    summary.message_count = stored.header.total_message_count;
    summary.created_at = stored.header.created_at;
    summary.updated_at = stored.header.updated_at;
    Ok(summary)
}

#[tauri::command]
pub fn chat_history_upsert_active_segment(
    state: State<'_, Arc<AppState>>,
    input: ChatHistorySegmentMutationInput,
) -> Result<ChatHistorySummary, String> {
    mutate_segmented_history(&state, input, false)
}

#[tauri::command]
pub fn chat_history_append_segment(
    state: State<'_, Arc<AppState>>,
    input: ChatHistorySegmentMutationInput,
) -> Result<ChatHistorySummary, String> {
    mutate_segmented_history(&state, input, true)
}

#[tauri::command(rename_all = "camelCase")]
pub fn chat_history_rename(
    state: State<'_, Arc<AppState>>,
    id: String,
    title: String,
) -> Result<ChatHistorySummary, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("history conversation title is required".to_string());
    }
    mutate_session_and_persist(&state, id.trim(), |record| {
        record.summary.title = title.to_string();
        record.summary.updated_at = now_ms();
        Ok(history_summary(record))
    })
}

fn normalize_history_batch_ids(ids: Vec<String>) -> Result<Vec<String>, String> {
    let mut normalized_ids = Vec::with_capacity(ids.len());
    let mut seen_ids = HashSet::with_capacity(ids.len());
    for raw_id in ids {
        let id = raw_id.trim();
        let is_valid_id = !id.is_empty()
            && id.len() <= MAX_SESSION_GOAL_SESSION_ID_BYTES
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
        if !is_valid_id {
            return Err("history conversation id is invalid".to_string());
        }
        if seen_ids.insert(id.to_string()) {
            normalized_ids.push(id.to_string());
        }
    }
    if normalized_ids.is_empty() {
        return Err("select at least one history conversation".to_string());
    }
    Ok(normalized_ids)
}

fn snapshot_project_history_records(
    state: &AppState,
    ids: &[String],
    requested_workdir: &Path,
) -> Result<Vec<(String, SessionRecord)>, String> {
    let sessions = state.sessions.lock();
    let mut snapshots = Vec::with_capacity(ids.len());
    for id in ids {
        let record = sessions
            .get(id)
            .cloned()
            .ok_or_else(|| "history conversation not found".to_string())?;
        let record_workdir = canonical_workdir(&record.summary.cwd)?;
        if record_workdir != requested_workdir {
            return Err("history conversation is outside the requested project".to_string());
        }
        snapshots.push((id.clone(), record));
    }
    Ok(snapshots)
}

fn restore_history_records(state: &AppState, snapshots: &[(String, SessionRecord)]) {
    let mut sessions = state.sessions.lock();
    for (id, record) in snapshots {
        sessions.insert(id.clone(), record.clone());
    }
}

fn revoke_session_owned_access(state: &AppState, session_ids: &HashSet<String>) {
    // Active runs are rejected before deletion. Remove any stale agent grants
    // defensively so deleted sessions cannot retain an otherwise unusable token.
    let stale_agent_tokens = state
        .capabilities
        .lock()
        .iter()
        .filter_map(|(token, grant)| {
            session_ids
                .contains(&grant.session_id)
                .then_some(token.clone())
        })
        .collect::<HashSet<_>>();
    for token in &stale_agent_tokens {
        clear_capability_state(state, token);
    }

    state
        .workspace_capabilities
        .lock()
        .retain(|_, grant| !session_ids.contains(&grant.session_id));
    // A native Full confirmation is minted before its agent run exists. If a
    // project is removed or its sessions are rebound in that window, revoke
    // the pending proof as well; re-registering the same path must not revive
    // an approval issued for the previous project binding.
    state
        .full_permission_grants
        .lock()
        .retain(|_, grant| !session_ids.contains(&grant.session_id));

    // Cancel agent-owned shell runs whose capability tokens were just revoked.
    let owned_shells = state
        .shell_runs
        .lock()
        .values()
        .filter(|shell| stale_agent_tokens.contains(&shell.capability_token))
        .map(|shell| shell.cancelled.clone())
        .collect::<Vec<_>>();
    for cancellation in owned_shells {
        cancellation.store(true, Ordering::SeqCst);
    }
}

fn bulk_set_history_archived(
    state: &AppState,
    ids: Vec<String>,
    is_archived: bool,
    workdir: String,
) -> Result<ChatHistoryBatchMutationResult, String> {
    state.require_persistence_ready()?;
    let ids = normalize_history_batch_ids(ids)?;
    let requested_workdir = canonical_workdir(workdir.trim())?;
    let _persist_guard = state.persist_lock.lock();
    let snapshots = snapshot_project_history_records(state, &ids, &requested_workdir)?;
    let mutation_time = now_ms();
    {
        let mut sessions = state.sessions.lock();
        for id in &ids {
            let record = sessions
                .get_mut(id)
                .expect("validated history conversation must remain available");
            record.archived_at = is_archived.then_some(mutation_time);
            record.summary.is_archived = is_archived;
            if is_archived {
                record.pinned_at = None;
                record.summary.is_pinned = false;
            }
            record.summary.updated_at = mutation_time;
        }
    }
    let persisted_sessions = {
        let sessions = state.sessions.lock();
        ids.iter()
            .filter_map(|id| sessions.get(id).map(stored_session_from_record))
            .collect::<Vec<_>>()
    };
    if let Err(error) = state
        .history
        .upsert_session_metadata_batch(&persisted_sessions)
    {
        restore_history_records(state, &snapshots);
        return Err(error);
    }
    Ok(ChatHistoryBatchMutationResult { affected_ids: ids })
}

fn bulk_delete_history(
    state: &AppState,
    ids: Vec<String>,
    workdir: String,
) -> Result<ChatHistoryBatchMutationResult, String> {
    state.require_persistence_ready()?;
    let ids = normalize_history_batch_ids(ids)?;
    let requested_workdir = canonical_workdir(workdir.trim())?;
    let _persist_guard = state.persist_lock.lock();
    let snapshots = snapshot_project_history_records(state, &ids, &requested_workdir)?;
    let requested_ids = ids.iter().cloned().collect::<HashSet<_>>();
    if state
        .active_runs
        .lock()
        .values()
        .any(|run| requested_ids.contains(&run.session_id))
    {
        return Err("cannot delete a session while its agent run is active".to_string());
    }
    {
        let mut sessions = state.sessions.lock();
        for id in &ids {
            sessions.remove(id);
        }
    }
    for id in &ids {
        if let Err(error) = state.history.delete_session_projection(id) {
            restore_history_records(state, &snapshots);
            return Err(error);
        }
    }
    revoke_session_owned_access(state, &requested_ids);
    // Never touch session media until every requested history projection has
    // been deleted successfully. A cleanup failure is deliberately nonfatal:
    // the durable conversation deletion remains authoritative.
    cleanup_deleted_sessions_composer_media(state, &ids);
    Ok(ChatHistoryBatchMutationResult { affected_ids: ids })
}

fn mutate_session_and_persist<T>(
    state: &AppState,
    id: &str,
    mutate: impl FnOnce(&mut SessionRecord) -> Result<T, String>,
) -> Result<T, String> {
    state.require_persistence_ready()?;
    let id = id.trim();
    if id.is_empty() {
        return Err("history conversation id is required".to_string());
    }
    let _persist_guard = state.persist_lock.lock();
    let previous = state
        .sessions
        .lock()
        .get(id)
        .cloned()
        .ok_or_else(|| "history conversation not found".to_string())?;
    let output = {
        let mut sessions = state.sessions.lock();
        let record = sessions
            .get_mut(id)
            .ok_or_else(|| "history conversation not found".to_string())?;
        mutate(record)?
    };
    if let Err(error) = state.persist_session_metadata_locked(id) {
        state.sessions.lock().insert(id.to_string(), previous);
        return Err(error);
    }
    Ok(output)
}

#[tauri::command(rename_all = "camelCase")]
pub fn chat_history_set_pinned(
    state: State<'_, Arc<AppState>>,
    id: String,
    is_pinned: bool,
) -> Result<ChatHistorySummary, String> {
    mutate_session_and_persist(&state, id.trim(), |record| {
        record.pinned_at = if is_pinned { Some(now_ms()) } else { None };
        record.summary.is_pinned = is_pinned;
        if is_pinned {
            record.archived_at = None;
            record.summary.is_archived = false;
        }
        record.summary.updated_at = now_ms();
        Ok(history_summary(record))
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn chat_history_set_model(
    state: State<'_, Arc<AppState>>,
    id: String,
    selected_model_json: String,
) -> Result<ChatHistorySummary, String> {
    let selected = parse_session_model_selection(&selected_model_json)?;
    let selected_model_json = serde_json::to_string(&selected)
        .map_err(|error| format!("selected model selection cannot be serialized: {error}"))?;
    mutate_session_and_persist(&state, id.trim(), |record| {
        record.selected_model_json = Some(selected_model_json);
        record.provider_id = selected.provider_id;
        record.model = selected.model_id;
        record.summary.updated_at = now_ms();
        Ok(history_summary(record))
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn chat_history_set_archived(
    state: State<'_, Arc<AppState>>,
    id: String,
    is_archived: bool,
) -> Result<ChatHistorySummary, String> {
    mutate_session_and_persist(&state, id.trim(), |record| {
        record.archived_at = if is_archived { Some(now_ms()) } else { None };
        record.summary.is_archived = is_archived;
        if is_archived {
            record.pinned_at = None;
            record.summary.is_pinned = false;
        }
        record.summary.updated_at = now_ms();
        Ok(history_summary(record))
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn chat_history_bulk_set_archived(
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    is_archived: bool,
    workdir: String,
) -> Result<ChatHistoryBatchMutationResult, String> {
    bulk_set_history_archived(&state, ids, is_archived, workdir)
}

#[tauri::command(rename_all = "camelCase")]
pub fn chat_history_bulk_delete(
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    workdir: String,
) -> Result<ChatHistoryBatchMutationResult, String> {
    bulk_delete_history(&state, ids, workdir)
}

#[tauri::command(rename_all = "camelCase")]
pub fn chat_history_branch(
    state: State<'_, Arc<AppState>>,
    id: String,
    title: Option<String>,
) -> Result<SessionSummary, String> {
    branch_session(&state, &id, title)
}

fn branch_session(
    state: &AppState,
    id: &str,
    title: Option<String>,
) -> Result<SessionSummary, String> {
    state.require_persistence_ready()?;
    let source_id = id.trim();
    if source_id.is_empty() {
        return Err("history conversation id is required".to_string());
    }
    let _persist_guard = state.persist_lock.lock();
    if state
        .active_runs
        .lock()
        .values()
        .any(|run| run.session_id == source_id)
    {
        return Err("cannot branch a session while its agent run is active".to_string());
    }
    let source = state
        .sessions
        .lock()
        .get(source_id)
        .cloned()
        .ok_or_else(|| "history conversation not found".to_string())?;
    let source_workdir = canonical_workdir(&source.summary.cwd)?;
    require_registered_project_workdir(state, &source_workdir)?;
    // Branching is a full-history operation. The in-memory record normally
    // contains only the most recent page, so hydrate from SQLite while the
    // persistence gate prevents a concurrent append from moving the boundary.
    let durable_messages = state.history.load_messages(source_id)?;
    let mut branch = source;
    branch.messages = durable_messages
        .into_iter()
        .map(message_record_from_stored)
        .collect();
    branch.message_count = branch.messages.len() as i64;
    branch.messages_loaded = true;
    branch.messages_complete = true;
    branch.summary.id = Uuid::new_v4().to_string();
    branch.summary.title = title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{} (分支)", branch.summary.title));
    branch.summary.updated_at = now_ms();
    branch.pinned_at = None;
    branch.archived_at = None;
    branch.summary.is_pinned = false;
    branch.summary.is_archived = false;
    branch.share_enabled = false;
    branch.share_token = None;
    branch.share_created_at = None;
    branch.share_updated_at = None;
    // A branch starts with its own objective. Do not leave a copied in-memory
    // goal that has no matching durable session_goals row.
    branch.goal = None;
    let media_ids = branch
        .messages
        .iter()
        .flat_map(|message| composer_media_ids_in_content(&message.content))
        .collect::<HashSet<_>>();
    for message in &mut branch.messages {
        message.id = Uuid::new_v4().to_string();
        message.turn_id = None;
        message.model = None;
        message.reasoning = None;
        message.finished_at = None;
    }
    let summary = branch.summary.clone();
    if let Err(error) = copy_composer_media_for_branch(
        state,
        source_id,
        &summary.id,
        media_ids.iter().map(String::as_str),
    ) {
        let _ = cleanup_deleted_session_composer_media(state, &summary.id);
        return Err(error);
    }
    state.sessions.lock().insert(summary.id.clone(), branch);
    if let Err(error) = state.persist_session_locked(&summary.id) {
        state.sessions.lock().remove(&summary.id);
        let _ = cleanup_deleted_session_composer_media(state, &summary.id);
        return Err(error);
    }
    Ok(summary)
}

#[tauri::command]
pub fn chat_history_truncate(
    state: State<'_, Arc<AppState>>,
    id: String,
    message_id: String,
) -> Result<ChatHistorySummary, String> {
    state.require_persistence_ready()?;
    let id = id.trim().to_string();
    let message_id = message_id.trim().to_string();
    if id.is_empty() || message_id.is_empty() {
        return Err("history conversation and message ids are required".to_string());
    }
    let _persist_guard = state.persist_lock.lock();
    if state
        .active_runs
        .lock()
        .values()
        .any(|run| run.session_id == id)
    {
        return Err("cannot truncate a session while its agent run is active".to_string());
    }
    let removed = state
        .sessions
        .lock()
        .get(&id)
        .cloned()
        .ok_or_else(|| "history conversation not found".to_string())?;
    // Durable truncation runs inside the history store's own transaction.
    state.history.truncate_session(&id, &message_id)?;
    // Keep only the messages strictly before the anchor (created_at is the
    // sort key, id the tie-breaker), matching the durable delete exactly.
    let mut session = removed;
    let anchor = session
        .messages
        .iter()
        .find(|message| message.id == message_id)
        .map(|message| (message.created_at, message.id.as_str()));
    session.messages = match anchor {
        Some((anchor_created_at, anchor_id)) => session
            .messages
            .iter()
            .filter(|message| {
                message.created_at < anchor_created_at
                    || (message.created_at == anchor_created_at
                        && message.id.as_str() < anchor_id)
            })
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    session.messages_complete = false;
    session.messages_loaded = true;
    state.sessions.lock().insert(id.clone(), session);
    let record = state
        .sessions
        .lock()
        .get(&id)
        .cloned()
        .ok_or_else(|| "history conversation not found".to_string())?;
    Ok(history_summary(&record))
}

#[tauri::command]
pub fn chat_history_delete(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    state.require_persistence_ready()?;
    let id = id.trim().to_string();
    if id.is_empty() {
        return Err("history conversation id is required".to_string());
    }
    let _persist_guard = state.persist_lock.lock();
    if state
        .active_runs
        .lock()
        .values()
        .any(|run| run.session_id == id)
    {
        return Err("cannot delete a session while its agent run is active".to_string());
    }
    let removed = state
        .sessions
        .lock()
        .remove(&id)
        .ok_or_else(|| "history conversation not found".to_string())?;
    if let Err(error) = state.history.delete_session_projection(&id) {
        state.sessions.lock().insert(id, removed);
        return Err(error);
    }
    let deleted_session_ids = vec![id.clone()];
    revoke_session_owned_access(&state, &HashSet::from([id]));
    // The durable history row is gone before any session-local attachment
    // directory is considered for cleanup. Keep a cleanup failure nonfatal so
    // the caller never sees a successful history deletion as a failed action.
    cleanup_deleted_sessions_composer_media(&state, &deleted_session_ids);
    Ok(())
}

fn history_share_status(record: &SessionRecord) -> Value {
    json!({
        "conversationId": record.summary.id,
        "enabled": record.share_enabled,
        "token": record.share_enabled.then(|| record.share_token.clone()).flatten(),
        "createdAt": record.share_created_at,
        "updatedAt": record.share_updated_at,
        "redactToolContent": record.redact_tool_content,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn chat_history_share_get(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<Value, String> {
    state
        .sessions
        .lock()
        .get(id.trim())
        .map(history_share_status)
        .ok_or_else(|| "history conversation not found".to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn chat_history_share_set(
    state: State<'_, Arc<AppState>>,
    id: String,
    enabled: bool,
    redact_tool_content: Option<bool>,
) -> Result<Value, String> {
    mutate_session_and_persist(&state, id.trim(), |record| {
        let now = now_ms();
        if enabled && !record.share_enabled {
            record.share_token = Some(format!("share-{}", Uuid::new_v4()));
            record.share_created_at = Some(now);
        }
        if !enabled {
            record.share_token = None;
            record.share_created_at = None;
        }
        record.share_enabled = enabled;
        record.share_updated_at = Some(now);
        if let Some(redact) = redact_tool_content {
            record.redact_tool_content = redact;
        }
        record.summary.updated_at = now;
        Ok(history_share_status(record))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Manager;

    #[test]
    fn system_info_exposes_only_safe_renderer_metadata() {
        let payload = serde_json::to_value(system_info())
            .expect("system info should serialize for the renderer");
        assert_eq!(payload["product"], "NovaVei");
        assert_eq!(payload["backend"], "tauri-rust");
        assert_eq!(payload["piRuntime"], "embedded-webview");
        assert!(payload.get("statePath").is_none());
    }

    #[test]
    fn portable_project_scope_rejects_another_machine_or_portable_folder() {
        let scope = json!({
            "version": PORTABLE_PROJECT_SCOPE_VERSION,
            "root": r"E:\NovaVei\release",
            "machine": "old-pc",
        });
        assert!(portable_project_scope_matches(
            Some(&scope),
            r"e:\novavei\release",
            Some("OLD-PC"),
        ));
        // A removable drive that received a different letter on the same
        // machine is still the same portable folder.
        assert!(portable_project_scope_matches(
            Some(&scope),
            r"f:\novavei\release",
            Some("old-pc"),
        ));
        assert!(!portable_project_scope_matches(
            Some(&scope),
            r"e:\другой\release",
            Some("old-pc"),
        ));
        assert!(!portable_project_scope_matches(
            Some(&scope),
            r"E:\NovaVei\release",
            Some("new-pc"),
        ));
        assert!(portable_project_scope_matches(
            None,
            r"E:\NovaVei\release",
            Some("new-pc"),
        ));
    }

    #[test]
    fn drive_agnostic_scope_keys_strip_only_drive_prefixes() {
        assert_eq!(drive_agnostic_scope_key(r"e:\novavei"), r"\novavei");
        assert_eq!(drive_agnostic_scope_key(r"f:\novavei"), r"\novavei");
        assert_eq!(
            drive_agnostic_scope_key(r"\\server\share\novavei"),
            r"\\server\share\novavei"
        );
        assert_eq!(drive_agnostic_scope_key("relative"), "relative");
        assert_eq!(workspace_drive_letter(r"e:\novavei"), Some('E'));
        assert_eq!(workspace_drive_letter(r"\\server\share"), None);
    }

    #[test]
    fn drive_remap_only_moves_entries_that_exist_on_the_new_drive() {
        let temp = std::env::temp_dir();
        let temp_display = path_for_display(&temp);
        let Some(drive) = workspace_drive_letter(&temp_display) else {
            return;
        };
        // The path exists on the current drive; pretend the scope recorded a
        // previous letter that no longer exists so the remap must fire.
        let previous: char = if drive == 'Q' { 'R' } else { 'Q' };
        let stale = format!("{previous}{}", &temp_display[1..]);
        let mut entry = json!({ "path": stale });
        remap_project_entry_drive(&mut entry, previous, drive);
        assert_eq!(
            entry.get("path").and_then(Value::as_str),
            Some(temp_display.as_str()),
            "an unreachable old-drive path should move to the reachable current drive"
        );

        // A path on an unrelated drive letter is never rewritten.
        let mut unrelated = json!({ "path": r"C:\Windows" });
        remap_project_entry_drive(&mut unrelated, previous, drive);
        assert_eq!(
            unrelated.get("path").and_then(Value::as_str),
            Some(r"C:\Windows")
        );
    }

    fn test_state() -> (AppState, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "novavei-backend-{}-{}.sqlite3",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let history = HistoryStore::new(path.clone());
        history
            .initialize()
            .expect("test history should initialize");
        let subagent_tasks = SubagentStore::new(path.clone());
        subagent_tasks
            .initialize()
            .expect("test subagent task store should initialize");
        let cwd = std::env::current_dir().expect("test cwd should exist");
        let approved_cwd = fs::canonicalize(&cwd).expect("test cwd should canonicalize");
        let record = new_session_record("test".to_string(), cwd.display().to_string());
        let session_id = record.summary.id.clone();
        let state = AppState {
            data_file: path.clone(),
            history,
            subagent_tasks,
            sessions: Mutex::new(HashMap::from([(session_id, record)])),
            history_search_index: Mutex::new(InMemoryHistorySearchIndex::default()),
            settings: Mutex::new(HashMap::new()),
            provider_import_previews: Mutex::new(HashMap::new()),
            provider_connection_drafts: Mutex::new(HashMap::new()),
            persist_lock: Mutex::new(()),
            settings_locked: AtomicBool::new(false),
            storage_recovery: StorageRecoveryStatus::ready(),
            approved_workdirs: Mutex::new(HashSet::from([approved_cwd])),
            picker_workdirs: Mutex::new(HashSet::new()),
            relocation_picker_grants: Mutex::new(HashMap::new()),
            relocation_conflict_grants: Mutex::new(HashMap::new()),
            active_runs: Mutex::new(HashMap::new()),
            capabilities: Mutex::new(HashMap::new()),
            full_permission_grants: Mutex::new(HashMap::new()),
            subagent_capabilities: Mutex::new(HashMap::new()),
            workspace_capabilities: Mutex::new(HashMap::new()),
            git_commit_capabilities: Mutex::new(HashMap::new()),
            pending_permissions: Mutex::new(HashMap::new()),
            tool_approvals: Mutex::new(HashMap::new()),
            shell_runs: Mutex::new(HashMap::new()),
        };
        (state, path)
    }

    fn register_test_project(state: &AppState, workdir: &Path) {
        state.settings.lock().insert(
            "projects".to_string(),
            json!({
                "version": PROJECT_SETTINGS_VERSION,
                "initialized": true,
                "entries": [{
                    "id": new_stable_project_id(),
                    "name": "Test project",
                    "path": path_for_display(workdir),
                    "lastSessionId": Value::Null,
                    "pinned": false
                }]
            }),
        );
        state.approved_workdirs.lock().insert(workdir.to_path_buf());
    }

    fn cleanup_test_db(path: &Path) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    fn composer_test_png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        bytes.extend_from_slice(b"IEND");
        bytes.extend_from_slice(&[0; 4]);
        bytes
    }

    #[test]
    fn composer_media_ipc_payload_keeps_descriptor_and_bytes_separate() {
        let descriptor = ComposerMediaAttachment {
            id: "media-123".to_string(),
            name: "test.png".to_string(),
            mime: "image/png".to_string(),
            kind: ComposerMediaKind::Image,
            size_bytes: 3,
        };
        let bytes = vec![0xde, 0xad, 0xbe];

        let payload = composer_media_ipc_payload(descriptor, bytes.clone())
            .expect("the IPC envelope should serialize");
        let header_len = u32::from_be_bytes(payload[..4].try_into().unwrap()) as usize;
        let header: Value = serde_json::from_slice(&payload[4..4 + header_len])
            .expect("the descriptor header should be JSON");

        assert_eq!(header["id"], "media-123");
        assert_eq!(header["name"], "test.png");
        assert_eq!(header["mime"], "image/png");
        assert_eq!(header["kind"], "image");
        assert_eq!(header["sizeBytes"], 3);
        assert_eq!(&payload[4 + header_len..], bytes.as_slice());
    }

    fn composer_pasted_image_ipc_payload_for_test(header: &[u8], body: &[u8]) -> Vec<u8> {
        let mut payload =
            Vec::with_capacity(COMPOSER_PASTED_IMAGE_IPC_PREFIX_BYTES + header.len() + body.len());
        payload.extend_from_slice(COMPOSER_PASTED_IMAGE_IPC_MAGIC);
        payload.push(COMPOSER_PASTED_IMAGE_IPC_VERSION);
        payload.extend_from_slice(&(header.len() as u32).to_be_bytes());
        payload.extend_from_slice(header);
        payload.extend_from_slice(body);
        payload
    }

    #[test]
    fn composer_pasted_image_ipc_payload_is_versioned_bounded_and_binary() {
        let header = br#"{"workdir":"workspace","sessionId":"session-1","name":"clip.png","mime":"image/png"}"#;
        let body = [0x89, b'P', b'N', b'G'];
        let payload = composer_pasted_image_ipc_payload_for_test(header, &body);
        let (decoded, decoded_body) = decode_composer_pasted_image_ipc_payload(&payload)
            .expect("a valid raw pasted-image envelope should decode");
        assert_eq!(decoded.workdir, "workspace");
        assert_eq!(decoded.session_id, "session-1");
        assert_eq!(decoded.name, "clip.png");
        assert_eq!(decoded.mime.as_deref(), Some("image/png"));
        assert_eq!(decoded_body, body.as_slice());

        let mut invalid_magic = payload.clone();
        invalid_magic[0] ^= 0xff;
        assert!(decode_composer_pasted_image_ipc_payload(&invalid_magic).is_err());

        let mut unsupported_version = payload.clone();
        unsupported_version[COMPOSER_PASTED_IMAGE_IPC_MAGIC.len()] += 1;
        assert!(decode_composer_pasted_image_ipc_payload(&unsupported_version).is_err());

        let duplicate_field = br#"{"workdir":"workspace","sessionId":"session-1","name":"first.png","name":"second.png","mime":"image/png"}"#;
        assert!(decode_composer_pasted_image_ipc_payload(
            &composer_pasted_image_ipc_payload_for_test(duplicate_field, &body),
        )
        .is_err());

        let unknown_field = br#"{"workdir":"workspace","sessionId":"session-1","name":"clip.png","mime":"image/png","extra":true}"#;
        assert!(decode_composer_pasted_image_ipc_payload(
            &composer_pasted_image_ipc_payload_for_test(unknown_field, &body),
        )
        .is_err());

        let nul_field = br#"{"workdir":"workspace","sessionId":"session-1","name":"clip\u0000.png","mime":"image/png"}"#;
        assert!(decode_composer_pasted_image_ipc_payload(
            &composer_pasted_image_ipc_payload_for_test(nul_field, &body),
        )
        .is_err());

        assert!(decode_composer_pasted_image_ipc_payload(
            &composer_pasted_image_ipc_payload_for_test(&[0xff], &body),
        )
        .is_err());

        assert!(decode_composer_pasted_image_ipc_payload(
            &composer_pasted_image_ipc_payload_for_test(header, &[]),
        )
        .is_err());

        let oversized_body = vec![0; MAX_COMPOSER_MEDIA_BYTES as usize + 1];
        assert!(decode_composer_pasted_image_ipc_payload(
            &composer_pasted_image_ipc_payload_for_test(header, &oversized_body),
        )
        .is_err());
    }

    #[test]
    fn composer_media_load_revalidates_stored_image_bounds() {
        let (state, database_path) = test_state();
        let session_id = state.sessions.lock().keys().next().cloned().unwrap();
        let valid_bytes = composer_test_png(1, 1);
        let descriptor = stage_composer_media_bytes(
            &state,
            &session_id,
            "test.png",
            &valid_bytes,
            (ComposerMediaKind::Image, "image/png"),
        )
        .expect("bounded image should stage");
        let session_root = composer_media_session_root(&state, &session_id).unwrap();
        let replacement = composer_test_png(MAX_COMPOSER_IMAGE_DIMENSION + 1, 1);
        assert_eq!(replacement.len(), valid_bytes.len());
        fs::write(
            session_root.join(format!("{}.bin", descriptor.id)),
            replacement,
        )
        .expect("replace staged image with a same-length payload");

        let error = load_composer_media_attachment(&state, &session_id, &descriptor.id)
            .expect_err("load must revalidate the stored image payload");
        assert!(error.contains("dimensions exceed the safe limit"));

        let _ = fs::remove_dir_all(session_root);
        drop(state);
        cleanup_test_db(&database_path);
    }

    #[test]
    fn composer_media_reader_binds_persisted_size_before_loading_bytes() {
        let (state, database_path) = test_state();
        let session_id = state.sessions.lock().keys().next().cloned().unwrap();
        let descriptor = stage_composer_media_bytes(
            &state,
            &session_id,
            "test.png",
            &composer_test_png(1, 1),
            (ComposerMediaKind::Image, "image/png"),
        )
        .expect("bounded image should stage");
        let session_root = composer_media_session_root(&state, &session_id).unwrap();
        let error = read_composer_media_file(
            &session_root,
            &session_root.join(format!("{}.bin", descriptor.id)),
            MAX_COMPOSER_MEDIA_BYTES,
            Some(descriptor.size_bytes + 1),
            "media data",
        )
        .expect_err("a persisted size mismatch must reject the file before loading it");
        assert!(error.contains("invalid"));

        let _ = fs::remove_dir_all(session_root);
        drop(state);
        cleanup_test_db(&database_path);
    }

    #[test]
    fn workspace_reveal_target_requires_an_exact_registered_project() {
        let (state, database_path) = test_state();
        let approved = state
            .approved_workdirs
            .lock()
            .iter()
            .next()
            .cloned()
            .expect("test state has an approved workspace");
        state.settings.lock().insert(
            "projects".to_string(),
            json!({
                "version": PROJECT_SETTINGS_VERSION,
                "initialized": true,
                "entries": [{
                    "id": "project-reveal-test",
                    "name": "Reveal test",
                    "path": path_for_display(&approved),
                    "pinned": false
                }]
            }),
        );
        let (_, displayed) =
            approved_workspace_reveal_target(&state, &approved.display().to_string())
                .expect("the approved root can be revealed");
        assert_eq!(displayed, path_for_display(&approved));

        let unapproved = std::env::temp_dir().join(format!(
            "novavei-unapproved-reveal-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&unapproved).expect("create unapproved test directory");
        let error = approved_workspace_reveal_target(&state, &unapproved.display().to_string())
            .expect_err("an arbitrary renderer path must not be revealable");
        assert!(error.contains("not a registered project"));
        let _ = fs::remove_dir_all(&unapproved);
        cleanup_test_db(&database_path);
    }

    #[test]
    fn shell_run_registration_is_capability_bound_and_cleaned_up() {
        let (state, database_path) = test_state();
        let registration = register_shell_run(
            &state,
            Some("  registration-test  "),
            Some("  capability-test  "),
        )
        .expect("shell registration should succeed")
        .expect("run id and capability should create a registration");

        assert!(cancel_shell_run(
            &state,
            "registration-test",
            Some("capability-test"),
        ));
        assert!(registration.cancellation_token().load(Ordering::SeqCst));
        assert!(!cancel_shell_run(
            &state,
            "registration-test",
            Some("wrong-capability"),
        ));

        drop(registration);
        assert!(!cancel_shell_run(
            &state,
            "registration-test",
            Some("capability-test"),
        ));
        cleanup_test_db(&database_path);
    }

    #[test]
    fn shell_output_collection_is_bounded_when_a_pipe_stays_open() {
        struct BlockingReader {
            entered: Arc<AtomicBool>,
            release: Arc<AtomicBool>,
        }

        impl std::io::Read for BlockingReader {
            fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
                self.entered.store(true, Ordering::SeqCst);
                while !self.release.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ok(0)
            }
        }

        let entered = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let capture = bounded_output(BlockingReader {
            entered: entered.clone(),
            release: release.clone(),
        });
        let reader_deadline = Instant::now() + Duration::from_secs(1);
        while !entered.load(Ordering::SeqCst) && Instant::now() < reader_deadline {
            std::thread::sleep(Duration::from_millis(5));
        }

        let started = Instant::now();
        let (_, truncated, still_open) =
            collect_bounded_output(Some(capture), Instant::now() + Duration::from_millis(25));
        release.store(true, Ordering::SeqCst);

        assert!(
            entered.load(Ordering::SeqCst),
            "reader did not begin blocking"
        );
        assert!(
            still_open,
            "open pipe should be reported without waiting forever"
        );
        assert!(truncated, "partial output must be marked incomplete");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "output collection exceeded its bounded drain window"
        );
    }

    #[cfg(windows)]
    fn wait_for_file(path: &Path, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if path.is_file() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// `Start-Process -ArgumentList` joins an array into one command line, so
    /// a nested `-Command` script loses its quoting on some Windows hosts.
    /// Write the child action to a test-owned `.ps1` file and pass one quoted
    /// `-File` argument instead. This keeps the regression test focused on
    /// process-tree termination rather than PowerShell argument reparsing.
    #[cfg(windows)]
    fn powershell_tree_child_script(
        marker_id: &str,
        ready_marker: &Path,
        survived_marker: &Path,
        survive_after_seconds: u64,
    ) -> PathBuf {
        let script_path = std::env::temp_dir().join(format!("{marker_id}-child.ps1"));
        let ready_literal = ready_marker.display().to_string().replace('\'', "''");
        let survived_literal = survived_marker.display().to_string().replace('\'', "''");
        fs::write(
            &script_path,
            format!(
                "[System.IO.File]::WriteAllText('{ready_literal}', 'ready')\n\
                 Start-Sleep -Seconds {survive_after_seconds}\n\
                 [System.IO.File]::WriteAllText('{survived_literal}', 'survived')\n\
                 Start-Sleep -Seconds 20\n"
            ),
        )
        .expect("test PowerShell child script should be writable");
        script_path
    }

    #[cfg(windows)]
    fn powershell_tree_parent_command(child_script: &Path) -> String {
        let child_path = child_script.display().to_string();
        // Quotes are part of the single ArgumentList value passed to
        // Start-Process; they survive its string join when Temp contains
        // whitespace. Escape apostrophes for the outer PowerShell literal.
        let child_args = format!(
            r#"-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{child_path}""#
        )
        .replace('\'', "''");
        format!(
            "$childArgs = '{child_args}'; \
             Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\\WindowsPowerShell\\v1.0\\powershell.exe') -ArgumentList $childArgs; \
             Start-Sleep -Seconds 20"
        )
    }

    #[cfg(windows)]
    #[test]
    fn shell_cancel_terminates_powershell_process_tree() {
        let (state, database_path) = test_state();
        let state = Arc::new(state);
        let workdir = std::env::current_dir().expect("test workdir should exist");
        let workdir = fs::canonicalize(workdir).expect("test workdir should canonicalize");
        let marker_id = format!(
            "novavei-shell-tree-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let ready_marker = std::env::temp_dir().join(format!("{marker_id}-ready"));
        let survived_marker = std::env::temp_dir().join(format!("{marker_id}-survived"));
        let child_script =
            powershell_tree_child_script(&marker_id, &ready_marker, &survived_marker, 3);
        let command = powershell_tree_parent_command(&child_script);

        let run_id = format!("{marker_id}-run");
        let capability_token = format!("{marker_id}-capability");
        let thread_state = state.clone();
        let thread_workdir = workdir.clone();
        let thread_run_id = run_id.clone();
        let thread_capability_token = capability_token.clone();
        let thread = std::thread::spawn(move || {
            run_shell_sync_authorized(
                thread_state.as_ref(),
                workdir.display().to_string(),
                command,
                None,
                Some(15_000),
                None,
                Some(thread_run_id),
                Some(thread_capability_token),
                thread_workdir,
                false,
            )
        });

        let ready = wait_for_file(&ready_marker, Duration::from_secs(5));
        let cancelled = cancel_shell_run(state.as_ref(), &run_id, Some(&capability_token));
        let response = thread
            .join()
            .expect("shell worker should not panic")
            .expect("cancelled shell should return a result");
        let child_survived = wait_for_file(&survived_marker, Duration::from_secs(4));
        let _ = fs::remove_file(ready_marker);
        let _ = fs::remove_file(survived_marker);
        let _ = fs::remove_file(child_script);
        cleanup_test_db(&database_path);

        assert!(
            ready,
            "PowerShell child did not signal that it started before cancellation"
        );
        assert!(
            cancelled,
            "shell cancellation token was not registered or capability-bound"
        );
        assert!(response.cancelled);
        assert!(!response.timed_out);
        assert!(
            !child_survived,
            "PowerShell child survived cancellation and continued executing"
        );
    }

    #[cfg(windows)]
    #[test]
    fn shell_timeout_terminates_powershell_process_tree() {
        let (state, database_path) = test_state();
        let state = Arc::new(state);
        let workdir = std::env::current_dir().expect("test workdir should exist");
        let workdir = fs::canonicalize(workdir).expect("test workdir should canonicalize");
        let marker_id = format!(
            "novavei-shell-timeout-tree-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let ready_marker = std::env::temp_dir().join(format!("{marker_id}-ready"));
        let survived_marker = std::env::temp_dir().join(format!("{marker_id}-survived"));
        let child_script =
            powershell_tree_child_script(&marker_id, &ready_marker, &survived_marker, 5);
        let command = powershell_tree_parent_command(&child_script);

        let run_id = format!("{marker_id}-run");
        let capability_token = format!("{marker_id}-capability");
        let thread_state = state.clone();
        let thread_workdir = workdir.clone();
        let thread = std::thread::spawn(move || {
            run_shell_sync_authorized(
                thread_state.as_ref(),
                workdir.display().to_string(),
                command,
                None,
                Some(3_000),
                None,
                Some(run_id),
                Some(capability_token),
                thread_workdir,
                false,
            )
        });

        let ready = wait_for_file(&ready_marker, Duration::from_secs(5));
        let response = thread
            .join()
            .expect("shell worker should not panic")
            .expect("timed-out shell should return a result");
        let child_survived = wait_for_file(&survived_marker, Duration::from_secs(4));
        let _ = fs::remove_file(ready_marker);
        let _ = fs::remove_file(survived_marker);
        let _ = fs::remove_file(child_script);
        cleanup_test_db(&database_path);

        assert!(
            ready,
            "PowerShell child did not signal that it started before timeout"
        );
        assert!(response.timed_out);
        assert!(!response.cancelled);
        assert!(
            !child_survived,
            "PowerShell child survived timeout and continued executing"
        );
    }

    #[test]
    fn provider_draft_normalization_rejects_authorization_custom_header() {
        let result = normalize_provider_connection_draft(json!({
            "id": "draft-provider",
            "name": "Draft provider",
            "type": "codex",
            "baseUrl": "https://provider.example/v1",
            "models": [{"id": "draft-model"}],
            "requestFormat": "openai-responses",
            "customHeaders": [{"key": "Authorization", "value": "Bearer prohibited"}],
        }));

        assert!(result.is_err());
    }

    #[test]
    fn provider_settings_save_uses_the_native_connection_normalizer() {
        let valid = json!([{
            "id": "gateway",
            "name": "Gateway",
            "type": "codex",
            "baseUrl": "HTTPS://API.EXAMPLE.COM/tenant-a/v1",
            "models": ["model-a"],
            "activeModels": ["model-a"],
            "requestFormat": "openai-responses",
            "customHeaders": [{"key": "X-Client", "value": "native"}],
        }]);
        let normalized = normalize_provider_settings_payload(valid).unwrap();
        assert_eq!(
            normalized[0]["baseUrl"],
            json!("https://api.example.com/tenant-a/v1")
        );
        assert_eq!(normalized[0]["protocol"], json!("openai-responses"));

        for base_url in [
            "https://secret@api.example.com/v1",
            "https://api.example.com/v1?tenant=other",
            "https://api.example.com/v1#fragment",
            "https://api.example.com/tenant-a/%2e%2e/tenant-b/v1",
        ] {
            let invalid = json!([{
                "id": "gateway",
                "name": "Gateway",
                "type": "codex",
                "baseUrl": base_url,
                "models": ["model-a"],
            }]);
            assert!(
                normalize_provider_settings_payload(invalid).is_err(),
                "save accepted unsafe provider URL: {base_url}"
            );
        }
    }

    #[test]
    fn system_settings_preserve_a_valid_multiline_global_prompt() {
        let prompt = "始终使用中文。\n\nKeep code examples concise. 🚀";
        let normalized = normalize_system_settings_payload(json!({
            "global_system_prompt": prompt,
            "defaultPermissionTier": "ask",
        }))
        .expect("valid system settings should normalize");

        assert_eq!(
            normalized.get("globalSystemPrompt").and_then(Value::as_str),
            Some(prompt),
        );
        assert!(normalized.get("global_system_prompt").is_none());
    }

    #[test]
    fn system_settings_reject_invalid_global_prompts() {
        assert!(normalize_system_settings_payload(json!({
            "globalSystemPrompt": ["not", "a", "string"],
        }))
        .is_err());

        assert!(normalize_system_settings_payload(json!({
            "globalSystemPrompt": "a".repeat(MAX_GLOBAL_SYSTEM_PROMPT_CHARS + 1),
        }))
        .is_err());
    }

    #[test]
    fn provider_draft_normalization_rejects_duplicate_custom_header_names() {
        let result = normalize_provider_connection_draft(json!({
            "id": "draft-provider",
            "name": "Draft provider",
            "type": "codex",
            "baseUrl": "https://provider.example/v1",
            "models": [{"id": "draft-model"}],
            "requestFormat": "openai-responses",
            "customHeaders": [
                {"key": "X-Tenant", "value": "one"},
                {"key": "x-tenant", "value": "two"},
            ],
        }));

        assert!(result.is_err());
    }

    #[test]
    fn expired_provider_draft_token_is_rejected_and_removed() {
        let (state, path) = test_state();
        let draft_token = "expired-provider-draft".to_string();
        state.provider_connection_drafts.lock().insert(
            draft_token.clone(),
            ProviderConnectionDraft {
                provider: json!({"id": "draft-provider"}),
                created_at: Instant::now() - PROVIDER_CONNECTION_DRAFT_TTL - Duration::from_secs(1),
            },
        );

        assert!(provider_connection_draft(&state, &draft_token).is_err());
        assert!(!state
            .provider_connection_drafts
            .lock()
            .contains_key(&draft_token));

        cleanup_test_db(&path);
    }

    #[test]
    fn delegation_is_a_separate_high_risk_capability_action() {
        assert_eq!(tool_action("DelegateReadOnly"), Some(ToolAction::Subagent));
        assert_eq!(
            permission_requirement(PermissionMode::Readonly, ToolAction::Subagent),
            PermissionRequirement::Deny,
        );
        assert_eq!(
            permission_requirement(PermissionMode::Ask, ToolAction::Subagent),
            PermissionRequirement::Approval,
        );
    }

    #[test]
    fn sessions_list_reflects_pin_and_archive_mutations() {
        let (state, path) = test_state();
        let app = tauri::test::mock_builder()
            .manage(Arc::new(state))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("test app should build");
        let state = app.state::<Arc<AppState>>();
        let session_id = state
            .sessions
            .lock()
            .keys()
            .next()
            .cloned()
            .expect("test state has a session");
        {
            let mut sessions = state.sessions.lock();
            let record = sessions
                .get_mut(&session_id)
                .expect("test state has the selected session");
            record.message_count = 3;
        }
        let count_summary = sessions_list(state.clone())
            .into_iter()
            .find(|summary| summary.id == session_id)
            .expect("session with updated count should be listed");
        assert_eq!(count_summary.message_count, 3);

        chat_history_set_pinned(state.clone(), session_id.clone(), true)
            .expect("pin mutation should succeed");
        let pinned_summary = sessions_list(state.clone())
            .into_iter()
            .find(|summary| summary.id == session_id)
            .expect("pinned session should be listed");
        assert!(pinned_summary.is_pinned);
        assert!(!pinned_summary.is_archived);

        chat_history_set_archived(state.clone(), session_id.clone(), true)
            .expect("archive mutation should succeed");
        let archived_summary = sessions_list(state)
            .into_iter()
            .find(|summary| summary.id == session_id)
            .expect("archived session should be listed");
        assert!(!archived_summary.is_pinned);
        assert!(archived_summary.is_archived);

        cleanup_test_db(&path);
    }

    #[test]
    fn sessions_list_exposes_the_latest_terminal_run_result() {
        let (state, path) = test_state();
        let app = tauri::test::mock_builder()
            .manage(Arc::new(state))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("test app should build");
        let state = app.state::<Arc<AppState>>();
        let session_id = state
            .sessions
            .lock()
            .keys()
            .next()
            .cloned()
            .expect("test state has a session");

        record_terminal_session_result(&state, &session_id, "done", 1_725_000_123_456);
        let summary = sessions_list(state)
            .into_iter()
            .find(|summary| summary.id == session_id)
            .expect("session should be listed");
        assert_eq!(summary.last_run_status.as_deref(), Some("completed"));
        assert_eq!(summary.last_run_finished_at, Some(1_725_000_123_456));

        cleanup_test_db(&path);
    }

    #[test]
    fn bulk_archive_deduplicates_ids_and_clears_pins() {
        let (state, path) = test_state();
        let session_id = state
            .sessions
            .lock()
            .keys()
            .next()
            .cloned()
            .expect("test state has a session");
        let workdir = state
            .sessions
            .lock()
            .get(&session_id)
            .expect("test session exists")
            .summary
            .cwd
            .clone();
        state
            .sessions
            .lock()
            .get_mut(&session_id)
            .expect("test session exists")
            .pinned_at = Some(now_ms());

        let result = bulk_set_history_archived(
            &state,
            vec![format!("  {session_id}  "), session_id.clone()],
            true,
            workdir,
        )
        .expect("bulk archive should succeed");

        assert_eq!(result.affected_ids, vec![session_id.clone()]);
        let sessions = state.sessions.lock();
        let record = sessions
            .get(&session_id)
            .expect("test session remains after archive");
        assert!(record.archived_at.is_some());
        assert!(record.pinned_at.is_none());
        drop(sessions);
        cleanup_test_db(&path);
    }

    #[test]
    fn bulk_mutations_reject_unknown_or_malformed_ids_without_changes() {
        let (state, path) = test_state();
        let session_id = state
            .sessions
            .lock()
            .keys()
            .next()
            .cloned()
            .expect("test state has a session");
        let workdir = state
            .sessions
            .lock()
            .get(&session_id)
            .expect("test session exists")
            .summary
            .cwd
            .clone();

        assert!(bulk_set_history_archived(
            &state,
            vec!["unknown-history-session".to_string()],
            true,
            workdir.clone(),
        )
        .is_err());
        assert!(bulk_set_history_archived(
            &state,
            vec!["invalid session id".to_string()],
            true,
            workdir,
        )
        .is_err());
        let sessions = state.sessions.lock();
        let record = sessions
            .get(&session_id)
            .expect("test session remains unchanged");
        assert!(record.archived_at.is_none());
        assert!(record.pinned_at.is_none());
        drop(sessions);
        cleanup_test_db(&path);
    }

    #[test]
    fn bulk_mutations_reject_sessions_outside_requested_project() {
        let (state, path) = test_state();
        let first_session_id = state
            .sessions
            .lock()
            .keys()
            .next()
            .cloned()
            .expect("test state has a session");
        let first_workdir = state
            .sessions
            .lock()
            .get(&first_session_id)
            .expect("test session exists")
            .summary
            .cwd
            .clone();
        let second_record = new_session_record(
            "other project session".to_string(),
            path_for_display(&std::env::temp_dir()),
        );
        let second_session_id = second_record.summary.id.clone();
        state
            .sessions
            .lock()
            .insert(second_session_id.clone(), second_record);

        assert!(bulk_set_history_archived(
            &state,
            vec![first_session_id.clone(), second_session_id.clone()],
            true,
            first_workdir,
        )
        .is_err());
        let sessions = state.sessions.lock();
        assert!(sessions
            .get(&first_session_id)
            .expect("first session remains")
            .archived_at
            .is_none());
        assert!(sessions
            .get(&second_session_id)
            .expect("other project session remains")
            .archived_at
            .is_none());
        drop(sessions);
        cleanup_test_db(&path);
    }

    #[test]
    fn bulk_delete_rejects_active_runs_without_partial_deletion() {
        let (state, path) = test_state();
        let first_session_id = state
            .sessions
            .lock()
            .keys()
            .next()
            .cloned()
            .expect("test state has a session");
        let workdir = state
            .sessions
            .lock()
            .get(&first_session_id)
            .expect("test session exists")
            .summary
            .cwd
            .clone();
        let second_record = new_session_record("second session".to_string(), workdir.clone());
        let second_session_id = second_record.summary.id.clone();
        state
            .sessions
            .lock()
            .insert(second_session_id.clone(), second_record);
        state.active_runs.lock().insert(
            "active-bulk-delete".to_string(),
            ActiveRun {
                session_id: second_session_id.clone(),
                conversation_id: second_session_id.clone(),
                turn_id: "turn-bulk-delete".to_string(),
                request_id: "active-bulk-delete".to_string(),
                proxy_provider_id: None,
                capability_token: "capability-bulk-delete".to_string(),
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        );

        assert!(bulk_delete_history(
            &state,
            vec![first_session_id.clone(), second_session_id.clone()],
            workdir,
        )
        .is_err());
        let sessions = state.sessions.lock();
        assert!(sessions.contains_key(&first_session_id));
        assert!(sessions.contains_key(&second_session_id));
        drop(sessions);
        cleanup_test_db(&path);
    }

    #[test]
    fn bulk_delete_removes_every_requested_session() {
        let (state, path) = test_state();
        let first_session_id = state
            .sessions
            .lock()
            .keys()
            .next()
            .cloned()
            .expect("test state has a session");
        let workdir = state
            .sessions
            .lock()
            .get(&first_session_id)
            .expect("test session exists")
            .summary
            .cwd
            .clone();
        let second_record = new_session_record("second session".to_string(), workdir.clone());
        let second_session_id = second_record.summary.id.clone();
        state
            .sessions
            .lock()
            .insert(second_session_id.clone(), second_record);

        let result = bulk_delete_history(
            &state,
            vec![first_session_id.clone(), second_session_id.clone()],
            workdir,
        )
        .expect("bulk delete should succeed");

        assert_eq!(
            result.affected_ids,
            vec![first_session_id.clone(), second_session_id.clone()]
        );
        let sessions = state.sessions.lock();
        assert!(!sessions.contains_key(&first_session_id));
        assert!(!sessions.contains_key(&second_session_id));
        drop(sessions);
        cleanup_test_db(&path);
    }

    #[test]
    fn bulk_archive_restores_all_records_when_persistence_fails() {
        let (state, path) = test_state();
        let session_id = state
            .sessions
            .lock()
            .keys()
            .next()
            .cloned()
            .expect("test state has a session");
        let workdir = state
            .sessions
            .lock()
            .get(&session_id)
            .expect("test session exists")
            .summary
            .cwd
            .clone();
        state.settings_locked.store(true, Ordering::Release);

        assert!(
            bulk_set_history_archived(&state, vec![session_id.clone()], true, workdir).is_err()
        );
        let sessions = state.sessions.lock();
        let record = sessions
            .get(&session_id)
            .expect("failed bulk archive restores the session");
        assert!(record.archived_at.is_none());
        assert!(record.pinned_at.is_none());
        drop(sessions);
        cleanup_test_db(&path);
    }

    #[test]
    fn bulk_delete_restores_all_records_when_persistence_fails() {
        let (state, path) = test_state();
        let first_session_id = state
            .sessions
            .lock()
            .keys()
            .next()
            .cloned()
            .expect("test state has a session");
        let workdir = state
            .sessions
            .lock()
            .get(&first_session_id)
            .expect("test session exists")
            .summary
            .cwd
            .clone();
        let second_record = new_session_record("second session".to_string(), workdir.clone());
        let second_session_id = second_record.summary.id.clone();
        state
            .sessions
            .lock()
            .insert(second_session_id.clone(), second_record);
        state.settings_locked.store(true, Ordering::Release);

        assert!(bulk_delete_history(
            &state,
            vec![first_session_id.clone(), second_session_id.clone()],
            workdir,
        )
        .is_err());
        let sessions = state.sessions.lock();
        assert!(sessions.contains_key(&first_session_id));
        assert!(sessions.contains_key(&second_session_id));
        drop(sessions);
        cleanup_test_db(&path);
    }

    #[test]
    fn session_goal_validation_keeps_a_bounded_consistent_dto() {
        assert_eq!(
            normalize_session_goal_text("  finish\n the   session goal  ".to_string()).unwrap(),
            "finish the session goal"
        );
        assert!(normalize_session_goal_text(String::new()).is_err());
        assert!(normalize_session_goal_text("x".repeat(MAX_SESSION_GOAL_TEXT_CHARS + 1)).is_err());
        assert!(validate_session_goal_state(SessionGoalStatus::Active, 0).is_ok());
        assert!(validate_session_goal_state(SessionGoalStatus::Completed, 100).is_ok());
        assert!(validate_session_goal_state(SessionGoalStatus::Active, 100).is_err());
        assert!(validate_session_goal_state(SessionGoalStatus::Completed, 99).is_err());
        assert!(validate_session_goal_session_id("session_goal-1").is_ok());
        assert!(validate_session_goal_session_id("session goal").is_err());
        assert!(session_goal_from_stored(&StoredSessionGoal {
            text: "valid goal".to_string(),
            status: "completed".to_string(),
            progress: 100,
            updated_at: 1,
        })
        .is_some());
        assert!(session_goal_from_stored(&StoredSessionGoal {
            text: "invalid goal".to_string(),
            status: "active".to_string(),
            progress: 100,
            updated_at: 1,
        })
        .is_none());
    }

    #[test]
    fn session_model_selection_is_a_bounded_credential_free_dto() {
        let selected = parse_session_model_selection(
            r#"{"providerId":"openai-compat","modelId":"gpt-5.6-sol"}"#,
        )
        .unwrap();
        assert_eq!(selected.provider_id, "openai-compat");
        assert_eq!(selected.model_id, "gpt-5.6-sol");
        assert!(parse_session_model_selection(
            r#"{"providerId":"openai-compat","modelId":"gpt-5.6-sol","apiKey":"not-allowed"}"#,
        )
        .is_err());
        assert!(parse_session_model_selection(
            r#"{"providerId":"bad/provider","modelId":"gpt-5.6-sol"}"#,
        )
        .is_err());
        assert!(parse_session_model_selection(
            r#"{"providerId":"openai-compat","modelId":"\u0000"}"#,
        )
        .is_err());
    }

    #[test]
    fn session_goal_update_changes_only_the_goal_record() {
        let (state, path) = test_state();
        state.persist().unwrap();
        let session_id = state
            .sessions
            .lock()
            .keys()
            .next()
            .cloned()
            .expect("test state has a session");
        let workdir = canonical_workdir(
            &state
                .sessions
                .lock()
                .get(&session_id)
                .expect("test session exists")
                .summary
                .cwd,
        )
        .unwrap();
        register_test_project(&state, &workdir);
        let saved = set_session_goal(
            &state,
            session_id.clone(),
            Some("Finish the native goal UI".to_string()),
            Some(SessionGoalStatus::Active),
            Some(42),
            Some(false),
        )
        .unwrap()
        .expect("set returns a goal");
        assert_eq!(saved.status, SessionGoalStatus::Active);
        assert_eq!(saved.progress, 42);
        assert_eq!(
            state.history.load_session_goals().unwrap()[&session_id].progress,
            42
        );
        let progressed = update_existing_session_goal_progress(
            &state,
            session_id.clone(),
            SessionGoalStatus::Active,
            73,
            saved.updated_at,
        )
        .unwrap();
        assert_eq!(progressed.text, "Finish the native goal UI");
        assert_eq!(progressed.status, SessionGoalStatus::Active);
        assert_eq!(progressed.progress, 73);
        assert!(progressed.updated_at > saved.updated_at);
        assert!(update_existing_session_goal_progress(
            &state,
            session_id.clone(),
            SessionGoalStatus::Active,
            74,
            saved.updated_at,
        )
        .is_err());
        let completed = update_existing_session_goal_progress(
            &state,
            session_id.clone(),
            SessionGoalStatus::Completed,
            100,
            progressed.updated_at,
        )
        .unwrap();
        assert_eq!(completed.text, "Finish the native goal UI");
        assert_eq!(completed.status, SessionGoalStatus::Completed);
        assert_eq!(completed.progress, 100);
        assert!(set_session_goal(
            &state,
            session_id.clone(),
            Some("not allowed".to_string()),
            Some(SessionGoalStatus::Active),
            Some(100),
            Some(false),
        )
        .is_err());
        let branch = branch_session(&state, &session_id, None).unwrap();
        assert!(state
            .sessions
            .lock()
            .get(&branch.id)
            .and_then(|record| record.goal.as_ref())
            .is_none());
        assert!(!state
            .history
            .load_session_goals()
            .unwrap()
            .contains_key(&branch.id));
        assert!(update_existing_session_goal_progress(
            &state,
            branch.id,
            SessionGoalStatus::Active,
            1,
            completed.updated_at,
        )
        .is_err());
        assert!(
            set_session_goal(&state, session_id.clone(), None, None, None, Some(true),)
                .unwrap()
                .is_none()
        );
        assert!(state.history.load_session_goals().unwrap().is_empty());
        cleanup_test_db(&path);
    }

    #[test]
    fn permission_modes_enforce_native_tool_matrix() {
        use PermissionRequirement::{Allow, Approval, Deny};

        for mode in [
            PermissionMode::Readonly,
            PermissionMode::Ask,
            PermissionMode::Full,
        ] {
            assert_eq!(permission_requirement(mode, ToolAction::Read), Allow);
        }
        for action in [
            ToolAction::Write,
            ToolAction::MemoryWrite,
            ToolAction::Edit,
            ToolAction::Delete,
            ToolAction::Shell,
            ToolAction::GitCommit,
        ] {
            assert_eq!(
                permission_requirement(PermissionMode::Readonly, action),
                Deny
            );
            assert_eq!(
                permission_requirement(PermissionMode::Ask, action),
                Approval
            );
            assert_eq!(permission_requirement(PermissionMode::Full, action), Allow);
        }
        assert_eq!(PermissionMode::parse(Some("unknown")), PermissionMode::Ask);
    }

    #[test]
    fn full_permission_grant_is_run_bound_and_single_use() {
        let (state, database_path) = test_state();
        let session = state
            .sessions
            .lock()
            .values()
            .next()
            .cloned()
            .expect("test state has a session");
        let workdir = canonical_workdir(&session.summary.cwd).expect("test workdir is valid");
        let session_id = session.summary.id;
        let conversation_id = session_id.clone();
        let request_id = "full-permission-request".to_string();
        let text = "make this exact approved change";
        let run_binding = normalize_full_permission_run_binding(
            Some("embedded"),
            Some("test-model"),
            Some("high"),
        )
        .expect("test run binding should be valid");

        // Renderer-writable preference data is intentionally insufficient to
        // mint Full. Only the private native grant map is consulted.
        state.settings.lock().insert(
            "system".to_string(),
            json!({
                "fullPermissionConfirmedRoots": {
                    "forged": { "root": path_for_display(&workdir) }
                }
            }),
        );
        let missing = consume_full_permission_grant(
            &state,
            None,
            &session_id,
            &conversation_id,
            &request_id,
            &workdir,
            text,
            &run_binding,
        )
        .expect_err("settings data must not authorize Full");
        assert_eq!(missing, FULL_PERMISSION_GRANT_REQUIRED);

        let token = "full-permission-test-token";
        state.full_permission_grants.lock().insert(
            token.to_string(),
            FullPermissionRunGrant {
                session_id: session_id.clone(),
                conversation_id: conversation_id.clone(),
                request_id: request_id.clone(),
                workdir: workdir.clone(),
                prompt_digest: full_permission_prompt_digest(text),
                run_binding: run_binding.clone(),
                expires_at: Instant::now() + Duration::from_secs(60),
            },
        );
        consume_full_permission_grant(
            &state,
            Some(token),
            &session_id,
            &conversation_id,
            &request_id,
            &workdir,
            text,
            &run_binding,
        )
        .expect("the exact native grant should authorize one run");
        assert!(
            !state.full_permission_grants.lock().contains_key(token),
            "a consumed grant must not be replayable"
        );
        assert_eq!(
            consume_full_permission_grant(
                &state,
                Some(token),
                &session_id,
                &conversation_id,
                &request_id,
                &workdir,
                text,
                &run_binding,
            )
            .expect_err("a consumed grant must be rejected"),
            FULL_PERMISSION_GRANT_REQUIRED
        );
        cleanup_test_db(&database_path);
    }

    #[test]
    fn full_permission_grant_consumes_a_mismatched_run_token() {
        let (state, database_path) = test_state();
        let session = state
            .sessions
            .lock()
            .values()
            .next()
            .cloned()
            .expect("test state has a session");
        let workdir = canonical_workdir(&session.summary.cwd).expect("test workdir is valid");
        let session_id = session.summary.id;
        let token = "full-permission-mismatch-token";
        let run_binding = normalize_full_permission_run_binding(
            Some("embedded"),
            Some("approved-model"),
            Some("medium"),
        )
        .expect("test run binding should be valid");
        state.full_permission_grants.lock().insert(
            token.to_string(),
            FullPermissionRunGrant {
                session_id: session_id.clone(),
                conversation_id: session_id.clone(),
                request_id: "approved-request".to_string(),
                workdir: workdir.clone(),
                prompt_digest: full_permission_prompt_digest("approved prompt"),
                run_binding: run_binding.clone(),
                expires_at: Instant::now() + Duration::from_secs(60),
            },
        );

        assert_eq!(
            consume_full_permission_grant(
                &state,
                Some(token),
                &session_id,
                &session_id,
                "approved-request",
                &workdir,
                "substituted prompt",
                &run_binding,
            )
            .expect_err("a token cannot authorize a substituted prompt"),
            FULL_PERMISSION_GRANT_REQUIRED
        );
        assert!(
            !state.full_permission_grants.lock().contains_key(token),
            "a failed binding comparison must still consume the token"
        );

        let model_token = "full-permission-model-mismatch-token";
        state.full_permission_grants.lock().insert(
            model_token.to_string(),
            FullPermissionRunGrant {
                session_id: session_id.clone(),
                conversation_id: session_id.clone(),
                request_id: "approved-model-request".to_string(),
                workdir: workdir.clone(),
                prompt_digest: full_permission_prompt_digest("approved prompt"),
                run_binding: run_binding.clone(),
                expires_at: Instant::now() + Duration::from_secs(60),
            },
        );
        let substituted_binding = normalize_full_permission_run_binding(
            Some("embedded"),
            Some("substituted-model"),
            Some("medium"),
        )
        .expect("substituted test binding should still be syntactically valid");
        assert_eq!(
            consume_full_permission_grant(
                &state,
                Some(model_token),
                &session_id,
                &session_id,
                "approved-model-request",
                &workdir,
                "approved prompt",
                &substituted_binding,
            )
            .expect_err("a token cannot authorize a substituted model"),
            FULL_PERMISSION_GRANT_REQUIRED
        );
        assert!(
            !state
                .full_permission_grants
                .lock()
                .contains_key(model_token),
            "a model mismatch must still consume the one-use token"
        );

        let expired_token = "full-permission-expired-token";
        state.full_permission_grants.lock().insert(
            expired_token.to_string(),
            FullPermissionRunGrant {
                session_id: session_id.clone(),
                conversation_id: session_id.clone(),
                request_id: "expired-request".to_string(),
                workdir: workdir.clone(),
                prompt_digest: full_permission_prompt_digest("expired prompt"),
                run_binding: run_binding.clone(),
                expires_at: Instant::now() - Duration::from_secs(1),
            },
        );
        assert_eq!(
            consume_full_permission_grant(
                &state,
                Some(expired_token),
                &session_id,
                &session_id,
                "expired-request",
                &workdir,
                "expired prompt",
                &run_binding,
            )
            .expect_err("an expired token must be rejected"),
            FULL_PERMISSION_GRANT_REQUIRED
        );
        assert!(
            !state
                .full_permission_grants
                .lock()
                .contains_key(expired_token),
            "expired grants must be pruned before they could be replayed"
        );
        cleanup_test_db(&database_path);
    }

    #[test]
    fn full_permission_grant_validates_prompt_payload() {
        let bounded_prompt = "界".repeat(MAX_FULL_PERMISSION_PROMPT_CHARS);
        validate_full_permission_prompt(&bounded_prompt)
            .expect("a bounded prompt should be accepted");

        let too_long = "x".repeat(MAX_FULL_PERMISSION_PROMPT_CHARS + 1);
        assert!(validate_full_permission_prompt(&too_long)
            .expect_err("an oversized prompt must not receive a grant")
            .contains("at most"));

        for hidden in [
            '\u{0000}',
            '\u{00ad}',
            '\u{061c}',
            '\u{200b}',
            '\u{202e}',
            '\u{2066}',
            '\u{feff}',
            '\u{e0061}',
        ] {
            let prompt = format!("visible prefix{hidden}hidden suffix");
            assert!(
                validate_full_permission_prompt(&prompt).is_err(),
                "U+{:04X} must not be accepted in a Full grant",
                hidden as u32
            );
        }
        validate_full_permission_prompt("普通文本\n第二行\t值")
            .expect("ordinary visible text and layout whitespace should remain usable");
        assert!(normalize_full_permission_run_binding(
            Some("embedded"),
            Some("model\u{202e}hidden"),
            Some("high"),
        )
        .is_err());
    }

    #[test]
    fn sensitive_workspace_paths_require_stricter_read_permissions() {
        use PermissionRequirement::{Allow, Approval, Deny};

        assert!(is_sensitive_workspace_path(Path::new(".env")));
        assert!(is_sensitive_workspace_path(Path::new(
            "nested/.env.production"
        )));
        assert!(is_sensitive_workspace_path(Path::new(
            "credentials-prod.json"
        )));
        assert!(is_sensitive_workspace_path(Path::new("nested/.ssh/id_rsa")));
        for path in [
            ".codex/auth.json",
            "nested/.claude/credentials.json",
            ".grok/auth.json",
            ".docker/config.json",
            ".kube/config",
            ".gnupg/private-keys-v1.d/key",
            ".netrc",
            "nested/.git-credentials",
        ] {
            assert!(
                is_sensitive_workspace_path(Path::new(path)),
                "expected {path} to require sensitive-read approval"
            );
        }
        assert!(!is_sensitive_workspace_path(Path::new("src/main.rs")));

        assert_eq!(
            sensitive_read_permission_requirement(PermissionMode::Readonly),
            Deny
        );
        assert_eq!(
            sensitive_read_permission_requirement(PermissionMode::Ask),
            Approval
        );
        assert_eq!(
            sensitive_read_permission_requirement(PermissionMode::Full),
            Allow
        );
    }

    #[test]
    fn global_read_requires_an_absolute_regular_file_path() {
        let root = std::env::temp_dir().join(format!(
            "novavei-global-read-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).unwrap();
        let readable_file = root.join("readable.txt");
        fs::write(&readable_file, "readable").unwrap();
        let target = canonical_global_read_target(&path_for_display(&readable_file))
            .expect("an existing absolute regular file should be accepted");
        assert_eq!(target, fs::canonicalize(&readable_file).unwrap());
        assert!(canonical_global_read_target("relative.txt").is_err());
        assert!(canonical_global_read_target(&path_for_display(&root)).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sensitive_reads_ignore_always_allow_read_and_require_one_use_approval() {
        let workdir = std::env::temp_dir().join(format!(
            "novavei-sensitive-read-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(workdir.join("nested")).unwrap();
        fs::write(
            workdir.join("nested").join(".env.production"),
            "TOKEN=secret",
        )
        .unwrap();
        let canonical_workdir = fs::canonicalize(&workdir).unwrap();

        let (state, database_path) = test_state();
        let session_id = state
            .sessions
            .lock()
            .keys()
            .next()
            .cloned()
            .expect("test state has a session");
        state
            .sessions
            .lock()
            .get_mut(&session_id)
            .expect("test session exists")
            .summary
            .cwd = path_for_display(&canonical_workdir);
        state
            .approved_workdirs
            .lock()
            .insert(canonical_workdir.clone());

        let capability_token = "native-sensitive-read-test".to_string();
        let cancelled = Arc::new(AtomicBool::new(false));
        state.capabilities.lock().insert(
            capability_token.clone(),
            CapabilityGrant {
                session_id: session_id.clone(),
                conversation_id: "conversation-sensitive-read".to_string(),
                turn_id: "turn-sensitive-read".to_string(),
                request_id: "request-sensitive-read".to_string(),
                workdir: canonical_workdir.clone(),
                permission_mode: PermissionMode::Ask,
                expires_at: Instant::now() + Duration::from_secs(60),
                cancelled: cancelled.clone(),
            },
        );
        state.active_runs.lock().insert(
            "request-sensitive-read".to_string(),
            ActiveRun {
                session_id,
                conversation_id: "conversation-sensitive-read".to_string(),
                turn_id: "turn-sensitive-read".to_string(),
                request_id: "request-sensitive-read".to_string(),
                proxy_provider_id: None,
                capability_token: capability_token.clone(),
                cancelled,
            },
        );
        state.tool_approvals.lock().insert(
            "sensitive-read-approval".to_string(),
            ToolApprovalGrant {
                capability_token: capability_token.clone(),
                action: ToolAction::Read,
                tool_name: "Read".to_string(),
                subagent_global_read: false,
                expires_at: Instant::now() + Duration::from_secs(60),
            },
        );

        let workdir_display = path_for_display(&canonical_workdir);
        assert!(
            require_sensitive_read_capability(
                &state,
                Some(&capability_token),
                &workdir_display,
                "Read",
                None,
            )
            .is_err(),
            "always-allow Read must not authorize sensitive workspace files"
        );
        assert!(require_sensitive_read_capability(
            &state,
            Some(&capability_token),
            &workdir_display,
            "Read",
            Some("sensitive-read-approval"),
        )
        .is_ok());
        assert!(
            require_sensitive_read_capability(
                &state,
                Some(&capability_token),
                &workdir_display,
                "Read",
                Some("sensitive-read-approval"),
            )
            .is_err(),
            "sensitive read approval is one-use"
        );

        for (expected_tool_name, approved_tool_name) in
            [("Read", "List"), ("List", "Read"), ("Grep", "Read")]
        {
            let approval_id =
                format!("sensitive-read-{expected_tool_name}-from-{approved_tool_name}");
            state.tool_approvals.lock().insert(
                approval_id.clone(),
                ToolApprovalGrant {
                    capability_token: capability_token.clone(),
                    action: ToolAction::Read,
                    tool_name: approved_tool_name.to_string(),
                    subagent_global_read: false,
                    expires_at: Instant::now() + Duration::from_secs(60),
                },
            );
            assert!(
                require_sensitive_read_capability(
                    &state,
                    Some(&capability_token),
                    &workdir_display,
                    expected_tool_name,
                    Some(&approval_id),
                )
                .is_err(),
                "a {approved_tool_name} approval must not be replayed by {expected_tool_name}"
            );
        }

        state
            .capabilities
            .lock()
            .get_mut(&capability_token)
            .expect("test capability exists")
            .permission_mode = PermissionMode::Readonly;
        assert!(require_sensitive_read_capability(
            &state,
            Some(&capability_token),
            &workdir_display,
            "Read",
            Some("unused"),
        )
        .is_err());

        state
            .capabilities
            .lock()
            .get_mut(&capability_token)
            .expect("test capability exists")
            .permission_mode = PermissionMode::Full;
        assert!(require_sensitive_read_capability(
            &state,
            Some(&capability_token),
            &workdir_display,
            "Read",
            None,
        )
        .is_ok());

        cleanup_test_db(&database_path);
        let _ = fs::remove_dir_all(&workdir);
    }

    #[test]
    fn grep_skips_sensitive_files_during_workspace_traversal() {
        let workdir = std::env::temp_dir().join(format!(
            "novavei-sensitive-grep-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&workdir).unwrap();
        fs::create_dir_all(workdir.join(".codex")).unwrap();
        fs::write(workdir.join("notes.txt"), "workspace-search-marker\n").unwrap();
        fs::write(workdir.join(".env"), "workspace-search-marker=secret\n").unwrap();
        fs::write(
            workdir.join(".codex").join("auth.json"),
            "workspace-search-marker=secret\n",
        )
        .unwrap();

        let result = fs_grep_sync(
            path_for_display(&workdir),
            None,
            "workspace-search-marker".to_string(),
            None,
            None,
            Some("files".to_string()),
            None,
            None,
            None,
            None,
            None,
            false,
            false,
        )
        .unwrap();

        assert_eq!(result.file_count, 1);
        assert_eq!(result.files[0].path, "notes.txt");
        let _ = fs::remove_dir_all(&workdir);
    }

    #[test]
    fn worktree_child_git_metadata_boundary_blocks_top_level_gitfile_paths() {
        let (state, database_path) = test_state();
        let session = state.sessions.lock().values().next().unwrap().clone();
        let workdir = canonical_workdir(&session.summary.cwd).unwrap();
        state.persist_locked().unwrap();
        let task_id = "worktree-git-metadata-test".to_string();
        let token = "worktree-git-metadata-capability";
        let cancelled = Arc::new(AtomicBool::new(false));
        let run = ActiveRun {
            session_id: session.summary.id.clone(),
            conversation_id: "worktree-git-metadata-conversation".to_string(),
            turn_id: "worktree-git-metadata-turn".to_string(),
            request_id: "worktree-git-metadata-request".to_string(),
            proxy_provider_id: None,
            capability_token: "parent-worktree-git-metadata-capability".to_string(),
            cancelled,
        };
        state
            .active_runs
            .lock()
            .insert(run.request_id.clone(), run.clone());
        state
            .subagent_tasks
            .create_task(NewSubagentTask {
                id: task_id.clone(),
                session_id: run.session_id.clone(),
                agent_id: "worktree-test-agent".to_string(),
                parent_turn_id: run.turn_id.clone(),
                parent_request_id: run.request_id.clone(),
                title: "Git metadata boundary".to_string(),
                created_at: now_ms(),
            })
            .unwrap();
        state
            .subagent_tasks
            .mark_running(&task_id, now_ms())
            .unwrap();
        state.subagent_capabilities.lock().insert(
            token.to_string(),
            SubagentCapabilityGrant {
                task_id,
                session_id: run.session_id,
                parent_turn_id: run.turn_id,
                parent_request_id: run.request_id,
                proxy_request_id: "worktree-git-metadata-proxy".to_string(),
                workdir: workdir.clone(),
                mode: SubagentCapabilityMode::Worktree,
                allow_global_read: false,
                expires_at: Instant::now() + Duration::from_secs(60),
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        );

        for raw_path in [".git", ".git/index", "./.GiT/config"] {
            let relative = relative_workspace_path(raw_path).unwrap();
            assert!(
                reject_worktree_child_git_metadata_access(
                    &state,
                    Some(token),
                    &workdir,
                    &relative,
                )
                .is_err(),
                "worktree child unexpectedly received {raw_path}"
            );
        }
        assert!(reject_worktree_child_git_metadata_access(
            &state,
            Some(token),
            &workdir,
            &relative_workspace_path("src/main.rs").unwrap(),
        )
        .is_ok());
        assert!(!is_top_level_git_metadata_path(Path::new(
            "nested/.git/config"
        )));

        cleanup_test_db(&database_path);
    }

    #[test]
    fn grep_hides_a_worktree_gitfile_even_when_hidden_entries_are_requested() {
        let workdir = std::env::temp_dir().join(format!(
            "novavei-worktree-gitfile-grep-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&workdir).unwrap();
        fs::write(workdir.join(".git"), "gitdir: private-index\n").unwrap();
        fs::write(workdir.join("visible.txt"), "worktree-gitfile-marker\n").unwrap();

        let result = fs_grep_sync(
            path_for_display(&workdir),
            None,
            "worktree-gitfile-marker|private-index".to_string(),
            None,
            None,
            Some("files".to_string()),
            None,
            None,
            None,
            None,
            Some(true),
            false,
            true,
        )
        .unwrap();
        assert_eq!(result.file_count, 1);
        assert_eq!(result.files[0].path, "visible.txt");
        let _ = fs::remove_dir_all(&workdir);
    }

    #[test]
    fn grep_hidden_entries_follow_include_hidden_without_bypassing_sensitive_filters() {
        let workdir = std::env::temp_dir().join(format!(
            "novavei-hidden-grep-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(workdir.join(".vscode")).unwrap();
        fs::write(workdir.join("visible.txt"), "hidden-search-marker\n").unwrap();
        fs::write(workdir.join(".editorconfig"), "hidden-search-marker\n").unwrap();
        fs::write(
            workdir.join(".vscode").join("settings.json"),
            "hidden-search-marker\n",
        )
        .unwrap();
        fs::write(workdir.join(".env"), "hidden-search-marker=secret\n").unwrap();

        let default_result = fs_grep_sync(
            path_for_display(&workdir),
            None,
            "hidden-search-marker".to_string(),
            None,
            None,
            Some("files".to_string()),
            None,
            None,
            None,
            None,
            None,
            false,
            false,
        )
        .unwrap();
        let default_paths = default_result
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            default_paths,
            vec![".editorconfig", ".vscode/settings.json", "visible.txt"]
        );

        let dock_result = fs_grep_sync(
            path_for_display(&workdir),
            None,
            "hidden-search-marker".to_string(),
            None,
            None,
            Some("files".to_string()),
            None,
            None,
            None,
            None,
            Some(false),
            false,
            false,
        )
        .unwrap();
        assert_eq!(dock_result.file_count, 1);
        assert_eq!(dock_result.files[0].path, "visible.txt");

        let explicit_hidden_target = fs_grep_sync(
            path_for_display(&workdir),
            Some(".vscode".to_string()),
            "hidden-search-marker".to_string(),
            None,
            None,
            Some("files".to_string()),
            None,
            None,
            None,
            None,
            Some(false),
            false,
            false,
        )
        .unwrap();
        assert_eq!(explicit_hidden_target.file_count, 1);
        assert_eq!(
            explicit_hidden_target.files[0].path,
            ".vscode/settings.json"
        );

        let _ = fs::remove_dir_all(&workdir);
    }

    #[test]
    fn startup_storage_errors_disable_initial_snapshot_persistence() {
        assert_eq!(
            startup_persistence_decision(false, true),
            StartupPersistenceDecision::SkipForStorageRecovery
        );
        assert_eq!(
            startup_persistence_decision(false, false),
            StartupPersistenceDecision::SkipForStorageRecovery
        );
        assert_eq!(
            startup_persistence_decision(true, false),
            StartupPersistenceDecision::SkipForSettingsRecovery
        );
        assert_eq!(
            startup_persistence_decision(true, true),
            StartupPersistenceDecision::Persist
        );
    }

    #[test]
    fn initial_persist_failure_blocks_writes_before_health_is_published() {
        let (mut state, path) = test_state();
        state.storage_recovery = initial_persist_failure_recovery_status();

        let status = state.storage_recovery_status();
        assert_eq!(status.state, "degraded");
        assert_eq!(status.errors, vec!["local_storage_unavailable"]);
        assert_eq!(
            state.app_health(true).session_store,
            AppHealthSessionStore::RecoveryRequired
        );
        assert_eq!(state.app_health(true).writes, AppHealthWrites::Blocked);
        assert!(state.require_persistence_ready().is_err());

        cleanup_test_db(&path);
    }

    #[test]
    fn history_upsert_restores_memory_when_snapshot_persistence_fails() {
        let (mut state, path) = test_state();
        let previous = state
            .sessions
            .lock()
            .values()
            .next()
            .cloned()
            .expect("test state has a session");
        state
            .persist_locked()
            .expect("baseline session should persist");
        let durable_history = state.history.clone();
        let failed_parent = std::env::temp_dir().join(format!(
            "novavei-history-upsert-failure-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        assert!(!failed_parent.exists(), "test failure path must not exist");
        state.history = HistoryStore::new(failed_parent.join("history.sqlite3"));
        let app = tauri::test::mock_builder()
            .manage(Arc::new(state))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("test app should build");
        let state = app.state::<Arc<AppState>>();

        let error = chat_history_upsert(
            state.clone(),
            json!({
                "id": previous.summary.id.clone(),
                "title": "must not become a ghost session",
                "cwd": previous.summary.cwd.clone(),
                "updatedAt": previous.summary.updated_at + 1,
            }),
        )
        .expect_err("history upsert must fail when its snapshot cannot persist");
        assert!(!error.is_empty());
        let restored = state
            .sessions
            .lock()
            .get(&previous.summary.id)
            .cloned()
            .expect("failed upsert must retain the original session");
        assert_eq!(restored.summary.title, previous.summary.title);
        assert_eq!(restored.summary.updated_at, previous.summary.updated_at);
        assert_eq!(state.sessions.lock().len(), 1);
        let durable = durable_history
            .load_sessions()
            .expect("baseline database remains readable");
        assert_eq!(durable.len(), 1);
        assert_eq!(durable[0].id, previous.summary.id);
        assert_eq!(durable[0].title, previous.summary.title);

        cleanup_test_db(&path);
    }

    #[test]
    fn degraded_storage_blocks_session_creation_before_mutating_memory() {
        let (mut state, path) = test_state();
        state.storage_recovery = StorageRecoveryStatus::degraded(vec![
            "initialize history store: simulated recovery failure".to_string(),
        ]);
        let app = tauri::test::mock_builder()
            .manage(Arc::new(state))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("test app should build");
        let state = app.state::<Arc<AppState>>();
        let before = sessions_list(state.clone())
            .into_iter()
            .map(|session| session.id)
            .collect::<HashSet<_>>();

        let status = storage_recovery_status(state.clone());
        assert_eq!(status.state, "degraded");
        assert!(!status.errors.is_empty());
        let error = sessions_create(state.clone(), Some("must not persist".to_string()), None)
            .expect_err("degraded storage must reject new sessions");
        assert!(error.contains("storage recovery is required"));

        let after = sessions_list(state)
            .into_iter()
            .map(|session| session.id)
            .collect::<HashSet<_>>();
        assert_eq!(after, before);
        cleanup_test_db(&path);
    }

    #[test]
    fn degraded_storage_refuses_renderer_workspace_capability_minting() {
        let (mut state, path) = test_state();
        let session = state
            .sessions
            .lock()
            .values()
            .next()
            .cloned()
            .expect("test state has a session");
        state.storage_recovery = StorageRecoveryStatus::degraded(vec![
            "initialize history store: simulated recovery failure".to_string(),
        ]);
        let app = tauri::test::mock_builder()
            .manage(Arc::new(state))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("test app should build");
        let state = app.state::<Arc<AppState>>();

        let workspace_error = workspace_capability_issue(
            state,
            session.summary.cwd.clone(),
            session.summary.id.clone(),
        )
        .expect_err("degraded storage must not mint a File Dock capability");
        assert!(workspace_error.contains("storage recovery is required"));

        cleanup_test_db(&path);
    }

    #[test]
    fn degraded_storage_blocks_agent_runtime_mutations_before_memory_changes() {
        let (mut state, path) = test_state();
        let session = state
            .sessions
            .lock()
            .values()
            .next()
            .cloned()
            .expect("test state has a session");
        let request_id = "degraded-agent-run".to_string();
        let cancelled = Arc::new(AtomicBool::new(false));
        state.active_runs.lock().insert(
            request_id.clone(),
            ActiveRun {
                session_id: session.summary.id.clone(),
                conversation_id: session.summary.id.clone(),
                turn_id: "degraded-agent-turn".to_string(),
                request_id: request_id.clone(),
                proxy_provider_id: None,
                capability_token: "degraded-agent-capability".to_string(),
                cancelled: cancelled.clone(),
            },
        );
        state.storage_recovery = StorageRecoveryStatus::degraded(vec![
            "initialize history store: simulated recovery failure".to_string(),
        ]);
        let app = tauri::test::mock_builder()
            .manage(Arc::new(state))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("test app should build");
        let state = app.state::<Arc<AppState>>();
        let handle = app.handle().clone();

        let cancellation = agent_cancel_inner(
            state.clone(),
            handle.clone(),
            Some(session.summary.id.clone()),
            Some(session.summary.id.clone()),
            Some("degraded-agent-turn".to_string()),
            Some(request_id.clone()),
        )
        .expect_err("degraded storage must reject cancellation before it changes a run");
        assert!(cancellation.contains("storage recovery is required"));
        assert!(!cancelled.load(Ordering::SeqCst));
        assert!(state.active_runs.lock().contains_key(&request_id));

        let event_error = agent_emit_event_inner(
            state.clone(),
            handle.clone(),
            json!({"type": "permission_requested"}),
        )
        .expect_err("degraded storage must reject agent events before they mutate permissions");
        assert!(event_error.contains("storage recovery is required"));
        assert!(state.pending_permissions.lock().is_empty());

        let permission_error = agent_permission_inner(
            state.clone(),
            handle,
            "missing-permission".to_string(),
            "allow".to_string(),
        )
        .expect_err("degraded storage must reject permission decisions before resolving them");
        assert!(permission_error.contains("storage recovery is required"));
        assert!(state.tool_approvals.lock().is_empty());
        cleanup_test_db(&path);
    }

    #[test]
    fn permission_decision_persistence_failure_restores_pending_request_without_granting_access() {
        let (mut state, path) = test_state();
        let session = state
            .sessions
            .lock()
            .values()
            .next()
            .cloned()
            .expect("test state has a session");
        let capability_token = "permission-persist-capability".to_string();
        let permission_id = "permission-persist-failure".to_string();
        let run_id = "permission-persist-run".to_string();
        let cancelled = Arc::new(AtomicBool::new(false));
        let workdir = canonical_workdir(&session.summary.cwd).expect("test workdir is available");
        state.capabilities.lock().insert(
            capability_token.clone(),
            CapabilityGrant {
                session_id: session.summary.id.clone(),
                conversation_id: session.summary.id.clone(),
                turn_id: "permission-persist-turn".to_string(),
                request_id: run_id.clone(),
                workdir,
                permission_mode: PermissionMode::Ask,
                expires_at: Instant::now() + Duration::from_secs(60),
                cancelled: cancelled.clone(),
            },
        );
        state.active_runs.lock().insert(
            run_id.clone(),
            ActiveRun {
                session_id: session.summary.id.clone(),
                conversation_id: session.summary.id.clone(),
                turn_id: "permission-persist-turn".to_string(),
                request_id: run_id.clone(),
                proxy_provider_id: None,
                capability_token: capability_token.clone(),
                cancelled,
            },
        );
        state.pending_permissions.lock().insert(
            permission_id.clone(),
            PendingPermission {
                capability_token: capability_token.clone(),
                session_id: session.summary.id.clone(),
                conversation_id: session.summary.id.clone(),
                turn_id: "permission-persist-turn".to_string(),
                request_id: run_id,
                tool_name: "Write".to_string(),
                subagent_global_read: false,
            },
        );
        let failed_parent = std::env::temp_dir().join(format!(
            "novavei-permission-save-failure-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        assert!(!failed_parent.exists(), "test failure path must not exist");
        state.history = HistoryStore::new(failed_parent.join("history.sqlite3"));
        let app = tauri::test::mock_builder()
            .manage(Arc::new(state))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("test app should build");
        let state = app.state::<Arc<AppState>>();

        let error = agent_permission_inner(
            state.clone(),
            app.handle().clone(),
            permission_id.clone(),
            "allow".to_string(),
        )
        .expect_err("permission decision must not be accepted when durable storage fails");
        assert!(error.contains("could not persist the permission decision"));
        assert!(state
            .pending_permissions
            .lock()
            .contains_key(&permission_id));
        assert!(state.tool_approvals.lock().is_empty());

        cleanup_test_db(&path);
    }

    #[test]
    fn mcp_tools_are_high_risk_and_keep_a_stable_permission_name() {
        use PermissionRequirement::{Allow, Approval, Deny};

        assert_eq!(
            tool_action("mcp__local-tools__search"),
            Some(ToolAction::Mcp)
        );
        assert_eq!(tool_action("mcp__"), None);
        assert_eq!(
            permission_requirement(PermissionMode::Readonly, ToolAction::Mcp),
            Deny
        );
        assert_eq!(
            permission_requirement(PermissionMode::Ask, ToolAction::Mcp),
            Approval
        );
        assert_eq!(
            permission_requirement(PermissionMode::Full, ToolAction::Mcp),
            Allow
        );
        assert_eq!(
            mcp_permission_tool_name("local-tools", "search").unwrap(),
            "mcp__local-tools__search"
        );
    }

    #[test]
    fn native_mcp_config_uses_keyed_server_and_normalizes_header_entries() {
        let (state, path) = test_state();
        state.settings.lock().insert(
            "mcp".to_string(),
            json!({
                "mcpServers": {
                    "local-tools": {
                        "transport": "http",
                        "endpoint": "http://127.0.0.1:43123/mcp",
                        "customHeaders": [{"key": "Authorization", "value": "native-secret"}],
                        "env": {"TOKEN": "env-secret"}
                    }
                }
            }),
        );

        let config = native_mcp_server_config(&state, "local-tools").unwrap();
        assert_eq!(config.id, "local-tools");
        assert!(config.enabled);
        assert_eq!(config.url.as_deref(), Some("http://127.0.0.1:43123/mcp"));
        assert_eq!(
            config
                .headers
                .as_ref()
                .and_then(|headers| headers.get("Authorization"))
                .map(String::as_str),
            Some("native-secret")
        );
        assert!(native_mcp_server_config(&state, "not-configured").is_err());
        cleanup_test_db(&path);
    }

    #[test]
    fn mcp_settings_save_canonicalizes_legacy_shapes_and_drops_redacted_secrets_when_endpoint_changes(
    ) {
        let existing = json!({
            "mcpServers": {
                "local-tools": {
                    "transport": "http",
                    "endpoint": "http://127.0.0.1:43123/old",
                    "headers": {"Authorization": "native-secret"}
                }
            }
        });
        let saved = merge_validate_and_canonicalize_mcp_settings(
            Some(&existing),
            json!([{
                "id": " local-tools ",
                "connectionType": "streamable-http",
                "endpoint": "http://127.0.0.1:43123/new",
                "customHeaders": [{"name": "authorization", "value": "[configured]"}],
                "timeout_ms": 1_000
            }]),
        )
        .unwrap();

        assert_eq!(
            saved,
            json!({
                "schemaVersion": 1,
                "servers": [{
                    "id": "local-tools",
                    "enabled": true,
                    "transport": "http",
                    "command": "",
                    "args": [],
                    "env": null,
                    "cwd": null,
                    "url": "http://127.0.0.1:43123/new",
                    "headers": null,
                    "timeoutMs": 1_000,
                    "messageUrl": null,
                    "allowRemote": false,
                    "stdioFraming": null
                }]
            })
        );
        assert!(!saved.to_string().contains("native-secret"));
    }

    #[test]
    fn mcp_settings_save_drops_redacted_secrets_when_execution_binding_changes() {
        let stdio_existing = json!({
            "servers": [{
                "id": "stdio-tools",
                "command": "tool-server",
                "args": ["--serve"],
                "cwd": "C:/tools",
                "stdioFraming": "jsonl",
                "env": {"TOKEN": "stdio-secret"}
            }]
        });
        for incoming in [
            json!({
                "id": "stdio-tools",
                "command": "other-server",
                "args": ["--serve"],
                "cwd": "C:/tools",
                "stdioFraming": "jsonl",
                "env": {"TOKEN": "[configured]"}
            }),
            json!({
                "id": "stdio-tools",
                "command": "tool-server",
                "args": ["--other"],
                "cwd": "C:/tools",
                "stdioFraming": "jsonl",
                "env": {"TOKEN": "[configured]"}
            }),
            json!({
                "id": "stdio-tools",
                "command": "tool-server",
                "args": ["--serve"],
                "cwd": "C:/other-tools",
                "stdioFraming": "jsonl",
                "env": {"TOKEN": "[configured]"}
            }),
            json!({
                "id": "stdio-tools",
                "command": "tool-server",
                "args": ["--serve"],
                "cwd": "C:/tools",
                "stdioFraming": "content-length",
                "env": {"TOKEN": "[configured]"}
            }),
        ] {
            let saved = merge_validate_and_canonicalize_mcp_settings(
                Some(&stdio_existing),
                json!({"servers": [incoming]}),
            )
            .unwrap();
            assert!(saved["servers"][0]["env"].is_null());
            assert!(!saved.to_string().contains("stdio-secret"));
        }

        let http_existing = json!({
            "servers": [{
                "id": "transport-tools",
                "transport": "http",
                "url": "http://127.0.0.1:43123/mcp",
                "headers": {"Authorization": "http-secret"}
            }]
        });
        let changed_transport = merge_validate_and_canonicalize_mcp_settings(
            Some(&http_existing),
            json!({
                "servers": [{
                    "id": "transport-tools",
                    "transport": "sse",
                    "url": "http://127.0.0.1:43123/mcp",
                    "headers": {"authorization": "[configured]"}
                }]
            }),
        )
        .unwrap();
        assert!(changed_transport["servers"][0]["headers"].is_null());
        assert!(!changed_transport.to_string().contains("http-secret"));
    }

    #[test]
    fn mcp_settings_save_rejects_duplicate_or_runtime_invalid_configurations() {
        let duplicate = merge_validate_and_canonicalize_mcp_settings(
            None,
            json!([
                {"id": "tools", "command": "first"},
                {"serverId": " tools ", "command": "second"}
            ]),
        )
        .unwrap_err();
        assert_eq!(duplicate, "MCP settings contain duplicate server ids");

        assert!(merge_validate_and_canonicalize_mcp_settings(
            None,
            json!([{
                "id": "remote-tools",
                "transport": "http",
                "url": "https://example.com/mcp"
            }]),
        )
        .is_err());

        assert!(merge_validate_and_canonicalize_mcp_settings(
            None,
            json!([{
                "id": "headers",
                "transport": "http",
                "url": "http://127.0.0.1:43123/mcp",
                "headers": {"X-Token": "first", "x-token": "second"}
            }]),
        )
        .is_err());

        assert!(merge_validate_and_canonicalize_mcp_settings(
            None,
            json!([{"id": "timeout", "command": "tool-server", "timeoutMs": 0}]),
        )
        .is_err());

        assert!(merge_validate_and_canonicalize_mcp_settings(
            None,
            json!([{"id": "command", "command": "tool\u{0}"}]),
        )
        .is_err());

        let conflicting_aliases = merge_validate_and_canonicalize_mcp_settings(
            None,
            json!([{
                "id": "conflicting-remote",
                "transport": "http",
                "connectionType": "streamable-http",
                "url": "http://127.0.0.1:43123/old",
                "endpoint": "http://127.0.0.1:43123/new"
            }]),
        )
        .unwrap_err();
        assert_eq!(conflicting_aliases, "MCP url aliases disagree");

        let mixed_record_and_collection = merge_validate_and_canonicalize_mcp_settings(
            None,
            json!({
                "id": "tools",
                "command": "tool-server",
                "servers": [{"id": "tools", "command": "other-server"}]
            }),
        )
        .unwrap_err();
        assert_eq!(
            mixed_record_and_collection,
            "MCP settings cannot mix a server record with a server collection"
        );

        let conflicting_header_entry = merge_validate_and_canonicalize_mcp_settings(
            None,
            json!([{
                "id": "header-aliases",
                "transport": "http",
                "url": "http://127.0.0.1:43123/mcp",
                "headers": [{"key": "X-Primary", "name": "X-Secondary", "value": "value"}]
            }]),
        )
        .unwrap_err();
        assert_eq!(conflicting_header_entry, "MCP header aliases disagree");

        let conflicting_map_key = merge_validate_and_canonicalize_mcp_settings(
            None,
            json!({
                "mcp_servers": {
                    "local-tools": {"id": "other-tools", "command": "tool-server"}
                }
            }),
        )
        .unwrap_err();
        assert_eq!(
            conflicting_map_key,
            "MCP server id does not match its collection key"
        );
    }

    #[test]
    fn mcp_settings_save_reorders_and_preserves_configured_secrets_when_binding_matches() {
        let existing = json!({
            "schemaVersion": 1,
            "servers": [
                {
                    "id": "alpha",
                    "command": "alpha-server",
                    "env": {"TOKEN": "alpha-secret"}
                },
                {
                    "id": "beta",
                    "transport": "http",
                    "url": "http://127.0.0.1:43123/beta",
                    "headers": {"Authorization": "beta-secret"}
                }
            ]
        });
        let saved = merge_validate_and_canonicalize_mcp_settings(
            Some(&existing),
            json!({
                "items": [
                    {
                        "server_id": " beta ",
                        "transport": "http",
                        "url": "http://127.0.0.1:43123/beta",
                        "headers": {"authorization": "[configured]"},
                        "timeout_ms": 2_000
                    },
                    {
                        "id": "new",
                        "command": "new-server",
                        "env": {"TOKEN": "[configured]"}
                    }
                ]
            }),
        )
        .unwrap();

        let servers = saved["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0]["id"], "beta");
        assert_eq!(servers[0]["headers"]["authorization"], "beta-secret");
        assert_eq!(servers[0]["timeoutMs"], 2_000);
        assert_eq!(servers[1]["id"], "new");
        assert!(servers[1]["env"].is_null());
        let rendered = saved.to_string();
        assert!(!rendered.contains("alpha-secret"));
        assert!(!rendered.contains("[configured]"));
    }

    #[test]
    fn mcp_settings_save_returns_safe_validation_errors() {
        let error = merge_validate_and_canonicalize_mcp_settings(
            None,
            json!({
                "servers": [{
                    "id": "remote-tools",
                    "transport": "http",
                    "url": "https://private.example.invalid/mcp",
                    "headers": {"Authorization": "mcp-secret"}
                }]
            }),
        )
        .unwrap_err();
        assert_eq!(error, "MCP url is remote; enable allowRemote explicitly");
        assert!(!error.contains("private.example.invalid"));
        assert!(!error.contains("mcp-secret"));
    }

    #[test]
    fn mcp_settings_save_exposes_only_closed_safe_error_codes() {
        let (state, path) = test_state();
        let invalid = save_mcp_settings(
            &state,
            json!({
                "servers": [{
                    "id": "remote-tools",
                    "transport": "http",
                    "url": "https://private.example.invalid/mcp",
                    "headers": {"Authorization": "mcp-secret"}
                }]
            }),
        )
        .unwrap_err();
        assert_eq!(invalid, MCP_SETTINGS_SAVE_INVALID_URL);
        assert!(!invalid.contains("private.example.invalid"));
        assert!(!invalid.contains("mcp-secret"));
        assert!(state.settings.lock().get("mcp").is_none());
        cleanup_test_db(&path);

        let (locked_state, locked_path) = test_state();
        locked_state.settings_locked.store(true, Ordering::Release);
        assert_eq!(
            save_mcp_settings(
                &locked_state,
                json!({"servers": [{"id": "local", "command": "tool-server"}]}),
            )
            .unwrap_err(),
            MCP_SETTINGS_SAVE_UNAVAILABLE
        );
        assert!(locked_state.settings.lock().get("mcp").is_none());
        cleanup_test_db(&locked_path);
    }

    #[test]
    fn mcp_settings_save_rolls_back_memory_and_database_when_persistence_fails() {
        let (mut state, path) = test_state();
        let previous = json!({
            "schemaVersion": 1,
            "servers": [{"id": "existing", "command": "existing-server"}]
        });
        state
            .settings
            .lock()
            .insert("mcp".to_string(), previous.clone());
        state
            .persist_locked()
            .expect("existing MCP settings should persist");
        let durable_history = state.history.clone();

        let failed_parent = std::env::temp_dir().join(format!(
            "novavei-mcp-save-failure-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        assert!(!failed_parent.exists(), "test failure path must not exist");
        let failed_path = failed_parent.join("snapshot.sqlite3");
        state.history = HistoryStore::new(failed_path);

        assert_eq!(
            save_mcp_settings(
                &state,
                json!({
                    "servers": [{"id": "replacement", "command": "replacement-server"}]
                }),
            )
            .unwrap_err(),
            MCP_SETTINGS_SAVE_UNAVAILABLE
        );
        assert_eq!(
            state.settings.lock().get("mcp").cloned(),
            Some(previous.clone())
        );
        assert_eq!(
            durable_history.load_settings().unwrap().get("mcp"),
            Some(&previous)
        );

        cleanup_test_db(&path);
    }

    #[test]
    fn mcp_result_redaction_removes_native_configured_values() {
        let mut fields = Map::new();
        fields.insert(
            "text".to_string(),
            Value::String("echo native-secret and env-secret".to_string()),
        );
        let response = McpCallToolResponse {
            content: vec![crate::mcp_runtime::McpContent {
                content_type: "text".to_string(),
                fields,
            }],
            is_error: false,
            details: json!({"Authorization": "native-secret", "nested": "env-secret"}),
        };
        let result = redact_mcp_call_response(
            response,
            &["native-secret".to_string(), "env-secret".to_string()],
        );
        assert_eq!(
            result.content[0].fields["text"],
            "echo [redacted] and [redacted]"
        );
        assert_eq!(result.details["Authorization"], "[redacted]");
        assert_eq!(result.details["nested"], "[redacted]");
    }

    #[test]
    fn mcp_approval_is_bound_to_one_dynamic_tool_name_and_one_use() {
        let (state, path) = test_state();
        let session = state.sessions.lock().values().next().unwrap().clone();
        let workdir = canonical_workdir(&session.summary.cwd).unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let run = ActiveRun {
            session_id: session.summary.id.clone(),
            conversation_id: session.summary.id.clone(),
            turn_id: "turn-mcp-grant".to_string(),
            request_id: "request-mcp-grant".to_string(),
            proxy_provider_id: None,
            capability_token: "cap-mcp-grant".to_string(),
            cancelled: cancelled.clone(),
        };
        state
            .active_runs
            .lock()
            .insert(run.request_id.clone(), run.clone());
        state.capabilities.lock().insert(
            run.capability_token.clone(),
            CapabilityGrant {
                session_id: run.session_id.clone(),
                conversation_id: run.conversation_id.clone(),
                turn_id: run.turn_id.clone(),
                request_id: run.request_id.clone(),
                workdir: workdir.clone(),
                permission_mode: PermissionMode::Ask,
                expires_at: Instant::now() + Duration::from_secs(60),
                cancelled,
            },
        );
        state.tool_approvals.lock().insert(
            "mcp-one-use-approval".to_string(),
            ToolApprovalGrant {
                capability_token: run.capability_token.clone(),
                action: ToolAction::Mcp,
                tool_name: "mcp__one__search".to_string(),
                subagent_global_read: false,
                expires_at: Instant::now() + Duration::from_secs(60),
            },
        );
        let workdir = workdir.display().to_string();
        assert!(require_capability_for_named(
            &state,
            Some("cap-mcp-grant"),
            &workdir,
            ToolAction::Mcp,
            Some("mcp-one-use-approval"),
            Some("mcp__one__search"),
        )
        .is_ok());
        assert!(require_capability_for_named(
            &state,
            Some("cap-mcp-grant"),
            &workdir,
            ToolAction::Mcp,
            Some("mcp-one-use-approval"),
            Some("mcp__two__search"),
        )
        .is_err());
        cleanup_test_db(&path);
    }

    #[test]
    fn workspace_approval_requires_the_exact_selected_root() {
        let approved = HashSet::from([PathBuf::from(r"C:\projects\alpha")]);
        assert!(workdir_is_approved(
            &approved,
            Path::new(r"C:\projects\alpha")
        ));
        assert!(!workdir_is_approved(&approved, Path::new(r"C:\projects")));
        assert!(!workdir_is_approved(
            &approved,
            Path::new(r"C:\projects\alpha\nested")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn workspace_path_key_normalizes_slashes_case_and_safe_trailing_separators() {
        let expected = workspace_path_key(r"E:\Work\Alpha").unwrap();
        assert_eq!(expected, workspace_path_key(r"e:/work/alpha/").unwrap());
        assert_eq!(expected, workspace_path_key(r"E:\\WORK\\ALPHA\\").unwrap());
        // Do not trim a drive root into the non-absolute `C:` form.
        assert_eq!(workspace_path_key(r"C:\").as_deref(), Some(r"c:\"));
        assert_eq!(workspace_path_key(r"c:/").as_deref(), Some(r"c:\"));
    }

    #[test]
    fn relocation_picker_grant_is_source_bound_and_single_use() {
        let (state, path) = test_state();
        let target = canonical_workdir(&current_workdir()).unwrap();
        let source_key = workspace_path_key(&path_for_display(&target)).unwrap();

        issue_relocation_picker_grant(&state, source_key.clone(), target.clone());
        assert!(consume_relocation_picker_grant(&state, &source_key, &target).is_ok());
        assert!(consume_relocation_picker_grant(&state, &source_key, &target).is_err());

        issue_relocation_picker_grant(&state, source_key.clone(), target.clone());
        assert!(consume_relocation_picker_grant(
            &state,
            &source_key,
            &target.join("different-target")
        )
        .is_err());
        // A mismatched request consumes the one-time grant as well.
        assert!(consume_relocation_picker_grant(&state, &source_key, &target).is_err());
        cleanup_test_db(&path);
    }

    #[test]
    fn relocation_rejects_a_previously_approved_target_without_a_fresh_picker_grant() {
        let source = std::env::temp_dir().join(format!(
            "novavei-relocation-source-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&source).unwrap();
        let source = fs::canonicalize(&source).unwrap();
        let target = canonical_workdir(&current_workdir()).unwrap();
        let source_display = path_for_display(&source);
        let target_display = path_for_display(&target);

        let (state, path) = test_state();
        let session_id = state.sessions.lock().keys().next().cloned().unwrap();
        state
            .sessions
            .lock()
            .get_mut(&session_id)
            .unwrap()
            .summary
            .cwd = source_display.clone();
        *state.approved_workdirs.lock() = HashSet::from([target.clone()]);
        let app = tauri::test::mock_builder()
            .manage(Arc::new(state))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("test app should build");

        let error = sessions_relocate_workspace(
            app.state::<Arc<AppState>>(),
            source_display.clone(),
            target_display.clone(),
            None,
            None,
        )
        .expect_err("ambient target approval must not authorize relocation");
        assert!(error.contains("fresh native folder selection"));

        let source_key = workspace_path_key(&source_display).unwrap();
        {
            let state = app.state::<Arc<AppState>>();
            issue_relocation_picker_grant(state.inner(), source_key, target);
        }
        let relocated = sessions_relocate_workspace(
            app.state::<Arc<AppState>>(),
            source_display,
            target_display.clone(),
            None,
            None,
        )
        .expect("matching fresh picker grant should authorize relocation");
        assert_eq!(relocated.updated_session_ids, vec![session_id]);
        assert_eq!(relocated.to_workdir, target_display);

        drop(app);
        cleanup_test_db(&path);
        let _ = fs::remove_dir_all(source);
    }

    fn run_relocation_conflict_resolution_case(
        resolution: WorkspaceRelocationConflictResolution,
        expected_project_id: &str,
        expected_project_name: &str,
        expected_reasoning: &str,
    ) {
        let fixture_root = std::env::temp_dir().join(format!(
            "novavei-relocation-conflict-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let source = fixture_root.join("source");
        let target = fixture_root.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        let source = fs::canonicalize(&source).unwrap();
        let target = fs::canonicalize(&target).unwrap();
        let source_display = path_for_display(&source);
        let target_display = path_for_display(&target);
        let source_id = "project-11111111-1111-4111-8111-111111111111";
        let target_id = "project-22222222-2222-4222-8222-222222222222";

        let (state, database_path) = test_state();
        let session_id = state.sessions.lock().keys().next().cloned().unwrap();
        state
            .sessions
            .lock()
            .get_mut(&session_id)
            .unwrap()
            .summary
            .cwd = source_display.clone();
        state.settings.lock().insert(
            "projects".to_string(),
            json!({
                "version": PROJECT_SETTINGS_VERSION,
                "initialized": true,
                "entries": [
                    {
                        "id": source_id,
                        "name": "Source project",
                        "path": source_display,
                        "preferences": { "reasoning": "high" }
                    },
                    {
                        "id": target_id,
                        "name": "Target project",
                        "path": target_display,
                        "preferences": { "reasoning": "low" }
                    }
                ]
            }),
        );
        *state.approved_workdirs.lock() = HashSet::from([target.clone()]);
        issue_relocation_picker_grant(&state, workspace_path_key(&source_display).unwrap(), target);
        let app = tauri::test::mock_builder()
            .manage(Arc::new(state))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("test app should build");

        let conflict = sessions_relocate_workspace(
            app.state::<Arc<AppState>>(),
            source_display.clone(),
            target_display.clone(),
            None,
            None,
        )
        .expect("the first call should return a conflict without mutating");
        assert_eq!(conflict.status, WorkspaceRelocationStatus::Conflict);
        assert!(conflict.updated_session_ids.is_empty());
        assert!(conflict.updated_sessions.is_empty());
        assert!(conflict.updated_project_ids.is_empty());
        assert_eq!(
            app.state::<Arc<AppState>>()
                .sessions
                .lock()
                .get(&session_id)
                .unwrap()
                .summary
                .cwd,
            source_display
        );
        assert_eq!(
            app.state::<Arc<AppState>>()
                .settings
                .lock()
                .get("projects")
                .and_then(|value| value.get("entries"))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        let conflict_payload = conflict.conflict.expect("conflict details are required");
        assert_eq!(
            conflict_payload
                .source_project
                .as_ref()
                .map(|project| project.id.as_str()),
            Some(source_id)
        );
        assert_eq!(conflict_payload.target_project.id, target_id);
        let conflict_token = conflict
            .conflict_token
            .expect("conflict token should bind the displayed project snapshot");

        let relocated = sessions_relocate_workspace(
            app.state::<Arc<AppState>>(),
            source_display.clone(),
            target_display.clone(),
            Some(resolution),
            Some(conflict_token.clone()),
        )
        .expect("the selected conflict resolution should relocate atomically");
        assert_eq!(relocated.status, WorkspaceRelocationStatus::Relocated);
        assert_eq!(relocated.updated_session_ids, vec![session_id.clone()]);
        assert_eq!(
            app.state::<Arc<AppState>>()
                .sessions
                .lock()
                .get(&session_id)
                .unwrap()
                .summary
                .cwd,
            target_display
        );
        let settings = app
            .state::<Arc<AppState>>()
            .settings
            .lock()
            .get("projects")
            .cloned()
            .unwrap();
        let entries = settings
            .get("entries")
            .and_then(Value::as_array)
            .expect("project entries should remain valid");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["id"], expected_project_id);
        assert_eq!(entries[0]["name"], expected_project_name);
        assert_eq!(entries[0]["path"], target_display);
        assert_eq!(entries[0]["preferences"]["reasoning"], expected_reasoning);

        let replay_error = sessions_relocate_workspace(
            app.state::<Arc<AppState>>(),
            source_display,
            target_display,
            Some(resolution),
            Some(conflict_token),
        )
        .expect_err("a conflict token must be single use");
        assert!(replay_error.contains("invalid or expired"));

        drop(app);
        cleanup_test_db(&database_path);
        let _ = fs::remove_dir_all(fixture_root);
    }

    #[test]
    fn relocation_conflict_resolutions_preserve_the_selected_project_metadata() {
        run_relocation_conflict_resolution_case(
            WorkspaceRelocationConflictResolution::KeepSource,
            "project-11111111-1111-4111-8111-111111111111",
            "Source project",
            "high",
        );
        run_relocation_conflict_resolution_case(
            WorkspaceRelocationConflictResolution::MergeIntoTarget,
            "project-22222222-2222-4222-8222-222222222222",
            "Target project",
            "low",
        );
    }

    #[test]
    fn relocation_conflict_grant_rejects_project_identity_drift_and_replay() {
        let (state, database_path) = test_state();
        let target = canonical_workdir(&current_workdir()).unwrap();
        let conflict = WorkspaceRelocationConflict {
            source_project: Some(WorkspaceRelocationConflictProject {
                id: "11111111-1111-4111-8111-111111111111".to_string(),
                name: "Source".to_string(),
                path: r"E:\historical".to_string(),
            }),
            target_project: WorkspaceRelocationConflictProject {
                id: "22222222-2222-4222-8222-222222222222".to_string(),
                name: "Target".to_string(),
                path: path_for_display(&target),
            },
        };
        let from_key = workspace_path_key(r"E:\historical").unwrap();
        let token =
            issue_relocation_conflict_grant(&state, from_key.clone(), target.clone(), &conflict)
                .unwrap();
        let mut changed = conflict.clone();
        changed.target_project.id = "33333333-3333-4333-8333-333333333333".to_string();
        let drift_error =
            consume_relocation_conflict_grant(&state, &token, &from_key, &target, Some(&changed))
                .expect_err("the target identity shown to the user changed");
        assert!(drift_error.contains("conflict changed"));
        assert!(consume_relocation_conflict_grant(
            &state,
            &token,
            &from_key,
            &target,
            Some(&conflict)
        )
        .is_err());
        cleanup_test_db(&database_path);
    }

    #[test]
    fn full_snapshot_propagates_incomplete_cache_read_failures() {
        let (mut state, database_path) = test_state();
        for record in state.sessions.lock().values_mut() {
            record.messages_complete = false;
        }
        let failed_parent = std::env::temp_dir().join(format!(
            "novavei-relocation-snapshot-failure-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        assert!(!failed_parent.exists());
        state.history = HistoryStore::new(failed_parent.join("history.sqlite3"));
        let error = state
            .snapshot_for_persistence_locked()
            .expect_err("an unreadable incomplete cache must abort the full snapshot");
        assert!(!error.trim().is_empty());
        cleanup_test_db(&database_path);
        let _ = fs::remove_dir_all(failed_parent);
    }

    #[test]
    fn known_history_requires_explicit_project_registration_before_access() {
        let (state, path) = test_state();
        let session = state.sessions.lock().values().next().unwrap().clone();
        let canonical = canonical_workdir(&session.summary.cwd).unwrap();
        state.approved_workdirs.lock().clear();
        assert!(require_approved_workdir(&state, &canonical).is_err());
        assert!(require_registered_project_workdir(&state, &canonical).is_err());
        assert!(branch_session(&state, &session.summary.id, None).is_err());
        assert!(composer_attachment_session(
            &state,
            session.summary.cwd.clone(),
            session.summary.id.clone(),
        )
        .is_err());

        let app = tauri::test::mock_builder()
            .manage(Arc::new(state))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("test app should build");
        let state = app.state::<Arc<AppState>>();
        let registered = workspace_register_project(
            state,
            session.summary.cwd.clone(),
            Some("Known history".to_string()),
        )
        .expect("known historical workspace should register without a second picker");
        assert!(registered.created);
        let state = app.state::<Arc<AppState>>();
        assert!(require_approved_workdir(state.inner(), &canonical).is_ok());
        assert!(require_registered_project_workdir(state.inner(), &canonical).is_ok());
        assert!(composer_attachment_session(
            state.inner(),
            session.summary.cwd,
            session.summary.id,
        )
        .is_ok());
        cleanup_test_db(&path);
    }

    #[test]
    fn removing_a_project_revokes_capabilities_and_restores_read_only_history() {
        let workspace = std::env::temp_dir().join(format!(
            "novavei-project-removal-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&workspace).unwrap();
        let workspace = fs::canonicalize(&workspace).unwrap();
        let workspace_display = path_for_display(&workspace);

        let (state, path) = test_state();
        let session_id = state.sessions.lock().keys().next().cloned().unwrap();
        state
            .sessions
            .lock()
            .get_mut(&session_id)
            .unwrap()
            .summary
            .cwd = workspace_display.clone();
        state.approved_workdirs.lock().insert(workspace.clone());
        state.picker_workdirs.lock().insert(workspace.clone());
        let app = tauri::test::mock_builder()
            .manage(Arc::new(state))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("test app should build");

        settings_save_projects(
            app.state::<Arc<AppState>>(),
            json!({
                "version": PROJECT_SETTINGS_VERSION,
                "initialized": true,
                "entries": [{
                    "id": "project-removal-test",
                    "name": "Removal test",
                    "path": workspace_display.clone(),
                    "pinned": false
                }]
            }),
        )
        .expect("fresh picker root should persist as a project");
        let state = app.state::<Arc<AppState>>();
        assert!(workdir_is_registered_project(state.inner(), &workspace));
        assert!(!state.picker_workdirs.lock().contains(&workspace));
        state.workspace_capabilities.lock().insert(
            "workspace-cap-removal-test".to_string(),
            WorkspaceCapabilityGrant {
                session_id: session_id.clone(),
                workdir: workspace.clone(),
                expires_at: Instant::now() + Duration::from_secs(60),
            },
        );
        state.full_permission_grants.lock().insert(
            "full-permission-removal-test".to_string(),
            FullPermissionRunGrant {
                session_id: session_id.clone(),
                conversation_id: session_id.clone(),
                request_id: "project-removal-request".to_string(),
                workdir: workspace.clone(),
                prompt_digest: [0_u8; 32],
                run_binding: FullPermissionRunBinding {
                    provider_id: "embedded".to_string(),
                    model_id: "test-model".to_string(),
                    reasoning: None,
                },
                expires_at: Instant::now() + Duration::from_secs(60),
            },
        );

        settings_save_projects(
            app.state::<Arc<AppState>>(),
            json!({
                "version": PROJECT_SETTINGS_VERSION,
                "initialized": true,
                "entries": []
            }),
        )
        .expect("project removal should persist");
        let state = app.state::<Arc<AppState>>();
        assert!(!workdir_is_registered_project(state.inner(), &workspace));
        assert!(!state.approved_workdirs.lock().contains(&workspace));
        assert!(state.workspace_capabilities.lock().is_empty());
        assert!(state.full_permission_grants.lock().is_empty());
        assert!(branch_session(state.inner(), &session_id, None).is_err());
        assert!(sessions_create(
            app.state::<Arc<AppState>>(),
            Some("must remain read-only".to_string()),
            Some(workspace_display.clone()),
        )
        .is_err());
        assert!(workspace_capability_issue(
            app.state::<Arc<AppState>>(),
            workspace_display,
            session_id,
        )
        .is_err());

        drop(app);
        cleanup_test_db(&path);
        let _ = fs::remove_dir_all(workspace);
    }

    #[cfg(windows)]
    #[test]
    fn history_workspace_filter_and_grouping_use_the_normalized_path_key() {
        let (state, path) = test_state();
        let mut first = new_session_record("first".to_string(), r"E:\Work\Alpha".to_string());
        first.summary.id = "normalized-workspace-first".to_string();
        let mut second = new_session_record("second".to_string(), r"e:/work/alpha/".to_string());
        second.summary.id = "normalized-workspace-second".to_string();
        state.sessions.lock().clear();
        state.sessions.lock().extend(HashMap::from([
            (first.summary.id.clone(), first),
            (second.summary.id.clone(), second),
        ]));

        let app = tauri::test::mock_builder()
            .manage(Arc::new(state))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("test app should build");
        let listed = chat_history_list(
            app.state::<Arc<AppState>>(),
            Some(1),
            Some(50),
            Some(r"E:/WORK/ALPHA/".to_string()),
            Some(false),
        );
        assert_eq!(listed.total_count, 2);
        let groups = chat_history_workdirs(app.state::<Arc<AppState>>());
        assert_eq!(groups["workdirs"].as_array().map(Vec::len), Some(1));
        cleanup_test_db(&path);
    }

    #[test]
    fn workdir_policy_defaults_to_project_root_and_rejects_extra_paths() {
        assert!(validate_system_workdir_policy(None).is_ok());
        assert!(validate_system_workdir_policy(Some(&json!({
            "workdirPolicy": "project"
        })))
        .is_ok());
        assert!(validate_system_workdir_policy(Some(&json!({
            "workdir_policy": "project"
        })))
        .is_ok());
        assert!(validate_system_workdir_policy(Some(&json!({
            "workdirPolicy": "extra"
        })))
        .is_err());
        assert!(validate_system_workdir_policy(Some(&json!({
            "workdirPolicy": "project",
            "workdir_policy": "extra"
        })))
        .is_err());
    }

    #[test]
    fn system_settings_canonicalize_project_policy_and_reject_extra_paths() {
        let normalized = normalize_system_settings_payload(json!({
            "theme": "dark",
            "workdir_policy": "project",
            "default_permission_tier": "auto"
        }))
        .unwrap();
        assert_eq!(
            normalized.get("workdirPolicy").and_then(Value::as_str),
            Some("project")
        );
        assert!(normalized.get("workdir_policy").is_none());
        // Legacy auto-approve is folded back to ask rather than retained.
        assert_eq!(
            normalized
                .get("defaultPermissionTier")
                .and_then(Value::as_str),
            Some("ask")
        );
        assert!(normalized.get("default_permission_tier").is_none());
        assert!(normalize_system_settings_payload(json!({
            "workdirPolicy": "extra"
        }))
        .is_err());
        assert!(normalize_system_settings_payload(json!({
            "defaultPermissionTier": "unrestricted"
        }))
        .is_err());
    }

    #[test]
    fn system_settings_default_to_strict_security_boundaries() {
        let defaulted = default_setting("system", "C:\\Workspace");
        assert_eq!(
            defaulted["security"]["requirePlanForMutableTools"],
            json!(true)
        );
        assert_eq!(
            defaulted["security"]["allowSubagentGlobalRead"],
            json!(false)
        );

        let normalized = normalize_system_settings_payload(json!({})).unwrap();
        assert_eq!(
            normalized["security"]["requirePlanForMutableTools"],
            json!(true)
        );
        assert_eq!(
            normalized["security"]["allowSubagentGlobalRead"],
            json!(false)
        );
    }

    #[test]
    fn system_settings_canonicalize_legacy_security_fields() {
        let normalized = normalize_system_settings_payload(json!({
            "require_plan_for_mutable_tools": false,
            "allowSubagentGlobalRead": true
        }))
        .unwrap();
        assert!(normalized.get("require_plan_for_mutable_tools").is_none());
        assert!(normalized.get("allowSubagentGlobalRead").is_none());
        assert_eq!(
            normalized["security"]["requirePlanForMutableTools"],
            json!(false)
        );
        assert_eq!(
            normalized["security"]["allowSubagentGlobalRead"],
            json!(true)
        );

        let canonical = normalize_system_settings_payload(json!({
            "security": {
                "require_plan_for_mutable_tools": true,
                "allow_subagent_global_read": false
            }
        }))
        .unwrap();
        assert_eq!(
            canonical["security"]["requirePlanForMutableTools"],
            json!(true)
        );
        assert_eq!(
            canonical["security"]["allowSubagentGlobalRead"],
            json!(false)
        );
    }

    #[test]
    fn system_settings_reject_invalid_security_fields() {
        assert!(normalize_system_settings_payload(json!({
            "security": "strict"
        }))
        .is_err());
        assert!(normalize_system_settings_payload(json!({
            "security": {
                "requirePlanForMutableTools": "yes"
            }
        }))
        .is_err());
        assert!(normalize_system_settings_payload(json!({
            "security": {
                "allowSubagentGlobalRead": false,
                "allow_subagent_global_read": true
            }
        }))
        .is_err());
        assert!(normalize_system_settings_payload(json!({
            "security": {
                "requirePlanForMutableTools": true,
                "unexpected": false
            }
        }))
        .is_err());
    }

    #[test]
    fn system_settings_normalize_secondary_launch_behavior() {
        let defaulted = normalize_system_settings_payload(json!({})).unwrap();
        assert_eq!(
            defaulted["secondaryLaunchBehavior"],
            json!(SECONDARY_LAUNCH_FOCUS_EXISTING)
        );
        let new_window = normalize_system_settings_payload(json!({
            "secondaryLaunchBehavior": SECONDARY_LAUNCH_NEW_WINDOW
        }))
        .unwrap();
        assert_eq!(
            new_window["secondaryLaunchBehavior"],
            json!(SECONDARY_LAUNCH_NEW_WINDOW)
        );
        assert!(normalize_system_settings_payload(json!({
            "secondaryLaunchBehavior": "another-process"
        }))
        .is_err());
        assert!(normalize_system_settings_payload(json!({
            "secondaryLaunchBehavior": SECONDARY_LAUNCH_NEW_WINDOW,
            "secondary_launch_behavior": SECONDARY_LAUNCH_FOCUS_EXISTING
        }))
        .is_err());
    }

    #[test]
    fn project_permission_preferences_are_canonical_and_never_persist_full_access() {
        let entry = json!({
            "preferences": {
                "permission": "ask"
            }
        });
        let normalized =
            normalize_project_preferences(entry.as_object().expect("project entry is an object"))
                .expect("ask permission should normalize")
                .expect("permission preference should be retained");
        assert_eq!(
            normalized.get("permission").and_then(Value::as_str),
            Some("ask")
        );

        // Legacy auto-approve is no longer a durable project preference.
        for permission in ["full", "unrestricted", "auto", "auto-approve", ""] {
            let entry = json!({
                "preferences": { "permission": permission }
            });
            assert!(normalize_project_preferences(
                entry.as_object().expect("project entry is an object"),
            )
            .is_err());
        }
    }

    #[test]
    fn project_settings_only_persist_approved_unique_roots() {
        let (state, data_file) = test_state();
        let cwd = state
            .sessions
            .lock()
            .values()
            .next()
            .unwrap()
            .summary
            .cwd
            .clone();
        let normalized = normalize_projects_settings_payload(
            &state,
            json!({
                "initialized": true,
                "entries": [{
                    "id": "project-main",
                    "name": "Main project",
                    "path": cwd,
                    "pinned": true
                }]
            }),
        )
        .unwrap();
        assert_eq!(normalized["version"], PROJECT_SETTINGS_VERSION);
        assert_eq!(normalized["initialized"], true);
        assert_eq!(normalized["entries"][0]["pinned"], true);

        let duplicate = normalized["entries"][0].clone();
        assert!(normalize_projects_settings_payload(
            &state,
            json!({"initialized": true, "entries": [duplicate.clone(), duplicate]})
        )
        .is_err());
        drop(state);
        let _ = fs::remove_file(data_file);
    }

    #[test]
    fn provider_protocol_prefers_known_protocol_over_generic_type() {
        let object = serde_json::from_value::<Map<String, Value>>(json!({
            "type": "custom",
            "protocol": "anthropic-messages"
        }))
        .unwrap();
        assert_eq!(provider_protocol(&object), "anthropic-messages");
    }

    #[test]
    fn provider_proxy_config_reads_all_native_inputs_from_one_snapshot() {
        let settings = HashMap::from([(
            "providers".to_string(),
            json!([{
                "id": "gateway",
                "protocol": "openai",
                "baseUrl": "HTTPS://API.EXAMPLE.COM/v1",
                "apiKey": "native-secret",
                "useSystemProxy": true,
                "customHeaders": [{"key": "X-Client-Request-Id", "value": "native-request"}]
            }]),
        )]);
        let config = provider_proxy_config_from_settings(&settings, "gateway")
            .expect("configured provider should resolve");
        assert_eq!(config.upstream_base_url, "https://api.example.com/v1");
        assert_eq!(
            config.headers,
            vec![
                (
                    "authorization".to_string(),
                    "Bearer native-secret".to_string()
                ),
                (
                    "X-Client-Request-Id".to_string(),
                    "native-request".to_string()
                ),
            ]
        );
        assert!(config.use_system_proxy);
    }

    #[test]
    fn provider_proxy_config_returns_none_for_unknown_provider() {
        let settings = HashMap::from([("providers".to_string(), json!([]))]);
        assert!(provider_proxy_config_from_settings(&settings, "missing").is_none());
    }

    #[test]
    fn proxy_provider_binding_uses_the_native_resolved_selection() {
        let (state, path) = test_state();
        state.settings.lock().insert(
            "providers".to_string(),
            json!([
                {
                    "id": "default-provider",
                    "enabled": true,
                    "isDefault": true,
                    "baseUrl": "https://default.example.test/v1",
                    "models": [{"id": "default-model", "enabled": true}],
                    "defaultModel": "default-model"
                },
                {
                    "id": "requested-provider",
                    "enabled": true,
                    "baseUrl": "https://requested.example.test/v1",
                    "models": [{"id": "requested-model", "enabled": true}],
                    "defaultModel": "requested-model"
                }
            ]),
        );
        assert_eq!(
            resolved_proxy_provider_id(&state, None, None),
            Some("default-provider".to_string())
        );
        assert_eq!(
            resolved_proxy_provider_id(&state, Some("requested-provider"), None),
            Some("requested-provider".to_string())
        );
        assert_eq!(
            resolved_proxy_provider_id(&state, Some("missing-provider"), None),
            None
        );
        cleanup_test_db(&path);
    }

    #[test]
    fn translation_context_resolves_the_default_provider_and_model() {
        let (state, path) = test_state();
        state.settings.lock().insert(
            "providers".to_string(),
            json!([
                {
                    "id": "default-provider",
                    "enabled": true,
                    "isDefault": true,
                    "baseUrl": "https://default.example.test/v1",
                    "models": [{"id": "default-model", "enabled": true}],
                    "defaultModel": "default-model"
                },
                {
                    "id": "second-provider",
                    "enabled": true,
                    "baseUrl": "https://second.example.test/v1",
                    "models": [{"id": "second-model", "enabled": true}],
                    "defaultModel": "second-model"
                }
            ]),
        );
        assert_eq!(
            resolve_translation_context(&state, None, None).unwrap(),
            (
                "default-provider".to_string(),
                "default-model".to_string()
            )
        );
        assert_eq!(
            resolve_translation_context(&state, None, Some("second-model")).unwrap(),
            (
                "second-provider".to_string(),
                "second-model".to_string()
            )
        );
        cleanup_test_db(&path);
    }

    #[test]
    fn translation_context_prefers_the_active_session_model_selection() {
        let (state, path) = test_state();
        state.settings.lock().insert(
            "providers".to_string(),
            json!([
                {
                    "id": "default-provider",
                    "enabled": true,
                    "isDefault": true,
                    "baseUrl": "https://default.example.test/v1",
                    "models": [{"id": "default-model", "enabled": true}],
                    "defaultModel": "default-model"
                },
                {
                    "id": "session-provider",
                    "enabled": true,
                    "baseUrl": "https://session.example.test/v1",
                    "models": [{"id": "session-model", "enabled": true}],
                    "defaultModel": "session-model"
                }
            ]),
        );
        let session_id = state.sessions.lock().keys().next().unwrap().clone();
        {
            let mut sessions = state.sessions.lock();
            let record = sessions.get_mut(&session_id).unwrap();
            record.selected_model_json = Some(
                json!({"providerId": "session-provider", "modelId": "session-model"}).to_string(),
            );
        }
        assert_eq!(
            resolve_translation_context(&state, Some(&session_id), None).unwrap(),
            (
                "session-provider".to_string(),
                "session-model".to_string()
            )
        );
        cleanup_test_db(&path);
    }

    #[test]
    fn translation_context_rejects_disabled_or_missing_providers() {
        let (state, path) = test_state();
        state.settings.lock().insert(
            "providers".to_string(),
            json!([{
                "id": "disabled-provider",
                "enabled": false,
                "baseUrl": "https://disabled.example.test/v1",
                "models": [{"id": "disabled-model", "enabled": true}],
                "defaultModel": "disabled-model"
            }]),
        );
        assert!(resolve_translation_context(&state, None, None).is_err());
        assert!(resolve_translation_context(&state, None, Some("missing-model")).is_err());
        cleanup_test_db(&path);
    }

    #[test]
    fn raw_translation_provider_reads_the_unredacted_record_by_id() {
        let settings = HashMap::from([(
            "providers".to_string(),
            json!([{
                "id": "native-provider",
                "enabled": true,
                "baseUrl": "https://native.example.test/v1",
                "models": [{"id": "native-model", "enabled": true}],
                "defaultModel": "native-model",
                "apiKey": "native-secret"
            }]),
        )]);
        let record = raw_translation_provider(&settings, "native-provider").unwrap();
        assert_eq!(
            record.pointer("/apiKey").and_then(Value::as_str),
            Some("native-secret")
        );
        assert!(raw_translation_provider(&settings, "missing-provider").is_err());
    }

    #[test]
    fn disabled_provider_has_no_native_proxy_or_model_probe_path() {
        let settings = HashMap::from([(
            "providers".to_string(),
            json!([{
                "id": "disabled-gateway",
                "name": "Disabled gateway",
                "type": "codex",
                "protocol": "openai-responses",
                "baseUrl": "https://gateway.example/tenant-a/v1",
                "models": ["model-a"],
                "activeModels": ["model-a"],
                "apiKey": "native-secret",
                "enabled": false,
            }]),
        )]);
        assert!(provider_proxy_config_from_settings(&settings, "disabled-gateway").is_none());
        assert!(provider_model_probe_config_from_settings(&settings, "disabled-gateway").is_err());
    }

    #[test]
    fn provider_model_allowlist_rejects_disabled_or_unknown_models() {
        let provider = serde_json::from_value::<Map<String, Value>>(json!({
            "models": [
                {"id": "model-a"},
                {"id": "model-b", "enabled": false}
            ],
            "activeModels": ["model-a", "model-b"]
        }))
        .unwrap();
        assert!(provider_model_is_enabled(&provider, "model-a"));
        assert!(!provider_model_is_enabled(&provider, "model-b"));
        assert!(!provider_model_is_enabled(&provider, "model-c"));
        assert_eq!(
            provider_default_enabled_model(&provider).as_deref(),
            Some("model-a")
        );
    }

    #[test]
    fn legacy_session_merge_keeps_sqlite_records_and_adds_missing_ones() {
        let mut sqlite_record = new_session_record("SQLite".to_string(), "C:\\sqlite".to_string());
        sqlite_record.summary.id = "shared".to_string();
        let mut legacy_shared =
            new_session_record("Legacy copy".to_string(), "C:\\legacy".to_string());
        legacy_shared.summary.id = "shared".to_string();
        let mut legacy_only =
            new_session_record("Legacy only".to_string(), "C:\\legacy-only".to_string());
        legacy_only.summary.id = "legacy-only".to_string();

        let mut sessions = HashMap::from([("shared".to_string(), sqlite_record)]);
        merge_legacy_sessions(&mut sessions, vec![legacy_shared, legacy_only]);

        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions["shared"].summary.title, "SQLite");
        assert_eq!(sessions["legacy-only"].summary.title, "Legacy only");
    }

    #[test]
    fn custom_header_arrays_are_normalized_without_proxy_headers() {
        let object = serde_json::from_value::<Map<String, Value>>(json!({
            "customHeaders": [
                {"key": "X-Client", "value": "client"},
                {"key": "X-NovaVei-Token", "value": "renderer"},
                {"key": "", "value": "ignored"}
            ]
        }))
        .unwrap();
        let mut headers = Vec::new();
        append_custom_headers(&object, &mut headers);
        assert_eq!(
            headers,
            vec![("X-Client".to_string(), "client".to_string())]
        );
    }

    #[test]
    fn redaction_preserves_header_names_but_not_values() {
        let value = redact_settings_secrets(&json!({
            "customHeaders": [{"key": "X-Client", "value": "secret"}],
            "apiKey": "secret-key"
        }));
        assert_eq!(value["customHeaders"][0]["key"], json!("X-Client"));
        assert_eq!(value["customHeaders"][0]["valueConfigured"], json!(true));
        assert_eq!(value["apiKeyConfigured"], json!(true));
        assert!(value["customHeaders"][0].get("value").is_none());
    }

    #[test]
    fn provider_save_preserves_native_secrets_from_redacted_payload() {
        let existing = json!([{
            "id": "local",
            "baseUrl": "http://127.0.0.1:11434/v1",
            "apiKey": "native-secret",
            "customHeaders": [{"key": "X-Client-Secret", "value": "header-secret"}]
        }]);
        let incoming = json!([{
            "id": "local",
            "name": "Local",
            "baseUrl": "http://127.0.0.1:11434/v1",
            "apiKeyConfigured": true,
            "customHeaders": [{"key": "X-Client-Secret", "valueConfigured": true}]
        }]);
        let merged = merge_provider_settings(Some(&existing), incoming);
        assert_eq!(merged[0]["apiKey"], json!("native-secret"));
        assert_eq!(
            merged[0]["customHeaders"][0]["value"],
            json!("header-secret")
        );
        assert!(merged[0].get("apiKeyConfigured").is_none());
        assert!(merged[0]["customHeaders"][0]
            .get("valueConfigured")
            .is_none());
    }

    #[test]
    fn provider_save_allows_explicit_secret_clear() {
        let existing = json!([{"id": "remote", "apiKey": "native-secret"}]);
        let incoming = json!([{"id": "remote", "apiKey": ""}]);
        let merged = merge_provider_settings(Some(&existing), incoming);
        assert!(merged[0].get("apiKey").is_none());
    }

    #[test]
    fn provider_save_preserves_unmentioned_records_without_explicit_removal() {
        let existing = json!([
            {
                "id": "alpha",
                "baseUrl": "https://alpha.example.com/v1",
                "models": ["alpha-1"]
            },
            {
                "id": "imported",
                "baseUrl": "https://imported.example.com/v1",
                "models": ["imported-1"]
            }
        ]);
        let incoming = json!([{
            "id": "alpha",
            "name": "Edited alpha",
            "baseUrl": "https://alpha.example.com/v1",
            "models": ["alpha-2"]
        }]);
        let preserved = merge_provider_settings_preserving_unmentioned(
            Some(&existing),
            incoming.clone(),
            &HashSet::new(),
        );
        assert_eq!(preserved.as_array().map(|items| items.len()), Some(2));
        assert_eq!(
            provider_record_for_id(&preserved, "alpha")
                .unwrap()
                .get("name")
                .and_then(Value::as_str),
            Some("Edited alpha")
        );
        assert!(provider_record_for_id(&preserved, "imported").is_some());

        let removed = HashSet::from(["imported".to_string()]);
        let merged =
            merge_provider_settings_preserving_unmentioned(Some(&existing), incoming, &removed);
        assert!(provider_record_for_id(&merged, "imported").is_none());
    }

    #[test]
    fn provider_save_validates_explicit_removal_identifiers() {
        assert_eq!(provider_removal_ids(None).unwrap(), HashSet::new());
        assert_eq!(
            provider_removal_ids(Some(vec!["one".to_string(), "two".to_string()])).unwrap(),
            HashSet::from(["one".to_string(), "two".to_string()])
        );
        assert!(provider_removal_ids(Some(vec!["../secret".to_string()])).is_err());
        assert!(provider_removal_ids(Some(vec!["same".to_string(), "same".to_string()])).is_err());
    }

    #[test]
    fn provider_save_keeps_secrets_for_same_endpoint_and_auth_family() {
        let existing = json!([{
            "id": "gateway",
            "type": "codex",
            "protocol": "openai-responses",
            "baseUrl": "https://api.example.com/v1",
            "apiKey": "native-secret",
            "customHeaders": [{"key": "X-Client-Secret", "value": "header-secret"}]
        }]);
        let incoming = json!([{
            "id": "gateway",
            "type": "codex",
            "protocol": "openai-completions",
            "baseUrl": "https://api.example.com/v1",
            "apiKeyConfigured": true,
            "customHeaders": [{"key": "X-Client-Secret", "valueConfigured": true}]
        }]);
        let merged = merge_provider_settings(Some(&existing), incoming);
        assert_eq!(merged[0]["apiKey"], json!("native-secret"));
        assert_eq!(
            merged[0]["customHeaders"][0]["value"],
            json!("header-secret")
        );
        assert_eq!(
            merged[0][PROVIDER_CREDENTIAL_BINDING_KEY]["origin"],
            json!("https://api.example.com")
        );
        assert_eq!(
            merged[0][PROVIDER_CREDENTIAL_BINDING_KEY]["endpoint"],
            json!("https://api.example.com/v1")
        );
        assert_eq!(
            merged[0][PROVIDER_CREDENTIAL_BINDING_KEY]["authFamily"],
            json!("openai")
        );
    }

    #[test]
    fn provider_save_drops_secrets_when_path_changes_at_same_origin() {
        let existing = json!([{
            "id": "gateway",
            "type": "codex",
            "baseUrl": "https://api.example.com/v1",
            "apiKey": "old-secret",
            "customHeaders": [{"key": "X-Client-Secret", "value": "old-header"}]
        }]);
        let incoming = json!([{
            "id": "gateway",
            "type": "codex",
            "baseUrl": "https://api.example.com/v2",
            "apiKeyConfigured": true,
            "customHeaders": [{"key": "X-Client-Secret", "valueConfigured": true}]
        }]);
        let merged = merge_provider_settings(Some(&existing), incoming);
        assert!(merged[0].get("apiKey").is_none());
        assert_eq!(merged[0]["customHeaders"], json!([]));
        assert!(merged[0].get(PROVIDER_CREDENTIAL_BINDING_KEY).is_none());
    }

    #[test]
    fn provider_save_drops_secrets_when_origin_changes() {
        let existing = json!([{
            "id": "gateway",
            "type": "codex",
            "baseUrl": "https://old.example.com/v1",
            "apiKey": "old-secret",
            "customHeaders": [{"key": "X-Client-Secret", "value": "old-header"}]
        }]);
        let incoming = json!([{
            "id": "gateway",
            "type": "codex",
            "baseUrl": "https://new.example.com/v1",
            "apiKeyConfigured": true,
            "customHeaders": [{"key": "X-Client-Secret", "valueConfigured": true}]
        }]);
        let merged = merge_provider_settings(Some(&existing), incoming);
        assert!(merged[0].get("apiKey").is_none());
        assert_eq!(merged[0]["customHeaders"], json!([]));
        assert!(merged[0].get(PROVIDER_CREDENTIAL_BINDING_KEY).is_none());
    }

    #[test]
    fn provider_save_drops_secrets_when_auth_family_changes() {
        let existing = json!([{
            "id": "gateway",
            "type": "codex",
            "protocol": "openai-responses",
            "baseUrl": "https://shared.example.com/v1",
            "apiKey": "openai-secret",
            "customHeaders": [{"key": "X-Client-Secret", "value": "old-header"}]
        }]);
        let incoming = json!([{
            "id": "gateway",
            "type": "gemini",
            "protocol": "google-generative-ai",
            "baseUrl": "https://shared.example.com/v1beta",
            "apiKeyConfigured": true,
            "customHeaders": [{"key": "X-Client-Secret", "valueConfigured": true}]
        }]);
        let merged = merge_provider_settings(Some(&existing), incoming);
        assert!(merged[0].get("apiKey").is_none());
        assert_eq!(merged[0]["customHeaders"], json!([]));
        assert!(merged[0].get(PROVIDER_CREDENTIAL_BINDING_KEY).is_none());
    }

    #[test]
    fn provider_save_binds_explicit_new_secrets_after_context_change() {
        let existing = json!([{
            "id": "gateway",
            "type": "codex",
            "baseUrl": "https://old.example.com/v1",
            "apiKey": "old-secret",
            "customHeaders": [{"key": "X-Client-Secret", "value": "old-header"}]
        }]);
        let incoming = json!([{
            "id": "gateway",
            "type": "codex",
            "baseUrl": "https://new.example.com/v1",
            "apiKey": "new-secret",
            "customHeaders": [{"key": "X-Client-Secret", "value": "new-header"}]
        }]);
        let merged = merge_provider_settings(Some(&existing), incoming);
        assert_eq!(merged[0]["apiKey"], json!("new-secret"));
        assert_eq!(merged[0]["customHeaders"][0]["value"], json!("new-header"));
        assert_eq!(
            merged[0][PROVIDER_CREDENTIAL_BINDING_KEY]["origin"],
            json!("https://new.example.com")
        );
    }

    #[test]
    fn provider_test_credentials_reject_stale_binding() {
        let object = serde_json::from_value::<Map<String, Value>>(json!({
            "id": "gateway",
            "type": "codex",
            "baseUrl": "https://new.example.com/v1",
            "apiKey": "stale-secret",
            "customHeaders": [{"key": "X-Client-Secret", "value": "stale-header"}],
            "__credentialBinding": {
                "origin": "https://old.example.com",
                "authFamily": "openai"
            }
        }))
        .unwrap();
        assert!(provider_credentials_from_object(&object).is_empty());
        assert!(provider_test_credentials_from_object(&object).is_err());
        assert!(!provider_has_credentials(&object));
    }

    #[test]
    fn keyless_provider_is_still_a_configured_proxy_target() {
        let object = serde_json::from_value::<Map<String, Value>>(json!({
            "id": "ollama",
            "type": "ollama",
            "baseUrl": "http://127.0.0.1:11434/v1"
        }))
        .unwrap();
        assert!(provider_credentials_from_object(&object).is_empty());
        assert!(provider_allows_keyless_local(&object));
    }

    #[test]
    fn keyless_provider_requires_a_loopback_url_host() {
        let remote = serde_json::from_value::<Map<String, Value>>(json!({
            "id": "spoofed-local",
            "baseUrl": "https://evil.example/localhost/v1"
        }))
        .unwrap();
        assert!(!provider_allows_keyless_local(&remote));
        let localhost = serde_json::from_value::<Map<String, Value>>(json!({
            "id": "local",
            "baseUrl": "http://service.localhost:11434/v1"
        }))
        .unwrap();
        assert!(provider_allows_keyless_local(&localhost));
    }

    #[test]
    fn provider_custom_headers_reject_reserved_transport_headers() {
        for name in [
            "Host",
            "Content-Length",
            "Connection",
            "Transfer-Encoding",
            "Forwarded",
            "X-Forwarded-For",
            "X-NovaVei-Internal",
        ] {
            assert!(is_reserved_provider_header(name), "reserved header: {name}");
        }
        assert!(!is_reserved_provider_header("Authorization"));
        assert!(!is_reserved_provider_header("X-Client-Request-Id"));
    }

    #[test]
    fn provider_import_sanitizes_exports_and_preview_never_serializes_credentials() {
        let (candidates, skipped) = parse_provider_import_export(
            r#"{
                "providers": [{
                    "id": "gateway",
                    "name": "Export gateway",
                    "type": "openai",
                    "baseUrl": "https://api.example.com/v1",
                    "models": [{"id": "gpt-5.6"}],
                    "apiKey": "export-api-key",
                    "customHeaders": {
                        "Authorization": "Bearer export-header",
                        "X-Client-Token": "export-token"
                    }
                }]
            }"#,
        )
        .unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].has_credential);
        let normalized = serde_json::to_string(&candidates[0].provider).unwrap();
        for secret in ["export-api-key", "export-header", "export-token"] {
            assert!(!normalized.contains(secret));
        }
        assert!(candidates[0].provider.get("apiKey").is_none());
        assert!(candidates[0].provider.get("customHeaders").is_none());

        let (state, path) = test_state();
        let preview = provider_import_preview_for_candidates(&state, candidates, skipped);
        assert!(preview.providers[0].has_credential);
        assert_eq!(preview.providers[0].host, "api.example.com");
        assert_eq!(preview.providers[0].api_root, "/v1");
        assert_eq!(preview.providers[0].protocol, "openai-responses");
        assert!(!preview.providers[0].requires_credential_reentry);
        let rendered = serde_json::to_string(&preview).unwrap();
        for secret in ["export-api-key", "export-header", "export-token"] {
            assert!(!rendered.contains(secret));
        }
        assert!(!rendered.contains("https://api.example.com/v1"));
        drop(state);
        cleanup_test_db(&path);
    }

    #[test]
    fn provider_import_preview_requires_credential_reentry_for_changed_api_root() {
        let (state, path) = test_state();
        state.settings.lock().insert(
            "providers".to_string(),
            json!([{
                "id": "gateway",
                "protocol": "openai-responses",
                "baseUrl": "https://api.example.com/v1",
                "apiKey": "native-api-key"
            }]),
        );

        let (same_endpoint, _) = parse_provider_import_export(
            r#"[{"id":"gateway","protocol":"openai-responses","baseUrl":"https://api.example.com/v1","models":["gpt-5.6"]}]"#,
        )
        .unwrap();
        let same_preview = provider_import_preview_for_candidates(&state, same_endpoint, 0);
        assert_eq!(same_preview.providers[0].api_root, "/v1");
        assert!(!same_preview.providers[0].requires_credential_reentry);

        let (changed_endpoint, _) = parse_provider_import_export(
            r#"[{"id":"gateway","protocol":"openai-responses","baseUrl":"https://api.example.com/api/v1","models":["gpt-5.6"]}]"#,
        )
        .unwrap();
        let changed_preview = provider_import_preview_for_candidates(&state, changed_endpoint, 0);
        assert_eq!(changed_preview.providers[0].api_root, "/api/v1");
        assert!(changed_preview.providers[0].requires_credential_reentry);

        drop(state);
        cleanup_test_db(&path);
    }

    #[test]
    fn provider_import_same_origin_keeps_native_credentials_but_not_export_credentials() {
        let existing = json!([{
            "id": "gateway",
            "type": "codex",
            "protocol": "openai-responses",
            "baseUrl": "https://api.example.com/v1",
            "apiKey": "native-api-key",
            "customHeaders": [{"key": "X-Native-Secret", "value": "native-header"}]
        }]);
        let (candidates, _) = parse_provider_import_export(
            r#"[{
                "id": "gateway",
                "name": "Updated gateway",
                "protocol": "openai-completions",
                "baseUrl": "https://api.example.com/v1",
                "models": ["gpt-5.6"],
                "apiKey": "export-api-key",
                "headers": {"Authorization": "Bearer export-header"}
            }]"#,
        )
        .unwrap();
        let merged =
            merge_provider_import_selection(Some(&existing), &[candidates[0].provider.clone()])
                .unwrap();
        assert_eq!(merged[0]["name"], json!("Updated gateway"));
        assert_eq!(merged[0]["apiKey"], json!("native-api-key"));
        assert_eq!(
            merged[0]["customHeaders"][0]["value"],
            json!("native-header")
        );
        let rendered = serde_json::to_string(&merged).unwrap();
        assert!(!rendered.contains("export-api-key"));
        assert!(!rendered.contains("export-header"));
    }

    #[test]
    fn provider_import_changed_origin_drops_native_credentials() {
        let existing = json!([{
            "id": "gateway",
            "type": "codex",
            "baseUrl": "https://old.example.com/v1",
            "apiKey": "native-api-key",
            "customHeaders": [{"key": "X-Native-Secret", "value": "native-header"}]
        }]);
        let (candidates, _) = parse_provider_import_export(
            r#"[{
                "id": "gateway",
                "baseUrl": "https://new.example.com/v1",
                "models": ["gpt-5.6"],
                "apiKey": "export-api-key"
            }]"#,
        )
        .unwrap();
        let merged =
            merge_provider_import_selection(Some(&existing), &[candidates[0].provider.clone()])
                .unwrap();
        assert!(merged[0].get("apiKey").is_none());
        assert!(merged[0].get("customHeaders").is_none());
        let rendered = serde_json::to_string(&merged).unwrap();
        assert!(!rendered.contains("export-api-key"));
        assert!(!rendered.contains("native-api-key"));
        assert!(!rendered.contains("native-header"));
    }

    #[test]
    fn provider_import_changed_path_drops_native_credentials() {
        let existing = json!([{
            "id": "gateway",
            "type": "codex",
            "baseUrl": "https://api.example.com/v1",
            "apiKey": "native-api-key",
            "customHeaders": [{"key": "X-Native-Secret", "value": "native-header"}]
        }]);
        let (candidates, _) = parse_provider_import_export(
            r#"[{
                "id": "gateway",
                "baseUrl": "https://api.example.com/api/v1",
                "models": ["gpt-5.6"]
            }]"#,
        )
        .unwrap();
        let merged =
            merge_provider_import_selection(Some(&existing), &[candidates[0].provider.clone()])
                .unwrap();
        assert!(merged[0].get("apiKey").is_none());
        assert!(merged[0].get("customHeaders").is_none());
    }

    #[test]
    fn provider_import_keeps_unselected_provider_records() {
        let existing = json!([
            {
                "id": "alpha",
                "baseUrl": "https://alpha.example.com/v1",
                "models": ["alpha-1"],
                "apiKey": "alpha-native-secret"
            },
            {
                "id": "beta",
                "baseUrl": "https://beta.example.com/v1",
                "models": ["beta-1"]
            }
        ]);
        let (candidates, _) = parse_provider_import_export(
            r#"[{"id":"beta","name":"Updated beta","baseUrl":"https://beta.example.com/api/v1","models":["beta-2"]}]"#,
        )
        .unwrap();
        let merged =
            merge_provider_import_selection(Some(&existing), &[candidates[0].provider.clone()])
                .unwrap();
        assert_eq!(merged.as_array().map(|items| items.len()), Some(2));
        let alpha = provider_record_for_id(&merged, "alpha").unwrap();
        let beta = provider_record_for_id(&merged, "beta").unwrap();
        assert_eq!(alpha["apiKey"], json!("alpha-native-secret"));
        assert_eq!(beta["name"], json!("Updated beta"));
        assert_eq!(beta["models"], json!(["beta-2"]));
    }

    #[test]
    fn provider_import_accepts_only_json_file_extensions() {
        assert!(provider_import_path_is_json(Path::new(
            "provider-export.json"
        )));
        assert!(provider_import_path_is_json(Path::new(
            "PROVIDER-EXPORT.JSON"
        )));
        assert!(!provider_import_path_is_json(Path::new(
            "provider-export.txt"
        )));
        assert!(!provider_import_path_is_json(Path::new("provider-export")));
    }

    #[test]
    fn provider_import_allows_only_public_api_root_paths() {
        for path in ["/", "/v1", "/v1/", "/api/v1", "/openai/v1", "/v1beta"] {
            assert!(provider_import_safe_api_path(path), "allowed path: {path}");
        }
        for path in ["/v1/export-secret", "/tenant-a/v1", "/v1%2Fsecret", "//v1"] {
            assert!(
                !provider_import_safe_api_path(path),
                "rejected path: {path}"
            );
        }
        assert_eq!(
            provider_import_api_root("https://api.example.com/api/v1/"),
            Some("/api/v1".to_string())
        );
        assert!(provider_import_api_root("https://api.example.com/tenant-a/v1").is_none());
    }

    #[test]
    fn provider_import_parser_rejects_invalid_and_oversized_exports() {
        assert!(parse_provider_import_export("{not json").is_err());
        assert!(parse_provider_import_export(
            r#"[{"id":"bad","baseUrl":"https://example.com/v1?token=secret","models":["m"]}]"#,
        )
        .is_err());
        assert!(parse_provider_import_export(
            r#"[{"id":"bad-path","baseUrl":"https://example.com/v1/export-secret","models":["m"]}]"#,
        )
        .is_err());

        let oversized = Value::Array(
            (0..=MAX_PROVIDER_IMPORT_RECORDS)
                .map(|index| {
                    json!({
                        "id": format!("provider-{index}"),
                        "baseUrl": "https://example.com/v1",
                        "models": ["m"]
                    })
                })
                .collect(),
        );
        let oversized = serde_json::to_string(&oversized).unwrap();
        assert!(parse_provider_import_export(&oversized).is_err());
    }

    #[test]
    fn redacted_named_records_do_not_inherit_credentials_by_index() {
        let existing = json!([
            {"id": "alpha", "env": {"TOKEN": "alpha-secret"}},
            {"id": "beta", "env": {"TOKEN": "beta-secret"}}
        ]);
        let incoming = json!([
            {"id": "new", "env": {"TOKEN": "[configured]"}},
            {"id": "beta", "env": {"TOKEN": "[configured]"}}
        ]);
        let merged = merge_redacted_settings(Some(&existing), &incoming);
        assert!(merged[0]["env"].get("TOKEN").is_none());
        assert_eq!(merged[1]["env"]["TOKEN"], json!("beta-secret"));
    }

    #[test]
    fn metadata_mutation_persists_and_survives_reload() {
        let (state, path) = test_state();
        let id = state.sessions.lock().keys().next().cloned().unwrap();
        {
            let mut sessions = state.sessions.lock();
            let record = sessions.get_mut(&id).unwrap();
            record.messages.push(MessageRecord {
                id: "metadata-message".to_string(),
                role: "user".to_string(),
                content: "leave this ciphertext untouched".to_string(),
                created_at: 1,
                turn_id: None,
                model: None,
                reasoning: None,
                finished_at: None,
                thinking: None,
            });
            record.message_count = 1;
            record.messages_loaded = true;
            record.messages_complete = true;
        }
        state.persist_session_locked(&id).unwrap();
        let ciphertext_before: String = rusqlite::Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT content FROM messages WHERE id = 'metadata-message'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let summary = mutate_session_and_persist(&state, &id, |record| {
            record.pinned_at = Some(10);
            record.archived_at = Some(11);
            record.provider_id = "gateway".to_string();
            record.model = "model-1".to_string();
            record.share_enabled = true;
            record.share_token = Some("share-test".to_string());
            Ok(history_summary(record))
        })
        .unwrap();
        assert!(summary.is_pinned);
        assert!(summary.is_archived);
        assert!(summary.is_shared);
        let loaded = state.history.load_sessions().unwrap();
        let loaded = loaded.iter().find(|record| record.id == id).unwrap();
        assert_eq!(loaded.provider_id, "gateway");
        assert_eq!(loaded.model, "model-1");
        assert_eq!(loaded.share_token.as_deref(), Some("share-test"));
        let ciphertext_after: String = rusqlite::Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT content FROM messages WHERE id = 'metadata-message'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ciphertext_after, ciphertext_before);
        cleanup_test_db(&path);
    }

    #[test]
    fn workspace_capability_is_read_only_and_session_bound() {
        let (state, path) = test_state();
        let (id, workdir) = {
            let record = state.sessions.lock().values().next().unwrap().clone();
            (
                record.summary.id,
                canonical_workdir(&record.summary.cwd).unwrap(),
            )
        };
        register_test_project(&state, &workdir);
        let token = "workspace-cap-test";
        state.workspace_capabilities.lock().insert(
            token.to_string(),
            WorkspaceCapabilityGrant {
                session_id: id.clone(),
                workdir: workdir.clone(),
                expires_at: Instant::now() + Duration::from_secs(60),
            },
        );
        assert!(
            require_read_capability(&state, Some(token), &workdir.display().to_string()).is_ok()
        );
        assert!(require_capability_for(
            &state,
            Some(token),
            &workdir.display().to_string(),
            ToolAction::Write,
            Some("tool")
        )
        .is_err());
        assert!(consume_git_commit_capability(
            &state,
            Some(token),
            &workdir.display().to_string(),
            "commit from read token"
        )
        .is_err());
        state.sessions.lock().remove(&id);
        assert!(
            require_read_capability(&state, Some(token), &workdir.display().to_string()).is_err()
        );
        cleanup_test_db(&path);
    }

    #[test]
    fn git_porcelain_status_parses_branch_counts_and_rename_records() {
        let output = b"## main...origin/main [ahead 2, behind 1]\0M  staged.txt\0 M unstaged.txt\0?? new.txt\0R  renamed.txt\0old-name.txt\0";
        let (branch, ahead, behind, entries) =
            parse_git_status(output).expect("porcelain v1 output should parse");
        assert_eq!(branch.as_deref(), Some("main"));
        assert_eq!(ahead, 2);
        assert_eq!(behind, 1);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].path, "staged.txt");
        assert_eq!(entries[1].worktree_status, "M");
        assert_eq!(entries[2].index_status, "?");
        assert_eq!(entries[3].path, "renamed.txt");
    }

    #[test]
    fn git_commit_message_is_bounded_and_never_allows_nul() {
        assert_eq!(
            normalized_git_commit_message("  Keep the Git Review usable  ").unwrap(),
            "Keep the Git Review usable"
        );
        assert!(normalized_git_commit_message("\0").is_err());
        assert!(
            normalized_git_commit_message(&"x".repeat(MAX_GIT_COMMIT_MESSAGE_CHARS + 1)).is_err()
        );
    }

    #[test]
    fn segmented_history_uses_the_explicit_conversation_id_and_round_trips() {
        let (state, path) = test_state();
        let cwd = std::env::current_dir().unwrap().display().to_string();
        let initial_count = state.sessions.lock().len();
        let first: ChatHistorySegmentMutationInput = serde_json::from_value(json!({
            "conversation": {
                "id": "explicit-conversation",
                "title": "Explicit",
                "providerId": "gateway",
                "model": "model-1",
                "sessionId": "pi-session",
                "cwd": cwd,
                "selectedModelJson": "{\"customProviderId\":\"gateway\",\"model\":\"model-1\"}",
                "contextMetaJson": "{\"schemaVersion\":3}",
                "activeSegmentIndex": 0,
                "totalSegmentCount": 1,
                "totalMessageCount": 1,
                "createdAt": 10,
                "updatedAt": 20
            },
            "segment": {
                "segmentIndex": 0,
                "segmentId": "segment-0",
                "summaryJson": null,
                "messagesJson": "[{\"role\":\"user\",\"content\":\"hello\"}]",
                "messageCount": 1,
                "startMessageId": "message-0",
                "endMessageId": "message-0",
                "createdAt": 10,
                "updatedAt": 20
            }
        }))
        .unwrap();
        let summary = mutate_segmented_history(&state, first, false).unwrap();
        assert_eq!(summary.id, "explicit-conversation");
        assert_eq!(state.sessions.lock().len(), initial_count + 1);
        assert!(state.sessions.lock().contains_key("explicit-conversation"));

        let second: ChatHistorySegmentMutationInput = serde_json::from_value(json!({
            "conversation": {
                "id": "explicit-conversation",
                "title": "Explicit",
                "providerId": "gateway",
                "model": "model-1",
                "sessionId": "pi-session",
                "cwd": std::env::current_dir().unwrap().display().to_string(),
                "contextMetaJson": "{\"schemaVersion\":3,\"activeSegmentIndex\":1}",
                "activeSegmentIndex": 1,
                "totalSegmentCount": 2,
                "totalMessageCount": 2,
                "createdAt": 10,
                "updatedAt": 30
            },
            "segment": {
                "segmentIndex": 1,
                "segmentId": "segment-1",
                "messagesJson": "[{\"role\":\"assistant\",\"content\":\"world\"}]",
                "messageCount": 1,
                "createdAt": 21,
                "updatedAt": 30
            }
        }))
        .unwrap();
        let summary = mutate_segmented_history(&state, second, true).unwrap();
        assert_eq!(summary.message_count, 2);
        let stored = state
            .history
            .load_segmented_history("explicit-conversation")
            .unwrap()
            .unwrap();
        assert_eq!(stored.header.session_id.as_deref(), Some("pi-session"));
        assert_eq!(
            stored.segments[0].messages_json,
            "[{\"role\":\"user\",\"content\":\"hello\"}]"
        );
        assert_eq!(stored.segments[1].segment_id, "segment-1");
        cleanup_test_db(&path);
    }

    #[test]
    fn segmented_history_rejects_unknown_wire_fields() {
        let parsed = serde_json::from_value::<ChatHistorySegmentMutationInput>(json!({
            "conversation": {
                "id": "conversation",
                "title": "title",
                "providerId": "provider",
                "model": "model",
                "contextMetaJson": "{}",
                "activeSegmentIndex": 0,
                "totalSegmentCount": 1,
                "totalMessageCount": 0,
                "updatedAt": 1,
                "unsupported": true
            },
            "segment": {
                "segmentIndex": 0,
                "segmentId": "segment",
                "messagesJson": "[]",
                "messageCount": 0,
                "createdAt": 1,
                "updatedAt": 1
            }
        }));
        assert!(parsed.is_err());
    }

    #[test]
    fn terminal_assistant_projection_is_idempotent_before_finalization() {
        let (state, path) = test_state();
        state.persist().unwrap();
        let session_id = state.sessions.lock().keys().next().cloned().unwrap();
        let run = ActiveRun {
            session_id: session_id.clone(),
            conversation_id: session_id.clone(),
            turn_id: "turn-terminal-projection".to_string(),
            request_id: "request-terminal-projection".to_string(),
            proxy_provider_id: None,
            capability_token: "cap-terminal-projection".to_string(),
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let payload = json!({
            "type": "done",
            "text": "durable answer",
            "timestamp": 42
        });
        let _guard = state.persist_lock.lock();
        persist_terminal_projection_locked(&state, &run, &payload).unwrap();
        persist_terminal_projection_locked(&state, &run, &payload).unwrap();
        drop(_guard);
        let messages = state.sessions.lock()[&session_id].messages.clone();
        let assistants = messages
            .iter()
            .filter(|message| message.turn_id.as_deref() == Some("turn-terminal-projection"))
            .collect::<Vec<_>>();
        assert_eq!(assistants.len(), 1);
        assert_eq!(assistants[0].id, "assistant:request-terminal-projection");
        assert_eq!(
            state
                .history
                .load_messages(&session_id)
                .unwrap()
                .iter()
                .filter(|message| message.turn_id.as_deref() == Some("turn-terminal-projection"))
                .count(),
            1
        );
        cleanup_test_db(&path);
    }

    #[test]
    fn history_search_is_bounded_recent_first_and_excludes_tool_content() {
        let mut session =
            new_session_record("Search\nSession".to_string(), "C:\\workspace".to_string());
        session.summary.id = "session-search".to_string();
        for index in 0..60 {
            session.messages.push(MessageRecord {
                id: format!("message-{index}"),
                role: if index % 2 == 0 {
                    "user".to_string()
                } else {
                    "assistant".to_string()
                },
                content: format!(
                    "entry {index} needle {} tail-secret-{index}",
                    "x".repeat(MAX_HISTORY_SEARCH_PREVIEW_CHARS * 2)
                ),
                created_at: index,
                turn_id: Some(format!("turn-{index}")),
                model: None,
                reasoning: None,
                finished_at: None,
                thinking: None,
            });
        }
        session.messages.push(MessageRecord {
            id: "tool-message".to_string(),
            role: "tool".to_string(),
            content: "needle tool-secret raw-result".to_string(),
            created_at: 100,
            turn_id: Some("turn-tool".to_string()),
            model: None,
            reasoning: None,
            finished_at: None,
            thinking: None,
        });
        let sessions = HashMap::from([(session.summary.id.clone(), session)]);

        let mut index = InMemoryHistorySearchIndex::default();
        let matches = history_search_matches(&mut index, &sessions, "  NEEDLE\n", Some(500), None);
        assert_eq!(matches.len(), MAX_HISTORY_SEARCH_RESULTS);
        assert_eq!(matches[0]["updatedAt"], json!(59));
        assert_eq!(matches[0]["conversationTitle"], json!("Search Session"));
        assert!(matches
            .iter()
            .all(|entry| matches!(entry["role"].as_str(), Some("user" | "assistant"))));
        assert!(matches.iter().all(|entry| {
            entry["text"].as_str().is_some_and(|text| {
                text.contains("needle")
                    && text.chars().count() <= MAX_HISTORY_SEARCH_PREVIEW_CHARS + 1
            })
        }));
        let serialized = serde_json::to_string(&matches).unwrap();
        assert!(!serialized.contains("tool-secret"));
        assert!(!serialized.contains("tail-secret"));
        assert!(history_search_matches(&mut index, &sessions, " \n\t", Some(12), None).is_empty());
        assert_eq!(
            history_search_normalize(
                &"q".repeat(MAX_HISTORY_SEARCH_QUERY_CHARS + 20),
                MAX_HISTORY_SEARCH_QUERY_CHARS,
            )
            .chars()
            .count(),
            MAX_HISTORY_SEARCH_QUERY_CHARS
        );
        let excessively_padded_query = format!(
            "{}needle",
            " ".repeat(MAX_HISTORY_SEARCH_QUERY_CHARS * 8 + 1)
        );
        assert!(history_search_normalize(
            &excessively_padded_query,
            MAX_HISTORY_SEARCH_QUERY_CHARS,
        )
        .is_empty());
    }

    #[test]
    fn metadata_search_normalizes_path_separators_and_projects_status() {
        let workdir = fs::canonicalize(current_workdir()).unwrap();
        let workdir = path_for_display(&workdir);
        let mut session = new_session_record("Metadata search".to_string(), workdir.clone());
        session.summary.id = "metadata-search-session".to_string();
        session.model = "gpt-metadata".to_string();
        let sessions = HashMap::from([(session.summary.id.clone(), session)]);
        let slash_query = workdir.replace('\\', "/");

        let matches = session_metadata_search_matches(&sessions, &[], &slash_query, Some(8));
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].conversation_id.as_deref(),
            Some("metadata-search-session")
        );
        let status = matches[0]
            .workspace_status
            .as_ref()
            .expect("metadata result should project its path status");
        assert!(status.accessible);
        assert_eq!(status.path, workdir);
        let serialized = serde_json::to_value(&matches[0]).unwrap();
        assert_eq!(serialized["workspaceStatus"]["accessible"], json!(true));
    }

    #[test]
    fn historical_trace_is_reply_scoped_and_never_serializes_tool_content() {
        let (state, path) = test_state();
        let session_id = state.sessions.lock().keys().next().cloned().unwrap();
        {
            let mut sessions = state.sessions.lock();
            sessions
                .get_mut(&session_id)
                .unwrap()
                .messages
                .push(MessageRecord {
                    id: "assistant-trace-message".to_string(),
                    role: "assistant".to_string(),
                    content: "durable answer".to_string(),
                    created_at: 10,
                    turn_id: Some("turn-trace".to_string()),
                    model: None,
                    reasoning: None,
                    finished_at: None,
                    thinking: None,
                });
        }
        state.persist().unwrap();
        state
            .history
            .upsert_turn(
                &session_id,
                &session_id,
                "turn-trace",
                "request-trace",
                "running",
                Some("provider"),
                Some("model"),
                None,
                None,
                10,
            )
            .unwrap();
        state
            .history
            .append_event(
                &json!({
                    "type": "tool_call",
                    "sessionId": session_id,
                    "turnId": "turn-trace",
                    "requestId": "request-trace",
                    "sequence": 1,
                    "toolCall": {
                        "id": "tool-trace",
                        "name": "Read",
                        "arguments": {"path": "secret.txt", "authorization": "raw-secret"}
                    }
                }),
                11,
            )
            .unwrap();
        state
            .history
            .append_event(
                &json!({
                    "type": "tool_result",
                    "sessionId": session_id,
                    "turnId": "turn-trace",
                    "requestId": "request-trace",
                    "sequence": 2,
                    "toolCall": {
                        "id": "tool-trace",
                        "name": "Read",
                        "status": "completed",
                        "result": "raw-secret result"
                    }
                }),
                12,
            )
            .unwrap();
        state
            .history
            .append_event(
                &json!({
                    "type": "done",
                    "sessionId": session_id,
                    "turnId": "turn-trace",
                    "requestId": "request-trace",
                    "sequence": 3,
                    "text": "durable answer"
                }),
                13,
            )
            .unwrap();

        let trace = load_chat_history_trace(
            &state,
            ChatHistoryTraceInput {
                session_id: session_id.clone(),
                message_id: "assistant-trace-message".to_string(),
                turn_id: "turn-trace".to_string(),
            },
        )
        .unwrap();
        assert_eq!(trace.session_id, session_id);
        assert_eq!(trace.turn_id, "turn-trace");
        assert_eq!(trace.status, "completed");
        assert_eq!(trace.tools.len(), 1);
        assert_eq!(trace.tools[0].name, "Read");
        assert_eq!(trace.tools[0].status, "completed");
        let serialized = serde_json::to_string(&trace).unwrap();
        for forbidden in [
            "raw-secret",
            "arguments",
            "result",
            "request-trace",
            "tool-trace",
        ] {
            assert!(!serialized.contains(forbidden), "trace leaked {forbidden}");
        }

        assert!(load_chat_history_trace(
            &state,
            ChatHistoryTraceInput {
                session_id: session_id.clone(),
                message_id: "wrong-message".to_string(),
                turn_id: "turn-trace".to_string(),
            },
        )
        .is_err());
        assert!(load_chat_history_trace(
            &state,
            ChatHistoryTraceInput {
                session_id,
                message_id: "assistant-trace-message".to_string(),
                turn_id: "later-turn".to_string(),
            },
        )
        .is_err());
        assert!(serde_json::from_value::<ChatHistoryTraceInput>(json!({
            "sessionId": "session",
            "messageId": "message",
            "turnId": "turn",
            "unexpected": true
        }))
        .is_err());
        cleanup_test_db(&path);
    }

    #[test]
    fn mutation_capability_recheck_observes_cancellation_and_run_removal() {
        let (state, path) = test_state();
        let session = state.sessions.lock().values().next().unwrap().clone();
        let workdir = canonical_workdir(&session.summary.cwd).unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let run = ActiveRun {
            session_id: session.summary.id.clone(),
            conversation_id: session.summary.id.clone(),
            turn_id: "turn-recheck".to_string(),
            request_id: "request-recheck".to_string(),
            proxy_provider_id: None,
            capability_token: "cap-recheck".to_string(),
            cancelled: cancelled.clone(),
        };
        state
            .active_runs
            .lock()
            .insert(run.request_id.clone(), run.clone());
        state.capabilities.lock().insert(
            run.capability_token.clone(),
            CapabilityGrant {
                session_id: run.session_id.clone(),
                conversation_id: run.conversation_id.clone(),
                turn_id: run.turn_id.clone(),
                request_id: run.request_id.clone(),
                workdir: workdir.clone(),
                permission_mode: PermissionMode::Full,
                expires_at: Instant::now() + Duration::from_secs(60),
                cancelled: cancelled.clone(),
            },
        );
        assert!(recheck_mutation_capability(
            &state,
            Some("cap-recheck"),
            &workdir,
            ToolAction::Write,
        )
        .is_ok());
        cancelled.store(true, Ordering::SeqCst);
        assert!(recheck_mutation_capability(
            &state,
            Some("cap-recheck"),
            &workdir,
            ToolAction::Write,
        )
        .is_err());
        cancelled.store(false, Ordering::SeqCst);
        state.active_runs.lock().clear();
        assert!(recheck_mutation_capability(
            &state,
            Some("cap-recheck"),
            &workdir,
            ToolAction::Write,
        )
        .is_err());
        cleanup_test_db(&path);
    }

    #[test]
    fn proxy_transport_authorization_requires_an_active_unexpired_uncancelled_grant() {
        let (state, path) = test_state();
        let session = state.sessions.lock().values().next().unwrap().clone();
        state
            .persist_locked()
            .expect("proxy transport test session should persist");
        state.settings.lock().insert(
            "providers".to_string(),
            json!([{
                "id": "gateway",
                "enabled": true,
                "baseUrl": "https://api.example.test/v1"
            }]),
        );
        let cancelled = Arc::new(AtomicBool::new(false));
        let run = ActiveRun {
            session_id: session.summary.id.clone(),
            conversation_id: session.summary.id.clone(),
            turn_id: "proxy-transport-turn".to_string(),
            request_id: "proxy-transport-request".to_string(),
            proxy_provider_id: Some("gateway".to_string()),
            capability_token: "proxy-transport-capability".to_string(),
            cancelled: cancelled.clone(),
        };
        state
            .active_runs
            .lock()
            .insert(run.request_id.clone(), run.clone());
        state.capabilities.lock().insert(
            run.capability_token.clone(),
            CapabilityGrant {
                session_id: run.session_id.clone(),
                conversation_id: run.conversation_id.clone(),
                turn_id: run.turn_id.clone(),
                request_id: run.request_id.clone(),
                workdir: canonical_workdir(&session.summary.cwd).unwrap(),
                permission_mode: PermissionMode::Full,
                expires_at: Instant::now() + Duration::from_secs(60),
                cancelled: cancelled.clone(),
            },
        );

        assert!(proxy_transport_authorized(
            &state,
            &run.request_id,
            &run.capability_token,
        ));
        assert_eq!(
            proxy_transport_grant(&state, &run.request_id, &run.capability_token)
                .expect("live parent should retain its native provider binding")
                .provider_id,
            "gateway"
        );
        assert!(!proxy_transport_authorized(
            &state,
            &run.request_id,
            "wrong-capability",
        ));

        let child_task_id = "subtask-proxy-transport".to_string();
        state
            .subagent_tasks
            .create_task(NewSubagentTask {
                id: child_task_id.clone(),
                session_id: run.session_id.clone(),
                agent_id: "proxy-test-agent".to_string(),
                parent_turn_id: run.turn_id.clone(),
                parent_request_id: run.request_id.clone(),
                title: "Proxy transport child".to_string(),
                created_at: now_ms(),
            })
            .unwrap();
        state
            .subagent_tasks
            .mark_running(&child_task_id, now_ms())
            .unwrap();
        let child_token = "subagent-proxy-transport-capability";
        let child_request_id =
            subagent_proxy_request_id(SubagentCapabilityMode::Readonly, &child_task_id);
        state.subagent_capabilities.lock().insert(
            child_token.to_string(),
            SubagentCapabilityGrant {
                task_id: child_task_id,
                session_id: run.session_id.clone(),
                parent_turn_id: run.turn_id.clone(),
                parent_request_id: run.request_id.clone(),
                proxy_request_id: child_request_id.clone(),
                workdir: canonical_workdir(&session.summary.cwd).unwrap(),
                mode: SubagentCapabilityMode::Readonly,
                allow_global_read: false,
                expires_at: Instant::now() + Duration::from_secs(60),
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        );
        assert!(proxy_transport_authorized(
            &state,
            &child_request_id,
            child_token,
        ));
        assert_eq!(
            proxy_transport_grant(&state, &child_request_id, child_token)
                .expect("live child should inherit the parent's native provider binding")
                .provider_id,
            "gateway"
        );
        assert!(!proxy_transport_authorized(
            &state,
            "subagent-run-wrong-task",
            child_token,
        ));

        cancelled.store(true, Ordering::SeqCst);
        assert!(!proxy_transport_authorized(
            &state,
            &run.request_id,
            &run.capability_token,
        ));
        assert!(!proxy_transport_authorized(
            &state,
            &child_request_id,
            child_token,
        ));
        cancelled.store(false, Ordering::SeqCst);
        state
            .capabilities
            .lock()
            .get_mut(&run.capability_token)
            .unwrap()
            .expires_at = Instant::now() - Duration::from_secs(1);
        assert!(!proxy_transport_authorized(
            &state,
            &run.request_id,
            &run.capability_token,
        ));
        state.active_runs.lock().remove(&run.request_id);
        assert!(!proxy_transport_authorized(
            &state,
            &run.request_id,
            &run.capability_token,
        ));

        cleanup_test_db(&path);
    }

    #[test]
    fn terminal_event_revokes_existing_proxy_transport_grants() {
        let (state, path) = test_state();
        let session = state.sessions.lock().values().next().unwrap().clone();
        state
            .persist_locked()
            .expect("terminal proxy test session should persist");
        let workdir = canonical_workdir(&session.summary.cwd).unwrap();
        state.settings.lock().insert(
            "providers".to_string(),
            json!([{
                "id": "gateway",
                "enabled": true,
                "baseUrl": "https://api.example.test/v1"
            }]),
        );
        let cancelled = Arc::new(AtomicBool::new(false));
        let run = ActiveRun {
            session_id: session.summary.id.clone(),
            conversation_id: session.summary.id.clone(),
            turn_id: "proxy-terminal-turn".to_string(),
            request_id: "proxy-terminal-request".to_string(),
            proxy_provider_id: Some("gateway".to_string()),
            capability_token: "proxy-terminal-capability".to_string(),
            cancelled: cancelled.clone(),
        };
        state
            .active_runs
            .lock()
            .insert(run.request_id.clone(), run.clone());
        state.capabilities.lock().insert(
            run.capability_token.clone(),
            CapabilityGrant {
                session_id: run.session_id.clone(),
                conversation_id: run.conversation_id.clone(),
                turn_id: run.turn_id.clone(),
                request_id: run.request_id.clone(),
                workdir,
                permission_mode: PermissionMode::Full,
                expires_at: Instant::now() + Duration::from_secs(60),
                cancelled: cancelled.clone(),
            },
        );
        let app = tauri::test::mock_builder()
            .manage(Arc::new(state))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let state = app.state::<Arc<AppState>>();
        assert!(proxy_transport_authorized(
            &state,
            &run.request_id,
            &run.capability_token,
        ));

        agent_emit_event_inner(
            state.clone(),
            app.handle().clone(),
            json!({
                "type": "done",
                "sessionId": run.session_id,
                "conversationId": run.conversation_id,
                "turnId": run.turn_id,
                "requestId": run.request_id,
                "text": "finished"
            }),
        )
        .expect("terminal event should persist");
        assert!(cancelled.load(Ordering::SeqCst));
        assert!(
            !proxy_transport_authorized(
                &state,
                "proxy-terminal-request",
                "proxy-terminal-capability"
            ),
            "a terminal run cannot retain a proxy authorization"
        );
        cleanup_test_db(&path);
    }

    #[test]
    fn picked_workspace_files_return_relative_paths_and_reject_outside_files() {
        let root = std::env::temp_dir().join(format!(
            "novavei-picker-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let nested = root.join("nested");
        let inside = nested.join("file.txt");
        let outside = root.with_file_name(format!(
            "{}-outside",
            root.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("picker")
        ));
        fs::create_dir_all(&nested).unwrap();
        fs::write(&inside, "hello").unwrap();
        fs::write(&outside, "outside").unwrap();
        let canonical = canonical_workdir(&root.display().to_string()).unwrap();
        let selected =
            validate_picked_workspace_files(&canonical, vec![fs::canonicalize(&inside).unwrap()])
                .unwrap();
        assert_eq!(selected[0].path, "nested/file.txt");
        assert_eq!(selected[0].name, "file.txt");
        assert!(validate_picked_workspace_files(&canonical, vec![outside.clone()]).is_err());
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_file(outside);
    }

    #[cfg(windows)]
    #[test]
    fn workspace_relative_path_reconciles_verbatim_and_picker_paths() {
        let root = PathBuf::from(r"\\?\C:\workspace");
        let selected = PathBuf::from(r"C:\WORKSPACE\nested\file.txt");
        let sibling = PathBuf::from(r"C:\workspace-other\file.txt");

        assert_eq!(
            workspace_relative_path(&selected, &root, "selected file").unwrap(),
            PathBuf::from(r"nested\file.txt")
        );
        assert!(workspace_relative_path(&sibling, &root, "selected file").is_err());
    }

    #[test]
    fn atomic_file_replace_keeps_target_intact_on_success() {
        let path = std::env::temp_dir().join(format!(
            "novavei-atomic-{}-{}.txt",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::write(&path, b"before").unwrap();
        atomic_replace_file(&path, b"after").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "after");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn provider_model_probe_uses_protocol_specific_paths() {
        assert_eq!(
            provider_models_url("https://api.openai.com/v1", "openai")
                .unwrap()
                .as_str(),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            provider_models_url("https://api.anthropic.com", "anthropic-messages")
                .unwrap()
                .as_str(),
            "https://api.anthropic.com/v1/models"
        );
        assert_eq!(
            provider_models_url(
                "https://generativelanguage.googleapis.com/v1beta",
                "google-generative-ai"
            )
            .unwrap()
            .as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models"
        );
    }

    #[test]
    fn provider_model_discovery_classifies_only_supported_catalogue_protocols() {
        assert_eq!(
            provider_model_list_protocol("openai-responses"),
            Some(ProviderModelListProtocol::OpenAiCompatible)
        );
        assert_eq!(
            provider_model_list_protocol("codex"),
            Some(ProviderModelListProtocol::OpenAiCompatible)
        );
        assert_eq!(
            provider_model_list_protocol("anthropic-messages"),
            Some(ProviderModelListProtocol::Anthropic)
        );
        assert_eq!(
            provider_model_list_protocol("google-generative-ai"),
            Some(ProviderModelListProtocol::Gemini)
        );
        assert_eq!(provider_model_list_protocol("ollama"), None);
    }

    #[test]
    fn gemini_discovery_normalizes_resource_names_for_pi() {
        let page = provider_model_page(
            &json!({
                "models": [{
                    "name": "models/gemini-2.5-pro",
                    "displayName": "Gemini 2.5 Pro",
                    "inputTokenLimit": 1_048_576,
                    "outputTokenLimit": 65_536,
                    "description": "not returned to the renderer",
                    "apiKey": "not returned to the renderer"
                }]
            }),
            ProviderModelListProtocol::Gemini,
            &[],
        )
        .unwrap();
        assert_eq!(page.models.len(), 1);
        assert_eq!(page.models[0].id, "gemini-2.5-pro");
        assert_eq!(page.models[0].label.as_deref(), Some("Gemini 2.5 Pro"));
        assert_eq!(page.models[0].context_window, Some(1_048_576));
        assert_eq!(page.models[0].max_output_token, Some(65_536));
        assert_eq!(
            serde_json::to_value(&page.models[0]).unwrap(),
            json!({
                "id": "gemini-2.5-pro",
                "label": "Gemini 2.5 Pro",
                "contextWindow": 1_048_576,
                "maxOutputToken": 65_536
            })
        );
    }

    #[test]
    fn provider_model_metadata_is_allowlisted_bounded_and_paginated() {
        let oversized_id = "x".repeat(MAX_PROVIDER_MODEL_ID_BYTES + 1);
        let page = provider_model_page(
            &json!({
                "data": [
                    {
                        "id": "claude-sonnet",
                        "display_name": "Claude Sonnet\n",
                        "owned_by": "secret-owner",
                        "description": "secret-description",
                        "headers": {"Authorization": "secret-header"}
                    },
                    {"id": oversized_id}
                ],
                "has_more": true,
                "last_id": "claude-sonnet"
            }),
            ProviderModelListProtocol::Anthropic,
            &[],
        )
        .unwrap();
        assert_eq!(page.models.len(), 1);
        assert_eq!(page.models[0].id, "claude-sonnet");
        assert_eq!(page.models[0].label.as_deref(), Some("Claude Sonnet"));
        assert_eq!(page.next_cursor.as_deref(), Some("claude-sonnet"));
        assert!(page.has_more);
        let rendered = serde_json::to_string(&page.models).unwrap();
        assert!(!rendered.contains("secret-owner"));
        assert!(!rendered.contains("secret-description"));
        assert!(!rendered.contains("secret-header"));
        assert_eq!(
            provider_models_page_url(
                "https://api.anthropic.com",
                "anthropic-messages",
                ProviderModelListProtocol::Anthropic,
                page.next_cursor.as_deref(),
            )
            .unwrap()
            .as_str(),
            "https://api.anthropic.com/v1/models?limit=100&after_id=claude-sonnet"
        );
    }

    #[test]
    fn provider_model_metadata_never_reflects_configured_credentials() {
        let secrets = provider_model_secret_values(&[
            (
                "authorization".to_string(),
                "Bearer native-secret".to_string(),
            ),
            ("x-client-token".to_string(), "header-secret".to_string()),
        ]);
        let page = provider_model_page(
            &json!({
                "data": [
                    {"id": "gpt-safe", "display_name": "Safe native-secret header-secret model"},
                    {"id": "gpt-native-secret"}
                ]
            }),
            ProviderModelListProtocol::OpenAiCompatible,
            &secrets,
        )
        .unwrap();
        assert_eq!(page.models.len(), 1);
        assert_eq!(page.models[0].id, "gpt-safe");
        assert_eq!(
            page.models[0].label.as_deref(),
            Some("Safe [redacted] [redacted] model")
        );
        let rendered = serde_json::to_string(&page.models).unwrap();
        assert!(!rendered.contains("native-secret"));
        assert!(!rendered.contains("header-secret"));
    }

    #[test]
    fn unsupported_provider_catalogue_never_needs_saved_credentials() {
        let settings = HashMap::from([(
            "providers".to_string(),
            json!([{
                "id": "local-ollama",
                "type": "ollama",
                "baseUrl": "http://127.0.0.1:11434/v1",
                "apiKey": "native-secret"
            }]),
        )]);
        let config = provider_model_probe_config_from_settings(&settings, "local-ollama").unwrap();
        assert_eq!(config.list_protocol, None);
        assert!(config.credentials.is_empty());
        assert_eq!(config.base_url, "http://127.0.0.1:11434/v1");
        assert!(provider_model_probe_config_from_settings(&settings, "../secret").is_err());
    }

    #[test]
    fn redaction_hides_object_style_header_values() {
        let value = redact_settings_secrets(&json!({
            "headers": {"Authorization": "Bearer native-secret", "X-Client": "visible-name-only"}
        }));
        assert_eq!(value["headers"]["Authorization"], json!("[configured]"));
        assert_eq!(value["headers"]["X-Client"], json!("[configured]"));
    }

    #[test]
    fn event_redaction_preserves_usage_but_hides_credentials() {
        let value = redact_event_value(
            &json!({
                "usage": {"inputTokens": 12, "outputTokens": 4, "totalTokens": 16},
                "capabilityToken": "native-capability",
                "customHeaders": [{"key": "X-Client", "value": "header-secret"}],
                "tokenCount": 16
            }),
            false,
        );
        assert_eq!(value["usage"]["totalTokens"], json!(16));
        assert_eq!(value["tokenCount"], json!(16));
        assert_eq!(value["capabilityToken"], json!("[redacted]"));
        assert_eq!(value["customHeaders"][0]["value"], json!("[redacted]"));
        assert_eq!(value["customHeaders"][0]["key"], json!("X-Client"));
    }

    #[test]
    fn sessions_get_enriches_assistant_messages_from_turn_metadata() {
        let (state, path) = test_state();
        let session_id = "session-meta-enrich".to_string();
        {
            let mut sessions = state.sessions.lock();
            sessions.insert(
                session_id.clone(),
                SessionRecord {
                    summary: SessionSummary {
                        id: session_id.clone(),
                        title: "meta".to_string(),
                        cwd: current_workdir(),
                        updated_at: 1,
                        message_count: 2,
                        is_pinned: false,
                        is_archived: false,
                        last_run_status: None,
                        last_run_finished_at: None,
                    },
                    messages: vec![
                        MessageRecord {
                            id: "user-1".to_string(),
                            role: "user".to_string(),
                            content: "hi".to_string(),
                            created_at: 1,
                            turn_id: Some("turn-meta".to_string()),
                            model: None,
                            reasoning: None,
                            finished_at: None,
                            thinking: None,
                        },
                        MessageRecord {
                            id: "assistant-1".to_string(),
                            role: "assistant".to_string(),
                            content: "hello".to_string(),
                            created_at: 2,
                            turn_id: Some("turn-meta".to_string()),
                            model: None,
                            reasoning: None,
                            finished_at: None,
                            thinking: None,
                        },
                    ],
                    message_count: 2,
                    messages_loaded: true,
                    messages_complete: true,
                    provider_id: "embedded".to_string(),
                    model: "session-model".to_string(),
                    selected_model_json: None,
                    pinned_at: None,
                    archived_at: None,
                    share_enabled: false,
                    share_token: None,
                    share_created_at: None,
                    share_updated_at: None,
                    redact_tool_content: true,
                    goal: None,
                },
            );
        }
        state.persist().unwrap();
        state
            .history
            .upsert_turn(
                &session_id,
                &session_id,
                "turn-meta",
                "request-meta",
                "running",
                Some("newapi"),
                Some("grok-4.5"),
                Some("xhigh"),
                None,
                10,
            )
            .unwrap();
        state
            .history
            .append_event(
                &json!({
                    "type": "done",
                    "sessionId": session_id,
                    "turnId": "turn-meta",
                    "requestId": "request-meta",
                    "text": "hello",
                    "thinking": "checked the available context"
                }),
                99,
            )
            .unwrap();

        let messages = enrich_messages_with_turn_metadata(
            &state.history,
            &session_id,
            &state.sessions.lock().get(&session_id).unwrap().messages,
        )
        .unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert!(messages[0].model.is_none());
        assert!(messages[0].reasoning.is_none());
        assert!(messages[0].finished_at.is_none());
        assert!(messages[0].thinking.is_none());
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].model.as_deref(), Some("grok-4.5"));
        assert_eq!(messages[1].reasoning.as_deref(), Some("xhigh"));
        assert_eq!(messages[1].finished_at, Some(99));
        assert_eq!(
            messages[1].thinking.as_deref(),
            Some("checked the available context")
        );

        // Legacy turn without reasoning still loads and never invents UI fallback text.
        state
            .history
            .upsert_turn(
                &session_id,
                &session_id,
                "turn-legacy",
                "request-legacy",
                "running",
                Some("newapi"),
                Some("legacy-model"),
                None,
                None,
                11,
            )
            .unwrap();
        {
            let mut sessions = state.sessions.lock();
            sessions
                .get_mut(&session_id)
                .unwrap()
                .messages
                .push(MessageRecord {
                    id: "assistant-legacy".to_string(),
                    role: "assistant".to_string(),
                    content: "old".to_string(),
                    created_at: 12,
                    turn_id: Some("turn-legacy".to_string()),
                    model: None,
                    reasoning: None,
                    finished_at: None,
                    thinking: None,
                });
        }
        let messages = enrich_messages_with_turn_metadata(
            &state.history,
            &session_id,
            &state.sessions.lock().get(&session_id).unwrap().messages,
        )
        .unwrap();
        let legacy = messages
            .iter()
            .find(|m| m.id == "assistant-legacy")
            .unwrap();
        assert_eq!(legacy.model.as_deref(), Some("legacy-model"));
        assert!(legacy.reasoning.is_none());
        assert!(legacy.thinking.is_none());
        assert!(!format!("{:?}", legacy).contains("未记录"));

        assert!(validate_turn_reasoning(Some("nope".into())).is_err());
        assert_eq!(
            validate_turn_reasoning(Some("HIGH".into()))
                .unwrap()
                .as_deref(),
            Some("high")
        );

        cleanup_test_db(&path);
    }
}
