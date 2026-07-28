//! Small SQLite store for NovaVei conversations, settings, and Pi events.
//!
//! The WebView owns the live Agent object, but native storage is the durable
//! source of truth.  The store deliberately uses plain JSON payload columns so
//! Pi message/tool shapes can evolve without a migration for every provider.

use parking_lot::{Mutex, MutexGuard};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const SCHEMA_VERSION: i64 = 6;
const SECURE_SETTINGS_MIGRATION_KEY: &str = "settings_dpapi_v1_compacted";
const MESSAGES_DPAPI_MIGRATION_KEY: &str = "messages_dpapi_v1_compacted";
const SEGMENT_SUMMARIES_DPAPI_MIGRATION_KEY: &str = "segment_summaries_dpapi_v1_compacted";
const PRIVATE_EVENT_FIELDS_DPAPI_MIGRATION_KEY: &str = "private_event_fields_dpapi_v1_compacted";
pub const MAX_SESSION_GOAL_TEXT_CHARS: usize = 600;
/// Default number of newest messages returned for first UI paint.
pub const DEFAULT_UI_MESSAGE_PAGE_SIZE: usize = 80;
/// Hard ceiling for UI page size requests.
pub const MAX_UI_MESSAGE_PAGE_SIZE: usize = 200;
// Terminal thinking is renderer-visible transcript metadata. Keep enough for
// useful inspection without allowing one provider event to create an
// unbounded durable/UI payload.
const MAX_STORED_TERMINAL_THINKING_CHARS: usize = 32 * 1024;
const HISTORY_BUSY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub struct HistoryStore {
    path: PathBuf,
    /// SQLite is serialized by design here: every durable operation uses the
    /// same connection and transaction boundary, instead of reopening a file
    /// handle for each row in a partial cache flush.
    #[cfg_attr(test, allow(dead_code))]
    connection: Arc<Mutex<Option<Connection>>>,
}

/// A borrowed production connection or a short-lived test connection.
///
/// Unit tests deliberately use the latter so their temporary SQLite files can
/// be removed before the `HistoryStore` value leaves scope on Windows.
#[allow(dead_code)]
enum HistoryConnection<'a> {
    Cached(MutexGuard<'a, Option<Connection>>),
    Transient(Connection),
}

impl Deref for HistoryConnection<'_> {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Cached(connection) => connection
                .as_ref()
                .expect("history connection is initialized before use"),
            Self::Transient(connection) => connection,
        }
    }
}

impl DerefMut for HistoryConnection<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Cached(connection) => connection
                .as_mut()
                .expect("history connection is initialized before use"),
            Self::Transient(connection) => connection,
        }
    }
}

/// Export-safe history metadata. This projection intentionally excludes
/// session identifiers, titles, workspace paths, messages, tool data, and
/// provider configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryDiagnosticsSummary {
    pub session_count: u64,
    pub turn_count: u64,
    pub turn_status_counts: BTreeMap<String, u64>,
    pub oldest_turn_started_at: Option<i64>,
    pub latest_turn_started_at: Option<i64>,
}

/// SQLite INTEGER values are signed. Counts must never be negative, so decode
/// them as `i64` and reject malformed database contents instead of relying on
/// a lossy or version-specific unsigned conversion.
fn nonnegative_sqlite_count(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let count: i64 = row.get(index)?;
    u64::try_from(count).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, count))
}

#[derive(Clone, Debug)]
pub struct StoredSession {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub updated_at: i64,
    pub provider_id: String,
    pub model: String,
    pub selected_model_json: Option<String>,
    pub pinned_at: Option<i64>,
    pub archived_at: Option<i64>,
    pub share_enabled: bool,
    pub share_token: Option<String>,
    pub share_created_at: Option<i64>,
    pub share_updated_at: Option<i64>,
    pub redact_tool_content: bool,
}

/// The latest terminal Pi result for one session. This intentionally exposes
/// only display-safe state and time, never provider output or error details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSessionRunSummary {
    pub status: String,
    pub finished_at: i64,
}

/// A deliberately small, session-owned goal record. It is separate from
/// transcript rows so opening a goal never loads conversation content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSessionGoal {
    pub text: String,
    pub status: String,
    pub progress: u8,
    pub updated_at: i64,
}

/// A local-only summary that replaces an older prefix only when a Pi context
/// is loaded. The original `messages` rows are deliberately never rewritten.
#[derive(Clone, Debug)]
pub struct StoredSessionContextCompaction {
    pub summary_text: String,
    pub source_message_count: usize,
    pub metadata: Value,
}

/// Native context plus the durable manual-compaction metadata that produced it.
/// The metadata is safe to expose to the renderer because it contains only
/// counts, ranges, a fingerprint, and a generation time.
#[derive(Clone, Debug)]
pub struct RuntimeContext {
    pub messages: Vec<Value>,
    pub manual_compaction: Option<Value>,
}

#[derive(Clone, Debug)]
pub struct StoredMessage {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub created_at: i64,
    pub turn_id: Option<String>,
}

/// One UI page of durable messages for a session.
///
/// `messages` are always delivered in chronological order (oldest → newest)
/// within the page so the renderer can append/prepend without re-sorting.
#[derive(Clone, Debug)]
pub struct MessagePage {
    pub messages: Vec<StoredMessage>,
    pub total_count: i64,
    /// Older messages exist before this page (toward the start of the transcript).
    pub has_more_before: bool,
}

/// A deliberately metadata-only projection of one durable Pi turn. The
/// renderer must never receive the stored tool arguments, results, errors, or
/// event payloads merely to show a historical trace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredTurnTrace {
    pub session_id: String,
    pub turn_id: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub tools: Vec<StoredTurnTraceTool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredTurnTraceTool {
    pub name: String,
    pub status: String,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

/// Per-turn display metadata used when projecting historical assistant messages.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct StoredTurnMetadata {
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub finished_at: Option<i64>,
    /// Display-safe, redacted thinking retained in the completed terminal event.
    pub thinking: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredHistoryHeader {
    pub conversation_id: String,
    pub session_id: Option<String>,
    pub context_meta_json: String,
    pub active_segment_index: i64,
    pub total_segment_count: i64,
    pub total_message_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredHistorySegment {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSegmentedHistory {
    pub header: StoredHistoryHeader,
    pub segments: Vec<StoredHistorySegment>,
}

impl HistoryStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            connection: Arc::new(Mutex::new(None)),
        }
    }

    /// Return aggregate turn health information without selecting any data
    /// that could identify a conversation, user, workspace, or model request.
    pub fn diagnostics_summary(&self) -> Result<HistoryDiagnosticsSummary, String> {
        let connection = self.open()?;
        let session_count = connection
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| {
                nonnegative_sqlite_count(row, 0)
            })
            .map_err(|error| format!("count diagnostic sessions: {error}"))?;
        let (turn_count, oldest_turn_started_at, latest_turn_started_at) = connection
            .query_row(
                "SELECT COUNT(*), MIN(started_at), MAX(started_at) FROM turns",
                [],
                |row| {
                    Ok((
                        nonnegative_sqlite_count(row, 0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                    ))
                },
            )
            .map_err(|error| format!("summarize diagnostic turns: {error}"))?;
        let mut statement = connection
            .prepare("SELECT status, COUNT(*) FROM turns GROUP BY status ORDER BY status")
            .map_err(|error| format!("prepare diagnostic turn statuses: {error}"))?;
        let statuses = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, nonnegative_sqlite_count(row, 1)?))
            })
            .map_err(|error| format!("query diagnostic turn statuses: {error}"))?;
        let mut turn_status_counts = BTreeMap::new();
        for status in statuses {
            let (status, count) =
                status.map_err(|error| format!("read diagnostic turn status: {error}"))?;
            turn_status_counts.insert(status, count);
        }
        Ok(HistoryDiagnosticsSummary {
            session_count,
            turn_count,
            turn_status_counts,
            oldest_turn_started_at,
            latest_turn_started_at,
        })
    }

    pub fn initialize(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create history directory: {error}"))?;
        }
        let connection = self.open()?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS schema_meta (
                   key TEXT PRIMARY KEY NOT NULL,
                   value TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS sessions (
                   id TEXT PRIMARY KEY NOT NULL,
                   title TEXT NOT NULL,
                   cwd TEXT NOT NULL,
                   created_at INTEGER NOT NULL,
                   updated_at INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS session_metadata (
                   session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                   provider_id TEXT NOT NULL DEFAULT 'embedded',
                   model TEXT NOT NULL DEFAULT '',
                   selected_model_json TEXT,
                   pinned_at INTEGER,
                   archived_at INTEGER,
                   share_enabled INTEGER NOT NULL DEFAULT 0,
                   share_token TEXT,
                   share_created_at INTEGER,
                   share_updated_at INTEGER,
                   redact_tool_content INTEGER NOT NULL DEFAULT 1
                 );
                 -- Added in schema v4. CREATE IF NOT EXISTS makes this a
                 -- forward-only, non-destructive migration for v3 databases.
                  CREATE TABLE IF NOT EXISTS session_goals (
                   session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                   text TEXT NOT NULL CHECK(length(text) BETWEEN 1 AND 600),
                   status TEXT NOT NULL CHECK(status IN ('active', 'completed')),
                   progress INTEGER NOT NULL CHECK(progress BETWEEN 0 AND 100),
                   updated_at INTEGER NOT NULL CHECK(updated_at > 0),
                    CHECK((status = 'active' AND progress < 100) OR (status = 'completed' AND progress = 100))
                  );
                  CREATE TABLE IF NOT EXISTS session_context_compactions (
                    session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                    summary_text TEXT NOT NULL,
                    source_message_count INTEGER NOT NULL CHECK(source_message_count > 0),
                    metadata_json TEXT NOT NULL,
                    created_at INTEGER NOT NULL CHECK(created_at > 0)
                  );
                  CREATE TABLE IF NOT EXISTS messages (
                   id TEXT PRIMARY KEY NOT NULL,
                   session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                   role TEXT NOT NULL,
                   content TEXT NOT NULL,
                   created_at INTEGER NOT NULL,
                   turn_id TEXT
                 );
                 CREATE INDEX IF NOT EXISTS messages_session_order
                   ON messages(session_id, created_at, id);
                 CREATE TABLE IF NOT EXISTS settings (
                   scope TEXT PRIMARY KEY NOT NULL,
                   payload_json TEXT NOT NULL,
                   updated_at INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS turns (
                   id TEXT PRIMARY KEY NOT NULL,
                   session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                   conversation_id TEXT NOT NULL,
                   request_id TEXT NOT NULL UNIQUE,
                   status TEXT NOT NULL,
                   provider_id TEXT,
                   model TEXT,
                   reasoning TEXT,
                   cwd TEXT,
                   started_at INTEGER NOT NULL,
                   finished_at INTEGER,
                   error TEXT,
                   usage_json TEXT
                 );
                 CREATE INDEX IF NOT EXISTS turns_session_finished_order
                   ON turns(session_id, finished_at DESC, started_at DESC, id DESC);
                 CREATE TABLE IF NOT EXISTS turn_events (
                   id TEXT PRIMARY KEY NOT NULL,
                   session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                   turn_id TEXT NOT NULL,
                   request_id TEXT NOT NULL,
                   sequence INTEGER,
                   event_type TEXT NOT NULL,
                   payload_json TEXT NOT NULL,
                   created_at INTEGER NOT NULL,
                   UNIQUE(request_id, sequence)
                 );
                 CREATE INDEX IF NOT EXISTS turn_events_turn_order
                   ON turn_events(turn_id, sequence, created_at);
                 CREATE INDEX IF NOT EXISTS turn_events_session_order
                   ON turn_events(session_id, created_at, sequence, id);
                 CREATE TABLE IF NOT EXISTS tool_calls (
                   id TEXT PRIMARY KEY NOT NULL,
                   turn_id TEXT NOT NULL,
                   session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                   name TEXT NOT NULL,
                   arguments_json TEXT,
                   status TEXT NOT NULL,
                   result_json TEXT,
                   error TEXT,
                   started_at INTEGER,
                   finished_at INTEGER
                 );
                 CREATE INDEX IF NOT EXISTS tool_calls_session_turn
                   ON tool_calls(session_id, turn_id);
                  CREATE TABLE IF NOT EXISTS permission_requests (
                   id TEXT PRIMARY KEY NOT NULL,
                   turn_id TEXT NOT NULL,
                   session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                   tool_name TEXT,
                   request_json TEXT NOT NULL,
                   decision TEXT,
                    requested_at INTEGER NOT NULL,
                    resolved_at INTEGER
                  );
                  CREATE INDEX IF NOT EXISTS permission_requests_session_order
                    ON permission_requests(session_id, requested_at, id);
                  CREATE TABLE IF NOT EXISTS history_segment_headers (
                    conversation_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                    source_session_id TEXT,
                    context_meta_json TEXT NOT NULL,
                    active_segment_index INTEGER NOT NULL,
                    total_segment_count INTEGER NOT NULL,
                    total_message_count INTEGER NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                  );
                  CREATE TABLE IF NOT EXISTS history_segments (
                    conversation_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                    segment_index INTEGER NOT NULL,
                    segment_id TEXT NOT NULL,
                    summary_json TEXT,
                    messages_json TEXT NOT NULL,
                    message_count INTEGER NOT NULL,
                    start_message_id TEXT,
                    end_message_id TEXT,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    PRIMARY KEY(conversation_id, segment_index),
                    UNIQUE(conversation_id, segment_id)
                  );",
            )
            .map_err(|error| format!("initialize history database: {error}"))?;
        // Schema v6: forward-only column for per-turn reasoning level.
        Self::ensure_turns_reasoning_column(&connection)?;
        connection
            .execute(
                "INSERT INTO schema_meta(key, value) VALUES('version', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![SCHEMA_VERSION.to_string()],
            )
            .map_err(|error| format!("write history schema version: {error}"))?;
        Ok(())
    }

    fn ensure_turns_reasoning_column(connection: &Connection) -> Result<(), String> {
        let mut statement = connection
            .prepare("PRAGMA table_info(turns)")
            .map_err(|error| format!("inspect turns schema: {error}"))?;
        let column_names = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|error| format!("query turns columns: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read turns columns: {error}"))?;
        if column_names.iter().any(|name| name == "reasoning") {
            return Ok(());
        }
        connection
            .execute("ALTER TABLE turns ADD COLUMN reasoning TEXT", [])
            .map_err(|error| format!("add turns.reasoning column: {error}"))?;
        Ok(())
    }

    fn open_connection(path: &PathBuf) -> Result<Connection, String> {
        let connection =
            Connection::open(path).map_err(|error| format!("open history database: {error}"))?;
        connection
            .busy_timeout(HISTORY_BUSY_TIMEOUT)
            .map_err(|error| format!("configure history database timeout: {error}"))?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA secure_delete = ON;",
            )
            .map_err(|error| format!("configure history database: {error}"))?;
        Ok(connection)
    }

    #[cfg(test)]
    fn open(&self) -> Result<HistoryConnection<'_>, String> {
        Self::open_connection(&self.path).map(HistoryConnection::Transient)
    }

    #[cfg(not(test))]
    fn open(&self) -> Result<HistoryConnection<'_>, String> {
        let mut connection = self.connection.lock();
        if connection.is_none() {
            *connection = Some(Self::open_connection(&self.path)?);
        }
        Ok(HistoryConnection::Cached(connection))
    }

    /// Remove plaintext settings remnants left by pre-DPAPI database pages.
    ///
    /// The schema marker makes the expensive VACUUM one-time and idempotent.
    /// A WAL checkpoint is still attempted when the marker already exists so
    /// a crash after writing the marker cannot leave a committed WAL behind.
    pub fn secure_compact_once(&self) -> Result<bool, String> {
        let completed = {
            let connection = self.open()?;
            connection
                .query_row(
                    "SELECT value FROM schema_meta WHERE key = ?1",
                    params![SECURE_SETTINGS_MIGRATION_KEY],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| format!("read secure settings migration marker: {error}"))?
                .is_some_and(|value| value == "1")
        };

        if completed {
            let connection = self.open()?;
            checkpoint_truncate(&connection)?;
            return Ok(false);
        }
        self.secure_compact_after_protection()?;
        let connection = self.open()?;
        connection
            .execute(
                "INSERT INTO schema_meta(key, value) VALUES(?1, '1')
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![SECURE_SETTINGS_MIGRATION_KEY],
            )
            .map_err(|error| format!("write secure settings migration marker: {error}"))?;
        Ok(true)
    }

    /// Force a WAL checkpoint and VACUUM after a portable unlock has replaced
    /// plaintext rows. Unlike [`secure_compact_once`], this intentionally does
    /// not consult a historical marker: a newly encrypted portable copy may
    /// inherit that marker while its current SQLite pages still contain
    /// plaintext remnants from the source database.
    pub fn secure_compact_after_protection(&self) -> Result<(), String> {
        let connection = self.open()?;
        checkpoint_truncate(&connection)?;
        connection
            .execute_batch("VACUUM;")
            .map_err(|error| format!("vacuum protected settings database: {error}"))?;
        checkpoint_truncate(&connection)
    }

    pub fn load_sessions(&self) -> Result<Vec<StoredSession>, String> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT sessions.id, sessions.title, sessions.cwd, sessions.updated_at,
                        COALESCE(session_metadata.provider_id, 'embedded'),
                        COALESCE(session_metadata.model, ''),
                        session_metadata.selected_model_json,
                        session_metadata.pinned_at,
                        session_metadata.archived_at,
                        COALESCE(session_metadata.share_enabled, 0),
                        session_metadata.share_token,
                        session_metadata.share_created_at,
                        session_metadata.share_updated_at,
                        COALESCE(session_metadata.redact_tool_content, 1)
                 FROM sessions
                 LEFT JOIN session_metadata ON session_metadata.session_id = sessions.id
                 ORDER BY sessions.updated_at DESC",
            )
            .map_err(|error| format!("prepare history sessions: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok(StoredSession {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    cwd: row.get(2)?,
                    updated_at: row.get(3)?,
                    provider_id: row.get(4)?,
                    model: row.get(5)?,
                    selected_model_json: row.get(6)?,
                    pinned_at: row.get(7)?,
                    archived_at: row.get(8)?,
                    share_enabled: row.get(9)?,
                    share_token: row.get(10)?,
                    share_created_at: row.get(11)?,
                    share_updated_at: row.get(12)?,
                    redact_tool_content: row.get(13)?,
                })
            })
            .map_err(|error| format!("query history sessions: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read history sessions: {error}"))
    }

    /// Load the latest durable terminal outcome for each session. The turn
    /// table is authoritative so reopening the desktop never relies on a
    /// transient WebView run map.
    pub fn load_session_run_summaries(
        &self,
    ) -> Result<HashMap<String, StoredSessionRunSummary>, String> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT session_id, status, finished_at
                 FROM (
                   SELECT session_id, status, finished_at,
                          ROW_NUMBER() OVER (
                            PARTITION BY session_id
                            ORDER BY finished_at DESC, started_at DESC, id DESC
                          ) AS session_rank
                   FROM turns
                   WHERE status IN ('completed', 'cancelled', 'error', 'interrupted')
                     AND finished_at IS NOT NULL
                 )
                 WHERE session_rank = 1",
            )
            .map_err(|error| format!("prepare session run summaries: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    StoredSessionRunSummary {
                        status: row.get(1)?,
                        finished_at: row.get(2)?,
                    },
                ))
            })
            .map_err(|error| format!("query session run summaries: {error}"))?;
        rows.collect::<Result<HashMap<_, _>, _>>()
            .map_err(|error| format!("read session run summaries: {error}"))
    }

    pub fn load_session_goals(&self) -> Result<HashMap<String, StoredSessionGoal>, String> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare("SELECT session_id, text, status, progress, updated_at FROM session_goals")
            .map_err(|error| format!("prepare session goals: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                let progress: i64 = row.get(3)?;
                Ok((
                    row.get::<_, String>(0)?,
                    StoredSessionGoal {
                        text: row.get(1)?,
                        status: row.get(2)?,
                        progress: u8::try_from(progress).unwrap_or(u8::MAX),
                        updated_at: row.get(4)?,
                    },
                ))
            })
            .map_err(|error| format!("query session goals: {error}"))?;
        rows.collect::<Result<HashMap<_, _>, _>>()
            .map_err(|error| format!("read session goals: {error}"))
    }

    pub fn upsert_session_goal(
        &self,
        session_id: &str,
        goal: &StoredSessionGoal,
    ) -> Result<(), String> {
        validate_stored_session_goal(goal)?;
        let connection = self.open()?;
        connection
            .execute(
                "INSERT INTO session_goals(session_id, text, status, progress, updated_at)
                 VALUES(?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(session_id) DO UPDATE SET
                   text=excluded.text,
                   status=excluded.status,
                   progress=excluded.progress,
                   updated_at=excluded.updated_at",
                params![
                    session_id,
                    goal.text,
                    goal.status,
                    goal.progress,
                    goal.updated_at,
                ],
            )
            .map_err(|error| format!("write session goal: {error}"))?;
        Ok(())
    }

    pub fn clear_session_goal(&self, session_id: &str) -> Result<(), String> {
        let connection = self.open()?;
        connection
            .execute(
                "DELETE FROM session_goals WHERE session_id = ?1",
                params![session_id],
            )
            .map_err(|error| format!("clear session goal: {error}"))?;
        Ok(())
    }

    /// Store one manually requested continuity reference. It is encrypted like
    /// the transcript because the deterministic excerpt can quote history.
    pub fn upsert_session_context_compaction(
        &self,
        session_id: &str,
        summary_text: &str,
        metadata: &Value,
    ) -> Result<StoredSessionContextCompaction, String> {
        let summary_text = summary_text.trim();
        if summary_text.is_empty() || summary_text.len() > 65_536 {
            return Err(
                "manual context summary must contain at most 65536 UTF-8 bytes".to_string(),
            );
        }
        let metadata = validate_manual_context_compaction(metadata)?;
        let source_message_count = metadata
            .get("sourceMessageEnd")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| "manual context summary source count is invalid".to_string())?;
        let metadata_json = serde_json::to_string(&metadata)
            .map_err(|error| format!("serialize manual context metadata: {error}"))?;
        let protected_summary = protect_stored_transcript_text(summary_text)?;
        let created_at = metadata
            .get("generatedAt")
            .and_then(Value::as_i64)
            .filter(|value| *value > 0)
            .ok_or_else(|| "manual context summary generation time is invalid".to_string())?;
        let connection = self.open()?;
        connection
            .execute(
                "INSERT INTO session_context_compactions(
                   session_id, summary_text, source_message_count, metadata_json, created_at
                 ) VALUES(?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(session_id) DO UPDATE SET
                   summary_text=excluded.summary_text,
                   source_message_count=excluded.source_message_count,
                   metadata_json=excluded.metadata_json,
                   created_at=excluded.created_at",
                params![
                    session_id,
                    protected_summary,
                    i64::try_from(source_message_count).map_err(|_| {
                        "manual context summary source count is too large".to_string()
                    })?,
                    metadata_json,
                    created_at,
                ],
            )
            .map_err(|error| format!("write manual context compaction: {error}"))?;
        Ok(StoredSessionContextCompaction {
            summary_text: summary_text.to_string(),
            source_message_count,
            metadata,
        })
    }

    pub fn clear_session_context_compaction(&self, session_id: &str) -> Result<(), String> {
        let connection = self.open()?;
        connection
            .execute(
                "DELETE FROM session_context_compactions WHERE session_id = ?1",
                params![session_id],
            )
            .map_err(|error| format!("clear manual context compaction: {error}"))?;
        Ok(())
    }

    fn load_session_context_compaction(
        &self,
        session_id: &str,
    ) -> Result<Option<StoredSessionContextCompaction>, String> {
        let connection = self.open()?;
        let row = connection
            .query_row(
                "SELECT summary_text, source_message_count, metadata_json
                 FROM session_context_compactions WHERE session_id = ?1",
                params![session_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("read manual context compaction: {error}"))?;
        let Some((summary_text, source_message_count, metadata_json)) = row else {
            return Ok(None);
        };
        let source_message_count = usize::try_from(source_message_count)
            .map_err(|_| "manual context summary source count is invalid".to_string())?;
        let summary_text = unprotect_stored_transcript_text(&summary_text)?;
        let metadata: Value = serde_json::from_str(&metadata_json)
            .map_err(|error| format!("parse manual context metadata: {error}"))?;
        let metadata = validate_manual_context_compaction(&metadata)?;
        let metadata_count = metadata
            .get("sourceMessageEnd")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| "manual context summary source count is invalid".to_string())?;
        if metadata_count != source_message_count {
            return Err(
                "manual context summary metadata count does not match stored boundary".to_string(),
            );
        }
        Ok(Some(StoredSessionContextCompaction {
            summary_text,
            source_message_count,
            metadata,
        }))
    }

    pub fn load_messages(&self, session_id: &str) -> Result<Vec<StoredMessage>, String> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT id, session_id, role, content, created_at, turn_id
                 FROM messages WHERE session_id = ?1 ORDER BY created_at, id",
            )
            .map_err(|error| format!("prepare history messages: {error}"))?;
        let rows = statement
            .query_map(params![session_id], |row| {
                Ok(StoredMessage {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    created_at: row.get(4)?,
                    turn_id: row.get(5)?,
                })
            })
            .map_err(|error| format!("query history messages: {error}"))?;
        let mut messages = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read history messages: {error}"))?;
        for message in &mut messages {
            message.content = unprotect_stored_transcript_text(&message.content)?;
        }
        Ok(messages)
    }

    /// Count messages for a session.
    pub fn count_messages(&self, session_id: &str) -> Result<i64, String> {
        let connection = self.open()?;
        connection
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("count history messages: {error}"))
    }

    /// Return the two transcript fields needed to validate a segmented
    /// checkpoint without reading or decrypting any message content.
    fn load_message_checkpoint(&self, session_id: &str) -> Result<(i64, Option<String>), String> {
        let connection = self.open()?;
        connection
            .query_row(
                "SELECT COUNT(*),
                        (SELECT id FROM messages
                         WHERE session_id = ?1
                         ORDER BY created_at DESC, id DESC
                         LIMIT 1)
                 FROM messages
                 WHERE session_id = ?1",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| format!("load history message checkpoint: {error}"))
    }

    /// Count messages for every session in one database round trip.
    ///
    /// Startup needs only the counts to decide whether an in-memory session
    /// projection is complete. Opening a separate SQLite connection for every
    /// saved session turns that metadata-only path into an N+1 query pattern.
    /// Sessions without messages are intentionally absent from the map and
    /// callers should treat them as zero.
    pub fn load_message_counts(&self) -> Result<HashMap<String, i64>, String> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare("SELECT session_id, COUNT(*) FROM messages GROUP BY session_id")
            .map_err(|error| format!("prepare history message counts: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|error| format!("query history message counts: {error}"))?;
        rows.collect::<Result<HashMap<_, _>, _>>()
            .map_err(|error| format!("read history message counts: {error}"))
    }

    /// True when the session has at least one user-authored message.
    pub fn has_user_message(&self, session_id: &str) -> Result<bool, String> {
        let connection = self.open()?;
        let found = connection
            .query_row(
                "SELECT 1 FROM messages
                 WHERE session_id = ?1 AND role IN ('user', 'User')
                 LIMIT 1",
                params![session_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("probe user message: {error}"))?;
        Ok(found.is_some())
    }

    /// True when the durable message row is an assistant reply for the given turn.
    pub fn message_matches_assistant_turn(
        &self,
        session_id: &str,
        message_id: &str,
        turn_id: &str,
    ) -> Result<bool, String> {
        let connection = self.open()?;
        let found = connection
            .query_row(
                "SELECT 1 FROM messages
                 WHERE session_id = ?1
                   AND id = ?2
                   AND turn_id = ?3
                   AND lower(role) = 'assistant'
                 LIMIT 1",
                params![session_id, message_id, turn_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("probe assistant turn message: {error}"))?;
        Ok(found.is_some())
    }

    fn clamp_ui_message_page_limit(limit: usize) -> usize {
        limit.clamp(1, MAX_UI_MESSAGE_PAGE_SIZE)
    }

    fn decrypt_stored_messages(messages: &mut [StoredMessage]) -> Result<(), String> {
        for message in messages {
            message.content = unprotect_stored_transcript_text(&message.content)?;
        }
        Ok(())
    }

    /// Load the newest `limit` messages (for first paint).
    ///
    /// Rows are fetched newest-first, then reversed so the page is chronological.
    pub fn load_messages_recent(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<MessagePage, String> {
        let limit = Self::clamp_ui_message_page_limit(limit);
        let total_count = self.count_messages(session_id)?;
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT id, session_id, role, content, created_at, turn_id
                 FROM messages
                 WHERE session_id = ?1
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?2",
            )
            .map_err(|error| format!("prepare recent history messages: {error}"))?;
        let rows = statement
            .query_map(params![session_id, limit as i64], |row| {
                Ok(StoredMessage {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    created_at: row.get(4)?,
                    turn_id: row.get(5)?,
                })
            })
            .map_err(|error| format!("query recent history messages: {error}"))?;
        let mut messages = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read recent history messages: {error}"))?;
        messages.reverse();
        Self::decrypt_stored_messages(&mut messages)?;
        let has_more_before = total_count > messages.len() as i64;
        Ok(MessagePage {
            messages,
            total_count,
            has_more_before,
        })
    }

    /// Load older messages strictly before `(created_at, id)` cursor, up to limit.
    ///
    /// The cursor is the oldest message currently shown in the UI.
    pub fn load_messages_before(
        &self,
        session_id: &str,
        before_created_at: i64,
        before_id: &str,
        limit: usize,
    ) -> Result<MessagePage, String> {
        let limit = Self::clamp_ui_message_page_limit(limit);
        let total_count = self.count_messages(session_id)?;
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT id, session_id, role, content, created_at, turn_id
                 FROM messages
                 WHERE session_id = ?1
                   AND (created_at < ?2 OR (created_at = ?2 AND id < ?3))
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?4",
            )
            .map_err(|error| format!("prepare older history messages: {error}"))?;
        let rows = statement
            .query_map(
                params![session_id, before_created_at, before_id, limit as i64],
                |row| {
                    Ok(StoredMessage {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        role: row.get(2)?,
                        content: row.get(3)?,
                        created_at: row.get(4)?,
                        turn_id: row.get(5)?,
                    })
                },
            )
            .map_err(|error| format!("query older history messages: {error}"))?;
        let mut messages = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read older history messages: {error}"))?;
        messages.reverse();
        Self::decrypt_stored_messages(&mut messages)?;
        let has_more_before = if messages.is_empty() {
            false
        } else {
            let oldest = &messages[0];
            let older_exists = connection
                .query_row(
                    "SELECT 1 FROM messages
                     WHERE session_id = ?1
                       AND (created_at < ?2 OR (created_at = ?2 AND id < ?3))
                     LIMIT 1",
                    params![session_id, oldest.created_at, oldest.id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(|error| format!("probe older history messages: {error}"))?
                .is_some();
            older_exists
        };
        Ok(MessagePage {
            messages,
            total_count,
            has_more_before,
        })
    }

    /// Upsert session row + metadata without rewriting messages.
    pub fn upsert_session_metadata(&self, session: &StoredSession) -> Result<(), String> {
        self.upsert_session_metadata_batch(std::slice::from_ref(session))
    }

    /// Upsert session rows and metadata in one transaction without touching
    /// their transcript projections. This preserves all-or-nothing batch
    /// archive/relocation updates while avoiding a corpus-wide snapshot.
    pub fn upsert_session_metadata_batch(&self, sessions: &[StoredSession]) -> Result<(), String> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("begin session metadata batch: {error}"))?;
        Self::upsert_session_metadata_batch_in_transaction(&transaction, sessions)?;
        transaction
            .commit()
            .map_err(|error| format!("commit session metadata batch: {error}"))
    }

    /// Atomically persist session metadata and selected setting scopes. This
    /// is used when a workspace relocation changes both durable projections.
    pub fn upsert_session_metadata_batch_and_settings(
        &self,
        sessions: &[StoredSession],
        settings: &HashMap<String, Value>,
    ) -> Result<(), String> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("begin session metadata/settings batch: {error}"))?;
        Self::upsert_session_metadata_batch_in_transaction(&transaction, sessions)?;
        Self::upsert_settings_in_transaction(&transaction, settings)?;
        transaction
            .commit()
            .map_err(|error| format!("commit session metadata/settings batch: {error}"))
    }

    /// Persist one session's metadata and any changed message rows in one
    /// transaction without deleting transcript rows absent from a partial
    /// in-memory cache.
    pub fn upsert_session_metadata_and_messages(
        &self,
        session: &StoredSession,
        messages: &[StoredMessage],
    ) -> Result<(), String> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("begin partial session projection: {error}"))?;
        Self::upsert_session_row_in_transaction(&transaction, session)?;
        for message in messages {
            if message.session_id == session.id {
                Self::upsert_message_in_transaction(&transaction, message)?;
            }
        }
        transaction
            .commit()
            .map_err(|error| format!("commit partial session projection: {error}"))
    }

    /// Upsert one session row + metadata and replace ONLY that session's messages.
    /// Does NOT touch other sessions or settings.
    ///
    /// Callers must only use this when the message slice is the complete transcript.
    pub fn upsert_session_projection(
        &self,
        session: &StoredSession,
        messages: &[StoredMessage],
    ) -> Result<(), String> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("begin session projection: {error}"))?;
        Self::upsert_session_projection_in_transaction(&transaction, session, messages)?;
        transaction
            .commit()
            .map_err(|error| format!("commit session projection: {error}"))
    }

    /// Delete one session. Child rows (messages, turns, segments, …) cascade via FK.
    pub fn delete_session_projection(&self, session_id: &str) -> Result<(), String> {
        let connection = self.open()?;
        connection
            .execute("DELETE FROM sessions WHERE id = ?1", params![session_id])
            .map_err(|error| format!("delete history session: {error}"))?;
        Ok(())
    }

    /// Upsert settings only (no session/message rewrite).
    pub fn upsert_settings(&self, settings: &HashMap<String, Value>) -> Result<(), String> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("begin settings upsert: {error}"))?;
        Self::upsert_settings_in_transaction(&transaction, settings)?;
        transaction
            .commit()
            .map_err(|error| format!("commit settings upsert: {error}"))
    }

    /// One-time migration: encrypt legacy plaintext message and segment blobs.
    ///
    /// Returns `true` when this call performed work and wrote the marker.
    /// Failures are left for the caller to log; the marker is only set after
    /// every eligible row was updated so a partial pass can retry.
    pub fn encrypt_legacy_transcripts_once(&self) -> Result<bool, String> {
        let mut connection = self.open()?;
        let messages_completed = connection
            .query_row(
                "SELECT value FROM schema_meta WHERE key = ?1",
                params![MESSAGES_DPAPI_MIGRATION_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("read transcript encryption migration marker: {error}"))?
            .is_some_and(|value| value == "1");
        let summaries_completed = connection
            .query_row(
                "SELECT value FROM schema_meta WHERE key = ?1",
                params![SEGMENT_SUMMARIES_DPAPI_MIGRATION_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("read segment summary encryption migration marker: {error}"))?
            .is_some_and(|value| value == "1");
        let private_fields_completed = connection
            .query_row(
                "SELECT value FROM schema_meta WHERE key = ?1",
                params![PRIVATE_EVENT_FIELDS_DPAPI_MIGRATION_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("read private event encryption migration marker: {error}"))?
            .is_some_and(|value| value == "1");
        if messages_completed && summaries_completed && private_fields_completed {
            return Ok(false);
        }

        let transaction = connection
            .transaction()
            .map_err(|error| format!("begin transcript encryption migration: {error}"))?;

        if !messages_completed {
            let mut statement = transaction
                .prepare("SELECT id, content FROM messages")
                .map_err(|error| format!("prepare legacy message scan: {error}"))?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|error| format!("query legacy messages: {error}"))?;
            let mut updates = Vec::new();
            for row in rows {
                let (id, content) = row.map_err(|error| format!("read legacy message: {error}"))?;
                if crate::secret_store::is_protected_transcript_text(&content) {
                    continue;
                }
                let protected = protect_stored_transcript_text(&content)?;
                if protected != content {
                    updates.push((id, protected));
                }
            }
            drop(statement);
            for (id, protected) in updates {
                transaction
                    .execute(
                        "UPDATE messages SET content = ?1 WHERE id = ?2",
                        params![protected, id],
                    )
                    .map_err(|error| format!("encrypt legacy message: {error}"))?;
            }
        }

        if !messages_completed {
            let mut statement = transaction
                .prepare(
                    "SELECT conversation_id, segment_index, messages_json FROM history_segments",
                )
                .map_err(|error| format!("prepare legacy segment scan: {error}"))?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|error| format!("query legacy segments: {error}"))?;
            let mut updates = Vec::new();
            for row in rows {
                let (conversation_id, segment_index, messages_json) =
                    row.map_err(|error| format!("read legacy segment: {error}"))?;
                if crate::secret_store::is_protected_transcript_text(&messages_json) {
                    continue;
                }
                let protected = protect_stored_transcript_text(&messages_json)?;
                if protected != messages_json {
                    updates.push((conversation_id, segment_index, protected));
                }
            }
            drop(statement);
            for (conversation_id, segment_index, protected) in updates {
                transaction
                    .execute(
                        "UPDATE history_segments SET messages_json = ?1
                         WHERE conversation_id = ?2 AND segment_index = ?3",
                        params![protected, conversation_id, segment_index],
                    )
                    .map_err(|error| format!("encrypt legacy segment: {error}"))?;
            }
        }

        if !summaries_completed {
            let mut statement = transaction
                .prepare(
                    "SELECT conversation_id, segment_index, summary_json FROM history_segments
                     WHERE summary_json IS NOT NULL",
                )
                .map_err(|error| format!("prepare legacy segment summary scan: {error}"))?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|error| format!("query legacy segment summaries: {error}"))?;
            let mut updates = Vec::new();
            for row in rows {
                let (conversation_id, segment_index, summary_json) =
                    row.map_err(|error| format!("read legacy segment summary: {error}"))?;
                if crate::secret_store::is_protected_transcript_text(&summary_json) {
                    continue;
                }
                let protected = protect_stored_transcript_text(&summary_json)?;
                if protected != summary_json {
                    updates.push((conversation_id, segment_index, protected));
                }
            }
            drop(statement);
            for (conversation_id, segment_index, protected) in updates {
                transaction
                    .execute(
                        "UPDATE history_segments SET summary_json = ?1
                         WHERE conversation_id = ?2 AND segment_index = ?3",
                        params![protected, conversation_id, segment_index],
                    )
                    .map_err(|error| format!("encrypt legacy segment summary: {error}"))?;
            }
        }

        if !private_fields_completed {
            migrate_private_event_fields(&transaction)?;
        }

        if !messages_completed {
            transaction
                .execute(
                    "INSERT INTO schema_meta(key, value) VALUES(?1, '1')
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![MESSAGES_DPAPI_MIGRATION_KEY],
                )
                .map_err(|error| {
                    format!("write transcript encryption migration marker: {error}")
                })?;
        }
        if !summaries_completed {
            transaction
                .execute(
                    "INSERT INTO schema_meta(key, value) VALUES(?1, '1')
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![SEGMENT_SUMMARIES_DPAPI_MIGRATION_KEY],
                )
                .map_err(|error| {
                    format!("write segment summary encryption migration marker: {error}")
                })?;
        }
        if !private_fields_completed {
            transaction
                .execute(
                    "INSERT INTO schema_meta(key, value) VALUES(?1, '1')
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![PRIVATE_EVENT_FIELDS_DPAPI_MIGRATION_KEY],
                )
                .map_err(|error| {
                    format!("write private event encryption migration marker: {error}")
                })?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit transcript encryption migration: {error}"))?;
        Ok(true)
    }

    pub fn load_settings(&self) -> Result<HashMap<String, Value>, String> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare("SELECT scope, payload_json FROM settings")
            .map_err(|error| format!("prepare history settings: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                let scope: String = row.get(0)?;
                let payload: String = row.get(1)?;
                Ok((scope, payload))
            })
            .map_err(|error| format!("query history settings: {error}"))?;
        let mut output = HashMap::new();
        for row in rows {
            let (scope, payload) = row.map_err(|error| format!("read history setting: {error}"))?;
            let value = serde_json::from_str::<Value>(&payload)
                .map_err(|error| format!("parse history setting {scope}: {error}"))?;
            output.insert(scope, value);
        }
        Ok(output)
    }

    pub fn replace_snapshot(
        &self,
        sessions: &[StoredSession],
        messages: &[StoredMessage],
        settings: &HashMap<String, Value>,
    ) -> Result<(), String> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("begin history snapshot: {error}"))?;
        Self::replace_snapshot_in_transaction(&transaction, sessions, messages, settings)?;
        transaction
            .commit()
            .map_err(|error| format!("commit history snapshot: {error}"))
    }

    /// Commit the session/message projection and one segmented-history mutation
    /// together. A checkpoint must never describe a snapshot that was not
    /// committed, or vice versa.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "exercised directly by segmented-history regression tests"
        )
    )]
    pub fn replace_snapshot_and_mutate_segment(
        &self,
        sessions: &[StoredSession],
        messages: &[StoredMessage],
        settings: &HashMap<String, Value>,
        header: &StoredHistoryHeader,
        segment: &StoredHistorySegment,
        append: bool,
    ) -> Result<(), String> {
        validate_segment_header_pair(header, segment)?;
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("begin atomic history segment mutation: {error}"))?;
        // Prefer a session-scoped write when the mutation targets one conversation
        // that is present in the snapshot. Fall back to a full corpus rewrite when
        // the caller is still migrating multi-session bulk state.
        if sessions.len() == 1
            && sessions[0].id == header.conversation_id
            && messages
                .iter()
                .all(|message| message.session_id == header.conversation_id)
        {
            Self::upsert_session_projection_in_transaction(&transaction, &sessions[0], messages)?;
            // Settings are optional on the hot segmented path; only rewrite when
            // the caller supplied a non-empty map so we do not wipe scopes.
            if !settings.is_empty() {
                Self::upsert_settings_in_transaction(&transaction, settings)?;
            }
        } else {
            Self::replace_snapshot_in_transaction(&transaction, sessions, messages, settings)?;
        }
        Self::mutate_segment_in_transaction(&transaction, header, segment, append)?;
        transaction
            .commit()
            .map_err(|error| format!("commit atomic history segment mutation: {error}"))
    }

    /// Session-scoped projection + segment mutation in one transaction.
    pub fn upsert_session_and_mutate_segment(
        &self,
        session: &StoredSession,
        messages: &[StoredMessage],
        header: &StoredHistoryHeader,
        segment: &StoredHistorySegment,
        append: bool,
    ) -> Result<(), String> {
        validate_segment_header_pair(header, segment)?;
        if session.id != header.conversation_id {
            return Err(
                "session projection id must match the segmented conversation id".to_string(),
            );
        }
        if messages
            .iter()
            .any(|message| message.session_id != session.id)
        {
            return Err("session projection messages must belong to the same session".to_string());
        }
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("begin session segment mutation: {error}"))?;
        Self::upsert_session_projection_in_transaction(&transaction, session, messages)?;
        Self::mutate_segment_in_transaction(&transaction, header, segment, append)?;
        transaction
            .commit()
            .map_err(|error| format!("commit session segment mutation: {error}"))
    }

    /// Prefer a current segmented checkpoint when it represents the exact
    /// durable transcript. Otherwise retain the event-aware raw replay path.
    /// This keeps an older checkpoint from hiding turns appended afterwards.
    /// The returned source intentionally ignores a manual compaction record so
    /// a new `/compact` always replaces it from the full durable transcript.
    pub fn load_runtime_context_source(&self, session_id: &str) -> Result<Vec<Value>, String> {
        let Some(history) = self.load_segmented_history(session_id)? else {
            return self.load_context(session_id);
        };
        let (message_count, raw_last_id) = self.load_message_checkpoint(session_id)?;
        let active = history
            .segments
            .iter()
            .find(|segment| segment.segment_index == history.header.active_segment_index);
        let checkpoint_matches_raw = history.header.total_message_count == message_count
            && active.and_then(|segment| segment.end_message_id.as_deref())
                == raw_last_id.as_deref();
        if !checkpoint_matches_raw {
            return self.load_context(session_id);
        }

        let mut context = Vec::with_capacity(message_count.max(0) as usize);
        for segment in &history.segments {
            // load_segmented_history already decrypts messages_json.
            let values = serde_json::from_str::<Value>(&segment.messages_json)
                .map_err(|error| format!("parse runtime history segment: {error}"))?;
            let values = values.as_array().ok_or_else(|| {
                "runtime history segment messages must be a JSON array".to_string()
            })?;
            if values.len() as i64 != segment.message_count {
                return Err("runtime history segment message count is inconsistent".to_string());
            }
            context.extend(values.iter().cloned());
        }
        if context.len() as i64 != history.header.total_message_count {
            return Err("runtime history checkpoint total is inconsistent".to_string());
        }
        Ok(context)
    }

    /// Apply the current manual continuity reference only at context-load time.
    /// This leaves transcript rows, UI history, search, and exports untouched.
    pub fn load_runtime_context_with_metadata(
        &self,
        session_id: &str,
    ) -> Result<RuntimeContext, String> {
        let mut messages = self.load_runtime_context_source(session_id)?;
        let Some(compaction) = self.load_session_context_compaction(session_id)? else {
            return Ok(RuntimeContext {
                messages,
                manual_compaction: None,
            });
        };
        // A retained record can become stale after a local recovery, import, or
        // history edit. Do not risk hiding a changed transcript; fall back to
        // the complete source until the user compacts it again.
        if compaction.source_message_count > messages.len() {
            return Ok(RuntimeContext {
                messages,
                manual_compaction: None,
            });
        }
        let suffix = messages.split_off(compaction.source_message_count);
        let mut compacted = Vec::with_capacity(suffix.len() + 1);
        compacted.push(json!({
            "role": "user",
            "content": compaction.summary_text,
        }));
        compacted.extend(suffix);
        Ok(RuntimeContext {
            messages: compacted,
            manual_compaction: Some(compaction.metadata),
        })
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "exercised directly by runtime-context regression tests"
        )
    )]
    pub fn load_runtime_context(&self, session_id: &str) -> Result<Vec<Value>, String> {
        self.load_runtime_context_with_metadata(session_id)
            .map(|context| context.messages)
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "exercised directly by segmented-history regression tests"
        )
    )]
    pub fn upsert_active_segment(
        &self,
        header: &StoredHistoryHeader,
        segment: &StoredHistorySegment,
    ) -> Result<(), String> {
        validate_segment_header_pair(header, segment)?;
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("begin active history segment: {error}"))?;
        Self::mutate_segment_in_transaction(&transaction, header, segment, false)?;
        transaction
            .commit()
            .map_err(|error| format!("commit active history segment: {error}"))
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "exercised directly by segmented-history regression tests"
        )
    )]
    pub fn append_segment(
        &self,
        header: &StoredHistoryHeader,
        segment: &StoredHistorySegment,
    ) -> Result<(), String> {
        validate_segment_header_pair(header, segment)?;
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("begin append history segment: {error}"))?;
        Self::mutate_segment_in_transaction(&transaction, header, segment, true)?;
        transaction
            .commit()
            .map_err(|error| format!("commit appended history segment: {error}"))
    }

    fn upsert_session_row_in_transaction(
        transaction: &Connection,
        session: &StoredSession,
    ) -> Result<(), String> {
        transaction
            .execute(
                "INSERT INTO sessions(id, title, cwd, created_at, updated_at)
                 VALUES(?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET title=excluded.title, cwd=excluded.cwd, updated_at=excluded.updated_at",
                params![
                    session.id,
                    session.title,
                    session.cwd,
                    session.updated_at,
                    session.updated_at
                ],
            )
            .map_err(|error| format!("write history session: {error}"))?;
        transaction
            .execute(
                "INSERT INTO session_metadata(
                   session_id, provider_id, model, selected_model_json, pinned_at, archived_at,
                   share_enabled, share_token, share_created_at, share_updated_at, redact_tool_content
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(session_id) DO UPDATE SET
                   provider_id=excluded.provider_id,
                   model=excluded.model,
                   selected_model_json=excluded.selected_model_json,
                   pinned_at=excluded.pinned_at,
                   archived_at=excluded.archived_at,
                   share_enabled=excluded.share_enabled,
                   share_token=excluded.share_token,
                   share_created_at=excluded.share_created_at,
                   share_updated_at=excluded.share_updated_at,
                   redact_tool_content=excluded.redact_tool_content",
                params![
                    session.id,
                    session.provider_id,
                    session.model,
                    session.selected_model_json,
                    session.pinned_at,
                    session.archived_at,
                    session.share_enabled,
                    session.share_token,
                    session.share_created_at,
                    session.share_updated_at,
                    session.redact_tool_content
                ],
            )
            .map_err(|error| format!("write history session metadata: {error}"))?;
        Ok(())
    }

    fn insert_message_in_transaction(
        transaction: &Connection,
        message: &StoredMessage,
    ) -> Result<(), String> {
        let protected_content = protect_stored_transcript_text(&message.content)?;
        transaction
            .execute(
                "INSERT INTO messages(id, session_id, role, content, created_at, turn_id)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    message.id,
                    message.session_id,
                    message.role,
                    protected_content,
                    message.created_at,
                    message.turn_id
                ],
            )
            .map_err(|error| format!("write history message: {error}"))?;
        Ok(())
    }

    /// INSERT OR REPLACE one message so incomplete caches can flush without
    /// deleting older durable rows that are not present in the working set.
    fn upsert_message_in_transaction(
        transaction: &Connection,
        message: &StoredMessage,
    ) -> Result<(), String> {
        let protected_content = protect_stored_transcript_text(&message.content)?;
        transaction
            .execute(
                "INSERT INTO messages(id, session_id, role, content, created_at, turn_id)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                   session_id=excluded.session_id,
                   role=excluded.role,
                   content=excluded.content,
                   created_at=excluded.created_at,
                   turn_id=excluded.turn_id",
                params![
                    message.id,
                    message.session_id,
                    message.role,
                    protected_content,
                    message.created_at,
                    message.turn_id
                ],
            )
            .map_err(|error| format!("upsert history message: {error}"))?;
        Ok(())
    }

    fn upsert_settings_in_transaction(
        transaction: &Connection,
        settings: &HashMap<String, Value>,
    ) -> Result<(), String> {
        for (scope, value) in settings {
            let payload = serde_json::to_string(value)
                .map_err(|error| format!("serialize setting {scope}: {error}"))?;
            transaction
                .execute(
                    "INSERT INTO settings(scope, payload_json, updated_at) VALUES(?1, ?2, strftime('%s','now') * 1000)
                     ON CONFLICT(scope) DO UPDATE SET payload_json = excluded.payload_json, updated_at = excluded.updated_at",
                    params![scope, payload],
                )
                .map_err(|error| format!("write history setting {scope}: {error}"))?;
        }
        Ok(())
    }

    fn upsert_session_projection_in_transaction(
        transaction: &Connection,
        session: &StoredSession,
        messages: &[StoredMessage],
    ) -> Result<(), String> {
        Self::upsert_session_row_in_transaction(transaction, session)?;
        transaction
            .execute(
                "DELETE FROM messages WHERE session_id = ?1",
                params![session.id],
            )
            .map_err(|error| format!("clear session messages: {error}"))?;
        for message in messages {
            if message.session_id != session.id {
                continue;
            }
            Self::insert_message_in_transaction(transaction, message)?;
        }
        Ok(())
    }

    fn upsert_session_metadata_batch_in_transaction(
        transaction: &Connection,
        sessions: &[StoredSession],
    ) -> Result<(), String> {
        for session in sessions {
            Self::upsert_session_row_in_transaction(transaction, session)?;
        }
        Ok(())
    }

    fn replace_snapshot_in_transaction(
        transaction: &Connection,
        sessions: &[StoredSession],
        messages: &[StoredMessage],
        settings: &HashMap<String, Value>,
    ) -> Result<(), String> {
        // Messages are a projection owned by AppState and can be rebuilt on
        // every snapshot. Turns/events/tools/permissions are the durable Pi
        // audit trail and must survive projection refreshes.
        if sessions.is_empty() {
            transaction
                .execute("DELETE FROM sessions", [])
                .map_err(|error| format!("clear history sessions: {error}"))?;
        } else {
            let existing = {
                let mut statement = transaction
                    .prepare("SELECT id FROM sessions")
                    .map_err(|error| format!("prepare stale history sessions: {error}"))?;
                let rows = statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(|error| format!("query stale history sessions: {error}"))?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|error| format!("read stale history sessions: {error}"))?
            };
            for id in existing {
                if !sessions.iter().any(|session| session.id == id) {
                    transaction
                        .execute("DELETE FROM sessions WHERE id = ?1", params![id])
                        .map_err(|error| format!("delete stale history session: {error}"))?;
                }
            }
        }
        transaction
            .execute("DELETE FROM messages", [])
            .map_err(|error| format!("clear history messages: {error}"))?;
        for session in sessions {
            Self::upsert_session_row_in_transaction(transaction, session)?;
        }
        for message in messages {
            if !sessions
                .iter()
                .any(|session| session.id == message.session_id)
            {
                continue;
            }
            Self::insert_message_in_transaction(transaction, message)?;
        }
        Self::upsert_settings_in_transaction(transaction, settings)?;
        Ok(())
    }

    pub fn load_segmented_history(
        &self,
        conversation_id: &str,
    ) -> Result<Option<StoredSegmentedHistory>, String> {
        let connection = self.open()?;
        let header = connection
            .query_row(
                "SELECT conversation_id, source_session_id, context_meta_json,
                        active_segment_index, total_segment_count, total_message_count,
                        created_at, updated_at
                 FROM history_segment_headers WHERE conversation_id = ?1",
                params![conversation_id],
                |row| {
                    Ok(StoredHistoryHeader {
                        conversation_id: row.get(0)?,
                        session_id: row.get(1)?,
                        context_meta_json: row.get(2)?,
                        active_segment_index: row.get(3)?,
                        total_segment_count: row.get(4)?,
                        total_message_count: row.get(5)?,
                        created_at: row.get(6)?,
                        updated_at: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("load history segment header: {error}"))?;
        let Some(header) = header else {
            return Ok(None);
        };
        let mut statement = connection
            .prepare(
                "SELECT segment_index, segment_id, summary_json, messages_json,
                        message_count, start_message_id, end_message_id, created_at, updated_at
                 FROM history_segments WHERE conversation_id = ?1 ORDER BY segment_index",
            )
            .map_err(|error| format!("prepare history segments: {error}"))?;
        let rows = statement
            .query_map(params![conversation_id], |row| {
                Ok(StoredHistorySegment {
                    segment_index: row.get(0)?,
                    segment_id: row.get(1)?,
                    summary_json: row.get(2)?,
                    messages_json: row.get(3)?,
                    message_count: row.get(4)?,
                    start_message_id: row.get(5)?,
                    end_message_id: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            })
            .map_err(|error| format!("query history segments: {error}"))?;
        let mut segments = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read history segments: {error}"))?;
        for segment in &mut segments {
            segment.summary_json = segment
                .summary_json
                .as_deref()
                .map(unprotect_stored_transcript_text)
                .transpose()?;
            segment.messages_json = unprotect_stored_transcript_text(&segment.messages_json)?;
        }
        verify_segment_totals(&connection, &header)?;
        Ok(Some(StoredSegmentedHistory { header, segments }))
    }

    fn mutate_segment_in_transaction(
        connection: &Connection,
        header: &StoredHistoryHeader,
        segment: &StoredHistorySegment,
        append: bool,
    ) -> Result<(), String> {
        let existing = load_segment_counts(connection, &header.conversation_id)?;
        if append {
            let (active, total) = existing.ok_or_else(|| {
                "append segment requires an existing segmented conversation".to_string()
            })?;
            if active != total - 1
                || segment.segment_index != total
                || header.active_segment_index != total
                || header.total_segment_count != total + 1
            {
                return Err(format!(
                    "append segment must extend the current tail at segmentIndex={total}"
                ));
            }
            upsert_segment_header(connection, header)?;
            let protected_summary_json = segment
                .summary_json
                .as_deref()
                .map(protect_stored_transcript_text)
                .transpose()?;
            let protected_messages_json = protect_stored_transcript_text(&segment.messages_json)?;
            connection
                .execute(
                    "INSERT INTO history_segments(
                       conversation_id, segment_index, segment_id, summary_json, messages_json,
                       message_count, start_message_id, end_message_id, created_at, updated_at
                     ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        header.conversation_id,
                        segment.segment_index,
                        segment.segment_id,
                        protected_summary_json,
                        protected_messages_json,
                        segment.message_count,
                        segment.start_message_id,
                        segment.end_message_id,
                        segment.created_at,
                        segment.updated_at
                    ],
                )
                .map_err(|error| format!("append history segment: {error}"))?;
        } else {
            match existing {
                Some((active, total)) => {
                    if active != header.active_segment_index || total != header.total_segment_count
                    {
                        return Err(
                            "active segment upsert cannot append, remove, or reorder segments"
                                .to_string(),
                        );
                    }
                }
                None if header.active_segment_index == 0 && header.total_segment_count == 1 => {}
                None => {
                    return Err(
                        "active segment upsert requires the preceding segments to exist"
                            .to_string(),
                    )
                }
            }
            upsert_segment_header(connection, header)?;
            let protected_summary_json = segment
                .summary_json
                .as_deref()
                .map(protect_stored_transcript_text)
                .transpose()?;
            let protected_messages_json = protect_stored_transcript_text(&segment.messages_json)?;
            connection
                .execute(
                    "INSERT INTO history_segments(
                       conversation_id, segment_index, segment_id, summary_json, messages_json,
                       message_count, start_message_id, end_message_id, created_at, updated_at
                     ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                     ON CONFLICT(conversation_id, segment_index) DO UPDATE SET
                       segment_id=excluded.segment_id,
                       summary_json=excluded.summary_json,
                       messages_json=excluded.messages_json,
                       message_count=excluded.message_count,
                       start_message_id=excluded.start_message_id,
                       end_message_id=excluded.end_message_id,
                       created_at=excluded.created_at,
                       updated_at=excluded.updated_at",
                    params![
                        header.conversation_id,
                        segment.segment_index,
                        segment.segment_id,
                        protected_summary_json,
                        protected_messages_json,
                        segment.message_count,
                        segment.start_message_id,
                        segment.end_message_id,
                        segment.created_at,
                        segment.updated_at
                    ],
                )
                .map_err(|error| format!("upsert active history segment: {error}"))?;
        }
        verify_segment_totals(connection, header)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the parameters map directly to the persisted turn columns"
    )]
    pub fn upsert_turn(
        &self,
        session_id: &str,
        conversation_id: &str,
        turn_id: &str,
        request_id: &str,
        status: &str,
        provider_id: Option<&str>,
        model: Option<&str>,
        reasoning: Option<&str>,
        cwd: Option<&str>,
        started_at: i64,
    ) -> Result<(), String> {
        let connection = self.open()?;
        connection
            .execute(
                "INSERT INTO turns(id, session_id, conversation_id, request_id, status, provider_id, model, reasoning, cwd, started_at)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(id) DO UPDATE SET
                   status=excluded.status,
                   provider_id=COALESCE(excluded.provider_id, turns.provider_id),
                   model=COALESCE(excluded.model, turns.model),
                   reasoning=COALESCE(excluded.reasoning, turns.reasoning),
                   cwd=COALESCE(excluded.cwd, turns.cwd)",
                params![
                    turn_id,
                    session_id,
                    conversation_id,
                    request_id,
                    status,
                    provider_id,
                    model,
                    reasoning,
                    cwd,
                    started_at
                ],
            )
            .map_err(|error| format!("upsert history turn: {error}"))?;
        Ok(())
    }

    pub fn load_turn_metadata(
        &self,
        session_id: &str,
    ) -> Result<HashMap<String, StoredTurnMetadata>, String> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT turns.id, turns.provider_id, turns.model, turns.reasoning, turns.finished_at,
                        (SELECT payload_json
                         FROM turn_events
                         WHERE turn_events.session_id = turns.session_id
                           AND turn_events.turn_id = turns.id
                           AND turn_events.event_type = 'done'
                         ORDER BY turn_events.created_at DESC, turn_events.id DESC
                         LIMIT 1)
                 FROM turns
                 WHERE session_id = ?1",
            )
            .map_err(|error| format!("prepare turn metadata: {error}"))?;
        let rows = statement
            .query_map(params![session_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    StoredTurnMetadata {
                        provider_id: row.get(1)?,
                        model: row.get(2)?,
                        reasoning: row.get(3)?,
                        finished_at: row.get(4)?,
                        thinking: row
                            .get::<_, Option<String>>(5)?
                            .as_deref()
                            .and_then(thinking_from_terminal_payload),
                    },
                ))
            })
            .map_err(|error| format!("query turn metadata: {error}"))?;
        let mut output = HashMap::new();
        for row in rows {
            let (turn_id, metadata) =
                row.map_err(|error| format!("read turn metadata: {error}"))?;
            output.insert(turn_id, metadata);
        }
        Ok(output)
    }

    pub fn delete_turn(&self, turn_id: &str) -> Result<(), String> {
        let connection = self.open()?;
        connection
            .execute("DELETE FROM turns WHERE id = ?1", params![turn_id])
            .map_err(|error| format!("delete history turn: {error}"))?;
        Ok(())
    }

    /// Load the safe, small trace projection for one exact session-owned turn.
    ///
    /// This intentionally does not expose `request_id`, tool ids, arguments,
    /// results, errors, or raw `turn_events` payloads. Those rows can contain
    /// sensitive workspace and provider-adjacent content and are not needed by
    /// the historical trace UI.
    pub fn load_turn_trace(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<Option<StoredTurnTrace>, String> {
        const MAX_TRACE_TOOLS: i64 = 128;

        let connection = self.open()?;
        let turn = connection
            .query_row(
                "SELECT session_id, id, status, started_at, finished_at
                 FROM turns
                 WHERE session_id = ?1 AND id = ?2",
                params![session_id, turn_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("load history trace turn: {error}"))?;
        let Some((session_id, turn_id, status, started_at, finished_at)) = turn else {
            return Ok(None);
        };

        let mut statement = connection
            .prepare(
                "SELECT name, status, started_at, finished_at
                 FROM tool_calls
                 WHERE session_id = ?1 AND turn_id = ?2
                 ORDER BY COALESCE(started_at, finished_at, 0), id
                 LIMIT ?3",
            )
            .map_err(|error| format!("prepare history trace tools: {error}"))?;
        let rows = statement
            .query_map(params![session_id, turn_id, MAX_TRACE_TOOLS], |row| {
                Ok(StoredTurnTraceTool {
                    name: row.get(0)?,
                    status: row.get(1)?,
                    started_at: row.get(2)?,
                    finished_at: row.get(3)?,
                })
            })
            .map_err(|error| format!("query history trace tools: {error}"))?;
        let tools = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read history trace tools: {error}"))?;

        Ok(Some(StoredTurnTrace {
            session_id,
            turn_id,
            status,
            started_at,
            finished_at,
            tools,
        }))
    }

    pub fn load_terminal_event(&self, request_id: &str) -> Result<Option<Value>, String> {
        let connection = self.open()?;
        let payload = connection
            .query_row(
                "SELECT payload_json FROM turn_events
                 WHERE request_id = ?1 AND event_type IN ('done','error','cancelled')
                 ORDER BY created_at, id LIMIT 1",
                params![request_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("load history terminal event: {error}"))?;
        payload
            .map(|payload| {
                let payload = unprotect_stored_transcript_text(&payload)?;
                serde_json::from_str(&payload)
                    .map_err(|error| format!("parse history terminal event: {error}"))
            })
            .transpose()
    }

    /// Append a Pi event and return whether it established a new terminal
    /// state. Terminal events are keyed by request id so native cancellation
    /// and the embedded Agent's `agent_end` cannot create two final turns.
    pub fn append_event(&self, payload: &Value, now_ms: i64) -> Result<bool, String> {
        let session_id = string_field(payload, &["sessionId", "session_id"])
            .ok_or_else(|| "event session id is required".to_string())?;
        let turn_id = string_field(payload, &["turnId", "turn_id"]).unwrap_or_default();
        let request_id = string_field(payload, &["requestId", "request_id"]).unwrap_or_default();
        let event_type = payload
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| "event type is required".to_string())?;
        let sequence = payload.get("sequence").and_then(Value::as_i64);
        let terminal = matches!(event_type, "done" | "error" | "cancelled");
        let event_id = if terminal && !request_id.is_empty() {
            format!("terminal:{request_id}")
        } else {
            string_field(payload, &["eventId", "event_id"])
                .unwrap_or_else(|| format!("{request_id}:{sequence:?}:{event_type}"))
        };
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("begin history event: {error}"))?;
        let redact_tool_content = session_redacts_tool_content(&transaction, &session_id)?;
        let stored_payload = if redact_tool_content {
            redact_event_for_storage(payload)
        } else {
            payload.clone()
        };
        // Tool-result redaction is configurable, but context-trim audit
        // metadata is always renderer-originated and therefore must never
        // bypass its strict shape/relationship validation.
        let stored_payload = sanitize_run_started_context_trim(stored_payload);
        let payload_json = protect_stored_transcript_text(
            &serde_json::to_string(&stored_payload)
                .map_err(|error| format!("serialize event: {error}"))?,
        )?;
        let prior_terminal = terminal
            && !request_id.is_empty()
            && transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM turn_events WHERE request_id = ?1 AND event_type IN ('done','error','cancelled'))",
                    params![request_id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| format!("check history terminal event: {error}"))?
                != 0;
        if prior_terminal {
            return Ok(false);
        }
        transaction
            .execute(
                "INSERT INTO turn_events(id, session_id, turn_id, request_id, sequence, event_type, payload_json, created_at)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET payload_json=excluded.payload_json, event_type=excluded.event_type",
                params![event_id, session_id, turn_id, request_id, sequence, event_type, payload_json, now_ms],
            )
            .map_err(|error| format!("append history event: {error}"))?;

        if let Some(tool) = stored_payload
            .get("toolCall")
            .or_else(|| stored_payload.get("tool_call"))
        {
            self.upsert_tool_call(
                &transaction,
                &session_id,
                &turn_id,
                tool,
                event_type,
                now_ms,
                redact_tool_content,
            )?;
        }
        if event_type == "permission_requested" {
            if let Some(permission) = stored_payload.get("permission") {
                let permission_id = string_field(permission, &["id"])
                    .unwrap_or_else(|| format!("permission:{request_id}:{now_ms}"));
                let tool_name = string_field(permission, &["toolName", "tool_name"]);
                let request_json = protect_stored_transcript_text(
                    &serde_json::to_string(permission)
                        .map_err(|error| format!("serialize permission request: {error}"))?,
                )?;
                transaction
                    .execute(
                        "INSERT INTO permission_requests(id, turn_id, session_id, tool_name, request_json, requested_at)
                         VALUES(?1, ?2, ?3, ?4, ?5, ?6)
                         ON CONFLICT(id) DO UPDATE SET request_json=excluded.request_json",
                        params![permission_id, turn_id, session_id, tool_name, request_json, now_ms],
                    )
                    .map_err(|error| format!("append permission request: {error}"))?;
            }
        }
        if matches!(event_type, "done" | "error" | "cancelled") && !turn_id.is_empty() {
            let status = if event_type == "done" {
                "completed"
            } else {
                event_type
            };
            let error = stored_payload
                .get("error")
                .and_then(Value::as_str)
                .map(protect_stored_transcript_text)
                .transpose()?;
            transaction
                .execute(
                    "UPDATE turns SET status=?1, finished_at=?2, error=COALESCE(?3,error), usage_json=COALESCE(?4,usage_json) WHERE id=?5",
                    params![status, now_ms, error, stored_payload.get("usage").map(Value::to_string), turn_id],
                )
                .map_err(|error| format!("finish history turn: {error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit history event: {error}"))?;
        Ok(true)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the parameters preserve the enclosing event transaction and redaction context"
    )]
    fn upsert_tool_call(
        &self,
        connection: &Connection,
        session_id: &str,
        turn_id: &str,
        tool: &Value,
        event_type: &str,
        now_ms: i64,
        redact_tool_content: bool,
    ) -> Result<(), String> {
        let Some(object) = tool.as_object() else {
            return Ok(());
        };
        let Some(id) = object.get("id").and_then(Value::as_str) else {
            return Ok(());
        };
        let name = object.get("name").and_then(Value::as_str).unwrap_or("tool");
        let status = object
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                if event_type == "tool_result" {
                    "completed"
                } else {
                    "running"
                }
            });
        let arguments = object
            .get("arguments")
            .or_else(|| object.get("args"))
            .map(Value::to_string)
            .map(|value| protect_stored_transcript_text(&value))
            .transpose()?;
        let result = object
            .get("result")
            .map(Value::to_string)
            .map(|value| protect_stored_transcript_text(&value))
            .transpose()?;
        let error = object
            .get("error")
            .and_then(Value::as_str)
            .map(protect_stored_transcript_text)
            .transpose()?;
        let started_at = (event_type != "tool_result").then_some(now_ms);
        let finished_at = (event_type == "tool_result").then_some(now_ms);
        connection
            .execute(
                "INSERT INTO tool_calls(id, turn_id, session_id, name, arguments_json, status, result_json, error, started_at, finished_at)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(id) DO UPDATE SET arguments_json=CASE WHEN ?11 THEN NULL ELSE COALESCE(excluded.arguments_json, tool_calls.arguments_json) END, status=excluded.status, result_json=CASE WHEN ?11 THEN NULL ELSE COALESCE(excluded.result_json, tool_calls.result_json) END, error=CASE WHEN ?11 THEN NULL ELSE COALESCE(excluded.error, tool_calls.error) END, started_at=COALESCE(tool_calls.started_at, excluded.started_at), finished_at=COALESCE(excluded.finished_at, tool_calls.finished_at)",
                params![id, turn_id, session_id, name, arguments, status, result, error, started_at, finished_at, redact_tool_content],
            )
            .map_err(|error| format!("upsert history tool call: {error}"))?;
        Ok(())
    }

    pub fn load_context(&self, session_id: &str) -> Result<Vec<Value>, String> {
        let messages = self.load_messages(session_id)?;
        let connection = self.open()?;
        let mut turns = BTreeMap::<String, ReplayTurn>::new();
        {
            let mut statement = connection
                .prepare(
                    "SELECT id, started_at FROM turns
                     WHERE session_id = ?1 ORDER BY started_at, id",
                )
                .map_err(|error| format!("prepare history context turns: {error}"))?;
            let rows = statement
                .query_map(params![session_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(|error| format!("query history context turns: {error}"))?;
            for row in rows {
                let (turn_id, started_at) =
                    row.map_err(|error| format!("read history context turn: {error}"))?;
                turns.insert(turn_id.clone(), ReplayTurn::new(turn_id, started_at));
            }
        }
        let mut statement = connection
            .prepare(
                "SELECT id, turn_id, sequence, event_type, payload_json, created_at FROM turn_events
                 WHERE session_id = ?1
                 ORDER BY created_at, COALESCE(sequence, 2147483647), id",
            )
            .map_err(|error| format!("prepare history context events: {error}"))?;
        let event_rows = statement
            .query_map(params![session_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(|error| format!("query history context events: {error}"))?;
        for row in event_rows {
            let (id, turn_id, sequence, event_type, payload_json, created_at) =
                row.map_err(|error| format!("read history context event: {error}"))?;
            let payload_json = unprotect_stored_transcript_text(&payload_json)?;
            let payload = serde_json::from_str::<Value>(&payload_json)
                .map_err(|error| format!("parse history context event {id}: {error}"))?;
            let turn = turns
                .entry(turn_id.clone())
                .or_insert_with(|| ReplayTurn::new(turn_id, created_at));
            turn.started_at = turn.started_at.min(created_at);
            turn.events.push(ReplayEvent {
                id,
                event_type,
                payload,
                sequence,
                created_at,
            });
        }

        let mut blocks = Vec::<ReplayBlock>::new();
        for message in messages {
            if let Some(turn_id) = message.turn_id.as_deref().filter(|value| !value.is_empty()) {
                let turn = turns
                    .entry(turn_id.to_string())
                    .or_insert_with(|| ReplayTurn::new(turn_id.to_string(), message.created_at));
                turn.started_at = turn.started_at.min(message.created_at);
                turn.messages.push(message);
            } else if let Some(value) = message_context(&message) {
                blocks.push(ReplayBlock {
                    created_at: message.created_at,
                    kind: 0,
                    id: message.id,
                    context: vec![value],
                });
            }
        }

        for (_, mut turn) in turns {
            turn.messages.sort_by(|left, right| {
                (left.created_at, &left.id).cmp(&(right.created_at, &right.id))
            });
            turn.events.sort_by(replay_event_order);
            let mut turn_context = Vec::new();
            let mut events_flushed = false;
            for message in &turn.messages {
                if message.role == "assistant" && !events_flushed {
                    append_tool_context(&mut turn_context, &turn.events);
                    events_flushed = true;
                }
                if let Some(value) = message_context(message) {
                    turn_context.push(value);
                }
            }
            if !events_flushed {
                append_tool_context(&mut turn_context, &turn.events);
            }
            if !turn_context.is_empty() {
                blocks.push(ReplayBlock {
                    created_at: turn.started_at,
                    kind: 1,
                    id: turn.id,
                    context: turn_context,
                });
            }
        }
        blocks.sort_by(|left, right| {
            (left.created_at, left.kind, &left.id).cmp(&(right.created_at, right.kind, &right.id))
        });
        Ok(blocks.into_iter().flat_map(|block| block.context).collect())
    }

    pub fn mark_interrupted_turns(&self, now_ms: i64) -> Result<(), String> {
        let connection = self.open()?;
        connection
            .execute(
                "UPDATE turns SET status='interrupted', finished_at=?1 WHERE status IN ('starting','running','waiting_permission')",
                params![now_ms],
            )
            .map_err(|error| format!("mark interrupted turns: {error}"))?;
        Ok(())
    }

    pub fn resolve_permission(&self, id: &str, decision: &str, now_ms: i64) -> Result<(), String> {
        let connection = self.open()?;
        connection
            .execute(
                "UPDATE permission_requests SET decision=?1, resolved_at=?2 WHERE id=?3",
                params![decision, now_ms, id],
            )
            .map_err(|error| format!("resolve permission request: {error}"))?;
        Ok(())
    }
}

/// Encrypt durable event/tool/permission/error fields that older releases
/// stored as plaintext. The caller owns one transaction and writes the marker
/// only after every table has been migrated, so a failed pass remains safe to
/// retry on the next startup or portable unlock.
fn migrate_private_event_fields(transaction: &rusqlite::Transaction<'_>) -> Result<(), String> {
    {
        let mut statement = transaction
            .prepare("SELECT id, payload_json FROM turn_events")
            .map_err(|error| format!("prepare legacy event scan: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("query legacy events: {error}"))?;
        let mut updates = Vec::new();
        for row in rows {
            let (id, payload) = row.map_err(|error| format!("read legacy event: {error}"))?;
            if crate::secret_store::is_protected_transcript_text(&payload) {
                continue;
            }
            let protected = protect_stored_transcript_text(&payload)?;
            if protected != payload {
                updates.push((id, protected));
            }
        }
        drop(statement);
        for (id, payload) in updates {
            transaction
                .execute(
                    "UPDATE turn_events SET payload_json = ?1 WHERE id = ?2",
                    params![payload, id],
                )
                .map_err(|error| format!("encrypt legacy event: {error}"))?;
        }
    }

    {
        let mut statement = transaction
            .prepare("SELECT id, arguments_json, result_json, error FROM tool_calls")
            .map_err(|error| format!("prepare legacy tool scan: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|error| format!("query legacy tools: {error}"))?;
        let mut updates = Vec::new();
        for row in rows {
            let (id, arguments, result, error) =
                row.map_err(|error| format!("read legacy tool: {error}"))?;
            let protected_arguments =
                protect_optional_stored_transcript_text(arguments.as_deref())?;
            let protected_result = protect_optional_stored_transcript_text(result.as_deref())?;
            let protected_error = protect_optional_stored_transcript_text(error.as_deref())?;
            if protected_arguments != arguments
                || protected_result != result
                || protected_error != error
            {
                updates.push((id, protected_arguments, protected_result, protected_error));
            }
        }
        drop(statement);
        for (id, arguments, result, error) in updates {
            transaction
                .execute(
                    "UPDATE tool_calls SET arguments_json = ?1, result_json = ?2, error = ?3 WHERE id = ?4",
                    params![arguments, result, error, id],
                )
                .map_err(|error| format!("encrypt legacy tool: {error}"))?;
        }
    }

    {
        let mut statement = transaction
            .prepare("SELECT id, request_json FROM permission_requests")
            .map_err(|error| format!("prepare legacy permission scan: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("query legacy permissions: {error}"))?;
        let mut updates = Vec::new();
        for row in rows {
            let (id, request) = row.map_err(|error| format!("read legacy permission: {error}"))?;
            if crate::secret_store::is_protected_transcript_text(&request) {
                continue;
            }
            let protected = protect_stored_transcript_text(&request)?;
            if protected != request {
                updates.push((id, protected));
            }
        }
        drop(statement);
        for (id, request) in updates {
            transaction
                .execute(
                    "UPDATE permission_requests SET request_json = ?1 WHERE id = ?2",
                    params![request, id],
                )
                .map_err(|error| format!("encrypt legacy permission: {error}"))?;
        }
    }

    {
        let mut statement = transaction
            .prepare("SELECT id, error FROM turns WHERE error IS NOT NULL")
            .map_err(|error| format!("prepare legacy turn error scan: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("query legacy turn errors: {error}"))?;
        let mut updates = Vec::new();
        for row in rows {
            let (id, error) = row.map_err(|error| format!("read legacy turn error: {error}"))?;
            if crate::secret_store::is_protected_transcript_text(&error) {
                continue;
            }
            let protected = protect_stored_transcript_text(&error)?;
            if protected != error {
                updates.push((id, protected));
            }
        }
        drop(statement);
        for (id, error) in updates {
            transaction
                .execute(
                    "UPDATE turns SET error = ?1 WHERE id = ?2",
                    params![error, id],
                )
                .map_err(|error| format!("encrypt legacy turn error: {error}"))?;
        }
    }

    Ok(())
}

fn protect_optional_stored_transcript_text(value: Option<&str>) -> Result<Option<String>, String> {
    value.map(protect_stored_transcript_text).transpose()
}

/// Protect transcript text for durable storage.
///
/// On Windows, DPAPI encryption is required for non-empty content. On other
/// platforms, `protect_transcript_text` is unavailable, so plaintext is kept
/// so unit tests and non-Windows CI still exercise the store.
fn protect_stored_transcript_text(value: &str) -> Result<String, String> {
    match crate::secret_store::protect_transcript_text(value) {
        Ok(protected) => Ok(protected),
        Err(error) if error.contains("only available on Windows") => {
            // Non-Windows builds cannot use DPAPI; leave plaintext intentionally.
            Ok(value.to_string())
        }
        Err(error) => Err(error),
    }
}

fn unprotect_stored_transcript_text(value: &str) -> Result<String, String> {
    match crate::secret_store::unprotect_transcript_text(value) {
        Ok(plain) => Ok(plain),
        Err(error) if error.contains("only available on Windows") => {
            // Ciphertext written on Windows cannot be opened elsewhere; surface
            // the failure rather than pretending the envelope is readable.
            Err(error)
        }
        Err(error) => Err(error),
    }
}

fn validate_stored_session_goal(goal: &StoredSessionGoal) -> Result<(), String> {
    let text = goal.text.trim();
    if text.is_empty() || text.chars().count() > MAX_SESSION_GOAL_TEXT_CHARS {
        return Err(format!(
            "session goal text must contain 1 to {MAX_SESSION_GOAL_TEXT_CHARS} characters"
        ));
    }
    if goal.updated_at <= 0 {
        return Err("session goal updated_at must be positive".to_string());
    }
    match goal.status.as_str() {
        "active" if goal.progress < 100 => Ok(()),
        "completed" if goal.progress == 100 => Ok(()),
        "active" | "completed" => {
            Err("session goal status and progress are inconsistent".to_string())
        }
        _ => Err("session goal status is invalid".to_string()),
    }
}

fn validate_segment_header_pair(
    header: &StoredHistoryHeader,
    segment: &StoredHistorySegment,
) -> Result<(), String> {
    if header.conversation_id.trim().is_empty() {
        return Err("history conversation id is required".to_string());
    }
    if header.total_segment_count <= 0
        || header.active_segment_index != header.total_segment_count - 1
        || header.total_message_count < 0
    {
        return Err("history segment header counts are inconsistent".to_string());
    }
    if segment.segment_index != header.active_segment_index
        || segment.segment_index < 0
        || segment.segment_id.trim().is_empty()
        || segment.message_count < 0
    {
        return Err("history active segment metadata is inconsistent".to_string());
    }
    Ok(())
}

fn load_segment_counts(
    connection: &Connection,
    conversation_id: &str,
) -> Result<Option<(i64, i64)>, String> {
    connection
        .query_row(
            "SELECT active_segment_index, total_segment_count
             FROM history_segment_headers WHERE conversation_id = ?1",
            params![conversation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("load history segment counts: {error}"))
}

fn upsert_segment_header(
    connection: &Connection,
    header: &StoredHistoryHeader,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO history_segment_headers(
               conversation_id, source_session_id, context_meta_json, active_segment_index,
               total_segment_count, total_message_count, created_at, updated_at
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(conversation_id) DO UPDATE SET
               source_session_id=excluded.source_session_id,
               context_meta_json=excluded.context_meta_json,
               active_segment_index=excluded.active_segment_index,
               total_segment_count=excluded.total_segment_count,
               total_message_count=excluded.total_message_count,
               updated_at=excluded.updated_at",
            params![
                header.conversation_id,
                header.session_id,
                header.context_meta_json,
                header.active_segment_index,
                header.total_segment_count,
                header.total_message_count,
                header.created_at,
                header.updated_at
            ],
        )
        .map_err(|error| format!("upsert history segment header: {error}"))?;
    Ok(())
}

fn verify_segment_totals(
    connection: &Connection,
    header: &StoredHistoryHeader,
) -> Result<(), String> {
    let (count, messages, min_index, max_index): (i64, i64, Option<i64>, Option<i64>) = connection
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(message_count), 0), MIN(segment_index), MAX(segment_index)
             FROM history_segments WHERE conversation_id = ?1",
            params![header.conversation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|error| format!("verify history segment totals: {error}"))?;
    if count != header.total_segment_count
        || messages != header.total_message_count
        || min_index != Some(0)
        || max_index != Some(header.active_segment_index)
    {
        return Err("history segment/message totals do not match the stored segments".to_string());
    }
    Ok(())
}

fn checkpoint_truncate(connection: &Connection) -> Result<(), String> {
    let busy: i64 = connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .map_err(|error| format!("checkpoint protected settings WAL: {error}"))?;
    if busy != 0 {
        return Err("checkpoint protected settings WAL: database is busy".to_string());
    }
    Ok(())
}

#[derive(Debug)]
struct ReplayEvent {
    id: String,
    event_type: String,
    payload: Value,
    sequence: Option<i64>,
    created_at: i64,
}

#[derive(Debug)]
struct ReplayTurn {
    id: String,
    started_at: i64,
    messages: Vec<StoredMessage>,
    events: Vec<ReplayEvent>,
}

impl ReplayTurn {
    fn new(id: String, started_at: i64) -> Self {
        Self {
            id,
            started_at,
            messages: Vec::new(),
            events: Vec::new(),
        }
    }
}

#[derive(Debug)]
struct ReplayBlock {
    created_at: i64,
    kind: u8,
    id: String,
    context: Vec<Value>,
}

fn replay_event_order(left: &ReplayEvent, right: &ReplayEvent) -> std::cmp::Ordering {
    match (left.sequence, right.sequence) {
        (Some(left_sequence), Some(right_sequence)) => left_sequence
            .cmp(&right_sequence)
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.id.cmp(&right.id)),
        _ => left
            .created_at
            .cmp(&right.created_at)
            .then_with(|| left.sequence.cmp(&right.sequence))
            .then_with(|| left.id.cmp(&right.id)),
    }
}

fn message_context(message: &StoredMessage) -> Option<Value> {
    let role = match message.role.as_str() {
        "user" => "user",
        "assistant" => "assistant",
        "tool" | "toolResult" | "tool_result" => "toolResult",
        _ => return None,
    };
    Some(json!({
        "role": role,
        "content": [{"type": "text", "text": message.content}],
        "timestamp": message.created_at,
    }))
}

fn append_tool_context(context: &mut Vec<Value>, events: &[ReplayEvent]) {
    for event in events {
        let event_type = event.event_type.as_str();
        let payload = &event.payload;
        let Some(tool) = payload.get("toolCall").or_else(|| payload.get("tool_call")) else {
            continue;
        };
        if event_type == "tool_call" {
            context.push(json!({
                "role": "assistant",
                "content": [{
                    "type": "toolCall",
                    "id": tool.get("id").and_then(Value::as_str).unwrap_or("history-tool"),
                    "name": tool.get("name").and_then(Value::as_str).unwrap_or("tool"),
                    "arguments": tool.get("arguments").or_else(|| tool.get("args")).cloned().unwrap_or_else(|| json!({})),
                }],
                "timestamp": event.created_at,
            }));
        } else if event_type == "tool_result" {
            let result = tool.get("result").cloned().unwrap_or(Value::Null);
            let result_text = result
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| result.to_string());
            context.push(json!({
                "role": "toolResult",
                "toolCallId": tool.get("id").and_then(Value::as_str).unwrap_or("history-tool"),
                "toolName": tool.get("name").and_then(Value::as_str).unwrap_or("tool"),
                "content": [{"type": "text", "text": result_text}],
                "details": result,
                "isError": tool.get("status").and_then(Value::as_str) == Some("failed") || tool.get("error").is_some(),
                "timestamp": event.created_at,
            }));
        }
    }
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str).map(str::to_string))
        .filter(|value| !value.trim().is_empty())
}

/// Terminal event JSON is an internal audit record. A malformed legacy value
/// must not prevent the transcript itself from loading.
fn thinking_from_terminal_payload(payload_json: &str) -> Option<String> {
    unprotect_stored_transcript_text(payload_json)
        .ok()
        .and_then(|payload_json| serde_json::from_str::<Value>(&payload_json).ok())
        .and_then(|payload| display_safe_terminal_thinking(&payload))
}

fn display_safe_terminal_thinking(payload: &Value) -> Option<String> {
    if !matches!(
        payload.get("type").and_then(Value::as_str),
        Some("done" | "error" | "cancelled")
    ) {
        return None;
    }
    let raw = payload.get("thinking").and_then(Value::as_str)?;
    let mut safe = String::with_capacity(raw.len().min(MAX_STORED_TERMINAL_THINKING_CHARS * 4));
    for character in raw.chars().take(MAX_STORED_TERMINAL_THINKING_CHARS) {
        match character {
            '\n' | '\t' => safe.push(character),
            '\r' => safe.push('\n'),
            character
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
                    ) =>
            {
                safe.push('\u{fffd}');
            }
            _ => safe.push(character),
        }
    }
    (!safe.trim().is_empty()).then_some(safe)
}

fn session_redacts_tool_content(connection: &Connection, session_id: &str) -> Result<bool, String> {
    let redact: i64 = connection
        .query_row(
            "SELECT COALESCE((
                 SELECT redact_tool_content
                 FROM session_metadata
                 WHERE session_id = ?1
             ), 1)",
            params![session_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("read session tool redaction policy: {error}"))?;
    Ok(redact != 0)
}

/// Build a metadata-only event projection before SQLite receives a redacted
/// session's event. This avoids relying on secret-shaped field names and
/// prevents future tool payload extensions from becoming durable by default.
fn bounded_context_trim_integer(
    source: &Map<String, Value>,
    key: &str,
    maximum: u64,
) -> Option<Value> {
    source
        .get(key)
        .and_then(Value::as_u64)
        .filter(|value| *value <= maximum)
        .map(Value::from)
}

fn context_fingerprint(value: &str) -> bool {
    value.len() == 16
        && value.starts_with("fnv1a32:")
        && value[8..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

/// File activity is renderer-generated metadata, so retain only the small,
/// canonical shape used by deterministic context compaction. The transcript
/// summary itself remains the authoritative encrypted copy; this projection is
/// solely for audit/UI accounting and must never accept control text.
fn sanitize_context_file_ledger(value: &Value) -> Option<Value> {
    const MAX_ENTRIES_PER_KIND: usize = 100;
    const MAX_PATH_CHARS: usize = 200;
    const MAX_TOTAL_PATH_CHARS: usize = 4_000;
    const MAX_OMITTED_COUNT: u64 = 1_000_000_000;
    let source = value.as_object()?;
    if source.get("version").and_then(Value::as_u64) != Some(1) {
        return None;
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut total_chars = 0usize;
    let sanitize_paths = |key: &str,
                          seen: &mut std::collections::BTreeSet<String>,
                          total_chars: &mut usize|
     -> Option<Vec<Value>> {
        let values = source.get(key)?.as_array()?;
        if values.len() > MAX_ENTRIES_PER_KIND {
            return None;
        }
        let mut stored = Vec::with_capacity(values.len());
        for value in values {
            let path = value.as_str()?;
            let path_chars = path.chars().count();
            if path.is_empty()
                || path.trim() != path
                || path_chars > MAX_PATH_CHARS
                || path.chars().any(char::is_control)
                || !seen.insert(path.to_string())
            {
                return None;
            }
            *total_chars = total_chars.checked_add(path_chars)?;
            if *total_chars > MAX_TOTAL_PATH_CHARS {
                return None;
            }
            stored.push(Value::from(path));
        }
        Some(stored)
    };
    let read = sanitize_paths("read", &mut seen, &mut total_chars)?;
    let modified = sanitize_paths("modified", &mut seen, &mut total_chars)?;
    let omitted_count = source
        .get("omittedCount")
        .and_then(Value::as_u64)
        .filter(|value| *value <= MAX_OMITTED_COUNT)?;
    let mut stored = Map::new();
    stored.insert("version".to_string(), Value::from(1));
    stored.insert("read".to_string(), Value::Array(read));
    stored.insert("modified".to_string(), Value::Array(modified));
    stored.insert("omittedCount".to_string(), Value::from(omitted_count));
    Some(Value::Object(stored))
}

fn sanitize_context_compaction(value: &Value) -> Option<Value> {
    const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
    const MAX_CONTEXT_METRIC: u64 = 1_000_000_000;
    let source = value.as_object()?;
    if source.get("version").and_then(Value::as_u64) != Some(1)
        || source.get("mode").and_then(Value::as_str) != Some("deterministic_structured")
        || !matches!(
            source.get("trigger").and_then(Value::as_str),
            Some("near_limit") | Some("overflow") | Some("manual")
        )
    {
        return None;
    }
    let fingerprint = source.get("sourceFingerprint")?.as_str()?;
    let summary_id = source.get("summaryId")?.as_str()?;
    if !context_fingerprint(fingerprint)
        || summary_id != format!("novavei-context-v1:{fingerprint}")
    {
        return None;
    }
    let metric_keys = [
        "sourceMessageStart",
        "sourceMessageEnd",
        "sourceTurnStart",
        "sourceTurnEnd",
        "sourceMessages",
        "sourceTurns",
        "sourceTokens",
        "summaryTokens",
        "targetTokens",
        "indexedTurns",
        "omittedTurns",
        "redactedFragments",
        "syntheticMessages",
    ];
    let mut stored = Map::new();
    stored.insert("version".to_string(), Value::from(1));
    stored.insert("summaryId".to_string(), Value::from(summary_id));
    stored.insert(
        "generatedAt".to_string(),
        bounded_context_trim_integer(source, "generatedAt", MAX_SAFE_INTEGER)?,
    );
    stored.insert("mode".to_string(), Value::from("deterministic_structured"));
    stored.insert("trigger".to_string(), source.get("trigger")?.clone());
    stored.insert("sourceFingerprint".to_string(), Value::from(fingerprint));
    for key in metric_keys {
        stored.insert(
            key.to_string(),
            bounded_context_trim_integer(source, key, MAX_CONTEXT_METRIC)?,
        );
    }
    let metric = |key: &str| stored.get(key).and_then(Value::as_u64);
    let source_messages = metric("sourceMessages")?;
    let source_turns = metric("sourceTurns")?;
    let source_tokens = metric("sourceTokens")?;
    let summary_tokens = metric("summaryTokens")?;
    let target_tokens = metric("targetTokens")?;
    if metric("sourceMessageStart")? != 1
        || metric("sourceMessageEnd")? != source_messages
        || metric("sourceTurnStart")? != 1
        || metric("sourceTurnEnd")? != source_turns
        || source_messages == 0
        || source_turns == 0
        || source_turns > source_messages
        || source_tokens == 0
        || summary_tokens == 0
        || !(128..=6_144).contains(&target_tokens)
        || metric("indexedTurns")?.checked_add(metric("omittedTurns")?)? != source_turns
        || metric("syntheticMessages")? != 1
        || summary_tokens > target_tokens
    {
        return None;
    }
    if let Some(file_ledger) = source.get("fileLedger") {
        stored.insert(
            "fileLedger".to_string(),
            sanitize_context_file_ledger(file_ledger)?,
        );
    }
    Some(Value::Object(stored))
}

fn validate_manual_context_compaction(value: &Value) -> Result<Value, String> {
    let sanitized = sanitize_context_compaction(value)
        .ok_or_else(|| "manual context summary metadata is invalid".to_string())?;
    if sanitized.get("trigger").and_then(Value::as_str) != Some("manual") {
        return Err("manual context summary must use the manual trigger".to_string());
    }
    Ok(sanitized)
}

fn sanitize_context_trim(value: &Value) -> Option<Value> {
    const MAX_CONTEXT_METRIC: u64 = 1_000_000_000;
    let source = value.as_object()?;
    let metric_keys = [
        "contextWindow",
        "maxOutputTokens",
        "fixedTokens",
        "historyBudgetTokens",
        "originalHistoryTokens",
        "keptHistoryTokens",
        "originalMessages",
        "keptMessages",
        "droppedMessages",
        "originalTurns",
        "keptTurns",
    ];
    let mut stored = Map::new();
    for key in metric_keys {
        stored.insert(
            key.to_string(),
            bounded_context_trim_integer(source, key, MAX_CONTEXT_METRIC)?,
        );
    }
    stored.insert(
        "trimmed".to_string(),
        Value::from(source.get("trimmed")?.as_bool()?),
    );
    let metric = |key: &str| stored.get(key).and_then(Value::as_u64);
    let context_window = metric("contextWindow")?;
    let max_output_tokens = metric("maxOutputTokens")?;
    let fixed_tokens = metric("fixedTokens")?;
    let history_budget_tokens = metric("historyBudgetTokens")?;
    let original_history_tokens = metric("originalHistoryTokens")?;
    let kept_history_tokens = metric("keptHistoryTokens")?;
    let original_messages = metric("originalMessages")?;
    let kept_messages = metric("keptMessages")?;
    let dropped_messages = metric("droppedMessages")?;
    let original_turns = metric("originalTurns")?;
    let kept_turns = metric("keptTurns")?;
    let trimmed = stored.get("trimmed")?.as_bool()?;
    let dropped_turns = original_turns.checked_sub(kept_turns)?;
    let safety_tokens = (context_window / 20).clamp(128, 2_048);
    let expected_history_budget = context_window
        .saturating_sub(max_output_tokens)
        .saturating_sub(safety_tokens)
        .saturating_sub(fixed_tokens);
    if context_window == 0
        || kept_messages > original_messages
        || dropped_messages != original_messages.checked_sub(kept_messages)?
        || kept_turns > original_turns
        || original_turns > original_messages
        || kept_turns > kept_messages
        || dropped_messages < dropped_turns
        || max_output_tokens > context_window
        || history_budget_tokens != expected_history_budget
        || kept_history_tokens > history_budget_tokens
        || trimmed != (dropped_messages > 0)
        || trimmed != (dropped_turns > 0)
    {
        return None;
    }
    if let Some(compaction) = source.get("compaction") {
        let sanitized = sanitize_context_compaction(compaction)?;
        let compact = sanitized.as_object()?;
        let compact_metric = |key: &str| compact.get(key).and_then(Value::as_u64);
        let compact_source_tokens = compact_metric("sourceTokens")?;
        let compact_summary_tokens = compact_metric("summaryTokens")?;
        let compact_target_tokens = compact_metric("targetTokens")?;
        let retained_history_tokens = original_history_tokens.checked_sub(compact_source_tokens)?;
        let minimum_compacted_tokens =
            retained_history_tokens.checked_add(compact_summary_tokens)?;
        if !trimmed
            || compact_metric("sourceMessages")? != dropped_messages
            || compact_metric("sourceTurns")? != dropped_turns
            || compact_metric("sourceTurns")?.checked_add(kept_turns)? != original_turns
            || compact_source_tokens == 0
            || compact_source_tokens > original_history_tokens
            || compact_summary_tokens > compact_target_tokens
            || kept_history_tokens < minimum_compacted_tokens
            || compact_target_tokens > history_budget_tokens
        {
            return None;
        }
        stored.insert("compaction".to_string(), sanitized);
    } else if kept_history_tokens > original_history_tokens
        || (!trimmed && kept_history_tokens != original_history_tokens)
    {
        return None;
    }
    Some(Value::Object(stored))
}

fn redact_event_for_storage(payload: &Value) -> Value {
    let mut stored = Map::new();
    for key in [
        "type",
        "sessionId",
        "session_id",
        "conversationId",
        "conversation_id",
        "turnId",
        "turn_id",
        "requestId",
        "request_id",
        "eventId",
        "event_id",
        "sequence",
    ] {
        if let Some(value) = payload.get(key) {
            stored.insert(key.to_string(), value.clone());
        }
    }
    if let Some(tool) = payload
        .get("toolCall")
        .or_else(|| payload.get("tool_call"))
        .and_then(Value::as_object)
    {
        let mut safe_tool = Map::new();
        for key in ["id", "name", "status"] {
            if let Some(value) = tool.get(key) {
                safe_tool.insert(key.to_string(), value.clone());
            }
        }
        stored.insert("toolCall".to_string(), Value::Object(safe_tool));
    }
    if let Some(permission) = payload.get("permission").and_then(Value::as_object) {
        let mut safe_permission = Map::new();
        for key in ["id", "toolName", "tool_name", "decision"] {
            if let Some(value) = permission.get(key) {
                safe_permission.insert(key.to_string(), value.clone());
            }
        }
        stored.insert("permission".to_string(), Value::Object(safe_permission));
    }
    if payload.get("type").and_then(Value::as_str) == Some("run_started") {
        if let Some(context_trim) = payload
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get("contextTrim"))
            .and_then(sanitize_context_trim)
        {
            stored.insert(
                "metadata".to_string(),
                json!({ "contextTrim": context_trim }),
            );
        }
    }
    if let Some(thinking) = display_safe_terminal_thinking(payload) {
        stored.insert("thinking".to_string(), Value::String(thinking));
    }
    Value::Object(stored)
}

/// Keep a user-configured full event payload intact while still projecting the
/// renderer-supplied context accounting through its fixed schema.  This must
/// run independently of the tool-result redaction preference.
fn sanitize_run_started_context_trim(mut payload: Value) -> Value {
    if payload.get("type").and_then(Value::as_str) != Some("run_started") {
        return payload;
    }
    if let Some(metadata) = payload
        .as_object_mut()
        .and_then(|root| root.get_mut("metadata"))
        .and_then(Value::as_object_mut)
    {
        let context_trim = metadata.get("contextTrim").and_then(sanitize_context_trim);
        if let Some(context_trim) = context_trim {
            metadata.insert("contextTrim".to_string(), context_trim);
        } else {
            metadata.remove("contextTrim");
        }
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> (HistoryStore, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "novavei-history-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let store = HistoryStore::new(path.clone());
        store
            .initialize()
            .expect("history schema should initialize");
        (store, path)
    }

    #[test]
    fn context_fingerprint_matches_the_typescript_fingerprint_shape() {
        assert!(context_fingerprint("fnv1a32:0123abcd"));
        assert!(!context_fingerprint("fnv1a32:0123abc"));
        assert!(!context_fingerprint("fnv1a32:0123abcde"));
        assert!(!context_fingerprint("fnv1a32:0123abcg"));
    }

    #[test]
    fn diagnostics_summary_returns_only_aggregate_turn_data() {
        let (store, path) = test_store();
        {
            let connection = store.open().unwrap();
            connection
                .execute(
                    "INSERT INTO sessions(id, title, cwd, created_at, updated_at)
                     VALUES('diagnostics-session', 'sensitive title', 'C:\\sensitive\\workspace', 1, 1)",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO turns(id, session_id, conversation_id, request_id, status, started_at)
                     VALUES('turn-completed', 'diagnostics-session', 'sensitive-conversation', 'sensitive-request', 'completed', 10),
                     ('turn-failed', 'diagnostics-session', 'another-conversation', 'another-request', 'failed', 20)",
                    [],
                )
                .unwrap();
        }

        let summary = store.diagnostics_summary().unwrap();

        assert_eq!(summary.session_count, 1);
        assert_eq!(summary.turn_count, 2);
        assert_eq!(summary.turn_status_counts.get("completed"), Some(&1));
        assert_eq!(summary.turn_status_counts.get("failed"), Some(&1));
        assert_eq!(summary.oldest_turn_started_at, Some(10));
        assert_eq!(summary.latest_turn_started_at, Some(20));
        drop(store);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn redacted_sessions_do_not_persist_tool_bodies_or_raw_errors() {
        let (store, path) = test_store();
        let session = StoredSession {
            id: "session-redacted-tools".to_string(),
            title: "redacted".to_string(),
            cwd: "C:\\workspace".to_string(),
            updated_at: 1,
            provider_id: "embedded".to_string(),
            model: String::new(),
            selected_model_json: None,
            pinned_at: None,
            archived_at: None,
            share_enabled: false,
            share_token: None,
            share_created_at: None,
            share_updated_at: None,
            redact_tool_content: true,
        };
        store
            .replace_snapshot(std::slice::from_ref(&session), &[], &HashMap::new())
            .unwrap();
        store
            .upsert_turn(
                &session.id,
                "conversation-redacted-tools",
                "turn-redacted-tools",
                "request-redacted-tools",
                "running",
                None,
                None,
                None,
                None,
                1,
            )
            .unwrap();

        store
            .append_event(
                &json!({
                    "type": "tool_result",
                    "sessionId": session.id,
                    "turnId": "turn-redacted-tools",
                    "requestId": "request-redacted-tools",
                    "sequence": 1,
                    "toolCall": {
                        "id": "tool-redacted-tools",
                        "name": "Read",
                        "arguments": {"path": ".env", "token": "tool-argument-secret"},
                        "result": "tool-result-secret",
                        "error": "tool-error-secret",
                        "status": "failed"
                    },
                    "error": "event-error-secret"
                }),
                2,
            )
            .unwrap();

        let connection = store.open().unwrap();
        let stored_event: String = connection
            .query_row(
                "SELECT payload_json FROM turn_events WHERE turn_id = 'turn-redacted-tools'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let stored_tool: (Option<String>, Option<String>, Option<String>) = connection
            .query_row(
                "SELECT arguments_json, result_json, error FROM tool_calls WHERE id = 'tool-redacted-tools'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert!(!stored_event.contains("tool-argument-secret"));
        assert!(!stored_event.contains("tool-result-secret"));
        assert!(!stored_event.contains("tool-error-secret"));
        assert!(!stored_event.contains("event-error-secret"));
        assert_eq!(stored_tool, (None, None, None));
        drop(connection);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn sessions_without_metadata_default_to_redacted_tool_storage() {
        let (store, path) = test_store();
        {
            let connection = store.open().unwrap();
            connection
                .execute(
                    "INSERT INTO sessions(id, title, cwd, created_at, updated_at)
                     VALUES('session-default-redaction', 'legacy', 'C:\\workspace', 1, 1)",
                    [],
                )
                .unwrap();
        }
        store
            .upsert_turn(
                "session-default-redaction",
                "conversation-default-redaction",
                "turn-default-redaction",
                "request-default-redaction",
                "running",
                None,
                None,
                None,
                None,
                1,
            )
            .unwrap();
        store
            .append_event(
                &json!({
                    "type": "tool_call",
                    "sessionId": "session-default-redaction",
                    "turnId": "turn-default-redaction",
                    "requestId": "request-default-redaction",
                    "toolCall": {
                        "id": "tool-default-redaction",
                        "name": "Read",
                        "arguments": {"token": "default-redaction-secret"}
                    }
                }),
                2,
            )
            .unwrap();

        let connection = store.open().unwrap();
        let stored_event: String = connection
            .query_row(
                "SELECT payload_json FROM turn_events WHERE turn_id = 'turn-default-redaction'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!stored_event.contains("default-redaction-secret"));
        drop(connection);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn explicit_non_redacted_sessions_preserve_tool_content() {
        let (store, path) = test_store();
        let session = StoredSession {
            id: "session-unredacted-tools".to_string(),
            title: "unredacted".to_string(),
            cwd: "C:\\workspace".to_string(),
            updated_at: 1,
            provider_id: "embedded".to_string(),
            model: String::new(),
            selected_model_json: None,
            pinned_at: None,
            archived_at: None,
            share_enabled: false,
            share_token: None,
            share_created_at: None,
            share_updated_at: None,
            redact_tool_content: false,
        };
        store
            .replace_snapshot(std::slice::from_ref(&session), &[], &HashMap::new())
            .unwrap();
        store
            .upsert_turn(
                &session.id,
                "conversation-unredacted-tools",
                "turn-unredacted-tools",
                "request-unredacted-tools",
                "running",
                None,
                None,
                None,
                None,
                1,
            )
            .unwrap();
        store
            .append_event(
                &json!({
                    "type": "tool_result",
                    "sessionId": session.id,
                    "turnId": "turn-unredacted-tools",
                    "requestId": "request-unredacted-tools",
                    "toolCall": {
                        "id": "tool-unredacted-tools",
                        "name": "Read",
                        "arguments": {"path": "notes.txt"},
                        "result": "explicitly-retained-tool-content",
                        "status": "completed"
                    }
                }),
                2,
            )
            .unwrap();

        let connection = store.open().unwrap();
        let stored_event: String = connection
            .query_row(
                "SELECT payload_json FROM turn_events WHERE turn_id = 'turn-unredacted-tools'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        #[cfg(windows)]
        assert!(crate::secret_store::is_protected_transcript_text(
            &stored_event
        ));
        let stored_event = unprotect_stored_transcript_text(&stored_event).unwrap();
        assert!(stored_event.contains("explicitly-retained-tool-content"));
        drop(connection);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn non_redacted_tool_payloads_are_encrypted_at_rest_and_round_trip() {
        let (store, path) = test_store();
        let session = StoredSession {
            id: "session-encrypted-tools".to_string(),
            title: "encrypted".to_string(),
            cwd: "C:\\workspace".to_string(),
            updated_at: 1,
            provider_id: "embedded".to_string(),
            model: String::new(),
            selected_model_json: None,
            pinned_at: None,
            archived_at: None,
            share_enabled: false,
            share_token: None,
            share_created_at: None,
            share_updated_at: None,
            redact_tool_content: false,
        };
        store
            .replace_snapshot(std::slice::from_ref(&session), &[], &HashMap::new())
            .unwrap();
        store
            .upsert_turn(
                &session.id,
                "conversation-encrypted-tools",
                "turn-encrypted-tools",
                "request-encrypted-tools",
                "running",
                None,
                None,
                None,
                None,
                1,
            )
            .unwrap();
        store
            .append_event(
                &json!({
                    "type": "tool_result",
                    "sessionId": session.id,
                    "turnId": "turn-encrypted-tools",
                    "requestId": "request-encrypted-tools",
                    "sequence": 1,
                    "toolCall": {
                        "id": "tool-encrypted-tools",
                        "name": "Read",
                        "arguments": {"path": ".env", "token": "tool-argument-secret-value"},
                        "result": "tool-result-secret-value",
                        "status": "completed"
                    }
                }),
                2,
            )
            .unwrap();
        store
            .append_event(
                &json!({
                    "type": "permission_requested",
                    "sessionId": session.id,
                    "turnId": "turn-encrypted-tools",
                    "requestId": "request-encrypted-tools",
                    "sequence": 2,
                    "permission": {
                        "id": "permission-encrypted-tools",
                        "toolName": "Bash",
                        "command": "curl -H 'Authorization: permission-secret-value'"
                    }
                }),
                3,
            )
            .unwrap();
        store
            .append_event(
                &json!({
                    "type": "error",
                    "sessionId": session.id,
                    "turnId": "turn-encrypted-tools",
                    "requestId": "request-encrypted-tools",
                    "error": "provider rejected key turn-error-secret-value"
                }),
                4,
            )
            .unwrap();

        // At rest, none of the durable event/tool/permission/error columns may
        // retain plaintext once the platform envelope is available.
        #[cfg(windows)]
        {
            let connection = store.open().unwrap();
            let mut statement = connection
                .prepare("SELECT payload_json FROM turn_events WHERE session_id = 'session-encrypted-tools'")
                .unwrap();
            let payloads = statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(!payloads.is_empty());
            for payload in &payloads {
                assert!(crate::secret_store::is_protected_transcript_text(payload));
                assert!(!payload.contains("secret-value"));
            }
            let (arguments, result): (Option<String>, Option<String>) = connection
                .query_row(
                    "SELECT arguments_json, result_json FROM tool_calls WHERE id = 'tool-encrypted-tools'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            for column in [arguments, result] {
                let value = column.expect("non-redacted tool columns should persist");
                assert!(crate::secret_store::is_protected_transcript_text(&value));
                assert!(!value.contains("secret-value"));
            }
            let request_json: String = connection
                .query_row(
                    "SELECT request_json FROM permission_requests WHERE id = 'permission-encrypted-tools'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(crate::secret_store::is_protected_transcript_text(
                &request_json
            ));
            assert!(!request_json.contains("secret-value"));
            let turn_error: Option<String> = connection
                .query_row(
                    "SELECT error FROM turns WHERE id = 'turn-encrypted-tools'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let turn_error = turn_error.expect("terminal error should persist");
            assert!(crate::secret_store::is_protected_transcript_text(
                &turn_error
            ));
            assert!(!turn_error.contains("secret-value"));
        }

        // Round trip: replay still surfaces the retained tool content.
        let context = store.load_context(&session.id).unwrap();
        let serialized = serde_json::to_string(&context).unwrap();
        assert!(serialized.contains("tool-result-secret-value"));
        let terminal = store
            .load_terminal_event("request-encrypted-tools")
            .unwrap()
            .expect("terminal event should load");
        assert_eq!(
            terminal.get("error").and_then(Value::as_str),
            Some("provider rejected key turn-error-secret-value")
        );
        drop(store);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn secure_compaction_enables_secure_delete_and_is_idempotent() {
        let (store, path) = test_store();
        const PLAINTEXT_SECRET: &str = "legacy-plaintext-secret-for-compaction-test";
        const PROTECTED_SECRET: &str = "__novavei_dpapi_v1:00112233445566778899aabbccddeeff";
        {
            let connection = store.open().unwrap();
            let secure_delete: i64 = connection
                .query_row("PRAGMA secure_delete", [], |row| row.get(0))
                .unwrap();
            assert_eq!(secure_delete, 1);
        }

        let mut settings = HashMap::new();
        settings.insert("providers".to_string(), json!({"apiKey": PLAINTEXT_SECRET}));
        store.replace_snapshot(&[], &[], &settings).unwrap();

        settings.insert("providers".to_string(), json!({"apiKey": PROTECTED_SECRET}));
        store.replace_snapshot(&[], &[], &settings).unwrap();
        assert_eq!(
            store.load_settings().unwrap()["providers"]["apiKey"],
            PROTECTED_SECRET
        );

        assert!(store.secure_compact_once().unwrap());
        assert!(!store.secure_compact_once().unwrap());

        let connection = store.open().unwrap();
        let marker: String = connection
            .query_row(
                "SELECT value FROM schema_meta WHERE key = ?1",
                params![SECURE_SETTINGS_MIGRATION_KEY],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker, "1");
        drop(connection);

        let plaintext = PLAINTEXT_SECRET.as_bytes();
        for database_file in [
            path.clone(),
            path.with_extension("sqlite3-wal"),
            path.with_extension("sqlite3-shm"),
        ] {
            if database_file.exists() {
                let bytes = fs::read(&database_file).unwrap();
                assert!(
                    !bytes
                        .windows(plaintext.len())
                        .any(|window| window == plaintext),
                    "plaintext secret remained in {}",
                    database_file.display()
                );
            }
        }

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn snapshot_roundtrips_session_metadata_and_cascades_on_delete() {
        let (store, path) = test_store();
        let session = StoredSession {
            id: "session-metadata".to_string(),
            title: "metadata".to_string(),
            cwd: "C:\\workspace".to_string(),
            updated_at: 42,
            provider_id: "provider-a".to_string(),
            model: "model-a".to_string(),
            selected_model_json: Some(r#"{"id":"model-a","label":"Model A"}"#.to_string()),
            pinned_at: Some(100),
            archived_at: Some(200),
            share_enabled: true,
            share_token: Some("share-token".to_string()),
            share_created_at: Some(300),
            share_updated_at: Some(400),
            redact_tool_content: true,
        };
        store
            .replace_snapshot(std::slice::from_ref(&session), &[], &HashMap::new())
            .unwrap();

        let loaded = store.load_sessions().unwrap();
        assert_eq!(loaded.len(), 1);
        let loaded = &loaded[0];
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.provider_id, session.provider_id);
        assert_eq!(loaded.model, session.model);
        assert_eq!(loaded.selected_model_json, session.selected_model_json);
        assert_eq!(loaded.pinned_at, session.pinned_at);
        assert_eq!(loaded.archived_at, session.archived_at);
        assert_eq!(loaded.share_enabled, session.share_enabled);
        assert_eq!(loaded.share_token, session.share_token);
        assert_eq!(loaded.share_created_at, session.share_created_at);
        assert_eq!(loaded.share_updated_at, session.share_updated_at);
        assert_eq!(loaded.redact_tool_content, session.redact_tool_content);

        {
            let connection = store.open().unwrap();
            let schema_version: String = connection
                .query_row(
                    "SELECT value FROM schema_meta WHERE key = 'version'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(schema_version, SCHEMA_VERSION.to_string());
            let metadata_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM session_metadata WHERE session_id = 'session-metadata'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(metadata_count, 1);
        }

        // A session written by the pre-metadata schema still loads with
        // stable defaults through the LEFT JOIN projection.
        {
            let connection = store.open().unwrap();
            connection
                .execute(
                    "INSERT INTO sessions(id, title, cwd, created_at, updated_at)
                     VALUES('session-legacy', 'legacy', 'C:\\legacy', 1, 1)",
                    [],
                )
                .unwrap();
        }
        let legacy = store
            .load_sessions()
            .unwrap()
            .into_iter()
            .find(|item| item.id == "session-legacy")
            .unwrap();
        assert_eq!(legacy.provider_id, "embedded");
        assert_eq!(legacy.model, "");
        assert_eq!(legacy.selected_model_json, None);
        assert_eq!(legacy.pinned_at, None);
        assert_eq!(legacy.archived_at, None);
        assert!(!legacy.share_enabled);
        assert_eq!(legacy.share_token, None);
        assert_eq!(legacy.share_created_at, None);
        assert_eq!(legacy.share_updated_at, None);
        assert!(legacy.redact_tool_content);

        store.replace_snapshot(&[], &[], &HashMap::new()).unwrap();
        let connection = store.open().unwrap();
        let sessions: i64 = connection
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        let metadata: i64 = connection
            .query_row("SELECT COUNT(*) FROM session_metadata", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(sessions, 0);
        assert_eq!(metadata, 0);
        drop(connection);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn session_goals_roundtrip_with_bounded_fields_and_cascade() {
        let (store, path) = test_store();
        let session = StoredSession {
            id: "session-goal".to_string(),
            title: "goal".to_string(),
            cwd: "C:\\workspace".to_string(),
            updated_at: 42,
            provider_id: "embedded".to_string(),
            model: String::new(),
            selected_model_json: None,
            pinned_at: None,
            archived_at: None,
            share_enabled: false,
            share_token: None,
            share_created_at: None,
            share_updated_at: None,
            redact_tool_content: true,
        };
        store
            .replace_snapshot(std::slice::from_ref(&session), &[], &HashMap::new())
            .unwrap();
        let goal = StoredSessionGoal {
            text: "Finish the native session goal boundary".to_string(),
            status: "active".to_string(),
            progress: 0,
            updated_at: 99,
        };
        store.upsert_session_goal(&session.id, &goal).unwrap();
        assert_eq!(
            store.load_session_goals().unwrap().get(&session.id),
            Some(&goal)
        );

        let invalid = StoredSessionGoal {
            status: "completed".to_string(),
            progress: 25,
            ..goal.clone()
        };
        assert!(store.upsert_session_goal(&session.id, &invalid).is_err());

        store.replace_snapshot(&[], &[], &HashMap::new()).unwrap();
        assert!(store.load_session_goals().unwrap().is_empty());
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn initialize_adds_session_goals_to_a_v3_database() {
        let path = std::env::temp_dir().join(format!(
            "novavei-history-v3-goals-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE schema_meta (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                     INSERT INTO schema_meta(key, value) VALUES('version', '3');
                     CREATE TABLE sessions (
                       id TEXT PRIMARY KEY NOT NULL,
                       title TEXT NOT NULL,
                       cwd TEXT NOT NULL,
                       created_at INTEGER NOT NULL,
                       updated_at INTEGER NOT NULL
                     );",
                )
                .unwrap();
        }
        let store = HistoryStore::new(path.clone());
        store.initialize().unwrap();
        let connection = store.open().unwrap();
        let table: String = connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'session_goals'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let version: String = connection
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table, "session_goals");
        assert_eq!(version, SCHEMA_VERSION.to_string());
        drop(connection);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn initialize_installs_session_indexes_and_connection_timeout() {
        let (store, path) = test_store();
        let connection = store.open().unwrap();
        let timeout: i64 = connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(timeout, HISTORY_BUSY_TIMEOUT.as_millis() as i64);
        for index in [
            "turns_session_finished_order",
            "turn_events_session_order",
            "tool_calls_session_turn",
            "permission_requests_session_order",
        ] {
            let exists: i64 = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
                    params![index],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(exists, 1, "missing index {index}");
        }
        drop(connection);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn snapshot_preserves_pi_audit_rows_and_foreign_keys() {
        let (store, path) = test_store();
        let session = StoredSession {
            id: "session-1".to_string(),
            title: "test".to_string(),
            cwd: std::env::current_dir().unwrap().display().to_string(),
            updated_at: 1,
            provider_id: "".to_string(),
            model: "".to_string(),
            selected_model_json: None,
            pinned_at: None,
            archived_at: None,
            share_enabled: false,
            share_token: None,
            share_created_at: None,
            share_updated_at: None,
            redact_tool_content: false,
        };
        let message = StoredMessage {
            id: "message-1".to_string(),
            session_id: session.id.clone(),
            role: "user".to_string(),
            content: "hello".to_string(),
            created_at: 1,
            turn_id: Some("turn-1".to_string()),
        };
        store
            .replace_snapshot(
                std::slice::from_ref(&session),
                std::slice::from_ref(&message),
                &HashMap::new(),
            )
            .unwrap();
        store
            .upsert_turn(
                &session.id,
                "conversation-1",
                "turn-1",
                "request-1",
                "running",
                Some("codex"),
                Some("model"),
                None,
                Some(&session.cwd),
                1,
            )
            .unwrap();
        store
            .append_event(
                &json!({
                    "type": "tool_call",
                    "sessionId": session.id,
                    "turnId": "turn-1",
                    "requestId": "request-1",
                    "sequence": 1,
                    "toolCall": {"id": "tool-1", "name": "Read", "arguments": {"path": "a.txt"}}
                }),
                2,
            )
            .unwrap();
        store
            .append_event(
                &json!({
                    "type": "tool_result",
                    "sessionId": session.id,
                    "turnId": "turn-1",
                    "requestId": "request-1",
                    "sequence": 2,
                    "toolCall": {"id": "tool-1", "name": "Read", "result": "contents", "status": "completed"}
                }),
                3,
            )
            .unwrap();
        store
            .replace_snapshot(
                std::slice::from_ref(&session),
                std::slice::from_ref(&message),
                &HashMap::new(),
            )
            .unwrap();
        let connection = store.open().unwrap();
        let turns: i64 = connection
            .query_row("SELECT COUNT(*) FROM turns WHERE id='turn-1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        let events: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM turn_events WHERE turn_id='turn-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(turns, 1);
        assert_eq!(events, 2);
        let context = store.load_context(&session.id).unwrap();
        assert!(context
            .iter()
            .any(|item| item.get("role").and_then(Value::as_str) == Some("toolResult")));
        drop(connection);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn first_terminal_event_wins_for_a_request() {
        let (store, path) = test_store();
        let session = StoredSession {
            id: "session-terminal".to_string(),
            title: "terminal".to_string(),
            cwd: std::env::current_dir().unwrap().display().to_string(),
            updated_at: 1,
            provider_id: "".to_string(),
            model: "".to_string(),
            selected_model_json: None,
            pinned_at: None,
            archived_at: None,
            share_enabled: false,
            share_token: None,
            share_created_at: None,
            share_updated_at: None,
            redact_tool_content: false,
        };
        store
            .replace_snapshot(std::slice::from_ref(&session), &[], &HashMap::new())
            .unwrap();
        store
            .upsert_turn(
                &session.id,
                "conversation-terminal",
                "turn-terminal",
                "request-terminal",
                "running",
                None,
                None,
                None,
                Some(&session.cwd),
                1,
            )
            .unwrap();
        assert!(store
            .append_event(
                &json!({
                    "type": "cancelled",
                    "sessionId": session.id,
                    "turnId": "turn-terminal",
                    "requestId": "request-terminal"
                }),
                2,
            )
            .unwrap());
        assert!(!store
            .append_event(
                &json!({
                    "type": "done",
                    "sessionId": session.id,
                    "turnId": "turn-terminal",
                    "requestId": "request-terminal",
                    "sequence": 9,
                    "text": "late answer"
                }),
                3,
            )
            .unwrap());
        let connection = store.open().unwrap();
        let terminal_events: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM turn_events WHERE request_id='request-terminal' AND event_type IN ('done','error','cancelled')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let status: String = connection
            .query_row(
                "SELECT status FROM turns WHERE request_id='request-terminal'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(terminal_events, 1);
        assert_eq!(status, "cancelled");
        let summaries = store.load_session_run_summaries().unwrap();
        assert_eq!(
            summaries.get(&session.id),
            Some(&StoredSessionRunSummary {
                status: "cancelled".to_string(),
                finished_at: 2,
            })
        );
        drop(connection);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn duplicate_tool_and_terminal_events_are_idempotent() {
        let (store, path) = test_store();
        let session = StoredSession {
            id: "session-idempotent".to_string(),
            title: "idempotent".to_string(),
            cwd: std::env::current_dir().unwrap().display().to_string(),
            updated_at: 1,
            provider_id: "".to_string(),
            model: "".to_string(),
            selected_model_json: None,
            pinned_at: None,
            archived_at: None,
            share_enabled: false,
            share_token: None,
            share_created_at: None,
            share_updated_at: None,
            redact_tool_content: false,
        };
        let message = StoredMessage {
            id: "message-idempotent".to_string(),
            session_id: session.id.clone(),
            role: "user".to_string(),
            content: "run the tool".to_string(),
            created_at: 1,
            turn_id: Some("turn-idempotent".to_string()),
        };
        store
            .replace_snapshot(
                std::slice::from_ref(&session),
                std::slice::from_ref(&message),
                &HashMap::new(),
            )
            .unwrap();
        store
            .upsert_turn(
                &session.id,
                "conversation-idempotent",
                "turn-idempotent",
                "request-idempotent",
                "running",
                Some("codex"),
                Some("model"),
                None,
                Some(&session.cwd),
                1,
            )
            .unwrap();

        let events = [
            json!({
                "eventId": "event-tool-call",
                "type": "tool_call",
                "sessionId": session.id,
                "turnId": "turn-idempotent",
                "requestId": "request-idempotent",
                "sequence": 1,
                "toolCall": {"id": "tool-idempotent", "name": "Read", "arguments": {"path": "a.txt"}}
            }),
            json!({
                "eventId": "event-tool-result",
                "type": "tool_result",
                "sessionId": session.id,
                "turnId": "turn-idempotent",
                "requestId": "request-idempotent",
                "sequence": 2,
                "toolCall": {"id": "tool-idempotent", "name": "Read", "result": "contents", "status": "completed"}
            }),
            json!({
                "eventId": "event-terminal",
                "type": "done",
                "sessionId": session.id,
                "turnId": "turn-idempotent",
                "requestId": "request-idempotent",
                "sequence": 3,
                "text": "finished"
            }),
        ];
        for (index, event) in events.iter().enumerate() {
            store.append_event(event, index as i64 + 2).unwrap();
            store.append_event(event, index as i64 + 20).unwrap();
        }

        let connection = store.open().unwrap();
        let event_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM turn_events WHERE turn_id='turn-idempotent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let tool_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM tool_calls WHERE id='tool-idempotent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let status: String = connection
            .query_row(
                "SELECT status FROM turns WHERE id='turn-idempotent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(event_count, 3);
        assert_eq!(tool_count, 1);
        assert_eq!(status, "completed");
        let context = store.load_context(&session.id).unwrap();
        assert_eq!(
            context
                .iter()
                .filter(|item| item.get("role").and_then(Value::as_str) == Some("toolResult"))
                .count(),
            1
        );
        drop(connection);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn segmented_history_round_trips_raw_transcript_and_append_contract() {
        let (store, path) = test_store();
        let cwd = std::env::current_dir().unwrap().display().to_string();
        let session = StoredSession {
            id: "segmented-session".to_string(),
            title: "segmented".to_string(),
            cwd,
            updated_at: 10,
            provider_id: "provider".to_string(),
            model: "model".to_string(),
            selected_model_json: None,
            pinned_at: None,
            archived_at: None,
            share_enabled: false,
            share_token: None,
            share_created_at: None,
            share_updated_at: None,
            redact_tool_content: true,
        };
        let raw_messages = vec![
            StoredMessage {
                id: "m0".to_string(),
                session_id: session.id.clone(),
                role: "user".to_string(),
                content: "raw one".to_string(),
                created_at: 1,
                turn_id: None,
            },
            StoredMessage {
                id: "m1".to_string(),
                session_id: session.id.clone(),
                role: "assistant".to_string(),
                content: "raw two".to_string(),
                created_at: 2,
                turn_id: None,
            },
        ];
        store
            .replace_snapshot(
                std::slice::from_ref(&session),
                &raw_messages,
                &HashMap::new(),
            )
            .unwrap();
        let first_header = StoredHistoryHeader {
            conversation_id: session.id.clone(),
            session_id: Some("pi-session".to_string()),
            context_meta_json: r#"{"schemaVersion":3}"#.to_string(),
            active_segment_index: 0,
            total_segment_count: 1,
            total_message_count: 1,
            created_at: 1,
            updated_at: 2,
        };
        let first_segment = StoredHistorySegment {
            segment_index: 0,
            segment_id: "segment-0".to_string(),
            summary_json: Some(r#"{"role":"summary","id":"s0"}"#.to_string()),
            messages_json: r#"[{"role":"user","content":"one"}]"#.to_string(),
            message_count: 1,
            start_message_id: Some("m0".to_string()),
            end_message_id: Some("m0".to_string()),
            created_at: 1,
            updated_at: 2,
        };
        store
            .upsert_active_segment(&first_header, &first_segment)
            .unwrap();
        #[cfg(windows)]
        {
            let connection = store.open().unwrap();
            let (stored_summary, stored_messages): (String, String) = connection
                .query_row(
                    "SELECT summary_json, messages_json FROM history_segments \
                     WHERE conversation_id=?1 AND segment_index=0",
                    params![session.id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert!(crate::secret_store::is_protected_transcript_text(
                &stored_summary
            ));
            assert!(crate::secret_store::is_protected_transcript_text(
                &stored_messages
            ));
            assert!(!stored_summary.contains("\"role\":\"summary\""));
            assert!(!stored_messages.contains("\"content\":\"one\""));
        }
        let second_header = StoredHistoryHeader {
            active_segment_index: 1,
            total_segment_count: 2,
            total_message_count: 2,
            updated_at: 4,
            ..first_header.clone()
        };
        let second_segment = StoredHistorySegment {
            segment_index: 1,
            segment_id: "segment-1".to_string(),
            summary_json: None,
            messages_json: r#"[{"role":"assistant","content":"two"}]"#.to_string(),
            message_count: 1,
            start_message_id: Some("m1".to_string()),
            end_message_id: Some("m1".to_string()),
            created_at: 3,
            updated_at: 4,
        };
        store
            .append_segment(&second_header, &second_segment)
            .unwrap();
        let loaded = store.load_segmented_history(&session.id).unwrap().unwrap();
        assert_eq!(loaded.header, second_header);
        assert_eq!(loaded.segments, vec![first_segment, second_segment]);
        assert_eq!(
            store.load_runtime_context(&session.id).unwrap(),
            vec![
                json!({"role":"user","content":"one"}),
                json!({"role":"assistant","content":"two"}),
            ]
        );
        assert!(store
            .append_segment(&second_header, &loaded.segments[1])
            .is_err());
        drop(store);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn atomic_segment_mutation_does_not_commit_its_snapshot_when_append_fails() {
        let (store, path) = test_store();
        let cwd = std::env::current_dir().unwrap().display().to_string();
        let original = StoredSession {
            id: "atomic-segment".to_string(),
            title: "before".to_string(),
            cwd: cwd.clone(),
            updated_at: 1,
            provider_id: "provider".to_string(),
            model: "model".to_string(),
            selected_model_json: None,
            pinned_at: None,
            archived_at: None,
            share_enabled: false,
            share_token: None,
            share_created_at: None,
            share_updated_at: None,
            redact_tool_content: true,
        };
        store
            .replace_snapshot(std::slice::from_ref(&original), &[], &HashMap::new())
            .unwrap();
        let replacement = StoredSession {
            title: "after".to_string(),
            updated_at: 2,
            ..original.clone()
        };
        let header = StoredHistoryHeader {
            conversation_id: original.id.clone(),
            session_id: Some(original.id.clone()),
            context_meta_json: "{}".to_string(),
            active_segment_index: 1,
            total_segment_count: 2,
            total_message_count: 1,
            created_at: 1,
            updated_at: 2,
        };
        let segment = StoredHistorySegment {
            segment_index: 1,
            segment_id: "missing-predecessor".to_string(),
            summary_json: None,
            messages_json: r#"[{"role":"assistant","content":"checkpoint"}]"#.to_string(),
            message_count: 1,
            start_message_id: Some("m0".to_string()),
            end_message_id: Some("m0".to_string()),
            created_at: 2,
            updated_at: 2,
        };
        assert!(store
            .replace_snapshot_and_mutate_segment(
                std::slice::from_ref(&replacement),
                &[],
                &HashMap::new(),
                &header,
                &segment,
                true,
            )
            .is_err());
        assert_eq!(store.load_sessions().unwrap()[0].title, "before");
        assert!(store
            .load_segmented_history(&original.id)
            .unwrap()
            .is_none());
        drop(store);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn load_context_replays_two_tool_rounds_in_sequence_after_reload() {
        let (store, path) = test_store();
        let cwd = std::env::current_dir().unwrap().display().to_string();
        let session = StoredSession {
            id: "context-sequence".to_string(),
            title: "context".to_string(),
            cwd: cwd.clone(),
            updated_at: 1,
            provider_id: "provider".to_string(),
            model: "model".to_string(),
            selected_model_json: None,
            pinned_at: None,
            archived_at: None,
            share_enabled: false,
            share_token: None,
            share_created_at: None,
            share_updated_at: None,
            redact_tool_content: true,
        };
        let messages = vec![
            StoredMessage {
                id: "u1".to_string(),
                session_id: session.id.clone(),
                role: "user".to_string(),
                content: "first".to_string(),
                created_at: 10,
                turn_id: Some("turn-1".to_string()),
            },
            StoredMessage {
                id: "a1".to_string(),
                session_id: session.id.clone(),
                role: "assistant".to_string(),
                content: "answer-1".to_string(),
                created_at: 90,
                turn_id: Some("turn-1".to_string()),
            },
            StoredMessage {
                id: "u2".to_string(),
                session_id: session.id.clone(),
                role: "user".to_string(),
                content: "second".to_string(),
                created_at: 100,
                turn_id: Some("turn-2".to_string()),
            },
            StoredMessage {
                id: "a2".to_string(),
                session_id: session.id.clone(),
                role: "assistant".to_string(),
                content: "answer-2".to_string(),
                created_at: 190,
                turn_id: Some("turn-2".to_string()),
            },
        ];
        store
            .replace_snapshot(std::slice::from_ref(&session), &messages, &HashMap::new())
            .unwrap();
        for (turn, request, started) in [
            ("turn-1", "request-1", 10_i64),
            ("turn-2", "request-2", 100_i64),
        ] {
            store
                .upsert_turn(
                    &session.id,
                    "conversation",
                    turn,
                    request,
                    "completed",
                    None,
                    None,
                    None,
                    Some(&cwd),
                    started,
                )
                .unwrap();
        }
        let events = [
            (
                "turn-1",
                "request-1",
                "call-1a",
                "tool_call",
                1,
                20,
                json!({"path":"a"}),
            ),
            (
                "turn-1",
                "request-1",
                "call-1a",
                "tool_result",
                2,
                40,
                json!({"result":"A"}),
            ),
            (
                "turn-1",
                "request-1",
                "call-1b",
                "tool_call",
                3,
                30,
                json!({"path":"b"}),
            ),
            (
                "turn-1",
                "request-1",
                "call-1b",
                "tool_result",
                4,
                50,
                json!({"result":"B"}),
            ),
            (
                "turn-2",
                "request-2",
                "call-2a",
                "tool_call",
                1,
                120,
                json!({"path":"c"}),
            ),
            (
                "turn-2",
                "request-2",
                "call-2a",
                "tool_result",
                2,
                140,
                json!({"result":"C"}),
            ),
            (
                "turn-2",
                "request-2",
                "call-2b",
                "tool_call",
                3,
                130,
                json!({"path":"d"}),
            ),
            (
                "turn-2",
                "request-2",
                "call-2b",
                "tool_result",
                4,
                150,
                json!({"result":"D"}),
            ),
        ];
        for (turn, request, id, event_type, sequence, created_at, detail) in events {
            let tool = if event_type == "tool_call" {
                json!({"id":id,"name":"Read","arguments":detail})
            } else {
                json!({"id":id,"name":"Read","result":detail["result"],"status":"completed"})
            };
            store.append_event(&json!({"eventId":format!("event-{id}-{event_type}"),"type":event_type,"sessionId":session.id,"turnId":turn,"requestId":request,"sequence":sequence,"toolCall":tool}), created_at).unwrap();
        }
        let reloaded = HistoryStore::new(path.clone());
        reloaded.initialize().unwrap();
        let context = reloaded.load_context(&session.id).unwrap();
        let markers = context
            .iter()
            .map(|item| match item["role"].as_str().unwrap_or("") {
                "user" => item["content"][0]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                "assistant" if item["content"][0]["type"] == json!("toolCall") => {
                    format!("call:{}", item["content"][0]["id"].as_str().unwrap_or(""))
                }
                "assistant" => item["content"][0]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                "toolResult" => format!("result:{}", item["toolCallId"].as_str().unwrap_or("")),
                other => other.to_string(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            markers,
            vec![
                "first",
                "call:call-1a",
                "result:call-1a",
                "call:call-1b",
                "result:call-1b",
                "answer-1",
                "second",
                "call:call-2a",
                "result:call-2a",
                "call:call-2b",
                "result:call-2b",
                "answer-2"
            ]
        );
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn turn_reasoning_and_finished_at_round_trip() {
        let (store, path) = test_store();
        let session = StoredSession {
            id: "session-reasoning".to_string(),
            title: "reasoning".to_string(),
            cwd: "C:\\workspace".to_string(),
            updated_at: 1,
            provider_id: "newapi".to_string(),
            model: "grok-4.5".to_string(),
            selected_model_json: None,
            pinned_at: None,
            archived_at: None,
            share_enabled: false,
            share_token: None,
            share_created_at: None,
            share_updated_at: None,
            redact_tool_content: true,
        };
        store
            .replace_snapshot(std::slice::from_ref(&session), &[], &HashMap::new())
            .unwrap();
        store
            .upsert_turn(
                &session.id,
                "conversation-reasoning",
                "turn-reasoning",
                "request-reasoning",
                "running",
                Some("newapi"),
                Some("grok-4.5"),
                Some("high"),
                Some(&session.cwd),
                100,
            )
            .unwrap();
        store
            .append_event(
                &json!({
                    "type": "done",
                    "sessionId": session.id,
                    "turnId": "turn-reasoning",
                    "requestId": "request-reasoning",
                    "text": "hello",
                    "thinking": "checked the available context",
                    "providerPayload": {"toolResult": "must-not-persist"}
                }),
                200,
            )
            .unwrap();
        let metadata = store.load_turn_metadata(&session.id).unwrap();
        let turn = metadata.get("turn-reasoning").expect("turn metadata");
        assert_eq!(turn.model.as_deref(), Some("grok-4.5"));
        assert_eq!(turn.provider_id.as_deref(), Some("newapi"));
        assert_eq!(turn.reasoning.as_deref(), Some("high"));
        assert_eq!(turn.finished_at, Some(200));
        assert_eq!(
            turn.thinking.as_deref(),
            Some("checked the available context")
        );
        let terminal = store
            .load_terminal_event("request-reasoning")
            .unwrap()
            .expect("terminal event");
        assert_eq!(
            terminal.get("thinking").and_then(Value::as_str),
            Some("checked the available context")
        );
        assert!(terminal.get("text").is_none());
        assert!(terminal.get("providerPayload").is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn redacted_terminal_thinking_is_bounded_and_display_safe() {
        let raw = format!(
            "safe\u{0000}\u{202e}\r\n{}",
            "x".repeat(MAX_STORED_TERMINAL_THINKING_CHARS + 10)
        );
        let stored = redact_event_for_storage(&json!({
            "type": "done",
            "sessionId": "session-safe-thinking",
            "thinking": raw,
            "toolPayload": {"result": "must-not-persist"}
        }));
        let thinking = stored["thinking"].as_str().expect("safe thinking");
        assert_eq!(thinking.chars().count(), MAX_STORED_TERMINAL_THINKING_CHARS);
        assert!(thinking.starts_with("safe\u{fffd}\u{fffd}\n\n"));
        assert!(!thinking.chars().any(|character| {
            (character.is_control() && !matches!(character, '\n' | '\t'))
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
        }));
        assert!(stored.get("toolPayload").is_none());
        assert!(redact_event_for_storage(&json!({
            "type": "thinking_delta",
            "sessionId": "session-safe-thinking",
            "thinking": "not terminal"
        }))
        .get("thinking")
        .is_none());
    }

    fn sample_session(id: &str, title: &str) -> StoredSession {
        StoredSession {
            id: id.to_string(),
            title: title.to_string(),
            cwd: "C:\\workspace".to_string(),
            updated_at: 1,
            provider_id: "embedded".to_string(),
            model: String::new(),
            selected_model_json: None,
            pinned_at: None,
            archived_at: None,
            share_enabled: false,
            share_token: None,
            share_created_at: None,
            share_updated_at: None,
            redact_tool_content: true,
        }
    }

    #[test]
    fn load_message_counts_groups_sessions_without_loading_transcripts() {
        let (store, path) = test_store();
        let session_a = sample_session("message-count-a", "A");
        let session_b = sample_session("message-count-b", "B");
        let empty_session = sample_session("message-count-empty", "empty");
        let messages = vec![
            StoredMessage {
                id: "message-count-a1".to_string(),
                session_id: session_a.id.clone(),
                role: "user".to_string(),
                content: "first".to_string(),
                created_at: 1,
                turn_id: None,
            },
            StoredMessage {
                id: "message-count-b1".to_string(),
                session_id: session_b.id.clone(),
                role: "user".to_string(),
                content: "second".to_string(),
                created_at: 2,
                turn_id: None,
            },
            StoredMessage {
                id: "message-count-b2".to_string(),
                session_id: session_b.id.clone(),
                role: "assistant".to_string(),
                content: "third".to_string(),
                created_at: 3,
                turn_id: None,
            },
        ];
        store
            .replace_snapshot(
                &[session_a.clone(), session_b.clone(), empty_session.clone()],
                &messages,
                &HashMap::new(),
            )
            .unwrap();

        let counts = store.load_message_counts().unwrap();

        assert_eq!(counts.get(&session_a.id), Some(&1));
        assert_eq!(counts.get(&session_b.id), Some(&2));
        assert_eq!(counts.get(&empty_session.id), None);
        drop(store);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn manual_context_compaction_is_durable_and_restore_keeps_original_messages() {
        let (store, path) = test_store();
        let session = sample_session("session-manual-context", "manual context");
        let messages = vec![
            StoredMessage {
                id: "manual-1".to_string(),
                session_id: session.id.clone(),
                role: "user".to_string(),
                content: "older request".to_string(),
                created_at: 1,
                turn_id: None,
            },
            StoredMessage {
                id: "manual-2".to_string(),
                session_id: session.id.clone(),
                role: "assistant".to_string(),
                content: "older answer".to_string(),
                created_at: 2,
                turn_id: None,
            },
            StoredMessage {
                id: "manual-3".to_string(),
                session_id: session.id.clone(),
                role: "user".to_string(),
                content: "recent request".to_string(),
                created_at: 3,
                turn_id: None,
            },
        ];
        store
            .upsert_session_projection(&session, &messages)
            .unwrap();
        let metadata = json!({
            "version": 1,
            "summaryId": "novavei-context-v1:fnv1a32:1234abcd",
            "generatedAt": 10,
            "mode": "deterministic_structured",
            "trigger": "manual",
            "sourceFingerprint": "fnv1a32:1234abcd",
            "sourceMessageStart": 1,
            "sourceMessageEnd": 2,
            "sourceTurnStart": 1,
            "sourceTurnEnd": 1,
            "sourceMessages": 2,
            "sourceTurns": 1,
            "sourceTokens": 100,
            "summaryTokens": 20,
            "targetTokens": 128,
            "indexedTurns": 1,
            "omittedTurns": 0,
            "redactedFragments": 0,
            "syntheticMessages": 1,
        });
        store
            .upsert_session_context_compaction(
                &session.id,
                "[Untrusted historical continuity reference] older request and answer",
                &metadata,
            )
            .unwrap();

        // Reopen through a new store instance: the manual record must survive
        // process state while the original messages remain readable from SQLite.
        drop(store);
        let reopened = HistoryStore::new(path.clone());
        let compacted = reopened
            .load_runtime_context_with_metadata(&session.id)
            .unwrap();
        assert_eq!(compacted.messages.len(), 2);
        assert_eq!(
            compacted.messages[0]["content"],
            "[Untrusted historical continuity reference] older request and answer"
        );
        assert_eq!(
            compacted.messages[1]["content"],
            json!([{"type": "text", "text": "recent request"}])
        );
        assert_eq!(
            compacted
                .manual_compaction
                .as_ref()
                .and_then(|value| value.get("trigger")),
            Some(&Value::String("manual".to_string()))
        );

        reopened
            .clear_session_context_compaction(&session.id)
            .unwrap();
        let restored = reopened
            .load_runtime_context_with_metadata(&session.id)
            .unwrap();
        assert_eq!(restored.messages.len(), 3);
        assert!(restored.manual_compaction.is_none());
        assert_eq!(reopened.load_messages(&session.id).unwrap().len(), 3);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn session_projection_is_scoped_and_does_not_touch_other_sessions() {
        let (store, path) = test_store();
        let session_a = sample_session("session-a", "A");
        let session_b = sample_session("session-b", "B");
        let messages_a = vec![StoredMessage {
            id: "msg-a1".to_string(),
            session_id: session_a.id.clone(),
            role: "user".to_string(),
            content: "alpha one".to_string(),
            created_at: 10,
            turn_id: None,
        }];
        let messages_b = vec![
            StoredMessage {
                id: "msg-b1".to_string(),
                session_id: session_b.id.clone(),
                role: "user".to_string(),
                content: "beta one".to_string(),
                created_at: 20,
                turn_id: None,
            },
            StoredMessage {
                id: "msg-b2".to_string(),
                session_id: session_b.id.clone(),
                role: "assistant".to_string(),
                content: "beta two".to_string(),
                created_at: 21,
                turn_id: None,
            },
        ];
        store
            .upsert_session_projection(&session_a, &messages_a)
            .unwrap();
        store
            .upsert_session_projection(&session_b, &messages_b)
            .unwrap();

        let rewritten_a = vec![
            StoredMessage {
                id: "msg-a1".to_string(),
                session_id: session_a.id.clone(),
                role: "user".to_string(),
                content: "alpha rewritten".to_string(),
                created_at: 30,
                turn_id: None,
            },
            StoredMessage {
                id: "msg-a2".to_string(),
                session_id: session_a.id.clone(),
                role: "assistant".to_string(),
                content: "alpha reply".to_string(),
                created_at: 31,
                turn_id: None,
            },
        ];
        store
            .upsert_session_projection(&session_a, &rewritten_a)
            .unwrap();

        let loaded_a = store.load_messages(&session_a.id).unwrap();
        let loaded_b = store.load_messages(&session_b.id).unwrap();
        assert_eq!(loaded_a.len(), 2);
        assert_eq!(loaded_a[0].content, "alpha rewritten");
        assert_eq!(loaded_a[1].content, "alpha reply");
        assert_eq!(loaded_b.len(), 2);
        assert_eq!(loaded_b[0].content, "beta one");
        assert_eq!(loaded_b[1].content, "beta two");

        let connection = store.open().unwrap();
        let count_b: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
                params![session_b.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count_b, 2);
        drop(connection);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn message_content_is_encrypted_on_disk_and_plaintext_on_load() {
        let (store, path) = test_store();
        let session = sample_session("session-cipher", "cipher");
        let plain = "sensitive transcript body";
        store
            .upsert_session_projection(
                &session,
                &[StoredMessage {
                    id: "msg-cipher".to_string(),
                    session_id: session.id.clone(),
                    role: "user".to_string(),
                    content: plain.to_string(),
                    created_at: 1,
                    turn_id: None,
                }],
            )
            .unwrap();

        let connection = store.open().unwrap();
        let stored: String = connection
            .query_row(
                "SELECT content FROM messages WHERE id = 'msg-cipher'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        drop(connection);

        #[cfg(windows)]
        {
            assert!(
                crate::secret_store::is_protected_transcript_text(&stored),
                "Windows must store transcript ciphertext"
            );
            assert_ne!(stored, plain);
        }
        #[cfg(not(windows))]
        {
            // Non-Windows keeps plaintext because DPAPI is unavailable.
            assert_eq!(stored, plain);
        }

        let loaded = store.load_messages(&session.id).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].content, plain);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn replace_snapshot_still_clears_empty_corpus() {
        let (store, path) = test_store();
        let session = sample_session("session-clear", "clear");
        store
            .upsert_session_projection(
                &session,
                &[StoredMessage {
                    id: "msg-clear".to_string(),
                    session_id: session.id.clone(),
                    role: "user".to_string(),
                    content: "will be cleared".to_string(),
                    created_at: 1,
                    turn_id: None,
                }],
            )
            .unwrap();
        store.replace_snapshot(&[], &[], &HashMap::new()).unwrap();
        assert!(store.load_sessions().unwrap().is_empty());
        assert!(store.load_messages(&session.id).unwrap().is_empty());
        let connection = store.open().unwrap();
        let remaining: i64 = connection
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
        drop(connection);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn message_paging_is_session_scoped_and_reports_has_more_before() {
        let (store, path) = test_store();
        let session_a = sample_session("session-page-a", "A");
        let session_b = sample_session("session-page-b", "B");
        let mut messages_a = Vec::new();
        for index in 0..5 {
            messages_a.push(StoredMessage {
                id: format!("a-{index}"),
                session_id: session_a.id.clone(),
                role: if index % 2 == 0 {
                    "user".to_string()
                } else {
                    "assistant".to_string()
                },
                content: format!("alpha {index}"),
                created_at: 100 + index as i64,
                turn_id: None,
            });
        }
        let messages_b = vec![StoredMessage {
            id: "b-0".to_string(),
            session_id: session_b.id.clone(),
            role: "user".to_string(),
            content: "beta only".to_string(),
            created_at: 50,
            turn_id: None,
        }];
        store
            .upsert_session_projection(&session_a, &messages_a)
            .unwrap();
        store
            .upsert_session_projection(&session_b, &messages_b)
            .unwrap();

        assert_eq!(store.count_messages(&session_a.id).unwrap(), 5);
        assert_eq!(store.count_messages(&session_b.id).unwrap(), 1);
        assert_eq!(
            store.load_message_checkpoint(&session_a.id).unwrap(),
            (5, Some("a-4".to_string()))
        );
        assert_eq!(
            store.load_message_checkpoint(&session_b.id).unwrap(),
            (1, Some("b-0".to_string()))
        );
        assert!(store.has_user_message(&session_a.id).unwrap());
        assert!(store.has_user_message(&session_b.id).unwrap());

        let recent = store.load_messages_recent(&session_a.id, 2).unwrap();
        assert_eq!(recent.total_count, 5);
        assert!(recent.has_more_before);
        assert_eq!(
            recent
                .messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a-3", "a-4"]
        );
        assert_eq!(recent.messages[0].content, "alpha 3");
        assert_eq!(recent.messages[1].content, "alpha 4");

        let older = store
            .load_messages_before(
                &session_a.id,
                recent.messages[0].created_at,
                &recent.messages[0].id,
                2,
            )
            .unwrap();
        assert_eq!(older.total_count, 5);
        assert!(older.has_more_before);
        assert_eq!(
            older
                .messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a-1", "a-2"]
        );

        let oldest = store
            .load_messages_before(
                &session_a.id,
                older.messages[0].created_at,
                &older.messages[0].id,
                2,
            )
            .unwrap();
        assert_eq!(oldest.total_count, 5);
        assert!(!oldest.has_more_before);
        assert_eq!(
            oldest
                .messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a-0"]
        );

        // Paging must not leak the other session's transcript.
        let recent_b = store.load_messages_recent(&session_b.id, 10).unwrap();
        assert_eq!(recent_b.total_count, 1);
        assert!(!recent_b.has_more_before);
        assert_eq!(recent_b.messages.len(), 1);
        assert_eq!(recent_b.messages[0].content, "beta only");

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn incomplete_cache_insert_does_not_delete_older_messages() {
        let (store, path) = test_store();
        let session = sample_session("session-partial", "partial");
        let full = vec![
            StoredMessage {
                id: "old-1".to_string(),
                session_id: session.id.clone(),
                role: "user".to_string(),
                content: "older user".to_string(),
                created_at: 10,
                turn_id: None,
            },
            StoredMessage {
                id: "old-2".to_string(),
                session_id: session.id.clone(),
                role: "assistant".to_string(),
                content: "older assistant".to_string(),
                created_at: 11,
                turn_id: Some("turn-old".to_string()),
            },
        ];
        store.upsert_session_projection(&session, &full).unwrap();

        // Incomplete cache path: persist metadata and changed rows together
        // without removing older rows absent from the working cache.
        store
            .upsert_session_metadata_and_messages(
                &session,
                &[
                    StoredMessage {
                        id: "new-3".to_string(),
                        session_id: session.id.clone(),
                        role: "user".to_string(),
                        content: "newest".to_string(),
                        created_at: 12,
                        turn_id: None,
                    },
                    StoredMessage {
                        id: "old-2".to_string(),
                        session_id: session.id.clone(),
                        role: "assistant".to_string(),
                        content: "older assistant updated".to_string(),
                        created_at: 11,
                        turn_id: Some("turn-old".to_string()),
                    },
                ],
            )
            .unwrap();

        let loaded = store.load_messages(&session.id).unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].id, "old-1");
        assert_eq!(loaded[0].content, "older user");
        assert_eq!(loaded[1].id, "old-2");
        assert_eq!(loaded[1].content, "older assistant updated");
        assert_eq!(loaded[2].id, "new-3");
        assert!(store
            .message_matches_assistant_turn(&session.id, "old-2", "turn-old")
            .unwrap());
        assert!(!store
            .message_matches_assistant_turn(&session.id, "new-3", "turn-old")
            .unwrap());

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
    }
}
