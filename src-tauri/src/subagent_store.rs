//! Durable lifecycle records for delegated subagent work.
//!
//! Child prompts, tool arguments, and raw tool output are never retained. A
//! stable agent may opt into one bounded, encrypted private report checkpoint
//! so a later delegated turn can continue its own work without exposing that
//! state in the parent transcript or task list.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;

const MAX_SUBAGENT_TASK_IDENTIFIER_BYTES: usize = 128;
const MAX_SUBAGENT_TITLE_CHARS: usize = 120;
const MAX_FAILURE_CODE_BYTES: usize = 64;
const MAX_SUBAGENT_AGENT_ID_BYTES: usize = 80;
const MAX_SUBAGENT_PRIVATE_CONTEXT_CHARS: usize = 12_000;
const MAX_SUBAGENT_MESSAGE_CHARS: usize = 2_000;
const MAX_SUBAGENT_MESSAGE_CHANNEL_BYTES: usize = 40;
const MAX_SUBAGENT_MESSAGES_PER_SESSION: usize = 100;
const LEGACY_SUBAGENT_AGENT_ID: &str = "legacy-agent";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentTaskStatus {
    Queued,
    AwaitingWorktreeApproval,
    Starting,
    Running,
    WaitingPermission,
    ReviewReady,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    CleanupPending,
}

impl SubagentTaskStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::AwaitingWorktreeApproval => "awaiting_worktree_approval",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::WaitingPermission => "waiting_permission",
            Self::ReviewReady => "review_ready",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
            Self::CleanupPending => "cleanup_pending",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "queued" => Ok(Self::Queued),
            "awaiting_worktree_approval" => Ok(Self::AwaitingWorktreeApproval),
            "starting" => Ok(Self::Starting),
            "running" => Ok(Self::Running),
            "waiting_permission" => Ok(Self::WaitingPermission),
            "review_ready" => Ok(Self::ReviewReady),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "interrupted" => Ok(Self::Interrupted),
            "cleanup_pending" => Ok(Self::CleanupPending),
            _ => Err("stored subagent task status is invalid".to_string()),
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}

/// The task waits in `CleanupPending` until the user explicitly removes the
/// native-managed checkout. Retaining the intended terminal outcome prevents
/// cleanup from converting a failed or cancelled child into a false success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorktreeCleanupDisposition {
    PatchApplied,
    ChildFailed,
    ChildCancelled,
}

impl WorktreeCleanupDisposition {
    fn failure_code(self) -> &'static str {
        match self {
            Self::PatchApplied => "worktree_patch_applied",
            Self::ChildFailed => "worktree_child_failed",
            Self::ChildCancelled => "worktree_child_cancelled",
        }
    }

    fn terminal_status(self) -> SubagentTaskStatus {
        match self {
            Self::PatchApplied => SubagentTaskStatus::Completed,
            Self::ChildFailed => SubagentTaskStatus::Failed,
            Self::ChildCancelled => SubagentTaskStatus::Cancelled,
        }
    }
}

pub fn worktree_cleanup_terminal_status(failure_code: Option<&str>) -> SubagentTaskStatus {
    match failure_code {
        Some("worktree_patch_applied") => {
            WorktreeCleanupDisposition::PatchApplied.terminal_status()
        }
        Some("worktree_child_failed") => WorktreeCleanupDisposition::ChildFailed.terminal_status(),
        // Older cleanup-pending entries did not persist a disposition. Treat
        // them as cancelled rather than incorrectly claiming patch success.
        _ => WorktreeCleanupDisposition::ChildCancelled.terminal_status(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewSubagentTask {
    pub id: String,
    pub session_id: String,
    pub agent_id: String,
    pub parent_turn_id: String,
    pub parent_request_id: String,
    pub title: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredSubagentTask {
    pub id: String,
    pub session_id: String,
    pub agent_id: String,
    pub parent_turn_id: String,
    pub parent_request_id: String,
    pub title: String,
    pub status: SubagentTaskStatus,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub updated_at: i64,
    /// Reserved for a future explicitly approved sanitized summary. First
    /// release always returns `None` so child output never becomes durable.
    pub result_summary: Option<String>,
    pub failure_code: Option<String>,
}

/// A bounded coordination message. `content` is decrypted only for the local
/// session UI after native capability checks; SQLite stores the encrypted form
/// and task lists intentionally do not project messages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredSubagentMessage {
    pub id: String,
    pub session_id: String,
    pub sender_agent_id: String,
    pub recipient: String,
    pub channel: String,
    pub content: String,
    pub created_at: i64,
}

/// Native-only worktree metadata. The actual patch is held in a managed file
/// outside SQLite so its full review content is never mixed with task output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredSubagentWorktree {
    pub task_id: String,
    pub repository_root: String,
    pub worktree_path: String,
    pub base_commit: String,
    pub patch_digest: Option<String>,
    pub changed_paths: Vec<String>,
    pub updated_at: i64,
}

#[derive(Clone, Debug)]
pub struct SubagentStore {
    database_path: PathBuf,
}

impl SubagentStore {
    pub fn new(database_path: PathBuf) -> Self {
        Self { database_path }
    }

    pub fn initialize(&self) -> Result<(), String> {
        let connection = self.open()?;
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS subagent_tasks (
                   id TEXT PRIMARY KEY NOT NULL,
                   session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                   agent_id TEXT NOT NULL DEFAULT '',
                   parent_turn_id TEXT NOT NULL,
                   parent_request_id TEXT NOT NULL,
                   title TEXT NOT NULL,
                   status TEXT NOT NULL CHECK(status IN (
                     'queued', 'awaiting_worktree_approval', 'starting', 'running',
                     'waiting_permission', 'review_ready', 'completed', 'failed',
                     'cancelled', 'interrupted', 'cleanup_pending'
                   )),
                   created_at INTEGER NOT NULL,
                   started_at INTEGER,
                   finished_at INTEGER,
                   updated_at INTEGER NOT NULL,
                   failure_code TEXT
                 );
                 CREATE INDEX IF NOT EXISTS subagent_tasks_session_order
                   ON subagent_tasks(session_id, created_at DESC, id);
                 CREATE INDEX IF NOT EXISTS subagent_tasks_parent_request
                   ON subagent_tasks(parent_request_id, updated_at DESC, id);
                 CREATE INDEX IF NOT EXISTS subagent_tasks_active
                   ON subagent_tasks(status, updated_at);
                 CREATE TABLE IF NOT EXISTS subagent_worktrees (
                   task_id TEXT PRIMARY KEY NOT NULL REFERENCES subagent_tasks(id) ON DELETE CASCADE,
                   repository_root TEXT NOT NULL,
                   worktree_path TEXT NOT NULL,
                   base_commit TEXT NOT NULL,
                   patch_digest TEXT,
                   changed_paths_json TEXT NOT NULL DEFAULT '[]',
                   updated_at INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS subagent_identities (
                   session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                   agent_id TEXT NOT NULL,
                   mode TEXT NOT NULL CHECK(mode IN ('readonly', 'worktree')),
                   created_at INTEGER NOT NULL,
                   updated_at INTEGER NOT NULL,
                   PRIMARY KEY(session_id, agent_id)
                 );
                 CREATE TABLE IF NOT EXISTS subagent_private_context (
                   session_id TEXT NOT NULL,
                   agent_id TEXT NOT NULL,
                   encrypted_report TEXT NOT NULL,
                   updated_at INTEGER NOT NULL,
                   PRIMARY KEY(session_id, agent_id),
                   FOREIGN KEY(session_id, agent_id)
                     REFERENCES subagent_identities(session_id, agent_id)
                     ON DELETE CASCADE
                 );
                 CREATE TABLE IF NOT EXISTS subagent_messages (
                   id TEXT PRIMARY KEY NOT NULL,
                   session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                   sender_agent_id TEXT NOT NULL,
                   recipient TEXT NOT NULL,
                   channel TEXT NOT NULL,
                   encrypted_content TEXT NOT NULL,
                   created_at INTEGER NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS subagent_messages_session_order
                   ON subagent_messages(session_id, created_at DESC, id DESC);",
            )
            .map_err(|error| format!("initialize subagent task store: {error}"))?;
        ensure_subagent_task_agent_column(&connection)?;
        connection
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS subagent_tasks_session_agent
                   ON subagent_tasks(session_id, agent_id, created_at DESC, id);",
            )
            .map_err(|error| format!("index subagent task identities: {error}"))
    }

    pub fn create_task(&self, task: NewSubagentTask) -> Result<StoredSubagentTask, String> {
        validate_task_identifier(&task.id, "task id")?;
        validate_task_identifier(&task.session_id, "session id")?;
        validate_agent_id(&task.agent_id)?;
        validate_task_identifier(&task.parent_turn_id, "parent turn id")?;
        validate_task_identifier(&task.parent_request_id, "parent request id")?;
        let title = normalize_title(&task.title)?;
        if task.created_at <= 0 {
            return Err("subagent task creation time is invalid".to_string());
        }
        let connection = self.open()?;
        connection
            .execute(
                "INSERT INTO subagent_tasks(
                   id, session_id, agent_id, parent_turn_id, parent_request_id, title, status,
                   created_at, updated_at
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, 'queued', ?7, ?7)",
                params![
                    task.id,
                    task.session_id,
                    task.agent_id,
                    task.parent_turn_id,
                    task.parent_request_id,
                    title,
                    task.created_at,
                ],
            )
            .map_err(|error| format!("create subagent task: {error}"))?;
        self.get_task_by_id(&connection, &task.id)?
            .ok_or_else(|| "created subagent task is unavailable".to_string())
    }

    pub fn mark_running(&self, task_id: &str, updated_at: i64) -> Result<(), String> {
        validate_task_identifier(task_id, "task id")?;
        if updated_at <= 0 {
            return Err("subagent task update time is invalid".to_string());
        }
        let connection = self.open()?;
        let changed = connection
            .execute(
                "UPDATE subagent_tasks
                 SET status = 'running',
                     started_at = COALESCE(started_at, ?2),
                     updated_at = ?2
                 WHERE id = ?1 AND status IN ('queued', 'starting')",
                params![task_id, updated_at],
            )
            .map_err(|error| format!("mark subagent task running: {error}"))?;
        if changed == 1 {
            Ok(())
        } else {
            Err("subagent task is not queued for execution".to_string())
        }
    }

    pub fn finish_task(
        &self,
        task_id: &str,
        status: SubagentTaskStatus,
        _ephemeral_result_summary: Option<String>,
        failure_code: Option<String>,
        finished_at: i64,
    ) -> Result<(), String> {
        validate_task_identifier(task_id, "task id")?;
        if !status.is_terminal() {
            return Err("subagent task finish status must be terminal".to_string());
        }
        if finished_at <= 0 {
            return Err("subagent task completion time is invalid".to_string());
        }
        let failure_code = failure_code
            .as_deref()
            .map(normalize_failure_code)
            .transpose()?;
        let connection = self.open()?;
        let changed = connection
            .execute(
                "UPDATE subagent_tasks
                 SET status = ?2,
                     finished_at = ?3,
                     updated_at = ?3,
                     failure_code = ?4
                 WHERE id = ?1
                   AND status NOT IN ('completed', 'failed', 'cancelled', 'interrupted')",
                params![task_id, status.as_str(), finished_at, failure_code],
            )
            .map_err(|error| format!("finish subagent task: {error}"))?;
        if changed == 1 {
            Ok(())
        } else {
            Err("subagent task is already terminal or unavailable".to_string())
        }
    }

    pub fn cancel_task(&self, task_id: &str, cancelled_at: i64) -> Result<(), String> {
        self.finish_task(
            task_id,
            SubagentTaskStatus::Cancelled,
            None,
            None,
            cancelled_at,
        )
    }

    pub fn save_worktree(
        &self,
        task_id: &str,
        repository_root: &str,
        worktree_path: &str,
        base_commit: &str,
        updated_at: i64,
    ) -> Result<(), String> {
        validate_task_identifier(task_id, "task id")?;
        validate_worktree_metadata(repository_root, "repository root")?;
        validate_worktree_metadata(worktree_path, "worktree path")?;
        validate_commit(base_commit)?;
        if updated_at <= 0 {
            return Err("worktree update time is invalid".to_string());
        }
        let connection = self.open()?;
        connection
            .execute(
                "INSERT INTO subagent_worktrees(
                   task_id, repository_root, worktree_path, base_commit, updated_at
                 ) VALUES(?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(task_id) DO UPDATE SET
                   repository_root=excluded.repository_root,
                   worktree_path=excluded.worktree_path,
                   base_commit=excluded.base_commit,
                   updated_at=excluded.updated_at",
                params![
                    task_id,
                    repository_root,
                    worktree_path,
                    base_commit,
                    updated_at
                ],
            )
            .map_err(|error| format!("save subagent worktree: {error}"))?;
        Ok(())
    }

    pub fn mark_review_ready(
        &self,
        task_id: &str,
        patch_digest: &str,
        changed_paths: &[String],
        updated_at: i64,
    ) -> Result<(), String> {
        validate_task_identifier(task_id, "task id")?;
        validate_patch_digest(patch_digest)?;
        if changed_paths.len() > 10_000 || updated_at <= 0 {
            return Err("worktree review metadata is invalid".to_string());
        }
        let paths_json = serde_json::to_string(changed_paths)
            .map_err(|error| format!("serialize changed worktree paths: {error}"))?;
        let connection = self.open()?;
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| format!("start worktree review transaction: {error}"))?;
        let worktree_changed = transaction
            .execute(
                "UPDATE subagent_worktrees
                 SET patch_digest = ?2, changed_paths_json = ?3, updated_at = ?4
                 WHERE task_id = ?1",
                params![task_id, patch_digest, paths_json, updated_at],
            )
            .map_err(|error| format!("save worktree review metadata: {error}"))?;
        let task_changed = transaction
            .execute(
                "UPDATE subagent_tasks
                 SET status = 'review_ready', updated_at = ?2
                 WHERE id = ?1 AND status = 'running'",
                params![task_id, updated_at],
            )
            .map_err(|error| format!("mark subagent review ready: {error}"))?;
        if worktree_changed != 1 || task_changed != 1 {
            return Err("subagent task is not running with a managed worktree".to_string());
        }
        transaction
            .commit()
            .map_err(|error| format!("commit worktree review transaction: {error}"))
    }

    /// Applying a reviewed patch deliberately leaves its detached checkout
    /// intact until the user explicitly requests cleanup. This status records
    /// that the patch outcome is final but native-managed files still remain.
    pub fn mark_cleanup_pending(
        &self,
        task_id: &str,
        disposition: WorktreeCleanupDisposition,
        updated_at: i64,
    ) -> Result<(), String> {
        validate_task_identifier(task_id, "task id")?;
        if updated_at <= 0 {
            return Err("subagent cleanup update time is invalid".to_string());
        }
        let connection = self.open()?;
        let changed = connection
            .execute(
                "UPDATE subagent_tasks
                 SET status = 'cleanup_pending',
                     updated_at = ?3,
                     failure_code = CASE
                       WHEN status = 'cleanup_pending' THEN failure_code
                       ELSE ?2
                     END
                 WHERE id = ?1 AND status IN ('running', 'review_ready', 'cleanup_pending')",
                params![task_id, disposition.failure_code(), updated_at],
            )
            .map_err(|error| format!("mark worktree cleanup pending: {error}"))?;
        if changed == 1 {
            Ok(())
        } else {
            Err("subagent task is not awaiting worktree cleanup".to_string())
        }
    }

    pub fn remove_worktree(&self, task_id: &str) -> Result<(), String> {
        validate_task_identifier(task_id, "task id")?;
        let connection = self.open()?;
        connection
            .execute(
                "DELETE FROM subagent_worktrees WHERE task_id = ?1",
                params![task_id],
            )
            .map_err(|error| format!("remove subagent worktree metadata: {error}"))?;
        Ok(())
    }

    pub fn get_worktree(&self, task_id: &str) -> Result<Option<StoredSubagentWorktree>, String> {
        validate_task_identifier(task_id, "task id")?;
        let connection = self.open()?;
        connection
            .query_row(
                "SELECT task_id, repository_root, worktree_path, base_commit,
                        patch_digest, changed_paths_json, updated_at
                 FROM subagent_worktrees WHERE task_id = ?1",
                params![task_id],
                read_stored_worktree,
            )
            .optional()
            .map_err(|error| format!("read subagent worktree: {error}"))
    }

    pub fn get_task(
        &self,
        session_id: &str,
        task_id: &str,
    ) -> Result<Option<StoredSubagentTask>, String> {
        validate_task_identifier(session_id, "session id")?;
        validate_task_identifier(task_id, "task id")?;
        let connection = self.open()?;
        connection
            .query_row(
                "SELECT id, session_id, agent_id, parent_turn_id, parent_request_id, title, status,
                        created_at, started_at, finished_at, updated_at, failure_code
                 FROM subagent_tasks WHERE session_id = ?1 AND id = ?2",
                params![session_id, task_id],
                read_stored_task,
            )
            .optional()
            .map_err(|error| format!("read subagent task: {error}"))
    }

    pub fn list_tasks(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<StoredSubagentTask>, String> {
        validate_task_identifier(session_id, "session id")?;
        let bounded_limit = limit.clamp(1, 100) as i64;
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT id, session_id, agent_id, parent_turn_id, parent_request_id, title, status,
                        created_at, started_at, finished_at, updated_at, failure_code
                 FROM subagent_tasks
                 WHERE session_id = ?1
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?2",
            )
            .map_err(|error| format!("prepare subagent task list: {error}"))?;
        let tasks = statement
            .query_map(params![session_id, bounded_limit], read_stored_task)
            .map_err(|error| format!("query subagent tasks: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read subagent tasks: {error}"))?;
        Ok(tasks)
    }

    /// Resolve a stable logical agent in one session. Its mode is immutable;
    /// changing readonly/worktree semantics must create a distinct agent id.
    /// Returning `None` is the normal first-run path.
    pub fn prepare_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        mode: &str,
        resume: bool,
        updated_at: i64,
    ) -> Result<Option<String>, String> {
        validate_task_identifier(session_id, "session id")?;
        validate_agent_id(agent_id)?;
        if !matches!(mode, "readonly" | "worktree") || updated_at <= 0 {
            return Err("subagent identity is invalid".to_string());
        }
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("start subagent identity transaction: {error}"))?;
        let previous_mode = transaction
            .query_row(
                "SELECT mode FROM subagent_identities WHERE session_id = ?1 AND agent_id = ?2",
                params![session_id, agent_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("read subagent identity: {error}"))?;
        if let Some(previous_mode) = previous_mode {
            if previous_mode != mode {
                return Err("subagent identity mode cannot change".to_string());
            }
            transaction
                .execute(
                    "UPDATE subagent_identities SET updated_at = ?3
                     WHERE session_id = ?1 AND agent_id = ?2",
                    params![session_id, agent_id, updated_at],
                )
                .map_err(|error| format!("touch subagent identity: {error}"))?;
        } else {
            transaction
                .execute(
                    "INSERT INTO subagent_identities(session_id, agent_id, mode, created_at, updated_at)
                     VALUES(?1, ?2, ?3, ?4, ?4)",
                    params![session_id, agent_id, mode, updated_at],
                )
                .map_err(|error| format!("create subagent identity: {error}"))?;
        }
        let previous = if resume {
            transaction
                .query_row(
                    "SELECT encrypted_report FROM subagent_private_context
                     WHERE session_id = ?1 AND agent_id = ?2",
                    params![session_id, agent_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| format!("read subagent private context: {error}"))?
                .map(|encrypted| {
                    crate::secret_store::unprotect_transcript_text(&encrypted)
                        .map_err(|_| "read subagent private context failed".to_string())
                })
                .transpose()?
        } else {
            transaction
                .execute(
                    "DELETE FROM subagent_private_context WHERE session_id = ?1 AND agent_id = ?2",
                    params![session_id, agent_id],
                )
                .map_err(|error| format!("clear subagent private context: {error}"))?;
            None
        };
        transaction
            .commit()
            .map_err(|error| format!("commit subagent identity transaction: {error}"))?;
        Ok(previous)
    }

    /// Persist only the child's final bounded report, encrypted with the same
    /// platform policy as transcript text. Tool calls/results never enter this
    /// checkpoint and no task-list API projects it.
    pub fn save_agent_private_context(
        &self,
        session_id: &str,
        agent_id: &str,
        report: &str,
        updated_at: i64,
    ) -> Result<(), String> {
        validate_task_identifier(session_id, "session id")?;
        validate_agent_id(agent_id)?;
        let report = normalize_private_context(report)?;
        if updated_at <= 0 {
            return Err("subagent private context time is invalid".to_string());
        }
        let connection = self.open()?;
        if !agent_identity_exists(&connection, session_id, agent_id)? {
            return Err("subagent identity is unavailable".to_string());
        }
        let encrypted = crate::secret_store::protect_transcript_text(&report)
            .map_err(|_| "protect subagent private context failed".to_string())?;
        let changed = connection
            .execute(
                "INSERT INTO subagent_private_context(session_id, agent_id, encrypted_report, updated_at)
                 VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(session_id, agent_id) DO UPDATE SET
                   encrypted_report = excluded.encrypted_report,
                   updated_at = excluded.updated_at",
                params![session_id, agent_id, encrypted, updated_at],
            )
            .map_err(|error| format!("save subagent private context: {error}"))?;
        if changed == 1 {
            Ok(())
        } else {
            Err("subagent identity is unavailable".to_string())
        }
    }

    pub fn agent_exists(&self, session_id: &str, agent_id: &str) -> Result<bool, String> {
        validate_task_identifier(session_id, "session id")?;
        validate_agent_id(agent_id)?;
        let connection = self.open()?;
        agent_identity_exists(&connection, session_id, agent_id)
    }

    pub fn save_message(
        &self,
        id: &str,
        session_id: &str,
        sender_agent_id: &str,
        recipient: &str,
        channel: &str,
        content: &str,
        created_at: i64,
    ) -> Result<StoredSubagentMessage, String> {
        validate_task_identifier(id, "message id")?;
        validate_task_identifier(session_id, "session id")?;
        validate_message_endpoint(sender_agent_id)?;
        validate_message_recipient(recipient)?;
        let channel = normalize_message_channel(channel)?;
        let content = normalize_message_content(content)?;
        if created_at <= 0 {
            return Err("subagent message time is invalid".to_string());
        }
        let encrypted = crate::secret_store::protect_transcript_text(&content)
            .map_err(|_| "protect subagent message failed".to_string())?;
        let connection = self.open()?;
        connection
            .execute(
                "INSERT INTO subagent_messages(
                   id, session_id, sender_agent_id, recipient, channel, encrypted_content, created_at
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id,
                    session_id,
                    sender_agent_id,
                    recipient,
                    channel,
                    encrypted,
                    created_at,
                ],
            )
            .map_err(|error| format!("save subagent message: {error}"))?;
        Ok(StoredSubagentMessage {
            id: id.to_string(),
            session_id: session_id.to_string(),
            sender_agent_id: sender_agent_id.to_string(),
            recipient: recipient.to_string(),
            channel,
            content,
            created_at,
        })
    }

    /// Session UI history only. It intentionally decrypts at most a small
    /// recent page and never projects messages through task-list APIs.
    pub fn list_messages(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<StoredSubagentMessage>, String> {
        validate_task_identifier(session_id, "session id")?;
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT id, session_id, sender_agent_id, recipient, channel, encrypted_content, created_at
                 FROM subagent_messages
                 WHERE session_id = ?1
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?2",
            )
            .map_err(|error| format!("prepare subagent message list: {error}"))?;
        let mut messages = statement
            .query_map(
                params![
                    session_id,
                    limit.clamp(1, MAX_SUBAGENT_MESSAGES_PER_SESSION) as i64
                ],
                read_stored_message,
            )
            .map_err(|error| format!("query subagent messages: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read subagent messages: {error}"))?;
        messages.reverse();
        Ok(messages)
    }

    pub fn mark_interrupted_tasks(&self, interrupted_at: i64) -> Result<usize, String> {
        if interrupted_at <= 0 {
            return Err("subagent task interruption time is invalid".to_string());
        }
        let connection = self.open()?;
        connection
            .execute(
                "UPDATE subagent_tasks
                 SET status = 'interrupted',
                     finished_at = ?1,
                     updated_at = ?1,
                     failure_code = 'app_restarted'
                 WHERE status IN ('queued', 'awaiting_worktree_approval', 'starting',
                                'running', 'waiting_permission', 'review_ready', 'cleanup_pending')",
                params![interrupted_at],
            )
            .map_err(|error| format!("recover interrupted subagent tasks: {error}"))
    }

    fn open(&self) -> Result<Connection, String> {
        let connection = Connection::open(&self.database_path)
            .map_err(|error| format!("open subagent task store: {error}"))?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(|error| format!("configure subagent task store timeout: {error}"))?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA secure_delete = ON;")
            .map_err(|error| format!("configure subagent task store: {error}"))?;
        Ok(connection)
    }

    fn get_task_by_id(
        &self,
        connection: &Connection,
        task_id: &str,
    ) -> Result<Option<StoredSubagentTask>, String> {
        connection
            .query_row(
                "SELECT id, session_id, agent_id, parent_turn_id, parent_request_id, title, status,
                        created_at, started_at, finished_at, updated_at, failure_code
                 FROM subagent_tasks WHERE id = ?1",
                params![task_id],
                read_stored_task,
            )
            .optional()
            .map_err(|error| format!("read created subagent task: {error}"))
    }
}

fn read_stored_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSubagentTask> {
    let status = row.get::<_, String>(6)?;
    let status = SubagentTaskStatus::parse(&status).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
        )
    })?;
    Ok(StoredSubagentTask {
        id: row.get(0)?,
        session_id: row.get(1)?,
        agent_id: row.get(2)?,
        parent_turn_id: row.get(3)?,
        parent_request_id: row.get(4)?,
        title: row.get(5)?,
        status,
        created_at: row.get(7)?,
        started_at: row.get(8)?,
        finished_at: row.get(9)?,
        updated_at: row.get(10)?,
        result_summary: None,
        failure_code: row.get(11)?,
    })
}

fn read_stored_worktree(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSubagentWorktree> {
    let changed_paths_json = row.get::<_, String>(5)?;
    let changed_paths = serde_json::from_str(&changed_paths_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
        )
    })?;
    Ok(StoredSubagentWorktree {
        task_id: row.get(0)?,
        repository_root: row.get(1)?,
        worktree_path: row.get(2)?,
        base_commit: row.get(3)?,
        patch_digest: row.get(4)?,
        changed_paths,
        updated_at: row.get(6)?,
    })
}

fn read_stored_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSubagentMessage> {
    let encrypted = row.get::<_, String>(5)?;
    let content = crate::secret_store::unprotect_transcript_text(&encrypted).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "decrypt subagent message",
            )),
        )
    })?;
    Ok(StoredSubagentMessage {
        id: row.get(0)?,
        session_id: row.get(1)?,
        sender_agent_id: row.get(2)?,
        recipient: row.get(3)?,
        channel: row.get(4)?,
        content,
        created_at: row.get(6)?,
    })
}

fn validate_task_identifier(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_SUBAGENT_TASK_IDENTIFIER_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!("subagent {label} is invalid"));
    }
    Ok(())
}

fn validate_agent_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_SUBAGENT_AGENT_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("subagent agent id is invalid".to_string());
    }
    Ok(())
}

fn normalize_private_context(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > MAX_SUBAGENT_PRIVATE_CONTEXT_CHARS {
        return Err("subagent private context is invalid".to_string());
    }
    if value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err("subagent private context contains control characters".to_string());
    }
    Ok(value.to_string())
}

fn validate_message_endpoint(value: &str) -> Result<(), String> {
    if value == "parent" || value == "*" {
        return Ok(());
    }
    validate_agent_id(value)
}

fn validate_message_recipient(value: &str) -> Result<(), String> {
    if value == "parent" || value == "*" {
        return Ok(());
    }
    validate_agent_id(value)
}

fn normalize_message_channel(value: &str) -> Result<String, String> {
    let channel = value.trim();
    if channel.is_empty()
        || channel.len() > MAX_SUBAGENT_MESSAGE_CHANNEL_BYTES
        || !channel.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
    {
        return Err("subagent message channel is invalid".to_string());
    }
    Ok(channel.to_string())
}

fn normalize_message_content(value: &str) -> Result<String, String> {
    let content = value.trim();
    if content.is_empty() || content.chars().count() > MAX_SUBAGENT_MESSAGE_CHARS {
        return Err("subagent message content is invalid".to_string());
    }
    if content
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err("subagent message content contains control characters".to_string());
    }
    Ok(content.to_string())
}

fn ensure_subagent_task_agent_column(connection: &Connection) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(subagent_tasks)")
        .map_err(|error| format!("inspect subagent task schema: {error}"))?;
    let mut rows = statement
        .query([])
        .map_err(|error| format!("read subagent task schema: {error}"))?;
    let mut has_agent_id = false;
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("iterate subagent task schema: {error}"))?
    {
        let name = row
            .get::<_, String>(1)
            .map_err(|error| format!("read subagent task schema column: {error}"))?;
        if name == "agent_id" {
            has_agent_id = true;
            break;
        }
    }
    if !has_agent_id {
        connection
            .execute_batch(
                "ALTER TABLE subagent_tasks
                   ADD COLUMN agent_id TEXT NOT NULL DEFAULT '';",
            )
            .map_err(|error| format!("migrate subagent agent identity: {error}"))?;
    }
    connection
        .execute(
            "UPDATE subagent_tasks
             SET agent_id = ?1
             WHERE agent_id = ''",
            params![LEGACY_SUBAGENT_AGENT_ID],
        )
        .map_err(|error| format!("backfill legacy subagent agent identity: {error}"))?;
    Ok(())
}

fn agent_identity_exists(
    connection: &Connection,
    session_id: &str,
    agent_id: &str,
) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT 1 FROM subagent_identities
             WHERE session_id = ?1 AND agent_id = ?2",
            params![session_id, agent_id],
            |_| Ok(()),
        )
        .optional()
        .map(|value| value.is_some())
        .map_err(|error| format!("read subagent identity: {error}"))
}

fn normalize_title(value: &str) -> Result<String, String> {
    let title = value.trim();
    if title.is_empty() || title.chars().count() > MAX_SUBAGENT_TITLE_CHARS {
        return Err("subagent task title is invalid".to_string());
    }
    if title.chars().any(char::is_control) {
        return Err("subagent task title contains control characters".to_string());
    }
    Ok(title.to_string())
}

fn normalize_failure_code(value: &str) -> Result<String, String> {
    let code = value.trim();
    if code.is_empty()
        || code.len() > MAX_FAILURE_CODE_BYTES
        || !code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || matches!(byte, b'_' | b'-'))
    {
        return Err("subagent task failure code is invalid".to_string());
    }
    Ok(code.to_string())
}

fn validate_worktree_metadata(value: &str, label: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 32_768 || value.chars().any(char::is_control) {
        return Err(format!("subagent worktree {label} is invalid"));
    }
    Ok(())
}

fn validate_commit(value: &str) -> Result<(), String> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("subagent worktree base commit is invalid".to_string());
    }
    Ok(())
}

fn validate_patch_digest(value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("subagent worktree patch digest is invalid".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history_store::HistoryStore;
    use chrono::Utc;
    use rusqlite::Connection;

    #[test]
    fn task_lifecycle_persists_metadata_without_raw_child_output() {
        let database_path = std::env::temp_dir().join(format!(
            "novavei-subagent-store-{}-{}.sqlite3",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        HistoryStore::new(database_path.clone())
            .initialize()
            .expect("history store should initialize first");
        Connection::open(&database_path)
            .expect("test connection should open")
            .execute(
                "INSERT INTO sessions(id, title, cwd, created_at, updated_at)
                 VALUES('session-1', 'test', 'C:\\workspace', 1, 1)",
                [],
            )
            .expect("test session should exist");
        let store = SubagentStore::new(database_path.clone());
        store.initialize().expect("task store should initialize");

        let task = store
            .create_task(NewSubagentTask {
                id: "task-1".to_string(),
                session_id: "session-1".to_string(),
                agent_id: "researcher".to_string(),
                parent_turn_id: "turn-1".to_string(),
                parent_request_id: "request-1".to_string(),
                title: "Read-only workspace analysis".to_string(),
                created_at: 1_725_000_000_000,
            })
            .expect("task should be queued");

        assert_eq!(task.status, SubagentTaskStatus::Queued);
        assert_eq!(task.title, "Read-only workspace analysis");
        assert!(task.result_summary.is_none());

        store
            .mark_running("task-1", 1_725_000_000_100)
            .expect("queued task should start");
        store
            .finish_task(
                "task-1",
                SubagentTaskStatus::Completed,
                Some("summary ".repeat(500)),
                None,
                1_725_000_000_200,
            )
            .expect("running task should complete");

        let stored = store
            .get_task("session-1", "task-1")
            .expect("task should be readable")
            .expect("task should exist");
        assert_eq!(stored.status, SubagentTaskStatus::Completed);
        assert_eq!(stored.parent_request_id, "request-1");
        assert_eq!(stored.failure_code, None);
        assert!(stored.result_summary.is_none());

        let _ = std::fs::remove_file(database_path);
    }

    #[test]
    fn worktree_cleanup_can_be_requested_before_patch_review() {
        let database_path = std::env::temp_dir().join(format!(
            "novavei-subagent-worktree-cleanup-{}-{}.sqlite3",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        HistoryStore::new(database_path.clone())
            .initialize()
            .expect("history store should initialize first");
        Connection::open(&database_path)
            .expect("test connection should open")
            .execute(
                "INSERT INTO sessions(id, title, cwd, created_at, updated_at)
                 VALUES('session-2', 'test', 'C:\\workspace', 1, 1)",
                [],
            )
            .expect("test session should exist");
        let store = SubagentStore::new(database_path.clone());
        store.initialize().expect("task store should initialize");
        store
            .create_task(NewSubagentTask {
                id: "worktree-task-1".to_string(),
                session_id: "session-2".to_string(),
                agent_id: "implementer".to_string(),
                parent_turn_id: "turn-2".to_string(),
                parent_request_id: "request-2".to_string(),
                title: "Isolated implementation".to_string(),
                created_at: 1_725_000_000_000,
            })
            .expect("worktree task should be created");
        store
            .mark_running("worktree-task-1", 1_725_000_000_100)
            .expect("worktree task should start");
        store
            .save_worktree(
                "worktree-task-1",
                "C:\\repository",
                "C:\\managed\\worktree-task-1",
                &"a".repeat(40),
                1_725_000_000_100,
            )
            .expect("worktree metadata should persist");

        store
            .mark_cleanup_pending(
                "worktree-task-1",
                WorktreeCleanupDisposition::ChildCancelled,
                1_725_000_000_200,
            )
            .expect("a stopped worktree must remain explicitly cleanable");

        let task = store
            .get_task("session-2", "worktree-task-1")
            .expect("task should be readable")
            .expect("task should exist");
        assert_eq!(task.status, SubagentTaskStatus::CleanupPending);
        assert_eq!(
            task.failure_code.as_deref(),
            Some("worktree_child_cancelled")
        );
        assert_eq!(
            worktree_cleanup_terminal_status(task.failure_code.as_deref()),
            SubagentTaskStatus::Cancelled,
        );
        assert!(store
            .get_worktree("worktree-task-1")
            .expect("worktree should be readable")
            .is_some());

        let _ = std::fs::remove_file(database_path);
    }

    #[test]
    fn coordination_messages_are_encrypted_and_session_scoped() {
        let database_path = std::env::temp_dir().join(format!(
            "novavei-subagent-message-store-{}-{}.sqlite3",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        HistoryStore::new(database_path.clone())
            .initialize()
            .expect("history store should initialize first");
        Connection::open(&database_path)
            .expect("test connection should open")
            .execute(
                "INSERT INTO sessions(id, title, cwd, created_at, updated_at)
                 VALUES('session-message', 'test', 'C:\\workspace', 1, 1)",
                [],
            )
            .expect("test session should exist");
        let store = SubagentStore::new(database_path.clone());
        store.initialize().expect("message store should initialize");

        let message = store
            .save_message(
                "message-1",
                "session-message",
                "researcher",
                "parent",
                "status",
                "Found the bounded evidence.",
                1_725_000_000_000,
            )
            .expect("message should save");
        assert_eq!(message.content, "Found the bounded evidence.");
        let messages = store
            .list_messages("session-message", 30)
            .expect("message should decrypt for its session UI");
        assert_eq!(messages, vec![message]);

        let raw: String = Connection::open(&database_path)
            .expect("test connection should open")
            .query_row(
                "SELECT encrypted_content FROM subagent_messages WHERE id = 'message-1'",
                [],
                |row| row.get(0),
            )
            .expect("encrypted message should be stored");
        assert!(!raw.contains("Found the bounded evidence."));

        let _ = std::fs::remove_file(database_path);
    }
}
