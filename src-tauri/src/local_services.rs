//! Local Skills, Memory, and Cron services for the NovaVei desktop shell.
//!
//! This module is self-contained: no external agent executable or gateway is
//! required. The renderer can register the command functions at the bottom of
//! this file and manage one `Arc<LocalServices>` as Tauri state.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard, PoisonError};
use std::time::{Duration as StdDuration, Instant};

use chrono::{Datelike, Days, Local, LocalResult, TimeZone, Utc};
use parking_lot::{Mutex, RwLock};
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::{Method, Url};
use rusqlite::{
    params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension, Transaction,
    TransactionBehavior,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tauri::State;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;
use walkdir::WalkDir;

use crate::history_store::HistoryStore;
use crate::path_display::path_for_display;
use crate::secret_store::{
    is_portable_local_service_text, protect_portable_local_service_text, protect_settings,
    unprotect_portable_local_service_text, unprotect_settings,
};

fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    Sha256::digest(bytes.as_ref())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

const DATABASE_FILE: &str = "local-services.sqlite3";
const SKILLS_DIRECTORY: &str = "skills";
const STAGING_DIRECTORY: &str = ".staging";

const MAX_SKILL_NAME_CHARS: usize = 64;
const MAX_SKILL_DESCRIPTION_CHARS: usize = 1_024;
const MAX_SKILL_MD_BYTES: u64 = 256 * 1024;
const MAX_SKILL_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SKILL_TOTAL_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SKILL_FILES: usize = 256;
const MAX_SKILL_DEPTH: usize = 16;

const CLAWHUB_API_ORIGIN: &str = "https://clawhub.ai/api/v1/";
const CLAWHUB_SITE_ORIGIN: &str = "https://clawhub.ai/";
const CLAWHUB_USER_AGENT: &str = "NovaVei/0.1 (Skills catalog; desktop)";
const CLAWHUB_CONNECT_TIMEOUT: StdDuration = StdDuration::from_secs(5);
const CLAWHUB_REQUEST_TIMEOUT: StdDuration = StdDuration::from_secs(15);
const MAX_CLAWHUB_JSON_BYTES: usize = 1024 * 1024;
const MAX_CLAWHUB_SEARCH_CHARS: usize = 120;
const MAX_CLAWHUB_CURSOR_BYTES: usize = 4096;
const MAX_CLAWHUB_VERSION_CHARS: usize = 128;
const MAX_CLAWHUB_SUMMARY_CHARS: usize = 1024;
const MAX_CLAWHUB_DISPLAY_NAME_CHARS: usize = 256;
const MAX_CLAWHUB_RISK_SUMMARY_CHARS: usize = 512;
const MAX_CLAWHUB_RETRY_AFTER_SECS: u64 = 3600;

const MAX_MEMORY_TITLE_CHARS: usize = 200;
const MAX_MEMORY_CONTENT_BYTES: usize = 64 * 1024;
const MAX_MEMORY_QUERY_CHARS: usize = 256;
const MAX_MEMORY_ENTRIES_PER_SCOPE: i64 = 5_000;
const MAX_MEMORY_LIST_LIMIT: usize = 200;
const MAX_MEMORY_SEARCH_LIMIT: usize = 50;
const MAX_MEMORY_OFFSET: usize = 10_000;
const MAX_MEMORY_ORGANIZE_ENTRIES: usize = 5_000;
const MAX_MEMORY_ORGANIZE_GROUPS: usize = 100;
const MAX_MEMORY_ORGANIZE_IDS: usize = 500;
const MAX_MEMORY_EXPORT_BYTES: usize = 16 * 1024 * 1024;
/// Portable search decrypts records in-process because SQLite FTS/LIKE indexes
/// would otherwise retain plaintext. Keep that work bounded even for a
/// deliberately large imported database.
const MAX_PORTABLE_DECRYPT_SEARCH_ROWS: usize = 10_000;

// Knowledge-base roots are granted only by the native folder picker.  The
// limits below keep a selected source tree from turning a settings action into
// an unbounded walk, SQLite write, or model-context disclosure.
const MAX_KNOWLEDGE_BASE_FOLDERS: usize = 32;
const MAX_KNOWLEDGE_BASE_DEPTH: usize = 16;
const MAX_KNOWLEDGE_BASE_FILES: usize = 10_000;
const MAX_KNOWLEDGE_BASE_FILE_BYTES: u64 = 512 * 1024;
const MAX_KNOWLEDGE_BASE_TOTAL_BYTES: u64 = 24 * 1024 * 1024;
const MAX_KNOWLEDGE_BASE_QUERY_CHARS: usize = 256;
const MAX_KNOWLEDGE_BASE_SEARCH_LIMIT: usize = 20;
const MAX_KNOWLEDGE_BASE_SNIPPET_CHARS: usize = 900;
const MAX_KNOWLEDGE_BASE_READ_CHARS: usize = 12_000;
const MAX_KNOWLEDGE_AGENT_SEARCHES_PER_TURN: usize = 6;
const MAX_KNOWLEDGE_AGENT_READS_PER_TURN: usize = 6;
const MAX_KNOWLEDGE_AGENT_READ_CHARS_PER_TURN: usize = 24_000;
const KNOWLEDGE_AGENT_ACCESS_TTL: StdDuration = StdDuration::from_secs(10 * 60);

const MAX_CRON_JOBS: i64 = 1_000;
const MAX_CRON_NAME_CHARS: usize = 120;
const MAX_CRON_PROMPT_BYTES: usize = 64 * 1024;
const MAX_CRON_COMMAND_BYTES: usize = 16 * 1024;
const MAX_CRON_URL_CHARS: usize = 2_048;
const MAX_CRON_HEADERS: usize = 32;
const MAX_CRON_HEADER_NAME_BYTES: usize = 128;
const MAX_CRON_HEADER_VALUE_BYTES: usize = 8 * 1024;
const MAX_CRON_HTTP_BODY_BYTES: usize = 128 * 1024;
const MAX_CRON_HTTP_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_CRON_RUN_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_CRON_RUNS_PER_JOB: usize = 1_000;
const MAX_CRON_RUN_LIST_LIMIT: usize = 200;
/// One-time migration marker for Prompt Cron jobs created before native
/// confirmation was required. Those legacy jobs are disabled on upgrade so a
/// scheduler can never execute a prompt that was persisted without the new
/// native approval boundary.
const CRON_PROMPT_NATIVE_CONFIRMATION_MIGRATION_KEY: &str = "cron_prompt_native_confirmation_v1";
/// A native confirmation never crosses IPC. Keeping its lifetime short is
/// defense in depth for accidental native-side retention; renderer callers
/// cannot construct or replay this private capability at all.
const NATIVE_CRON_APPROVAL_TTL: StdDuration = StdDuration::from_secs(60);
/// Global capacity for all native Cron work. A due run is only claimed after
/// the scheduler has reserved one of these ten slots, while a user-triggered
/// run must reserve the same capacity before it creates a `running` record.
pub(crate) const CRON_WORKER_POOL_SIZE: usize = 10;
const MAX_CRON_CLAIM_LIMIT: usize = CRON_WORKER_POOL_SIZE;
const MAX_CRON_SHELL_TIMEOUT_MS: u64 = 120_000;
const MAX_CRON_PROMPT_TIMEOUT_MS: u64 = 120_000;
const MAX_CRON_PROMPT_MESSAGES: usize = 1;
// One-click translation of native service details uses the same native
// provider boundary as Prompt Cron, with a tighter interactive timeout and
// its own bounded input/output ceilings.
pub(crate) const MAX_TRANSLATION_INPUT_CHARS: usize = 64 * 1024;
const MAX_TRANSLATION_OUTPUT_CHARS: usize = 16 * 1024;
const MAX_TRANSLATION_RESPONSE_BYTES: usize = 128 * 1024;
const MAX_TRANSLATION_TIMEOUT_MS: u64 = 45_000;
// Shell and Prompt jobs have a two minute execution ceiling. Keep the lease
// comfortably above that ceiling so a healthy native executor is never
// recovered while it is still completing bounded I/O.
const CRON_RUN_LEASE_MS: i64 = 5 * 60 * 1_000;
// Cron results are intentionally retained as metadata only. Commands, prompts,
// HTTP responses, and process output can carry secrets, so the SQLite run
// history records whether a value existed without persisting its contents.
const HISTORY_DATABASE_FILE: &str = "chat-history.sqlite3";
// Native-only metadata binds a stored credential to its intended endpoint and
// authentication family. Prompt Cron reads the same protected settings as the
// interactive runtime, so it must enforce this boundary before it reads a key.
const CRON_PROVIDER_CREDENTIAL_BINDING_KEY: &str = "__credentialBinding";

const BUILTIN_SKILLS: &[(&str, &str)] = &[
    (
        "skills-creator",
        r#"---
name: skills-creator
description: Create small, focused Agent Skills with clear triggers, bounded resources, and verifiable workflows.
---

# Skills Creator

Use this skill when a user asks to create or refine an Agent Skill.

1. Define a narrow trigger and explicit non-goals.
2. Keep the entry instructions self-contained and deterministic.
3. Put reusable scripts or references in dedicated subdirectories.
4. Validate paths, inputs, side effects, and the final artifact.
"#,
    ),
    (
        "skills-installer",
        r#"---
name: skills-installer
description: Inspect and install a local Agent Skill only after validating its metadata, directory boundary, and file limits.
---

# Skills Installer

Use the native folder picker to select one Skill directory. Review its
`SKILL.md`, reject links or mismatched names, and install only after the staged
copy passes validation. Existing Skills are never replaced implicitly.
"#,
    ),
];

static SKILLS_WRITE_LOCK: StdMutex<()> = StdMutex::new(());

/// Process-local execution capacity shared by scheduled and user-triggered
/// Cron work. It deliberately has no queue: callers only create a run after
/// they hold a permit, keeping persisted active-run state truthful.
#[derive(Clone)]
pub(crate) struct CronExecutionPool {
    permits: Arc<Semaphore>,
}

impl CronExecutionPool {
    pub(crate) fn new() -> Self {
        Self {
            permits: Arc::new(Semaphore::new(CRON_WORKER_POOL_SIZE)),
        }
    }

    /// Reserve only capacity that is immediately executable. The scheduler
    /// uses this before it claims due rows, leaving excess work due for a
    /// later tick instead of marking it running while it waits in memory.
    pub(crate) fn reserve_available_slots(&self, limit: usize) -> Vec<OwnedSemaphorePermit> {
        (0..limit.min(CRON_WORKER_POOL_SIZE))
            .filter_map(|_| Arc::clone(&self.permits).try_acquire_owned().ok())
            .collect()
    }

    /// User-triggered runs use the same immediate-only policy as the
    /// scheduler, preventing the command endpoint from becoming an unbounded
    /// in-memory queue or bypassing the global execution limit.
    fn try_reserve_slot(&self) -> Result<OwnedSemaphorePermit, String> {
        Arc::clone(&self.permits).try_acquire_owned().map_err(|_| {
            "Cron execution capacity is exhausted; wait for an active run to finish".to_string()
        })
    }
}

fn skills_write_guard() -> StdMutexGuard<'static, ()> {
    SKILLS_WRITE_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// State for all local services. Initialization failures are retained and
/// retried by later operations instead of panicking during application start.
pub struct LocalServices {
    data_root: PathBuf,
    skills_root: PathBuf,
    database_path: PathBuf,
    initialization_lock: Mutex<()>,
    initialization_error: Mutex<Option<String>>,
    /// A locked instance deliberately skips eager initialization. Keep
    /// that state distinct from "last initialization did not fail" so the
    /// first post-unlock operation still creates and migrates its store.
    initialized: AtomicBool,
    cron_startup_recovery_completed: AtomicBool,
    cron_execution_pool: CronExecutionPool,
    fts_available: AtomicBool,
    knowledge_fts_available: AtomicBool,
    /// Reads hold this for their complete lifetime. Revoking consent, removing
    /// a root, and rebuilding an index take the write side so an already
    /// started Agent call cannot disclose content after revocation completes.
    knowledge_access_lock: RwLock<()>,
    knowledge_agent_access: Mutex<HashMap<String, KnowledgeAgentAccess>>,
    security_gate: LocalServicesSecurityGate,
}

type LocalServicesSecurityGate = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

fn default_local_services_security_gate() -> Result<(), String> {
    if !crate::secret_store::app_security_needs_unlock() {
        return Ok(());
    }
    if crate::storage::is_portable() {
        Err("portable storage is locked; unlock it before using local services".to_string())
    } else {
        Err("application password is required".to_string())
    }
}

impl LocalServices {
    pub fn new() -> Self {
        Self::from_root(local_services_data_root())
    }

    fn from_root(data_root: PathBuf) -> Self {
        Self::from_root_with_security_gate(
            data_root,
            Arc::new(default_local_services_security_gate),
        )
    }

    fn from_root_with_security_gate(
        data_root: PathBuf,
        security_gate: LocalServicesSecurityGate,
    ) -> Self {
        let service = Self {
            skills_root: data_root.join(SKILLS_DIRECTORY),
            database_path: data_root.join(DATABASE_FILE),
            data_root,
            initialization_lock: Mutex::new(()),
            initialization_error: Mutex::new(None),
            initialized: AtomicBool::new(false),
            cron_startup_recovery_completed: AtomicBool::new(false),
            cron_execution_pool: CronExecutionPool::new(),
            fts_available: AtomicBool::new(false),
            knowledge_fts_available: AtomicBool::new(false),
            knowledge_access_lock: RwLock::new(()),
            knowledge_agent_access: Mutex::new(HashMap::new()),
            security_gate,
        };
        // A locked package must not create its database, Skills directory, FTS
        // tables, or Cron recovery state before the startup password is done.
        if service.check_security_ready().is_ok() {
            let _ = service.initialize();
        }
        service
    }

    #[cfg(test)]
    fn for_test(data_root: PathBuf) -> Self {
        Self::from_root_with_security_gate(data_root, Arc::new(|| Ok(())))
    }

    #[cfg(test)]
    fn for_test_with_security_gate(
        data_root: PathBuf,
        security_gate: LocalServicesSecurityGate,
    ) -> Self {
        Self::from_root_with_security_gate(data_root, security_gate)
    }

    #[cfg(test)]
    fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub(crate) fn cron_execution_pool(&self) -> CronExecutionPool {
        self.cron_execution_pool.clone()
    }

    /// Idempotently create the directory tree, database schema, FTS index, and
    /// builtin Skills. It is safe to call this again after a transient error.
    pub fn initialize(&self) -> Result<(), String> {
        let _guard = self.initialization_lock.lock();
        if let Err(error) = self.check_security_ready() {
            *self.initialization_error.lock() = Some(error.clone());
            return Err(error);
        }
        let result = self.initialize_inner();
        if result.is_ok() {
            self.initialized.store(true, Ordering::Release);
        }
        *self.initialization_error.lock() = result.as_ref().err().cloned();
        result
    }

    fn initialize_inner(&self) -> Result<(), String> {
        fs::create_dir_all(&self.data_root)
            .map_err(|error| format!("create local service data directory: {error}"))?;
        fs::create_dir_all(&self.skills_root)
            .map_err(|error| format!("create Skills directory: {error}"))?;
        let staging_root = self.skills_root.join(STAGING_DIRECTORY);
        fs::create_dir_all(&staging_root)
            .map_err(|error| format!("create Skills staging directory: {error}"))?;
        ensure_plain_directory(&self.skills_root)?;
        ensure_plain_directory(&staging_root)?;

        let connection = self.open_unchecked()?;
        initialize_schema(&connection)?;
        // `initialize` is also the retry path for a failed startup. Only the
        // first successful initialization belongs to this process's startup;
        // later reinitialization must not interrupt a healthy active worker.
        if !self.cron_startup_recovery_completed.load(Ordering::Acquire) {
            recover_interrupted_cron_runs_on_startup(&connection, now_ms())?;
            self.cron_startup_recovery_completed
                .store(true, Ordering::Release);
        }
        if crate::storage::is_portable() {
            let migrated = migrate_portable_local_service_fields(&connection)?;
            let indexes_removed = disable_portable_full_text_indexes(&connection)?;
            if migrated || indexes_removed {
                scrub_portable_sqlite_plaintext(&connection)?;
            }
        }
        let fts_available = if crate::storage::is_portable() {
            false
        } else {
            initialize_fts(&connection)
        };
        self.fts_available.store(fts_available, Ordering::Release);
        let knowledge_fts_available = if crate::storage::is_portable() {
            false
        } else {
            initialize_knowledge_fts(&connection)
        };
        self.knowledge_fts_available
            .store(knowledge_fts_available, Ordering::Release);
        self.seed_builtin_skills(&connection)?;
        Ok(())
    }

    fn check_security_ready(&self) -> Result<(), String> {
        (self.security_gate)()
    }

    fn ensure_ready(&self) -> Result<(), String> {
        self.check_security_ready()?;
        if !self.initialized.load(Ordering::Acquire) || self.initialization_error.lock().is_some() {
            self.initialize()?;
        }
        self.initialization_error
            .lock()
            .clone()
            .map_or(Ok(()), |error| {
                Err(format!("local services unavailable: {error}"))
            })
    }

    fn open_unchecked(&self) -> Result<Connection, String> {
        let connection = Connection::open(&self.database_path)
            .map_err(|error| format!("open local service database: {error}"))?;
        connection
            .busy_timeout(StdDuration::from_secs(5))
            .map_err(|error| format!("configure local service database: {error}"))?;
        connection
            .execute_batch("PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON;")
            .map_err(|error| format!("configure local service database: {error}"))?;
        Ok(connection)
    }

    fn open(&self) -> Result<Connection, String> {
        self.ensure_ready()?;
        self.open_unchecked()
    }
}

impl Default for LocalServices {
    fn default() -> Self {
        Self::new()
    }
}

fn local_services_data_root() -> PathBuf {
    crate::storage::application_data_dir()
}

fn initialize_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            r#"
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS local_service_meta (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );
            INSERT INTO local_service_meta(key, value) VALUES('schema_version', '5')
            ON CONFLICT(key) DO UPDATE SET value=excluded.value;

            CREATE TABLE IF NOT EXISTS skill_state (
                name TEXT PRIMARY KEY NOT NULL,
                enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS memory_entries (
                id TEXT PRIMARY KEY NOT NULL,
                scope TEXT NOT NULL CHECK(scope IN ('global', 'project')),
                workdir TEXT,
                kind TEXT NOT NULL CHECK(kind IN ('user', 'feedback', 'project', 'reference', 'daily')),
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                CHECK((scope='global' AND workdir IS NULL) OR (scope='project' AND workdir IS NOT NULL)),
                CHECK(kind!='daily' OR scope='global')
            );
            CREATE INDEX IF NOT EXISTS memory_entries_scope_updated
                ON memory_entries(scope, workdir, updated_at DESC);
            CREATE INDEX IF NOT EXISTS memory_entries_kind_updated
                ON memory_entries(kind, updated_at DESC);

            CREATE TABLE IF NOT EXISTS memory_usage_weekly (
                filter_scope TEXT NOT NULL CHECK(filter_scope IN ('global', 'project', 'all')),
                workdir TEXT NOT NULL,
                week_started_at INTEGER NOT NULL,
                searches INTEGER NOT NULL DEFAULT 0 CHECK(searches >= 0),
                writes INTEGER NOT NULL DEFAULT 0 CHECK(writes >= 0),
                PRIMARY KEY(filter_scope, workdir, week_started_at),
                CHECK((filter_scope='global' AND workdir='') OR (filter_scope!='global' AND workdir!=''))
            );
            CREATE INDEX IF NOT EXISTS memory_usage_weekly_started
                ON memory_usage_weekly(week_started_at, filter_scope, workdir);
            INSERT INTO local_service_meta(key, value)
                VALUES('memory_usage_tracking_started_at', CAST(strftime('%s', 'now') AS INTEGER) * 1000)
                ON CONFLICT(key) DO NOTHING;

            CREATE TABLE IF NOT EXISTS knowledge_base_folders (
                id TEXT PRIMARY KEY NOT NULL,
                canonical_path TEXT UNIQUE NOT NULL,
                display_name TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                last_indexed_at INTEGER,
                document_count INTEGER NOT NULL DEFAULT 0 CHECK(document_count >= 0),
                indexed_bytes INTEGER NOT NULL DEFAULT 0 CHECK(indexed_bytes >= 0)
            );
            CREATE TABLE IF NOT EXISTS knowledge_base_documents (
                id TEXT PRIMARY KEY NOT NULL,
                folder_id TEXT NOT NULL REFERENCES knowledge_base_folders(id) ON DELETE CASCADE,
                relative_path TEXT NOT NULL,
                title TEXT NOT NULL,
                extension TEXT NOT NULL,
                size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0),
                modified_at INTEGER,
                content_hash TEXT NOT NULL,
                content TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                UNIQUE(folder_id, relative_path)
            );
            CREATE INDEX IF NOT EXISTS knowledge_base_documents_folder_path
                ON knowledge_base_documents(folder_id, relative_path);

            INSERT INTO local_service_meta(key, value) VALUES('knowledge_base_enabled', '0')
                ON CONFLICT(key) DO NOTHING;

            CREATE TABLE IF NOT EXISTS cron_jobs (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL CHECK(kind IN ('prompt', 'http', 'shell')),
                schedule TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
                next_run_at INTEGER,
                last_run_at INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS cron_jobs_due
                ON cron_jobs(enabled, next_run_at);

            CREATE TABLE IF NOT EXISTS cron_runs (
                id TEXT PRIMARY KEY NOT NULL,
                job_id TEXT NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
                status TEXT NOT NULL CHECK(status IN ('dispatched', 'running', 'completed', 'failed')),
                scheduled_for INTEGER,
                started_at INTEGER NOT NULL,
                lease_expires_at INTEGER,
                completed_at INTEGER,
                has_output INTEGER NOT NULL DEFAULT 0 CHECK(has_output IN (0, 1)),
                has_error INTEGER NOT NULL DEFAULT 0 CHECK(has_error IN (0, 1)),
                output TEXT,
                error TEXT
            );
            CREATE INDEX IF NOT EXISTS cron_runs_job_started
                ON cron_runs(job_id, started_at DESC);
            "#,
        )
        .map_err(|error| format!("initialize local service database: {error}"))?;
    ensure_cron_run_lease_column(connection)?;
    ensure_cron_run_result_retention(connection)?;
    disable_legacy_unconfirmed_prompt_cron_jobs(connection)
}

/// Prompt Cron jobs used to be allowed to save enabled without a native
/// confirmation. Disable those historical rows exactly once during upgrade;
/// later enables must pass through the target-bound native capability.
fn disable_legacy_unconfirmed_prompt_cron_jobs(connection: &Connection) -> Result<(), String> {
    let timestamp = now_ms();
    connection
        .execute(
            "UPDATE cron_jobs SET enabled=0, next_run_at=NULL, updated_at=?1 \
             WHERE kind='prompt' AND enabled=1 \
             AND NOT EXISTS (SELECT 1 FROM local_service_meta WHERE key=?2)",
            params![timestamp, CRON_PROMPT_NATIVE_CONFIRMATION_MIGRATION_KEY],
        )
        .map_err(|error| format!("disable legacy unconfirmed Prompt Cron jobs: {error}"))?;
    // If another local process reaches this migration at the same time, both
    // updates are safe and only one marker needs to persist.
    connection
        .execute(
            "INSERT INTO local_service_meta(key, value) VALUES(?1, '1') \
             ON CONFLICT(key) DO NOTHING",
            params![CRON_PROMPT_NATIVE_CONFIRMATION_MIGRATION_KEY],
        )
        .map_err(|error| format!("record Prompt Cron confirmation migration: {error}"))?;
    Ok(())
}

fn ensure_cron_run_lease_column(connection: &Connection) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(cron_runs)")
        .map_err(|error| format!("inspect Cron run schema: {error}"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("inspect Cron run schema: {error}"))?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|error| format!("inspect Cron run schema: {error}"))?;
    if !columns.contains("lease_expires_at") {
        connection
            .execute(
                "ALTER TABLE cron_runs ADD COLUMN lease_expires_at INTEGER",
                [],
            )
            .map_err(|error| format!("migrate Cron run lease schema: {error}"))?;
    }
    connection
        .execute(
            "CREATE INDEX IF NOT EXISTS cron_runs_active_lease \
             ON cron_runs(status, lease_expires_at)",
            [],
        )
        .map_err(|error| format!("create Cron run lease index: {error}"))?;
    Ok(())
}

/// Move legacy Cron result text to explicit presence flags and clear the text
/// columns. The columns are deliberately kept for a forward-only compatible
/// migration: older app versions can still open the database, but new writes
/// never put untrusted output or error data into SQLite.
fn ensure_cron_run_result_retention(connection: &Connection) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(cron_runs)")
        .map_err(|error| format!("inspect Cron run result schema: {error}"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("inspect Cron run result schema: {error}"))?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|error| format!("inspect Cron run result schema: {error}"))?;
    drop(statement);

    for (name, definition) in [
        (
            "has_output",
            "INTEGER NOT NULL DEFAULT 0 CHECK(has_output IN (0, 1))",
        ),
        (
            "has_error",
            "INTEGER NOT NULL DEFAULT 0 CHECK(has_error IN (0, 1))",
        ),
    ] {
        if !columns.contains(name) {
            connection
                .execute(
                    &format!("ALTER TABLE cron_runs ADD COLUMN {name} {definition}"),
                    [],
                )
                .map_err(|error| format!("migrate Cron run result schema: {error}"))?;
        }
    }

    // Existing databases may contain complete command output, HTTP bodies, or
    // provider errors. Preserve only whether each value was present before
    // clearing it. `secure_delete=ON` is configured for every connection; a
    // best-effort WAL checkpoint below also removes old WAL frames when no
    // other process is holding a reader open.
    connection
        .execute(
            "UPDATE cron_runs \
             SET has_output=CASE WHEN output IS NULL THEN has_output ELSE 1 END, \
                 has_error=CASE WHEN error IS NULL THEN has_error ELSE 1 END, \
                 output=NULL, error=NULL \
             WHERE output IS NOT NULL OR error IS NOT NULL",
            [],
        )
        .map_err(|error| format!("minimize retained Cron run results: {error}"))?;
    // A busy checkpoint must not stop the application after the logical
    // migration succeeded. Attempt it on every initialization, including
    // later retries after the rows have already been redacted, so old WAL
    // frames get another chance to be truncated once readers have released.
    let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    Ok(())
}

/// Startup proves that no executor from the previous application process can
/// still complete these rows. Terminate every active run immediately rather
/// than waiting for its lease, and never put the scheduled occurrence back on
/// the queue.
fn recover_interrupted_cron_runs_on_startup(
    connection: &Connection,
    timestamp: i64,
) -> Result<usize, String> {
    connection
        .execute(
            "UPDATE cron_runs SET status='failed', completed_at=?1, lease_expires_at=NULL, \
              has_error=1, error=NULL \
              WHERE status IN ('dispatched', 'running')",
            params![timestamp],
        )
        .map_err(|error| format!("recover interrupted Cron runs on startup: {error}"))
}

/// Recover a lease that expired while the application remains open. Unlike
/// startup recovery, this cannot prove the owner has stopped, so it only
/// touches expired or legacy no-lease rows.
fn recover_expired_cron_runs(connection: &Connection, timestamp: i64) -> Result<usize, String> {
    connection
        .execute(
            "UPDATE cron_runs SET status='failed', completed_at=?1, lease_expires_at=NULL, \
              has_error=1, error=NULL \
              WHERE status IN ('dispatched', 'running') \
              AND (lease_expires_at IS NULL OR lease_expires_at<=?1)",
            params![timestamp],
        )
        .map_err(|error| format!("recover expired Cron runs: {error}"))
}

fn initialize_fts(connection: &Connection) -> bool {
    // Creating the virtual table and populating it in separate auto-commit
    // statements lets a process crash with a durable, empty index. Rebuild in
    // one transaction instead; either the complete replacement is visible, or
    // the next startup retries it and searches fall back to LIKE meanwhile.
    let result = connection.execute_batch(
        "BEGIN IMMEDIATE;
         DROP TABLE IF EXISTS memory_fts;
         CREATE VIRTUAL TABLE memory_fts USING fts5(
           id UNINDEXED, title, content, tokenize='unicode61'
         );
         INSERT INTO memory_fts(id, title, content)
           SELECT id, title, content FROM memory_entries;
         COMMIT;",
    );
    if result.is_ok() {
        true
    } else {
        let _ = connection.execute_batch("ROLLBACK;");
        false
    }
}

/// Existing portable databases may have been created before LocalServices
/// field encryption shipped. Rewrite the three content-bearing columns in a
/// single transaction before any IPC can read them. The portable marker makes
/// this idempotent and lets an interrupted migration safely resume.
fn migrate_portable_local_service_fields(connection: &Connection) -> Result<bool, String> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("start portable local-service migration: {error}"))?;
    let mut changed = false;
    for (table, id_column, value_column) in [
        ("memory_entries", "id", "content"),
        ("knowledge_base_documents", "id", "content"),
    ] {
        let sql = format!("SELECT {id_column}, {value_column} FROM {table}");
        let mut statement = transaction
            .prepare(&sql)
            .map_err(|error| format!("read portable {table} migration rows: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("read portable {table} migration rows: {error}"))?;
        let rows = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read portable {table} migration rows: {error}"))?;
        drop(statement);
        for (id, value) in rows {
            if is_portable_local_service_text(&value) {
                continue;
            }
            let encrypted = protect_portable_local_service_text(&value)?;
            let update = format!("UPDATE {table} SET {value_column}=?2 WHERE {id_column}=?1");
            transaction
                .execute(&update, params![id, encrypted])
                .map_err(|error| format!("encrypt portable {table} row: {error}"))?;
            changed = true;
        }
    }

    let mut statement = transaction
        .prepare("SELECT id, payload_json FROM cron_jobs")
        .map_err(|error| format!("read portable Cron migration rows: {error}"))?;
    let jobs = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("read portable Cron migration rows: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read portable Cron migration rows: {error}"))?;
    drop(statement);
    for (id, value) in jobs {
        if is_portable_local_service_text(&value) {
            continue;
        }
        // Legacy values used the settings envelope. Decode before rewrapping
        // so the entire Cron payload, rather than only credential-shaped
        // fields, is protected in portable mode.
        let payload = decode_cron_payload(&value)?;
        let plain = serde_json::to_string(&payload)
            .map_err(|_| "serialize portable Cron migration payload".to_string())?;
        let encrypted = protect_portable_local_service_text(&plain)?;
        transaction
            .execute(
                "UPDATE cron_jobs SET payload_json=?2 WHERE id=?1",
                params![id, encrypted],
            )
            .map_err(|error| format!("encrypt portable Cron row: {error}"))?;
        changed = true;
    }
    transaction
        .commit()
        .map_err(|error| format!("commit portable local-service migration: {error}"))?;
    Ok(changed)
}

/// FTS is a plaintext shadow copy. Portable mode uses bounded in-process
/// decrypt-and-search instead, and removes any legacy FTS tables before local
/// services are exposed.
fn disable_portable_full_text_indexes(connection: &Connection) -> Result<bool, String> {
    let has_indexes: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name IN ('memory_fts', 'knowledge_base_fts'))",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("inspect portable full-text indexes: {error}"))?;
    connection
        .execute_batch("DROP TABLE IF EXISTS memory_fts; DROP TABLE IF EXISTS knowledge_base_fts;")
        .map_err(|error| format!("remove plaintext portable full-text indexes: {error}"))?;
    Ok(has_indexes)
}

/// Reclaim SQLite pages and truncate the WAL only after the migration has
/// committed. This prevents the pre-migration plaintext and dropped FTS pages
/// from remaining as an easily readable adjacent file in a portable folder.
fn scrub_portable_sqlite_plaintext(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(|error| format!("scrub migrated portable local-service database: {error}"))
}

/// Keep external knowledge separate from user-authored Memory.  FTS is an
/// optional accelerator: a bundled SQLite normally has it, but a missing FTS5
/// module must leave a bounded LIKE search available instead of preventing the
/// desktop application from starting.
fn initialize_knowledge_fts(connection: &Connection) -> bool {
    match knowledge_base_fts_is_current(connection) {
        Ok(true) => true,
        Ok(false) | Err(_) => rebuild_knowledge_fts(connection),
    }
}

fn knowledge_base_fts_table_exists(connection: &Connection) -> Result<bool, rusqlite::Error> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='knowledge_base_fts'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map(|value| value.is_some())
}

/// A table name alone is not evidence that a prior initialisation completed:
/// SQLite can persist the DDL if a process exits before the backfill. Compare
/// the durable index to its source rows and rebuild mismatches atomically.
fn knowledge_base_fts_is_current(connection: &Connection) -> Result<bool, rusqlite::Error> {
    if !knowledge_base_fts_table_exists(connection)? {
        return Ok(false);
    }
    connection
        .query_row(
            "SELECT (SELECT COUNT(*) FROM knowledge_base_fts) = \
                (SELECT COUNT(*) FROM knowledge_base_documents)",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|matches| matches != 0)
}

fn rebuild_knowledge_fts(connection: &Connection) -> bool {
    // Keep DDL and backfill in one transaction: a power loss then leaves the
    // old complete index or no index, never a durable empty partial table.
    let rebuild = connection.execute_batch(
        "BEGIN IMMEDIATE;
         DROP TABLE IF EXISTS knowledge_base_fts;
         CREATE VIRTUAL TABLE knowledge_base_fts USING fts5(
           document_id UNINDEXED, folder_id UNINDEXED, title, relative_path, content,
           tokenize='unicode61'
         );
         INSERT INTO knowledge_base_fts(document_id, folder_id, title, relative_path, content)
           SELECT id, folder_id, title, relative_path, content FROM knowledge_base_documents;
         COMMIT;",
    );
    if rebuild.is_ok() {
        return true;
    }
    let _ = connection.execute_batch("ROLLBACK;");
    false
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn bounded_nonempty(value: &str, field: &str, max_chars: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{field} is required"));
    }
    if value.chars().count() > max_chars {
        return Err(format!("{field} exceeds the {max_chars} character limit"));
    }
    Ok(value.to_string())
}

fn bounded_bytes(value: &str, field: &str, max_bytes: usize) -> Result<String, String> {
    if value.trim().is_empty() {
        return Err(format!("{field} is required"));
    }
    if value.len() > max_bytes {
        return Err(format!("{field} exceeds the {max_bytes} byte limit"));
    }
    Ok(value.to_string())
}

fn validate_record_id(value: &str, field: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(format!("{field} is invalid"));
    }
    Ok(value.to_string())
}

// ---------------------------------------------------------------------------
// Skills

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub built_in: bool,
    pub root_dir: String,
    pub skill_file: String,
    pub file_count: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvalidSkill {
    pub directory: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsListResponse {
    pub root_dir: String,
    pub skills: Vec<SkillSummary>,
    pub invalid: Vec<InvalidSkill>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillReadResponse {
    pub skill: SkillSummary,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEnabledResponse {
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallResponse {
    pub skill: SkillSummary,
}

/// Renderer-safe ClawHub card data. The native layer fixes the registry
/// origin and intentionally returns only bounded catalog metadata.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsCatalogItem {
    pub slug: String,
    pub reference: Option<String>,
    pub owner_handle: Option<String>,
    pub display_name: String,
    pub summary: Option<String>,
    pub latest_version: Option<String>,
    pub downloads: Option<u64>,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsCatalogListResponse {
    pub items: Vec<SkillsCatalogItem>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsCatalogOwnerChoice {
    pub owner_handle: String,
    pub slug: String,
    pub reference: String,
    pub source_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsCatalogScannerSummary {
    pub name: String,
    pub status: Option<String>,
    pub verdict: Option<String>,
    pub severity: Option<String>,
    pub recommendation: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsCatalogSecuritySummary {
    pub status: String,
    pub has_warnings: bool,
    pub installable: bool,
    pub install_block_reason: Option<String>,
    pub scanners: Vec<SkillsCatalogScannerSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsCatalogDetail {
    pub reference: String,
    pub slug: String,
    pub owner_handle: String,
    pub owner_display_name: Option<String>,
    pub display_name: String,
    pub summary: Option<String>,
    pub version: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub local_skill_name: String,
    pub source_url: String,
    pub security: SkillsCatalogSecuritySummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "data")]
pub enum SkillsCatalogDetailResponse {
    Found(SkillsCatalogDetail),
    Ambiguous {
        slug: String,
        matches: Vec<SkillsCatalogOwnerChoice>,
    },
    Blocked {
        reference: String,
        slug: String,
        owner_handle: String,
        owner_display_name: Option<String>,
        display_name: String,
        summary: Option<String>,
        source_url: String,
        security: SkillsCatalogSecuritySummary,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsCatalogInstallInput {
    pub reference: String,
    pub version: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsCatalogInstallResponse {
    pub skill: SkillSummary,
    pub reference: String,
    pub version: String,
    pub source_url: String,
    pub security: SkillsCatalogSecuritySummary,
}

#[derive(Debug, Clone)]
struct ClawHubReference {
    owner_handle: Option<String>,
    slug: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubListResponse {
    #[serde(default)]
    items: Vec<ClawHubListItem>,
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubListItem {
    slug: String,
    display_name: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    stats: Option<ClawHubStats>,
    #[serde(default)]
    updated_at: Option<i64>,
    #[serde(default)]
    latest_version: Option<ClawHubLatestVersion>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubSearchResponse {
    #[serde(default)]
    results: Vec<ClawHubSearchItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubSearchItem {
    slug: Option<String>,
    display_name: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    downloads: Option<u64>,
    #[serde(default)]
    updated_at: Option<i64>,
    #[serde(default)]
    owner_handle: Option<String>,
    #[serde(default)]
    owner: Option<ClawHubOwner>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubStats {
    #[serde(default)]
    downloads: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubLatestVersion {
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubDetailResponse {
    skill: Option<ClawHubDetailSkill>,
    #[serde(default)]
    latest_version: Option<ClawHubLatestVersion>,
    #[serde(default)]
    owner: Option<ClawHubOwner>,
    #[serde(default)]
    moderation: Option<ClawHubModeration>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubDetailSkill {
    slug: String,
    display_name: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    tags: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubOwner {
    handle: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubModeration {
    #[serde(default)]
    is_suspicious: bool,
    #[serde(default)]
    is_malware_blocked: bool,
    #[serde(default)]
    verdict: Option<String>,
    #[serde(default)]
    summary: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubVersionResponse {
    #[serde(default)]
    version: Option<ClawHubVersion>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubVersion {
    version: String,
    #[serde(default)]
    files: Vec<ClawHubFile>,
    #[serde(default)]
    security: Option<ClawHubSecurity>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubFile {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubSecurity {
    status: Option<String>,
    #[serde(default)]
    has_warnings: bool,
    #[serde(default)]
    scanners: Option<ClawHubScanners>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubScanners {
    #[serde(default)]
    vt: Option<ClawHubScanner>,
    #[serde(default)]
    skillspector: Option<ClawHubScanner>,
    #[serde(default)]
    llm: Option<ClawHubScanner>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubScanner {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    verdict: Option<String>,
    #[serde(default)]
    normalized_status: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    recommendation: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    risk_summary: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubAmbiguousResponse {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    matches: Vec<ClawHubAmbiguousMatch>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawHubAmbiguousMatch {
    owner_handle: Option<String>,
    slug: Option<String>,
}

#[derive(Debug, Clone)]
struct SkillMetadata {
    name: String,
    description: String,
    file_count: usize,
    total_bytes: u64,
}

struct StagingDirectory {
    cleanup_path: PathBuf,
    path: PathBuf,
    armed: bool,
}

impl StagingDirectory {
    fn new(root: &Path, skill_name: &str) -> Result<Self, String> {
        let cleanup_path = root.join(STAGING_DIRECTORY).join(format!(
            "install-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        fs::create_dir(&cleanup_path)
            .map_err(|error| format!("create Skill staging container: {error}"))?;
        let path = cleanup_path.join(skill_name);
        fs::create_dir(&path).map_err(|error| format!("create staged Skill: {error}"))?;
        Ok(Self {
            cleanup_path,
            path,
            armed: true,
        })
    }

    fn disarm(&mut self) {
        let _ = fs::remove_dir(&self.cleanup_path);
        self.armed = false;
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.cleanup_path);
        }
    }
}

impl LocalServices {
    fn seed_builtin_skills(&self, connection: &Connection) -> Result<(), String> {
        for (name, content) in BUILTIN_SKILLS {
            let target = self.skills_root.join(name);
            if !target.exists() {
                let mut staged = StagingDirectory::new(&self.skills_root, name)?;
                fs::write(staged.path.join("SKILL.md"), content)
                    .map_err(|error| format!("write builtin Skill: {error}"))?;
                validate_skill_directory(&staged.path)?;
                let _write_guard = skills_write_guard();
                if !target.exists() {
                    fs::rename(&staged.path, &target)
                        .map_err(|error| format!("activate builtin Skill: {error}"))?;
                    staged.disarm();
                }
            }
            if target.exists() {
                validate_skill_directory(&target)?;
                connection
                    .execute(
                        "INSERT OR IGNORE INTO skill_state(name, enabled, updated_at) VALUES(?1, 1, ?2)",
                        params![name, now_ms()],
                    )
                    .map_err(|error| format!("initialize builtin Skill state: {error}"))?;
            }
        }
        Ok(())
    }

    pub fn list_skills(&self) -> Result<SkillsListResponse, String> {
        let connection = self.open()?;
        let mut enabled = HashMap::new();
        {
            let mut statement = connection
                .prepare("SELECT name, enabled FROM skill_state")
                .map_err(|error| format!("read Skill state: {error}"))?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
                })
                .map_err(|error| format!("read Skill state: {error}"))?;
            for row in rows {
                let (name, value) = row.map_err(|error| format!("read Skill state: {error}"))?;
                enabled.insert(name, value);
            }
        }

        let mut skills = Vec::new();
        let mut invalid = Vec::new();
        let entries = fs::read_dir(&self.skills_root)
            .map_err(|error| format!("list Skills directory: {error}"))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("list Skills directory: {error}"))?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            match validate_skill_directory(&path) {
                Ok(metadata) => skills.push(skill_summary(
                    &path,
                    metadata,
                    enabled.get(&name).copied().unwrap_or(true),
                )),
                Err(error) => invalid.push(InvalidSkill {
                    directory: name,
                    error,
                }),
            }
        }
        skills.sort_by(|left, right| left.name.cmp(&right.name));
        invalid.sort_by(|left, right| left.directory.cmp(&right.directory));
        Ok(SkillsListResponse {
            root_dir: path_for_display(&self.skills_root),
            skills,
            invalid,
        })
    }

    pub fn read_skill(&self, name: &str) -> Result<SkillReadResponse, String> {
        self.ensure_ready()?;
        let name = validate_skill_name(name)?;
        let target = self.skills_root.join(&name);
        let metadata = validate_skill_directory(&target)?;
        let content = fs::read_to_string(target.join("SKILL.md"))
            .map_err(|error| format!("read Skill instructions: {error}"))?;
        let enabled = self.skill_enabled(&name)?;
        Ok(SkillReadResponse {
            skill: skill_summary(&target, metadata, enabled),
            content,
        })
    }

    /// Settings needs to keep disabled Skills visible so a user can inspect
    /// and re-enable them. Agent capabilities use the separate methods below,
    /// which take an enabled-state snapshot under the same lock as state
    /// updates and never expose disabled Skill content.
    pub fn list_agent_skills(&self) -> Result<SkillsListResponse, String> {
        let _skills_guard = skills_write_guard();
        let mut response = self.list_skills()?;
        response.skills.retain(|skill| skill.enabled);
        // Invalid directories are useful diagnostics in Settings, but are not
        // executable Agent Skills and should not become Agent-visible records.
        response.invalid.clear();
        Ok(response)
    }

    pub fn read_agent_skill(&self, name: &str) -> Result<SkillReadResponse, String> {
        let name = validate_skill_name(name)?;
        let _skills_guard = skills_write_guard();
        if !self.skill_enabled(&name)? {
            return Err("Skill is disabled for Agent use".to_string());
        }
        self.read_skill(&name)
    }

    fn skill_enabled(&self, name: &str) -> Result<bool, String> {
        self.open()?
            .query_row(
                "SELECT enabled FROM skill_state WHERE name=?1",
                params![name],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .map_err(|error| format!("read Skill state: {error}"))
            .map(|enabled| enabled.unwrap_or(true))
    }

    pub fn set_skill_enabled(
        &self,
        name: &str,
        enabled: bool,
    ) -> Result<SkillEnabledResponse, String> {
        self.ensure_ready()?;
        let name = validate_skill_name(name)?;
        let _skills_guard = skills_write_guard();
        validate_skill_directory(&self.skills_root.join(&name))?;
        self.open()?
            .execute(
                "INSERT INTO skill_state(name, enabled, updated_at) VALUES(?1, ?2, ?3) \
                 ON CONFLICT(name) DO UPDATE SET enabled=excluded.enabled, updated_at=excluded.updated_at",
                params![name, enabled, now_ms()],
            )
            .map_err(|error| format!("update Skill state: {error}"))?;
        Ok(SkillEnabledResponse { name, enabled })
    }

    pub fn install_skill_from_directory(
        &self,
        source: &Path,
    ) -> Result<SkillInstallResponse, String> {
        self.ensure_ready()?;
        let source_metadata = validate_skill_directory(source)?;
        let mut staged = StagingDirectory::new(&self.skills_root, &source_metadata.name)?;
        copy_skill_tree(source, &staged.path)?;
        let staged_metadata = validate_skill_directory(&staged.path)?;
        if staged_metadata.name != source_metadata.name {
            return Err("staged Skill metadata changed during installation".to_string());
        }

        let target = self.skills_root.join(&staged_metadata.name);
        let connection = self.open()?;
        let _write_guard = skills_write_guard();
        if target.exists() {
            return Err("a Skill with this name is already installed".to_string());
        }
        fs::rename(&staged.path, &target)
            .map_err(|error| format!("activate installed Skill: {error}"))?;
        staged.disarm();

        if let Err(error) = connection.execute(
            "INSERT INTO skill_state(name, enabled, updated_at) VALUES(?1, 1, ?2) \
             ON CONFLICT(name) DO UPDATE SET enabled=1, updated_at=excluded.updated_at",
            params![staged_metadata.name, now_ms()],
        ) {
            let _ = fs::remove_dir_all(&target);
            return Err(format!("record installed Skill state: {error}"));
        }
        Ok(SkillInstallResponse {
            skill: skill_summary(&target, staged_metadata, true),
        })
    }

    pub async fn skills_catalog_list(
        &self,
        limit: Option<usize>,
        cursor: Option<String>,
        sort: Option<String>,
    ) -> Result<SkillsCatalogListResponse, String> {
        self.ensure_ready()?;
        let limit = catalog_limit(limit)?;
        let cursor = validate_catalog_cursor(cursor)?;
        let sort = validate_catalog_sort(sort)?;
        let mut query = vec![
            ("limit".to_string(), limit.to_string()),
            ("sort".to_string(), sort),
            ("nonSuspiciousOnly".to_string(), "true".to_string()),
        ];
        if let Some(cursor) = cursor {
            query.push(("cursor".to_string(), cursor));
        }
        let response: ClawHubListResponse = clawhub_json(&["skills"], &query).await?;
        let mut items = Vec::new();
        for item in response.items {
            let Ok(slug) = validate_catalog_slug(&item.slug) else {
                continue;
            };
            let display_name = catalog_display_name(&item.display_name, &slug);
            items.push(SkillsCatalogItem {
                slug,
                reference: None,
                owner_handle: None,
                display_name,
                summary: catalog_optional_text(item.summary, MAX_CLAWHUB_SUMMARY_CHARS),
                latest_version: item
                    .latest_version
                    .and_then(|version| validate_catalog_version(&version.version).ok()),
                downloads: item.stats.and_then(|stats| stats.downloads),
                updated_at: item.updated_at,
            });
        }
        Ok(SkillsCatalogListResponse {
            items,
            next_cursor: response
                .next_cursor
                .and_then(|value| validate_catalog_cursor(Some(value)).ok().flatten()),
        })
    }

    pub async fn skills_catalog_search(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> Result<SkillsCatalogListResponse, String> {
        self.ensure_ready()?;
        let query_text = validate_catalog_query(query)?;
        let limit = catalog_limit(limit)?;
        let response: ClawHubSearchResponse = clawhub_json(
            &["search"],
            &[
                ("q".to_string(), query_text),
                ("limit".to_string(), limit.to_string()),
                ("nonSuspiciousOnly".to_string(), "true".to_string()),
            ],
        )
        .await?;
        let mut items = Vec::new();
        for item in response.results {
            let Some(raw_slug) = item.slug else {
                continue;
            };
            let Ok(slug) = validate_catalog_slug(&raw_slug) else {
                continue;
            };
            let raw_owner = item
                .owner_handle
                .or_else(|| item.owner.and_then(|owner| owner.handle));
            let Some(raw_owner) = raw_owner else {
                continue;
            };
            let Ok(owner_handle) = validate_catalog_owner_handle(&raw_owner) else {
                continue;
            };
            let reference = catalog_reference_text(&owner_handle, &slug);
            items.push(SkillsCatalogItem {
                slug: slug.clone(),
                reference: Some(reference),
                owner_handle: Some(owner_handle),
                display_name: item
                    .display_name
                    .as_deref()
                    .map(|value| catalog_display_name(value, &slug))
                    .unwrap_or_else(|| slug.clone()),
                summary: catalog_optional_text(item.summary, MAX_CLAWHUB_SUMMARY_CHARS),
                latest_version: item
                    .version
                    .and_then(|version| validate_catalog_version(&version).ok()),
                downloads: item.downloads,
                updated_at: item.updated_at,
            });
        }
        Ok(SkillsCatalogListResponse {
            items,
            next_cursor: None,
        })
    }

    pub async fn skills_catalog_detail(
        &self,
        reference: &str,
        version: Option<String>,
    ) -> Result<SkillsCatalogDetailResponse, String> {
        self.ensure_ready()?;
        let requested = parse_catalog_reference(reference, false)?;
        let detail = match clawhub_detail(&requested).await? {
            ClawHubDetailLookup::Found(detail) => detail,
            ClawHubDetailLookup::Ambiguous { slug, matches } => {
                return Ok(SkillsCatalogDetailResponse::Ambiguous { slug, matches });
            }
        };
        let resolved = resolve_catalog_detail_reference(&requested, &detail)?;
        let source_url = clawhub_source_url(&resolved)?;
        let skill = detail
            .skill
            .as_ref()
            .ok_or_else(|| "ClawHub detail response did not contain a Skill record".to_string())?;
        let display_name = catalog_display_name(&skill.display_name, &resolved.slug);
        let summary = catalog_optional_text(skill.summary.clone(), MAX_CLAWHUB_SUMMARY_CHARS);
        let owner_display_name = detail.owner.as_ref().and_then(|owner| {
            catalog_optional_text(owner.display_name.clone(), MAX_CLAWHUB_DISPLAY_NAME_CHARS)
        });

        if let Some(moderation) = detail.moderation.as_ref() {
            if catalog_moderation_blocks_install(moderation) {
                return Ok(SkillsCatalogDetailResponse::Blocked {
                    reference: catalog_reference_text(
                        resolved.owner_handle.as_deref().expect("resolved owner"),
                        &resolved.slug,
                    ),
                    slug: resolved.slug,
                    owner_handle: resolved.owner_handle.expect("resolved owner"),
                    owner_display_name,
                    display_name,
                    summary,
                    source_url,
                    security: catalog_moderation_summary(moderation),
                });
            }
        }

        let selected_version = catalog_selected_version(&detail, version)?;
        let version_response = clawhub_version(&resolved, &selected_version).await?;
        let remote_version = version_response.version.ok_or_else(|| {
            "ClawHub version response did not contain the requested version".to_string()
        })?;
        if remote_version.version != selected_version {
            return Err("ClawHub returned a different version than was requested".to_string());
        }
        let files = validate_catalog_files(remote_version.files)?;
        let security = catalog_security_summary(remote_version.security.as_ref());
        let owner_handle = resolved.owner_handle.clone().expect("resolved owner");
        let canonical_reference = catalog_reference_text(&owner_handle, &resolved.slug);
        if !security.installable {
            return Ok(SkillsCatalogDetailResponse::Blocked {
                reference: canonical_reference,
                slug: resolved.slug,
                owner_handle,
                owner_display_name,
                display_name,
                summary,
                source_url,
                security,
            });
        }
        let skill_file = files.skill_file();
        let skill_bytes = clawhub_file(&resolved, &selected_version, skill_file).await?;
        verify_catalog_file(skill_file, &skill_bytes)?;
        let local_skill_name = parse_catalog_skill_name(&skill_bytes)?;

        Ok(SkillsCatalogDetailResponse::Found(SkillsCatalogDetail {
            reference: canonical_reference,
            slug: resolved.slug,
            owner_handle,
            owner_display_name,
            display_name,
            summary,
            version: selected_version,
            file_count: files.entries.len(),
            total_bytes: files.total_bytes,
            local_skill_name,
            source_url,
            security,
        }))
    }

    pub async fn skills_catalog_install(
        &self,
        input: SkillsCatalogInstallInput,
    ) -> Result<SkillsCatalogInstallResponse, String> {
        self.ensure_ready()?;
        let requested = parse_catalog_reference(&input.reference, true)?;
        let requested_version = validate_catalog_version(&input.version)?;
        let requested_reference = catalog_reference_text(
            requested.owner_handle.as_deref().expect("owner required"),
            &requested.slug,
        );
        let expected_confirmation =
            catalog_install_confirmation(&requested_reference, &requested_version);
        if input.confirmation.trim() != expected_confirmation {
            return Err("ClawHub installation requires an explicit confirmation for this exact Skill and version".to_string());
        }

        let detail = match clawhub_detail(&requested).await? {
            ClawHubDetailLookup::Found(detail) => detail,
            ClawHubDetailLookup::Ambiguous { .. } => {
                return Err(
                    "ClawHub requires an owner-qualified Skill reference for installation"
                        .to_string(),
                );
            }
        };
        let resolved = resolve_catalog_detail_reference(&requested, &detail)?;
        if resolved.owner_handle != requested.owner_handle || resolved.slug != requested.slug {
            return Err(
                "ClawHub Skill owner or slug did not match the approved reference".to_string(),
            );
        }
        if let Some(moderation) = detail.moderation.as_ref() {
            if catalog_moderation_blocks_install(moderation) {
                return Err(catalog_moderation_summary(moderation)
                    .install_block_reason
                    .unwrap_or_else(|| {
                        "ClawHub moderation does not allow installation".to_string()
                    }));
            }
        }

        let version_response = clawhub_version(&resolved, &requested_version).await?;
        let remote_version = version_response.version.ok_or_else(|| {
            "ClawHub version response did not contain the requested version".to_string()
        })?;
        if remote_version.version != requested_version {
            return Err("ClawHub returned a different version than was approved".to_string());
        }
        let files = validate_catalog_files(remote_version.files)?;
        let security = catalog_security_summary(remote_version.security.as_ref());
        if !security.installable {
            return Err(security.install_block_reason.clone().unwrap_or_else(|| {
                "ClawHub did not provide an installable security result".to_string()
            }));
        }

        let skill_file = files.skill_file();
        let skill_bytes = clawhub_file(&resolved, &requested_version, skill_file).await?;
        verify_catalog_file(skill_file, &skill_bytes)?;
        let local_skill_name = parse_catalog_skill_name(&skill_bytes)?;
        let staged = StagingDirectory::new(&self.skills_root, &local_skill_name)?;
        write_catalog_file(&staged.path, skill_file, &skill_bytes)?;
        for file in files.entries.iter().filter(|file| file.path != "SKILL.md") {
            let bytes = clawhub_file(&resolved, &requested_version, file).await?;
            verify_catalog_file(file, &bytes)?;
            write_catalog_file(&staged.path, file, &bytes)?;
        }
        validate_skill_directory(&staged.path)?;
        let installed = self.install_skill_from_directory(&staged.path)?;
        Ok(SkillsCatalogInstallResponse {
            skill: installed.skill,
            reference: requested_reference,
            version: requested_version,
            source_url: clawhub_source_url(&resolved)?,
            security,
        })
    }
}

#[derive(Debug)]
enum ClawHubDetailLookup {
    Found(ClawHubDetailResponse),
    Ambiguous {
        slug: String,
        matches: Vec<SkillsCatalogOwnerChoice>,
    },
}

#[derive(Debug)]
struct ValidatedCatalogFiles {
    entries: Vec<ClawHubFile>,
    total_bytes: u64,
    skill_file_index: usize,
}

impl ValidatedCatalogFiles {
    fn skill_file(&self) -> &ClawHubFile {
        &self.entries[self.skill_file_index]
    }
}

fn catalog_limit(limit: Option<usize>) -> Result<usize, String> {
    let limit = limit.unwrap_or(12);
    if !(1..=20).contains(&limit) {
        return Err("ClawHub catalog limit must be between 1 and 20".to_string());
    }
    Ok(limit)
}

fn validate_catalog_sort(sort: Option<String>) -> Result<String, String> {
    let sort = sort.unwrap_or_else(|| "recommended".to_string());
    let sort = sort.trim();
    if [
        "recommended",
        "default",
        "updated",
        "createdAt",
        "newest",
        "downloads",
        "stars",
        "rating",
        "installs",
        "trending",
    ]
    .contains(&sort)
    {
        Ok(sort.to_string())
    } else {
        Err("ClawHub catalog sort is not supported".to_string())
    }
}

fn validate_catalog_cursor(cursor: Option<String>) -> Result<Option<String>, String> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    let cursor = cursor.trim();
    if cursor.is_empty() {
        return Ok(None);
    }
    if cursor.len() > MAX_CLAWHUB_CURSOR_BYTES || cursor.chars().any(char::is_control) {
        return Err("ClawHub catalog cursor is invalid".to_string());
    }
    Ok(Some(cursor.to_string()))
}

fn validate_catalog_query(query: &str) -> Result<String, String> {
    let query = query.trim();
    if query.is_empty()
        || query.chars().count() > MAX_CLAWHUB_SEARCH_CHARS
        || query.chars().any(char::is_control)
    {
        return Err("ClawHub search text must be 1-120 printable characters".to_string());
    }
    Ok(query.to_string())
}

fn validate_catalog_slug(raw: &str) -> Result<String, String> {
    let slug = raw.trim().to_ascii_lowercase();
    let valid = !slug.is_empty()
        && slug.len() <= MAX_SKILL_NAME_CHARS
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && !slug.contains("--")
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !valid {
        return Err("ClawHub Skill slug is invalid".to_string());
    }
    Ok(slug)
}

fn validate_catalog_owner_handle(raw: &str) -> Result<String, String> {
    let handle = raw.trim().trim_start_matches('@').to_ascii_lowercase();
    let valid = !handle.is_empty()
        && handle.len() <= 128
        && !handle.starts_with(['.', '-', '_'])
        && !handle.ends_with(['.', '-', '_'])
        && handle.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        });
    if !valid {
        return Err("ClawHub owner handle is invalid".to_string());
    }
    Ok(handle)
}

fn validate_catalog_version(raw: &str) -> Result<String, String> {
    let version = raw.trim();
    let valid = !version.is_empty()
        && version.len() <= MAX_CLAWHUB_VERSION_CHARS
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'));
    if !valid {
        return Err("ClawHub Skill version is invalid".to_string());
    }
    Ok(version.to_string())
}

fn parse_catalog_reference(raw: &str, require_owner: bool) -> Result<ClawHubReference, String> {
    let reference = raw.trim();
    if let Some(owner_reference) = reference.strip_prefix('@') {
        let (owner_handle, slug) = owner_reference
            .split_once('/')
            .ok_or_else(|| "ClawHub Skill reference must use @owner/slug".to_string())?;
        if slug.contains('/') {
            return Err("ClawHub Skill reference must use exactly one owner and slug".to_string());
        }
        return Ok(ClawHubReference {
            owner_handle: Some(validate_catalog_owner_handle(owner_handle)?),
            slug: validate_catalog_slug(slug)?,
        });
    }
    if require_owner {
        return Err("ClawHub installation requires an @owner/slug reference".to_string());
    }
    Ok(ClawHubReference {
        owner_handle: None,
        slug: validate_catalog_slug(reference)?,
    })
}

fn catalog_reference_text(owner_handle: &str, slug: &str) -> String {
    format!("@{owner_handle}/{slug}")
}

fn catalog_install_confirmation(reference: &str, version: &str) -> String {
    format!("INSTALL_CLAWHUB_SKILL:{reference}@{version}")
}

fn catalog_display_name(raw: &str, fallback: &str) -> String {
    catalog_optional_text(Some(raw.to_string()), MAX_CLAWHUB_DISPLAY_NAME_CHARS)
        .unwrap_or_else(|| fallback.to_string())
}

fn catalog_optional_text(value: Option<String>, max_chars: usize) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        if value.is_empty()
            || value
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        {
            return None;
        }
        let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut chars = normalized.chars();
        let bounded: String = chars.by_ref().take(max_chars).collect();
        Some(if chars.next().is_some() {
            format!("{bounded}…")
        } else {
            bounded
        })
    })
}

fn resolve_catalog_detail_reference(
    requested: &ClawHubReference,
    detail: &ClawHubDetailResponse,
) -> Result<ClawHubReference, String> {
    let skill = detail
        .skill
        .as_ref()
        .ok_or_else(|| "ClawHub detail response did not contain a Skill record".to_string())?;
    let slug = validate_catalog_slug(&skill.slug)?;
    if slug != requested.slug {
        return Err("ClawHub returned a different Skill slug than was requested".to_string());
    }
    let owner = detail
        .owner
        .as_ref()
        .and_then(|owner| owner.handle.as_deref())
        .ok_or_else(|| "ClawHub detail response did not contain an owner handle".to_string())?;
    let owner_handle = validate_catalog_owner_handle(owner)?;
    if let Some(expected_owner) = requested.owner_handle.as_deref() {
        if expected_owner != owner_handle {
            return Err("ClawHub returned a different Skill owner than was requested".to_string());
        }
    }
    Ok(ClawHubReference {
        owner_handle: Some(owner_handle),
        slug,
    })
}

fn catalog_selected_version(
    detail: &ClawHubDetailResponse,
    requested: Option<String>,
) -> Result<String, String> {
    if let Some(version) = requested {
        return validate_catalog_version(&version);
    }
    let latest = detail
        .latest_version
        .as_ref()
        .map(|version| version.version.as_str())
        .or_else(|| {
            detail
                .skill
                .as_ref()
                .and_then(|skill| skill.tags.get("latest"))
                .map(String::as_str)
        })
        .ok_or_else(|| "ClawHub did not provide a latest Skill version".to_string())?;
    validate_catalog_version(latest)
}

fn catalog_moderation_blocks_install(moderation: &ClawHubModeration) -> bool {
    moderation.is_malware_blocked
        || moderation.is_suspicious
        || moderation
            .verdict
            .as_deref()
            .map(normalize_catalog_status)
            .map(|value| value != "clean")
            .unwrap_or(false)
}

fn catalog_moderation_summary(moderation: &ClawHubModeration) -> SkillsCatalogSecuritySummary {
    let status = if moderation.is_malware_blocked {
        "malicious".to_string()
    } else if moderation.is_suspicious {
        "suspicious".to_string()
    } else {
        moderation
            .verdict
            .as_deref()
            .map(normalize_catalog_status)
            .unwrap_or_else(|| "unavailable".to_string())
    };
    let reason = catalog_optional_text(moderation.summary.clone(), MAX_CLAWHUB_RISK_SUMMARY_CHARS)
        .unwrap_or_else(|| {
            "ClawHub moderation does not allow this Skill to be installed.".to_string()
        });
    SkillsCatalogSecuritySummary {
        status,
        has_warnings: true,
        installable: false,
        install_block_reason: Some(reason.clone()),
        scanners: vec![SkillsCatalogScannerSummary {
            name: "ClawHub moderation".to_string(),
            status: None,
            verdict: moderation.verdict.as_deref().map(normalize_catalog_status),
            severity: None,
            recommendation: Some("DO_NOT_INSTALL".to_string()),
            summary: Some(reason),
        }],
    }
}

fn catalog_security_summary(security: Option<&ClawHubSecurity>) -> SkillsCatalogSecuritySummary {
    let Some(security) = security else {
        return SkillsCatalogSecuritySummary {
            status: "unavailable".to_string(),
            has_warnings: true,
            installable: false,
            install_block_reason: Some(
                "ClawHub did not provide a security result for this exact version.".to_string(),
            ),
            scanners: Vec::new(),
        };
    };
    let status = security
        .status
        .as_deref()
        .map(normalize_catalog_status)
        .unwrap_or_else(|| "unavailable".to_string());
    let mut scanners = Vec::new();
    let mut block_reason = if status == "clean" {
        None
    } else {
        Some(format!(
            "ClawHub security status for this exact version is {status}, not clean."
        ))
    };
    if let Some(all_scanners) = security.scanners.as_ref() {
        for (name, scanner) in [
            ("VirusTotal", all_scanners.vt.as_ref()),
            ("SkillSpector", all_scanners.skillspector.as_ref()),
            ("LLM analysis", all_scanners.llm.as_ref()),
        ] {
            let Some(scanner) = scanner else {
                continue;
            };
            let scanner_status = scanner
                .normalized_status
                .as_deref()
                .or(scanner.status.as_deref())
                .map(normalize_catalog_status);
            let verdict = scanner.verdict.as_deref().map(normalize_catalog_status);
            let severity = scanner.severity.as_deref().map(normalize_catalog_status);
            let recommendation = scanner
                .recommendation
                .as_deref()
                .map(normalize_catalog_status);
            if block_reason.is_none()
                && (scanner_status
                    .as_deref()
                    .is_some_and(catalog_status_is_dangerous)
                    || verdict.as_deref().is_some_and(catalog_status_is_dangerous)
                    || severity
                        .as_deref()
                        .is_some_and(|value| matches!(value, "high" | "critical"))
                    || recommendation.as_deref().is_some_and(|value| {
                        matches!(
                            value,
                            "do_not_install" | "do-not-install" | "do not install"
                        )
                    })
                    || scanner
                        .risk_summary
                        .as_ref()
                        .is_some_and(catalog_risk_summary_is_dangerous))
            {
                block_reason = Some(format!(
                    "{name} reported a high-risk result for this version."
                ));
            }
            scanners.push(SkillsCatalogScannerSummary {
                name: name.to_string(),
                status: scanner_status,
                verdict,
                severity,
                recommendation,
                summary: catalog_optional_text(
                    scanner.summary.clone(),
                    MAX_CLAWHUB_RISK_SUMMARY_CHARS,
                ),
            });
        }
    }
    SkillsCatalogSecuritySummary {
        status,
        has_warnings: security.has_warnings,
        installable: block_reason.is_none(),
        install_block_reason: block_reason,
        scanners,
    }
}

fn normalize_catalog_status(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn catalog_status_is_dangerous(value: &str) -> bool {
    matches!(
        value,
        "malicious" | "suspicious" | "blocked" | "pending" | "error" | "unsafe" | "dangerous"
    )
}

fn catalog_risk_summary_is_dangerous(value: &Value) -> bool {
    match value {
        Value::Object(values) => values.iter().any(|(key, value)| {
            let key = key.trim().to_ascii_lowercase();
            let relevant = matches!(
                key.as_str(),
                "risk"
                    | "risk_level"
                    | "risklevel"
                    | "severity"
                    | "recommendation"
                    | "verdict"
                    | "decision"
            );
            if relevant {
                if let Some(value) = value.as_str() {
                    let value = normalize_catalog_status(value);
                    return catalog_status_is_dangerous(&value)
                        || matches!(value.as_str(), "high" | "critical" | "do_not_install");
                }
            }
            matches!(value, Value::Object(_) | Value::Array(_))
                && catalog_risk_summary_is_dangerous(value)
        }),
        Value::Array(values) => values.iter().any(catalog_risk_summary_is_dangerous),
        _ => false,
    }
}

fn validate_catalog_files(entries: Vec<ClawHubFile>) -> Result<ValidatedCatalogFiles, String> {
    if entries.is_empty() || entries.len() > MAX_SKILL_FILES {
        return Err(format!(
            "ClawHub Skill must contain between 1 and {MAX_SKILL_FILES} files"
        ));
    }
    let mut seen = HashSet::new();
    let mut total_bytes = 0_u64;
    let mut skill_file_index = None;
    for (index, file) in entries.iter().enumerate() {
        validate_catalog_file_path(&file.path)?;
        let key = file.path.to_lowercase();
        if !seen.insert(key) {
            return Err("ClawHub Skill contains duplicate file paths".to_string());
        }
        if !valid_sha256(&file.sha256) {
            return Err("ClawHub Skill file metadata has an invalid SHA-256 hash".to_string());
        }
        let file_limit = if file.path == "SKILL.md" {
            MAX_SKILL_MD_BYTES
        } else {
            MAX_SKILL_FILE_BYTES
        };
        if file.size > file_limit {
            return Err(format!(
                "ClawHub file {} exceeds the local Skill size limit",
                file.path
            ));
        }
        total_bytes = total_bytes
            .checked_add(file.size)
            .ok_or_else(|| "ClawHub Skill byte count overflow".to_string())?;
        if total_bytes > MAX_SKILL_TOTAL_BYTES {
            return Err("ClawHub Skill exceeds the local total size limit".to_string());
        }
        if file.path == "SKILL.md" {
            skill_file_index = Some(index);
        }
    }
    Ok(ValidatedCatalogFiles {
        entries,
        total_bytes,
        skill_file_index: skill_file_index
            .ok_or_else(|| "ClawHub Skill version does not contain SKILL.md".to_string())?,
    })
}

fn validate_catalog_file_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > 1024
        || path.starts_with('/')
        || path.contains('\\')
        || path.chars().any(char::is_control)
    {
        return Err("ClawHub Skill file path is invalid".to_string());
    }
    let components: Vec<_> = path.split('/').collect();
    if components.is_empty() || components.len() > MAX_SKILL_DEPTH {
        return Err("ClawHub Skill file path exceeds the local depth limit".to_string());
    }
    for component in components {
        if component.is_empty()
            || matches!(component, "." | "..")
            || component.ends_with(['.', ' '])
            || is_windows_reserved_file_name(component)
            || component
                .chars()
                .any(|character| matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        {
            return Err("ClawHub Skill file path is not safe for the local filesystem".to_string());
        }
    }
    Ok(())
}

fn is_windows_reserved_file_name(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn verify_catalog_file(file: &ClawHubFile, bytes: &[u8]) -> Result<(), String> {
    let expected_size = usize::try_from(file.size)
        .map_err(|_| "ClawHub file size cannot be represented locally".to_string())?;
    if bytes.len() != expected_size {
        return Err(format!(
            "ClawHub file {} did not match its advertised size",
            file.path
        ));
    }
    let actual = sha256_hex(bytes);
    if !actual.eq_ignore_ascii_case(&file.sha256) {
        return Err(format!(
            "ClawHub file {} did not match its advertised SHA-256 hash",
            file.path
        ));
    }
    Ok(())
}

fn parse_catalog_skill_name(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_SKILL_MD_BYTES as usize {
        return Err("ClawHub SKILL.md exceeds the local Skill size limit".to_string());
    }
    let content =
        std::str::from_utf8(bytes).map_err(|_| "ClawHub SKILL.md must be UTF-8".to_string())?;
    let (name, description) = parse_skill_frontmatter(content)?;
    let name = validate_skill_name(&name)?;
    let description = bounded_nonempty(
        &description,
        "Skill description",
        MAX_SKILL_DESCRIPTION_CHARS,
    )?;
    if description.contains(['<', '>']) {
        return Err("Skill description cannot contain angle brackets".to_string());
    }
    Ok(name)
}

fn write_catalog_file(root: &Path, file: &ClawHubFile, bytes: &[u8]) -> Result<(), String> {
    validate_catalog_file_path(&file.path)?;
    let mut target = root.to_path_buf();
    for component in file.path.split('/') {
        target.push(component);
    }
    let parent = target
        .parent()
        .ok_or_else(|| "ClawHub Skill file does not have a parent directory".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("create ClawHub Skill directory: {error}"))?;
    fs::write(&target, bytes).map_err(|error| format!("write ClawHub Skill file: {error}"))
}

fn clawhub_source_url(reference: &ClawHubReference) -> Result<String, String> {
    let owner = reference
        .owner_handle
        .as_deref()
        .ok_or_else(|| "ClawHub source URL requires an owner handle".to_string())?;
    let mut url = Url::parse(CLAWHUB_SITE_ORIGIN)
        .map_err(|_| "ClawHub source URL configuration is invalid".to_string())?;
    url.path_segments_mut()
        .map_err(|_| "ClawHub source URL configuration is invalid".to_string())?
        .extend([owner, "skills", reference.slug.as_str()]);
    Ok(url.to_string())
}

async fn clawhub_detail(reference: &ClawHubReference) -> Result<ClawHubDetailLookup, String> {
    let response = clawhub_get(
        &["skills", reference.slug.as_str()],
        &clawhub_owner_query(reference),
        MAX_CLAWHUB_JSON_BYTES,
    )
    .await?;
    if response.status == 409 {
        let ambiguous: ClawHubAmbiguousResponse = serde_json::from_slice(&response.body)
            .map_err(|_| "ClawHub returned an invalid ambiguous Skill response".to_string())?;
        if ambiguous.code.as_deref() != Some("AMBIGUOUS_SKILL_SLUG") {
            return Err("ClawHub could not uniquely resolve this Skill slug".to_string());
        }
        let slug = validate_catalog_slug(ambiguous.slug.as_deref().unwrap_or(&reference.slug))?;
        let mut matches = Vec::new();
        for candidate in ambiguous.matches {
            let (Some(owner), Some(candidate_slug)) = (candidate.owner_handle, candidate.slug)
            else {
                continue;
            };
            let Ok(owner_handle) = validate_catalog_owner_handle(&owner) else {
                continue;
            };
            let Ok(candidate_slug) = validate_catalog_slug(&candidate_slug) else {
                continue;
            };
            if candidate_slug != slug {
                continue;
            }
            let candidate_reference = ClawHubReference {
                owner_handle: Some(owner_handle.clone()),
                slug: candidate_slug.clone(),
            };
            matches.push(SkillsCatalogOwnerChoice {
                reference: catalog_reference_text(&owner_handle, &candidate_slug),
                owner_handle,
                slug: candidate_slug,
                source_url: clawhub_source_url(&candidate_reference)?,
            });
        }
        if matches.is_empty() {
            return Err(
                "ClawHub could not provide safe owner choices for this Skill slug".to_string(),
            );
        }
        return Ok(ClawHubDetailLookup::Ambiguous { slug, matches });
    }
    if !(200..300).contains(&response.status) {
        return Err(clawhub_http_error(response.status));
    }
    let detail = serde_json::from_slice(&response.body)
        .map_err(|_| "ClawHub returned invalid Skill detail JSON".to_string())?;
    Ok(ClawHubDetailLookup::Found(detail))
}

async fn clawhub_version(
    reference: &ClawHubReference,
    version: &str,
) -> Result<ClawHubVersionResponse, String> {
    clawhub_json(
        &["skills", reference.slug.as_str(), "versions", version],
        &clawhub_owner_query(reference),
    )
    .await
}

async fn clawhub_file(
    reference: &ClawHubReference,
    version: &str,
    file: &ClawHubFile,
) -> Result<Vec<u8>, String> {
    let mut query = clawhub_owner_query(reference);
    query.push(("path".to_string(), file.path.clone()));
    query.push(("version".to_string(), version.to_string()));
    let max_bytes = usize::try_from(file.size)
        .map_err(|_| "ClawHub file size cannot be represented locally".to_string())?;
    let response = clawhub_get(
        &["skills", reference.slug.as_str(), "file"],
        &query,
        max_bytes,
    )
    .await?;
    if !(200..300).contains(&response.status) {
        return Err(clawhub_http_error(response.status));
    }
    Ok(response.body)
}

fn clawhub_owner_query(reference: &ClawHubReference) -> Vec<(String, String)> {
    reference
        .owner_handle
        .as_ref()
        .map(|owner| vec![("ownerHandle".to_string(), owner.clone())])
        .unwrap_or_default()
}

async fn clawhub_json<T: DeserializeOwned>(
    path: &[&str],
    query: &[(String, String)],
) -> Result<T, String> {
    let response = clawhub_get(path, query, MAX_CLAWHUB_JSON_BYTES).await?;
    if !(200..300).contains(&response.status) {
        return Err(clawhub_http_error(response.status));
    }
    serde_json::from_slice(&response.body)
        .map_err(|_| "ClawHub returned invalid catalog JSON".to_string())
}

struct ClawHubHttpResponse {
    status: u16,
    body: Vec<u8>,
}

async fn clawhub_get(
    path: &[&str],
    query: &[(String, String)],
    max_bytes: usize,
) -> Result<ClawHubHttpResponse, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CLAWHUB_CONNECT_TIMEOUT)
        .timeout(CLAWHUB_REQUEST_TIMEOUT)
        .user_agent(CLAWHUB_USER_AGENT)
        .build()
        .map_err(|_| "create ClawHub catalog client".to_string())?;
    let url = clawhub_api_url(path, query)?;
    let mut response =
        client.get(url).send().await.map_err(|_| {
            "ClawHub catalog request failed; check the network connection".to_string()
        })?;
    let status = response.status();
    if status.as_u16() == 429 {
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|value| *value <= MAX_CLAWHUB_RETRY_AFTER_SECS);
        return Err(match retry_after {
            Some(seconds) => {
                format!("ClawHub catalog is rate limited; retry after {seconds} seconds")
            }
            None => "ClawHub catalog is rate limited; retry later".to_string(),
        });
    }
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err("ClawHub response exceeds the local safety limit".to_string());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "ClawHub catalog response could not be read".to_string())?
    {
        let remaining = max_bytes.saturating_sub(body.len());
        if chunk.len() > remaining {
            return Err("ClawHub response exceeds the local safety limit".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(ClawHubHttpResponse {
        status: status.as_u16(),
        body,
    })
}

fn clawhub_api_url(path: &[&str], query: &[(String, String)]) -> Result<Url, String> {
    let mut url = Url::parse(CLAWHUB_API_ORIGIN)
        .map_err(|_| "ClawHub catalog URL configuration is invalid".to_string())?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "ClawHub catalog URL configuration is invalid".to_string())?;
        segments.pop_if_empty();
        segments.extend(path.iter().copied());
    }
    {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
    }
    Ok(url)
}

fn clawhub_http_error(status: u16) -> String {
    match status {
        300..=399 => "ClawHub returned a redirect, which NovaVei does not follow".to_string(),
        403 => "ClawHub does not allow access to this Skill or version".to_string(),
        404 => "ClawHub Skill or version was not found".to_string(),
        410 => "ClawHub Skill version is no longer available".to_string(),
        _ => format!("ClawHub catalog request failed with HTTP {status}"),
    }
}

fn skill_summary(path: &Path, metadata: SkillMetadata, enabled: bool) -> SkillSummary {
    let built_in = BUILTIN_SKILLS
        .iter()
        .any(|(name, _)| *name == metadata.name);
    SkillSummary {
        name: metadata.name,
        description: metadata.description,
        enabled,
        built_in,
        root_dir: path_for_display(path),
        skill_file: path_for_display(&path.join("SKILL.md")),
        file_count: metadata.file_count,
        total_bytes: metadata.total_bytes,
    }
}

fn validate_skill_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    let valid = !name.is_empty()
        && name.chars().count() <= MAX_SKILL_NAME_CHARS
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !valid {
        return Err(
            "Skill name must use 1-64 lowercase letters, digits, or single hyphens".to_string(),
        );
    }
    Ok(name.to_string())
}

fn validate_skill_directory(path: &Path) -> Result<SkillMetadata, String> {
    ensure_plain_directory(path)?;
    let directory_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Skill directory name is not valid UTF-8".to_string())?;
    let skill_file = path.join("SKILL.md");
    let skill_metadata = fs::symlink_metadata(&skill_file)
        .map_err(|_| "Skill directory must contain SKILL.md".to_string())?;
    if !skill_metadata.is_file() || is_link_or_reparse(&skill_metadata) {
        return Err("SKILL.md must be a regular non-link file".to_string());
    }
    if skill_metadata.len() > MAX_SKILL_MD_BYTES {
        return Err(format!(
            "SKILL.md exceeds the {MAX_SKILL_MD_BYTES} byte limit"
        ));
    }
    let bytes = fs::read(&skill_file).map_err(|error| format!("read SKILL.md: {error}"))?;
    let content = std::str::from_utf8(&bytes).map_err(|_| "SKILL.md must be UTF-8".to_string())?;
    let (name, description) = parse_skill_frontmatter(content)?;
    let name = validate_skill_name(&name)?;
    if directory_name != name {
        return Err("Skill frontmatter name must exactly match its directory name".to_string());
    }
    let description = bounded_nonempty(
        &description,
        "Skill description",
        MAX_SKILL_DESCRIPTION_CHARS,
    )?;
    if description.contains(['<', '>']) {
        return Err("Skill description cannot contain angle brackets".to_string());
    }

    let mut file_count = 0_usize;
    let mut total_bytes = 0_u64;
    for entry in WalkDir::new(path).follow_links(false).min_depth(1) {
        let entry = entry.map_err(|error| format!("inspect Skill directory: {error}"))?;
        if entry.depth() > MAX_SKILL_DEPTH {
            return Err(format!(
                "Skill directory exceeds the depth limit of {MAX_SKILL_DEPTH}"
            ));
        }
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("inspect Skill entry: {error}"))?;
        if is_link_or_reparse(&metadata) {
            return Err("links and Windows reparse points are not allowed in Skills".to_string());
        }
        if metadata.is_dir() {
            continue;
        }
        if !metadata.is_file() {
            return Err("Skills may contain only regular files and directories".to_string());
        }
        file_count += 1;
        if file_count > MAX_SKILL_FILES {
            return Err(format!("Skill exceeds the {MAX_SKILL_FILES} file limit"));
        }
        if metadata.len() > MAX_SKILL_FILE_BYTES && entry.path() != skill_file {
            return Err(format!(
                "a Skill file exceeds the {MAX_SKILL_FILE_BYTES} byte limit"
            ));
        }
        total_bytes = total_bytes
            .checked_add(metadata.len())
            .ok_or_else(|| "Skill byte count overflow".to_string())?;
        if total_bytes > MAX_SKILL_TOTAL_BYTES {
            return Err(format!(
                "Skill exceeds the {MAX_SKILL_TOTAL_BYTES} total byte limit"
            ));
        }
    }
    Ok(SkillMetadata {
        name,
        description,
        file_count,
        total_bytes,
    })
}

fn ensure_plain_directory(path: &Path) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("inspect directory: {error}"))?;
    if !metadata.is_dir() || is_link_or_reparse(&metadata) {
        return Err("directory must be a regular non-link directory".to_string());
    }
    Ok(())
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

fn parse_skill_frontmatter(content: &str) -> Result<(String, String), String> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let lines = content.lines().collect::<Vec<_>>();
    if lines.first().map(|line| line.trim()) != Some("---") {
        return Err("SKILL.md frontmatter must start with ---".to_string());
    }
    let end = lines
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, line)| (line.trim() == "---").then_some(index))
        .ok_or_else(|| "SKILL.md frontmatter is not closed".to_string())?;
    let frontmatter = &lines[1..end];
    let name = parse_frontmatter_field(frontmatter, "name")?
        .ok_or_else(|| "Skill frontmatter is missing name".to_string())?;
    let description = parse_frontmatter_field(frontmatter, "description")?
        .ok_or_else(|| "Skill frontmatter is missing description".to_string())?;
    Ok((name, description))
}

fn parse_frontmatter_field(lines: &[&str], field: &str) -> Result<Option<String>, String> {
    let prefix = format!("{field}:");
    let mut result = None;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if line.starts_with(char::is_whitespace) || line.trim_start().starts_with('#') {
            index += 1;
            continue;
        }
        let Some(raw) = line.strip_prefix(&prefix) else {
            index += 1;
            continue;
        };
        if result.is_some() {
            return Err(format!("Skill frontmatter contains duplicate {field}"));
        }
        let raw = raw.trim();
        if raw == "|" || raw == ">" || raw == "|-" || raw == ">-" {
            let folded = raw.starts_with('>');
            index += 1;
            let mut block = Vec::new();
            while index < lines.len() {
                let block_line = lines[index];
                if !block_line.is_empty() && !block_line.starts_with(char::is_whitespace) {
                    break;
                }
                block.push(block_line.trim_start());
                index += 1;
            }
            result = Some(if folded {
                block.join(" ")
            } else {
                block.join("\n")
            });
            continue;
        }
        result = Some(parse_yaml_scalar(raw)?);
        index += 1;
    }
    Ok(result)
}

fn parse_yaml_scalar(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.starts_with('"') {
        return serde_json::from_str::<String>(raw)
            .map_err(|_| "Skill frontmatter contains an invalid quoted scalar".to_string());
    }
    if raw.starts_with('\'') {
        if raw.len() < 2 || !raw.ends_with('\'') {
            return Err("Skill frontmatter contains an invalid quoted scalar".to_string());
        }
        return Ok(raw[1..raw.len() - 1].replace("''", "'"));
    }
    Ok(raw.to_string())
}

fn copy_skill_tree(source: &Path, target: &Path) -> Result<(), String> {
    for entry in WalkDir::new(source).follow_links(false).min_depth(1) {
        let entry = entry.map_err(|error| format!("inspect Skill source: {error}"))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("inspect Skill source entry: {error}"))?;
        if is_link_or_reparse(&metadata) {
            return Err("links and Windows reparse points are not allowed in Skills".to_string());
        }
        let relative = entry
            .path()
            .strip_prefix(source)
            .map_err(|_| "Skill source escaped its selected directory".to_string())?;
        let destination = target.join(relative);
        if metadata.is_dir() {
            fs::create_dir(&destination)
                .map_err(|error| format!("create staged Skill directory: {error}"))?;
        } else if metadata.is_file() {
            fs::copy(entry.path(), &destination)
                .map_err(|error| format!("copy staged Skill file: {error}"))?;
        } else {
            return Err("Skills may contain only regular files and directories".to_string());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Memory

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub id: String,
    pub scope: String,
    pub workdir: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub content: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCreateInput {
    pub scope: String,
    pub workdir: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryUpdateInput {
    pub id: String,
    pub workdir: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub title: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFilter {
    pub scope: Option<String>,
    pub workdir: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryListResponse {
    pub items: Vec<MemoryEntry>,
    pub total: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchResponse {
    pub items: Vec<MemoryEntry>,
    pub backend: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStatBucket {
    pub key: String,
    pub entries: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStats {
    pub total_entries: usize,
    pub total_bytes: u64,
    pub by_scope: Vec<MemoryStatBucket>,
    pub by_type: Vec<MemoryStatBucket>,
    pub weekly_searches: usize,
    pub weekly_writes: usize,
    pub week_started_at: i64,
    pub tracking_started_at: i64,
    pub capacity: MemoryCapacity,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCapacity {
    pub used_entries: usize,
    pub max_entries: usize,
    pub remaining_entries: usize,
    pub used_percent: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryClearInput {
    pub scope: String,
    pub workdir: Option<String>,
    pub confirmation: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryClearResponse {
    pub scope: String,
    pub workdir: Option<String>,
    pub removed: usize,
    pub reclaimed_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryOrganizeInput {
    pub scope: Option<String>,
    pub workdir: Option<String>,
    #[serde(default = "default_true")]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDuplicateGroup {
    pub keeper_id: String,
    pub duplicate_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryOrganizeResponse {
    pub dry_run: bool,
    pub inspected: usize,
    pub duplicate_groups: usize,
    pub duplicate_entries: usize,
    pub removed: usize,
    pub reclaimed_bytes: u64,
    pub groups: Vec<MemoryDuplicateGroup>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryExportResponse {
    pub path: String,
    pub format: String,
    pub entries: usize,
    pub bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryUsageExportResponse {
    pub path: String,
    pub format: String,
    pub bytes: usize,
    pub week_started_at: i64,
}

fn default_true() -> bool {
    true
}

impl LocalServices {
    pub fn memory_create(&self, input: MemoryCreateInput) -> Result<MemoryEntry, String> {
        let (scope, workdir) = validate_memory_scope(&input.scope, input.workdir.as_deref())?;
        let kind = validate_memory_kind(&input.kind, &scope)?;
        let title = bounded_nonempty(&input.title, "memory title", MAX_MEMORY_TITLE_CHARS)?;
        let content = bounded_bytes(&input.content, "memory content", MAX_MEMORY_CONTENT_BYTES)?;
        let stored_content = protect_portable_local_service_text(&content)?;
        let id = Uuid::new_v4().to_string();
        let timestamp = now_ms();
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start memory write: {error}"))?;
        let count: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM memory_entries \
                 WHERE scope=?1 AND (scope='global' OR workdir=?2)",
                params![scope, workdir],
                |row| row.get(0),
            )
            .map_err(|error| format!("check memory quota: {error}"))?;
        if count >= MAX_MEMORY_ENTRIES_PER_SCOPE {
            return Err(format!(
                "memory scope has reached its {MAX_MEMORY_ENTRIES_PER_SCOPE} entry limit"
            ));
        }
        transaction
            .execute(
                "INSERT INTO memory_entries(id, scope, workdir, kind, title, content, created_at, updated_at) \
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![id, scope, workdir, kind, title, stored_content, timestamp],
            )
            .map_err(|error| format!("create memory entry: {error}"))?;
        self.fts_insert(&transaction, &id, &title, &content);
        record_memory_usage(
            &transaction,
            &scope,
            workdir.as_deref(),
            MemoryUsageOperation::Write,
            timestamp,
        )?;
        transaction
            .commit()
            .map_err(|error| format!("commit memory entry: {error}"))?;
        Ok(MemoryEntry {
            id,
            scope,
            workdir,
            kind,
            title,
            content,
            created_at: timestamp,
            updated_at: timestamp,
        })
    }

    pub fn memory_get(&self, id: &str, workdir: Option<&str>) -> Result<MemoryEntry, String> {
        let id = validate_record_id(id, "memory id")?;
        let requested_workdir = canonical_optional_workdir(workdir)?;
        let entry = self
            .open()?
            .query_row(
                "SELECT id, scope, workdir, kind, title, content, created_at, updated_at \
                 FROM memory_entries WHERE id=?1",
                params![id],
                row_to_memory_entry,
            )
            .optional()
            .map_err(|error| format!("read memory entry: {error}"))?
            .ok_or_else(|| "memory entry was not found".to_string())?;
        require_memory_access(&entry, requested_workdir.as_deref())?;
        Ok(entry)
    }

    pub fn memory_update(&self, input: MemoryUpdateInput) -> Result<MemoryEntry, String> {
        let id = validate_record_id(&input.id, "memory id")?;
        if input.kind.is_none() && input.title.is_none() && input.content.is_none() {
            return Err("memory update has no changes".to_string());
        }
        let requested_workdir = canonical_optional_workdir(input.workdir.as_deref())?;
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start memory update: {error}"))?;
        let existing = transaction
            .query_row(
                "SELECT id, scope, workdir, kind, title, content, created_at, updated_at \
                 FROM memory_entries WHERE id=?1",
                params![id],
                row_to_memory_entry,
            )
            .optional()
            .map_err(|error| format!("read memory entry: {error}"))?
            .ok_or_else(|| "memory entry was not found".to_string())?;
        require_memory_access(&existing, requested_workdir.as_deref())?;

        let kind = input
            .kind
            .as_deref()
            .map(|kind| validate_memory_kind(kind, &existing.scope))
            .transpose()?
            .unwrap_or(existing.kind);
        let title = input
            .title
            .as_deref()
            .map(|title| bounded_nonempty(title, "memory title", MAX_MEMORY_TITLE_CHARS))
            .transpose()?
            .unwrap_or(existing.title);
        let content = input
            .content
            .as_deref()
            .map(|content| bounded_bytes(content, "memory content", MAX_MEMORY_CONTENT_BYTES))
            .transpose()?
            .unwrap_or(existing.content);
        let stored_content = protect_portable_local_service_text(&content)?;
        let timestamp = now_ms();
        transaction
            .execute(
                "UPDATE memory_entries SET kind=?2, title=?3, content=?4, updated_at=?5 WHERE id=?1",
                params![id, kind, title, stored_content, timestamp],
            )
            .map_err(|error| format!("update memory entry: {error}"))?;
        self.fts_replace(&transaction, &id, &title, &content);
        record_memory_usage(
            &transaction,
            &existing.scope,
            existing.workdir.as_deref(),
            MemoryUsageOperation::Write,
            timestamp,
        )?;
        transaction
            .commit()
            .map_err(|error| format!("commit memory update: {error}"))?;
        Ok(MemoryEntry {
            id,
            scope: existing.scope,
            workdir: existing.workdir,
            kind,
            title,
            content,
            created_at: existing.created_at,
            updated_at: timestamp,
        })
    }

    pub fn memory_delete(&self, id: &str, workdir: Option<&str>) -> Result<MemoryEntry, String> {
        let id = validate_record_id(id, "memory id")?;
        let requested_workdir = canonical_optional_workdir(workdir)?;
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start memory delete: {error}"))?;
        let entry = transaction
            .query_row(
                "SELECT id, scope, workdir, kind, title, content, created_at, updated_at \
                 FROM memory_entries WHERE id=?1",
                params![id],
                row_to_memory_entry,
            )
            .optional()
            .map_err(|error| format!("read memory entry: {error}"))?
            .ok_or_else(|| "memory entry was not found".to_string())?;
        require_memory_access(&entry, requested_workdir.as_deref())?;
        transaction
            .execute("DELETE FROM memory_entries WHERE id=?1", params![id])
            .map_err(|error| format!("delete memory entry: {error}"))?;
        self.fts_delete(&transaction, &id);
        transaction
            .commit()
            .map_err(|error| format!("commit memory delete: {error}"))?;
        Ok(entry)
    }

    pub fn memory_list(
        &self,
        filter: MemoryFilter,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<MemoryListResponse, String> {
        let (scope, workdir) = validate_memory_filter(filter)?;
        let limit = limit.unwrap_or(50).clamp(1, MAX_MEMORY_LIST_LIMIT);
        let offset = offset.unwrap_or(0);
        if offset > MAX_MEMORY_OFFSET {
            return Err(format!("memory offset exceeds {MAX_MEMORY_OFFSET}"));
        }
        let connection = self.open()?;
        let total: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM memory_entries \
                 WHERE (?1 IS NULL OR scope=?1) \
                 AND (scope='global' OR workdir=?2)",
                params![scope, workdir],
                |row| row.get(0),
            )
            .map_err(|error| format!("count memory entries: {error}"))?;
        let mut statement = connection
            .prepare(
                "SELECT id, scope, workdir, kind, title, content, created_at, updated_at \
                 FROM memory_entries \
                 WHERE (?1 IS NULL OR scope=?1) \
                 AND (scope='global' OR workdir=?2) \
                 ORDER BY updated_at DESC, id ASC LIMIT ?3 OFFSET ?4",
            )
            .map_err(|error| format!("list memory entries: {error}"))?;
        let rows = statement
            .query_map(
                params![scope, workdir, limit as i64, offset as i64],
                row_to_memory_entry,
            )
            .map_err(|error| format!("list memory entries: {error}"))?;
        let items = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("list memory entries: {error}"))?;
        let total = usize::try_from(total).unwrap_or(usize::MAX);
        Ok(MemoryListResponse {
            truncated: offset.saturating_add(items.len()) < total,
            items,
            total,
        })
    }

    pub fn memory_search(
        &self,
        query: &str,
        filter: MemoryFilter,
        limit: Option<usize>,
    ) -> Result<MemorySearchResponse, String> {
        let query = bounded_nonempty(query, "memory search query", MAX_MEMORY_QUERY_CHARS)?;
        let (scope, workdir) = validate_memory_filter(filter)?;
        let limit = limit.unwrap_or(20).clamp(1, MAX_MEMORY_SEARCH_LIMIT);
        let response = if self.fts_available.load(Ordering::Acquire) {
            match self.memory_search_fts(&query, scope.as_deref(), workdir.as_deref(), limit) {
                Ok(response) => response,
                Err(()) => {
                    self.fts_available.store(false, Ordering::Release);
                    self.memory_search_like(&query, scope.as_deref(), workdir.as_deref(), limit)?
                }
            }
        } else {
            self.memory_search_like(&query, scope.as_deref(), workdir.as_deref(), limit)?
        };
        let usage_scope =
            scope
                .as_deref()
                .unwrap_or(if workdir.is_some() { "all" } else { "global" });
        let connection = self.open()?;
        record_memory_usage(
            &connection,
            usage_scope,
            workdir.as_deref(),
            MemoryUsageOperation::Search,
            now_ms(),
        )?;
        Ok(response)
    }

    fn memory_search_fts(
        &self,
        query: &str,
        scope: Option<&str>,
        workdir: Option<&str>,
        limit: usize,
    ) -> Result<MemorySearchResponse, ()> {
        let connection = self.open().map_err(|_| ())?;
        let fts_query = query
            .split_whitespace()
            .filter(|part| !part.is_empty())
            .map(|part| format!("\"{}\"", part.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ");
        let fts_query = if fts_query.is_empty() {
            format!("\"{}\"", query.replace('"', "\"\""))
        } else {
            fts_query
        };
        let mut statement = connection
            .prepare(
                "SELECT e.id, e.scope, e.workdir, e.kind, e.title, e.content, e.created_at, e.updated_at \
                 FROM memory_fts f JOIN memory_entries e ON e.id=f.id \
                 WHERE memory_fts MATCH ?1 \
                 AND (?2 IS NULL OR e.scope=?2) \
                 AND (e.scope='global' OR e.workdir=?3) \
                 ORDER BY bm25(memory_fts), e.updated_at DESC LIMIT ?4",
            )
            .map_err(|_| ())?;
        let rows = statement
            .query_map(
                params![fts_query, scope, workdir, (limit + 1) as i64],
                row_to_memory_entry,
            )
            .map_err(|_| ())?;
        let mut items = rows.collect::<Result<Vec<_>, _>>().map_err(|_| ())?;
        let truncated = items.len() > limit;
        items.truncate(limit);
        Ok(MemorySearchResponse {
            items,
            backend: "fts5".to_string(),
            truncated,
        })
    }

    fn memory_search_like(
        &self,
        query: &str,
        scope: Option<&str>,
        workdir: Option<&str>,
        limit: usize,
    ) -> Result<MemorySearchResponse, String> {
        if crate::storage::is_portable() {
            return self.memory_search_portable_decrypted(query, scope, workdir, limit);
        }
        let pattern = format!("%{}%", escape_like(query));
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT id, scope, workdir, kind, title, content, created_at, updated_at \
                 FROM memory_entries \
                 WHERE (title LIKE ?1 ESCAPE '\\' OR content LIKE ?1 ESCAPE '\\') \
                 AND (?2 IS NULL OR scope=?2) \
                 AND (scope='global' OR workdir=?3) \
                 ORDER BY CASE WHEN title LIKE ?1 ESCAPE '\\' THEN 0 ELSE 1 END, updated_at DESC \
                 LIMIT ?4",
            )
            .map_err(|error| format!("prepare memory search: {error}"))?;
        let rows = statement
            .query_map(
                params![pattern, scope, workdir, (limit + 1) as i64],
                row_to_memory_entry,
            )
            .map_err(|error| format!("search memory entries: {error}"))?;
        let mut items = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("search memory entries: {error}"))?;
        let truncated = items.len() > limit;
        items.truncate(limit);
        Ok(MemorySearchResponse {
            items,
            backend: "like".to_string(),
            truncated,
        })
    }

    fn memory_search_portable_decrypted(
        &self,
        query: &str,
        scope: Option<&str>,
        workdir: Option<&str>,
        limit: usize,
    ) -> Result<MemorySearchResponse, String> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT id, scope, workdir, kind, title, content, created_at, updated_at \
                 FROM memory_entries WHERE (?1 IS NULL OR scope=?1) \
                 AND (scope='global' OR workdir=?2) ORDER BY updated_at DESC, id ASC LIMIT ?3",
            )
            .map_err(|error| format!("prepare portable memory search: {error}"))?;
        let mut rows = statement
            .query(params![
                scope,
                workdir,
                (MAX_PORTABLE_DECRYPT_SEARCH_ROWS + 1) as i64
            ])
            .map_err(|error| format!("search portable memory entries: {error}"))?;
        let needle = query.to_lowercase();
        let item_cap = limit.saturating_add(1);
        let mut items = Vec::with_capacity(item_cap);
        let mut row_count = 0usize;
        while let Some(row) = rows
            .next()
            .map_err(|error| format!("search portable memory entries: {error}"))?
        {
            row_count = row_count.saturating_add(1);
            if row_count > MAX_PORTABLE_DECRYPT_SEARCH_ROWS {
                return Err(
                    "portable memory search has too many records; narrow the scope before searching"
                        .to_string(),
                );
            }
            // The SQL ordering already gives the first `limit + 1` matches.
            // Continue stepping afterwards to retain the fail-closed row cap,
            // but avoid decrypting or retaining records that cannot affect the
            // response.
            if items.len() >= item_cap {
                continue;
            }
            let entry = row_to_memory_entry(row)
                .map_err(|error| format!("search portable memory entries: {error}"))?;
            if local_service_text_contains_case_insensitive(&entry.title, &needle)
                || local_service_text_contains_case_insensitive(&entry.content, &needle)
            {
                items.push(entry);
            }
        }
        let truncated = items.len() > limit;
        items.truncate(limit);
        Ok(MemorySearchResponse {
            items,
            backend: "portable-decrypted".to_string(),
            truncated,
        })
    }

    pub fn memory_stats(&self, filter: MemoryFilter) -> Result<MemoryStats, String> {
        let (scope, workdir) = validate_memory_filter(filter)?;
        let connection = self.open()?;
        let (total_entries, total_bytes): (i64, i64) = connection
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(length(CAST(title AS BLOB)) + length(CAST(content AS BLOB))), 0) \
                 FROM memory_entries WHERE (?1 IS NULL OR scope=?1) AND (scope='global' OR workdir=?2)",
                params![scope, workdir],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| format!("read memory statistics: {error}"))?;
        let by_scope =
            memory_stat_buckets(&connection, "scope", scope.as_deref(), workdir.as_deref())?;
        let by_type =
            memory_stat_buckets(&connection, "kind", scope.as_deref(), workdir.as_deref())?;
        let week_started_at = memory_week_start_ms(now_ms())?;
        let (weekly_searches, weekly_writes): (i64, i64) = connection
            .query_row(
                "SELECT COALESCE(SUM(searches), 0), COALESCE(SUM(writes), 0) \
                 FROM memory_usage_weekly WHERE week_started_at=?3 AND ( \
                   (?1='global' AND filter_scope='global') OR \
                   (?1='project' AND filter_scope='project' AND workdir=?2) OR \
                   (?1 IS NULL AND (filter_scope='global' OR \
                     (?2 IS NOT NULL AND workdir=?2 AND filter_scope IN ('project', 'all')))) \
                 )",
                params![scope, workdir, week_started_at],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| format!("read memory weekly usage: {error}"))?;
        let tracking_started_at = connection
            .query_row(
                "SELECT value FROM local_service_meta WHERE key='memory_usage_tracking_started_at'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("read memory usage tracking start: {error}"))?
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(week_started_at);
        let total_entries = usize::try_from(total_entries).unwrap_or(usize::MAX);
        let visible_scopes = if scope.is_none() && workdir.is_some() {
            2
        } else {
            1
        };
        let max_entries = usize::try_from(MAX_MEMORY_ENTRIES_PER_SCOPE)
            .unwrap_or(usize::MAX)
            .saturating_mul(visible_scopes);
        let used_percent = if max_entries == 0 {
            0.0
        } else {
            ((total_entries.min(max_entries) as f64 / max_entries as f64) * 10_000.0).round()
                / 100.0
        };
        Ok(MemoryStats {
            total_entries,
            total_bytes: u64::try_from(total_bytes).unwrap_or(u64::MAX),
            by_scope,
            by_type,
            weekly_searches: usize::try_from(weekly_searches).unwrap_or(usize::MAX),
            weekly_writes: usize::try_from(weekly_writes).unwrap_or(usize::MAX),
            week_started_at,
            tracking_started_at,
            capacity: MemoryCapacity {
                used_entries: total_entries,
                max_entries,
                remaining_entries: max_entries.saturating_sub(total_entries),
                used_percent,
            },
        })
    }

    pub fn memory_clear(&self, input: MemoryClearInput) -> Result<MemoryClearResponse, String> {
        let (scope, workdir) = validate_memory_scope(&input.scope, input.workdir.as_deref())?;
        let expected_confirmation = match scope.as_str() {
            "global" => "CLEAR_GLOBAL_MEMORY",
            "project" => "CLEAR_PROJECT_MEMORY",
            _ => unreachable!("validated memory scope"),
        };
        if input.confirmation != expected_confirmation {
            return Err("memory clear requires explicit scope confirmation".to_string());
        }
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start memory clear: {error}"))?;
        let (count, reclaimed_bytes): (i64, i64) = transaction
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(length(CAST(title AS BLOB)) + length(CAST(content AS BLOB))), 0) \
                 FROM memory_entries WHERE scope=?1 AND (scope='global' OR workdir=?2)",
                params![scope, workdir],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| format!("measure memory clear: {error}"))?;
        if self.fts_available.load(Ordering::Acquire) {
            let _ = transaction.execute(
                "DELETE FROM memory_fts WHERE id IN (SELECT id FROM memory_entries \
                 WHERE scope=?1 AND (scope='global' OR workdir=?2))",
                params![scope, workdir],
            );
        }
        transaction
            .execute(
                "DELETE FROM memory_entries WHERE scope=?1 AND (scope='global' OR workdir=?2)",
                params![scope, workdir],
            )
            .map_err(|error| format!("clear memory scope: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit memory clear: {error}"))?;
        Ok(MemoryClearResponse {
            scope,
            workdir,
            removed: usize::try_from(count).unwrap_or(usize::MAX),
            reclaimed_bytes: u64::try_from(reclaimed_bytes).unwrap_or(u64::MAX),
        })
    }

    pub fn memory_organize(
        &self,
        input: MemoryOrganizeInput,
    ) -> Result<MemoryOrganizeResponse, String> {
        let (scope, workdir) = validate_memory_filter(MemoryFilter {
            scope: input.scope,
            workdir: input.workdir,
        })?;
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start memory organization: {error}"))?;
        let entries = {
            let mut statement = transaction
                .prepare(
                    "SELECT id, scope, workdir, kind, title, content, created_at, updated_at \
                     FROM memory_entries WHERE (?1 IS NULL OR scope=?1) AND (scope='global' OR workdir=?2) \
                     ORDER BY created_at ASC, id ASC LIMIT ?3",
                )
                .map_err(|error| format!("scan memory entries: {error}"))?;
            let rows = statement
                .query_map(
                    params![scope, workdir, (MAX_MEMORY_ORGANIZE_ENTRIES + 1) as i64],
                    row_to_memory_entry,
                )
                .map_err(|error| format!("scan memory entries: {error}"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("scan memory entries: {error}"))?
        };
        if entries.len() > MAX_MEMORY_ORGANIZE_ENTRIES {
            return Err(format!(
                "memory organization exceeds the {MAX_MEMORY_ORGANIZE_ENTRIES} entry limit"
            ));
        }

        type MemoryDeduplicationKey = (String, Option<String>, String, String, String);
        let mut exact_groups: HashMap<MemoryDeduplicationKey, Vec<&MemoryEntry>> = HashMap::new();
        for entry in &entries {
            exact_groups
                .entry((
                    entry.scope.clone(),
                    entry.workdir.clone(),
                    entry.kind.clone(),
                    entry.title.trim().to_string(),
                    entry.content.trim().to_string(),
                ))
                .or_default()
                .push(entry);
        }
        let mut duplicate_groups = exact_groups
            .into_values()
            .filter(|group| group.len() > 1)
            .collect::<Vec<_>>();
        duplicate_groups.sort_by(|left, right| left[0].id.cmp(&right[0].id));
        let duplicate_group_count = duplicate_groups.len();
        let duplicate_entry_count = duplicate_groups
            .iter()
            .map(|group| group.len().saturating_sub(1))
            .sum::<usize>();
        let reclaimed_bytes = duplicate_groups
            .iter()
            .flat_map(|group| group.iter().skip(1))
            .map(|entry| (entry.title.len() + entry.content.len()) as u64)
            .sum::<u64>();

        let mut response_groups = Vec::new();
        let mut returned_ids = 0_usize;
        for group in duplicate_groups.iter().take(MAX_MEMORY_ORGANIZE_GROUPS) {
            if returned_ids >= MAX_MEMORY_ORGANIZE_IDS {
                break;
            }
            let remaining = MAX_MEMORY_ORGANIZE_IDS - returned_ids;
            let duplicate_ids = group
                .iter()
                .skip(1)
                .take(remaining)
                .map(|entry| entry.id.clone())
                .collect::<Vec<_>>();
            returned_ids += duplicate_ids.len();
            response_groups.push(MemoryDuplicateGroup {
                keeper_id: group[0].id.clone(),
                duplicate_ids,
            });
        }

        let mut removed = 0_usize;
        if !input.dry_run {
            for entry in duplicate_groups
                .iter()
                .flat_map(|group| group.iter().skip(1))
            {
                transaction
                    .execute("DELETE FROM memory_entries WHERE id=?1", params![entry.id])
                    .map_err(|error| format!("remove duplicate memory entry: {error}"))?;
                self.fts_delete(&transaction, &entry.id);
                removed += 1;
            }
        }
        transaction
            .commit()
            .map_err(|error| format!("commit memory organization: {error}"))?;
        Ok(MemoryOrganizeResponse {
            dry_run: input.dry_run,
            inspected: entries.len(),
            duplicate_groups: duplicate_group_count,
            duplicate_entries: duplicate_entry_count,
            removed,
            reclaimed_bytes: if input.dry_run || removed > 0 {
                reclaimed_bytes
            } else {
                0
            },
            truncated: response_groups.len() < duplicate_group_count
                || returned_ids < duplicate_entry_count,
            groups: response_groups,
        })
    }

    fn memory_export_data(
        &self,
        filter: MemoryFilter,
        format: &str,
    ) -> Result<(Vec<u8>, usize), String> {
        let (scope, workdir) = validate_memory_filter(filter)?;
        let connection = self.open()?;
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM memory_entries WHERE (?1 IS NULL OR scope=?1) AND (scope='global' OR workdir=?2)",
                params![scope, workdir],
                |row| row.get(0),
            )
            .map_err(|error| format!("count memory export: {error}"))?;
        if count > MAX_MEMORY_ORGANIZE_ENTRIES as i64 {
            return Err(format!(
                "memory export exceeds the {MAX_MEMORY_ORGANIZE_ENTRIES} entry limit"
            ));
        }
        let mut statement = connection
            .prepare(
                "SELECT id, scope, workdir, kind, title, content, created_at, updated_at \
                 FROM memory_entries WHERE (?1 IS NULL OR scope=?1) AND (scope='global' OR workdir=?2) \
                 ORDER BY scope, workdir, kind, created_at, id",
            )
            .map_err(|error| format!("prepare memory export: {error}"))?;
        let entries = statement
            .query_map(params![scope, workdir], row_to_memory_entry)
            .map_err(|error| format!("read memory export: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("read memory export: {error}"))?;
        let bytes = match format {
            "json" => serde_json::to_vec_pretty(&entries)
                .map_err(|_| "serialize memory export".to_string())?,
            "markdown" => render_memory_markdown(&entries).into_bytes(),
            _ => return Err("memory export format must be json or markdown".to_string()),
        };
        if bytes.len() > MAX_MEMORY_EXPORT_BYTES {
            return Err(format!(
                "memory export exceeds the {MAX_MEMORY_EXPORT_BYTES} byte limit"
            ));
        }
        Ok((bytes, entries.len()))
    }

    fn memory_usage_report_data(
        &self,
        filter: MemoryFilter,
    ) -> Result<(Vec<u8>, MemoryStats), String> {
        let (scope, workdir) = validate_memory_filter(filter.clone())?;
        let stats = self.memory_stats(filter)?;
        let scope_label = match (scope.as_deref(), workdir.as_deref()) {
            (Some("global"), _) | (None, None) => "global".to_string(),
            (Some("project"), Some(workdir)) => format!("project: {workdir}"),
            (None, Some(workdir)) => format!("global + project: {workdir}"),
            _ => return Err("memory usage report scope is invalid".to_string()),
        };
        let bytes = render_memory_usage_markdown(&stats, &scope_label).into_bytes();
        if bytes.len() > MAX_MEMORY_EXPORT_BYTES {
            return Err(format!(
                "memory usage report exceeds the {MAX_MEMORY_EXPORT_BYTES} byte limit"
            ));
        }
        Ok((bytes, stats))
    }

    fn fts_insert(&self, transaction: &Transaction<'_>, id: &str, title: &str, content: &str) {
        if self.fts_available.load(Ordering::Acquire)
            && transaction
                .execute(
                    "INSERT INTO memory_fts(id, title, content) VALUES(?1, ?2, ?3)",
                    params![id, title, content],
                )
                .is_err()
        {
            self.fts_available.store(false, Ordering::Release);
        }
    }

    fn fts_delete(&self, transaction: &Transaction<'_>, id: &str) {
        if self.fts_available.load(Ordering::Acquire)
            && transaction
                .execute("DELETE FROM memory_fts WHERE id=?1", params![id])
                .is_err()
        {
            self.fts_available.store(false, Ordering::Release);
        }
    }

    fn fts_replace(&self, transaction: &Transaction<'_>, id: &str, title: &str, content: &str) {
        self.fts_delete(transaction, id);
        self.fts_insert(transaction, id, title, content);
    }
}

fn row_to_memory_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryEntry> {
    let stored_content: String = row.get(5)?;
    let content = unprotect_portable_local_service_text(&stored_content).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
        )
    })?;
    Ok(MemoryEntry {
        id: row.get(0)?,
        scope: row.get(1)?,
        workdir: row.get(2)?,
        kind: row.get(3)?,
        title: row.get(4)?,
        content,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

#[derive(Clone, Copy)]
enum MemoryUsageOperation {
    Search,
    Write,
}

fn memory_week_start_ms(reference_ms: i64) -> Result<i64, String> {
    let reference = match Local.timestamp_millis_opt(reference_ms) {
        LocalResult::Single(value) => value,
        LocalResult::Ambiguous(first, second) => first.min(second),
        LocalResult::None => return Err("memory usage reference time is invalid".to_string()),
    };
    let monday = reference
        .date_naive()
        .checked_sub_days(Days::new(reference.weekday().num_days_from_monday().into()))
        .ok_or_else(|| "memory usage week is outside the supported range".to_string())?;
    let midnight = monday
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| "memory usage week start is invalid".to_string())?;
    match Local.from_local_datetime(&midnight) {
        LocalResult::Single(value) => Ok(value.timestamp_millis()),
        LocalResult::Ambiguous(first, second) => Ok(first.min(second).timestamp_millis()),
        LocalResult::None => Err("memory usage week start is unavailable locally".to_string()),
    }
}

fn record_memory_usage(
    connection: &Connection,
    filter_scope: &str,
    workdir: Option<&str>,
    operation: MemoryUsageOperation,
    occurred_at: i64,
) -> Result<(), String> {
    let workdir = match filter_scope {
        "global" if workdir.is_none() => "",
        "project" | "all" => workdir
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "scoped memory usage requires workdir".to_string())?,
        "global" => return Err("global memory usage cannot have workdir".to_string()),
        _ => return Err("memory usage scope must be global, project, or all".to_string()),
    };
    let week_started_at = memory_week_start_ms(occurred_at)?;
    let (searches, writes) = match operation {
        MemoryUsageOperation::Search => (1_i64, 0_i64),
        MemoryUsageOperation::Write => (0_i64, 1_i64),
    };
    connection
        .execute(
            "INSERT INTO memory_usage_weekly(filter_scope, workdir, week_started_at, searches, writes) \
             VALUES(?1, ?2, ?3, ?4, ?5) ON CONFLICT(filter_scope, workdir, week_started_at) \
             DO UPDATE SET searches=searches + excluded.searches, writes=writes + excluded.writes",
            params![filter_scope, workdir, week_started_at, searches, writes],
        )
        .map_err(|error| format!("record memory usage: {error}"))?;
    Ok(())
}

fn validate_memory_scope(
    scope: &str,
    workdir: Option<&str>,
) -> Result<(String, Option<String>), String> {
    match scope.trim() {
        "global" => {
            if workdir.is_some_and(|value| !value.trim().is_empty()) {
                return Err("global memory cannot have a workdir".to_string());
            }
            Ok(("global".to_string(), None))
        }
        "project" => Ok((
            "project".to_string(),
            Some(canonical_workdir(workdir.ok_or_else(|| {
                "project memory requires workdir".to_string()
            })?)?),
        )),
        _ => Err("memory scope must be global or project".to_string()),
    }
}

fn validate_memory_kind(kind: &str, scope: &str) -> Result<String, String> {
    let kind = kind.trim();
    if !matches!(
        kind,
        "user" | "feedback" | "project" | "reference" | "daily"
    ) {
        return Err("memory type must be user, feedback, project, reference, or daily".to_string());
    }
    if kind == "daily" && scope != "global" {
        return Err("daily memory must use global scope".to_string());
    }
    Ok(kind.to_string())
}

fn canonical_workdir(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.chars().count() > 2_048 {
        return Err("workdir is missing or too long".to_string());
    }
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err("workdir must be absolute".to_string());
    }
    let canonical = fs::canonicalize(path).map_err(|_| "workdir is not accessible".to_string())?;
    ensure_plain_directory(&canonical)
        .map_err(|_| "workdir must be a regular directory".to_string())?;
    Ok(path_for_display(&canonical))
}

fn canonical_optional_workdir(workdir: Option<&str>) -> Result<Option<String>, String> {
    workdir
        .filter(|value| !value.trim().is_empty())
        .map(canonical_workdir)
        .transpose()
}

fn validate_memory_filter(
    filter: MemoryFilter,
) -> Result<(Option<String>, Option<String>), String> {
    let scope = filter
        .scope
        .as_deref()
        .map(|scope| match scope.trim() {
            "global" => Ok("global".to_string()),
            "project" => Ok("project".to_string()),
            _ => Err("memory scope must be global or project".to_string()),
        })
        .transpose()?;
    let workdir = canonical_optional_workdir(filter.workdir.as_deref())?;
    if scope.as_deref() == Some("project") && workdir.is_none() {
        return Err("project memory filter requires workdir".to_string());
    }
    Ok((scope, workdir))
}

fn require_memory_access(entry: &MemoryEntry, workdir: Option<&str>) -> Result<(), String> {
    if entry.scope == "global" || entry.workdir.as_deref() == workdir {
        Ok(())
    } else {
        Err("project memory does not belong to this workdir".to_string())
    }
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Match portable-search text without cloning every ASCII document. The
/// Unicode fallback deliberately matches the previous `to_lowercase()`
/// behavior, including Unicode characters whose lowercase representation is
/// not ASCII.
fn local_service_text_contains_case_insensitive(haystack: &str, lowered_needle: &str) -> bool {
    if haystack.is_ascii() && lowered_needle.is_ascii() {
        return haystack
            .as_bytes()
            .windows(lowered_needle.len())
            .any(|candidate| candidate.eq_ignore_ascii_case(lowered_needle.as_bytes()));
    }
    haystack.to_lowercase().contains(lowered_needle)
}

fn memory_stat_buckets(
    connection: &Connection,
    column: &str,
    scope: Option<&str>,
    workdir: Option<&str>,
) -> Result<Vec<MemoryStatBucket>, String> {
    let column = match column {
        "scope" => "scope",
        "kind" => "kind",
        _ => return Err("invalid memory statistic grouping".to_string()),
    };
    let sql = format!(
        "SELECT {column}, COUNT(*), COALESCE(SUM(length(CAST(title AS BLOB)) + length(CAST(content AS BLOB))), 0) \
         FROM memory_entries WHERE (?1 IS NULL OR scope=?1) AND (scope='global' OR workdir=?2) \
         GROUP BY {column} ORDER BY {column}"
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("prepare memory statistics: {error}"))?;
    let rows = statement
        .query_map(params![scope, workdir], |row| {
            Ok(MemoryStatBucket {
                key: row.get(0)?,
                entries: usize::try_from(row.get::<_, i64>(1)?).unwrap_or(usize::MAX),
                bytes: u64::try_from(row.get::<_, i64>(2)?).unwrap_or(u64::MAX),
            })
        })
        .map_err(|error| format!("read memory statistics: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read memory statistics: {error}"))
}

fn render_memory_markdown(entries: &[MemoryEntry]) -> String {
    let mut output = String::from("# NovaVei Memory Export\n\n");
    for entry in entries {
        output.push_str("## ");
        output.push_str(&entry.title.replace('\n', " "));
        output.push_str("\n\n");
        output.push_str("- ID: `");
        output.push_str(&entry.id);
        output.push_str("`\n- Scope: `");
        output.push_str(&entry.scope);
        output.push_str("`\n- Type: `");
        output.push_str(&entry.kind);
        output.push_str("`\n");
        if let Some(workdir) = &entry.workdir {
            output.push_str("- Workdir: `");
            output.push_str(&workdir.replace('`', "\\`"));
            output.push_str("`\n");
        }
        output.push('\n');
        output.push_str(&entry.content);
        output.push_str("\n\n");
    }
    output
}

fn timestamp_label(value: i64) -> String {
    match Utc.timestamp_millis_opt(value) {
        LocalResult::Single(value) => value.to_rfc3339(),
        LocalResult::Ambiguous(first, second) => first.min(second).to_rfc3339(),
        LocalResult::None => value.to_string(),
    }
}

fn render_memory_usage_markdown(stats: &MemoryStats, scope_label: &str) -> String {
    let scope_label = scope_label.replace(['\r', '\n'], " ");
    format!(
        "# NovaVei Memory Usage Report\n\n\
- Generated at: `{}`\n\
- Scope: `{}`\n\
- Tracking started at: `{}`\n\
- Current local week started at: `{}`\n\n\
## Storage and capacity\n\n\
- Entries: {}\n\
- Stored title/content bytes: {}\n\
- Entry capacity: {} / {} ({:.2}%)\n\
- Remaining entry capacity: {}\n\n\
## Current week activity\n\n\
- Successful searches: {}\n\
- Successful writes: {}\n\n\
## Definitions\n\n\
- Searches count successful native `memory_search` calls from the tracking start time.\n\
- Writes count successful native memory create and update commits from the tracking start time.\n\
- Stored bytes count UTF-8 bytes in entry titles and content; SQLite indexes and metadata are excluded.\n\
- Capacity uses the enforced {}-entry limit for each visible memory scope.\n",
        timestamp_label(now_ms()),
        scope_label.replace('`', "\\`"),
        timestamp_label(stats.tracking_started_at),
        timestamp_label(stats.week_started_at),
        stats.total_entries,
        stats.total_bytes,
        stats.capacity.used_entries,
        stats.capacity.max_entries,
        stats.capacity.used_percent,
        stats.capacity.remaining_entries,
        stats.weekly_searches,
        stats.weekly_writes,
        MAX_MEMORY_ENTRIES_PER_SCOPE,
    )
}

// ---------------------------------------------------------------------------
// Knowledge base

/// A user-approved source directory.  `canonical_path` is intentionally
/// returned only to the settings surface; Agent search results use the folder
/// label and a relative document path so the model never learns unrelated
/// absolute host paths.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeBaseFolder {
    pub id: String,
    pub canonical_path: String,
    pub display_name: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_indexed_at: Option<i64>,
    pub document_count: usize,
    pub indexed_bytes: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeBaseConsent {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeBaseListResponse {
    pub enabled: bool,
    pub consent: Option<KnowledgeBaseConsent>,
    pub folders: Vec<KnowledgeBaseFolder>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeBaseIndexResult {
    pub folder: KnowledgeBaseFolder,
    pub indexed_files: usize,
    pub skipped_files: usize,
    pub indexed_bytes: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeBaseSearchItem {
    pub document_id: String,
    pub folder_id: String,
    pub folder_name: String,
    pub title: String,
    pub relative_path: String,
    pub snippet: String,
    pub score: f64,
    pub modified_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeBaseSearchResponse {
    pub items: Vec<KnowledgeBaseSearchItem>,
    pub backend: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeBaseDocument {
    pub document_id: String,
    pub folder_id: String,
    pub title: String,
    pub relative_path: String,
    pub content: String,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
struct ScannedKnowledgeDocument {
    relative_path: String,
    title: String,
    extension: String,
    size_bytes: u64,
    modified_at: Option<i64>,
    content_hash: String,
    content: String,
}

/// A short-lived, per-turn native budget. The opaque token never appears in a
/// model-visible schema; it is captured only by the renderer's tool closures.
struct KnowledgeAgentAccess {
    created_at: Instant,
    document_ids: HashSet<String>,
    searches: usize,
    reads: usize,
    read_chars: usize,
}

impl LocalServices {
    /// Add a folder only after the native picker selected it.  The method is
    /// kept path-based for the picker and focused tests, but no Tauri command
    /// accepts a renderer-provided path.
    pub fn knowledge_base_add_folder(
        &self,
        source: &Path,
    ) -> Result<KnowledgeBaseIndexResult, String> {
        self.ensure_ready()?;
        let (canonical, canonical_display) = canonical_knowledge_base_root(source)?;
        let access = self.knowledge_access_lock.write();
        let (id, changed_sources) = {
            let mut connection = self.open()?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| format!("start knowledge base add: {error}"))?;
            if let Some(id) = transaction
                .query_row(
                    "SELECT id FROM knowledge_base_folders WHERE canonical_path=?1",
                    params![canonical_display],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| format!("find knowledge base folder: {error}"))?
            {
                transaction
                    .commit()
                    .map_err(|error| format!("commit existing knowledge base folder: {error}"))?;
                (id, false)
            } else {
                let folder_count: i64 = transaction
                    .query_row("SELECT COUNT(*) FROM knowledge_base_folders", [], |row| {
                        row.get(0)
                    })
                    .map_err(|error| format!("count knowledge base folders: {error}"))?;
                if folder_count >= MAX_KNOWLEDGE_BASE_FOLDERS as i64 {
                    return Err(format!(
                        "knowledge base supports at most {MAX_KNOWLEDGE_BASE_FOLDERS} folders"
                    ));
                }
                let timestamp = now_ms();
                let id = Uuid::new_v4().to_string();
                let display_name = canonical
                    .file_name()
                    .filter(|name| !name.is_empty())
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| canonical_display.clone());
                transaction
                    .execute(
                        "INSERT INTO knowledge_base_folders(\
                         id, canonical_path, display_name, created_at, updated_at, last_indexed_at, document_count, indexed_bytes\
                         ) VALUES(?1, ?2, ?3, ?4, ?4, NULL, 0, 0)",
                        params![id, canonical_display, display_name, timestamp],
                    )
                    .map_err(|error| format!("add knowledge base folder: {error}"))?;
                // Consent is specific to the exact source set. A newly added
                // root can never inherit approval granted for older roots.
                revoke_knowledge_base_consent(&transaction)?;
                transaction
                    .commit()
                    .map_err(|error| format!("commit knowledge base add: {error}"))?;
                (id, true)
            }
        };
        if changed_sources {
            self.knowledge_agent_access.lock().clear();
        }
        drop(access);
        self.knowledge_base_refresh(&id)
    }

    pub fn knowledge_base_list(&self) -> Result<KnowledgeBaseListResponse, String> {
        let connection = self.open()?;
        let consent = knowledge_base_consent(&connection)?;
        // Older databases can contain the former boolean-only setting. Treat
        // it as disabled until the user explicitly approves a provider/model.
        let enabled = knowledge_base_enabled(&connection)? && consent.is_some();
        let mut statement = connection
            .prepare(
                "SELECT id, canonical_path, display_name, created_at, updated_at, last_indexed_at, document_count, indexed_bytes \
                 FROM knowledge_base_folders ORDER BY created_at ASC, id ASC",
            )
            .map_err(|error| format!("prepare knowledge base folders: {error}"))?;
        let folders = statement
            .query_map([], row_to_knowledge_base_folder)
            .map_err(|error| format!("list knowledge base folders: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("list knowledge base folders: {error}"))?;
        Ok(KnowledgeBaseListResponse {
            enabled,
            consent,
            folders,
        })
    }

    pub fn knowledge_base_set_enabled(
        &self,
        enabled: bool,
        consent: Option<KnowledgeBaseConsent>,
    ) -> Result<KnowledgeBaseListResponse, String> {
        let consent = if enabled {
            Some(validate_knowledge_base_consent(consent)?)
        } else {
            None
        };
        let _access = self.knowledge_access_lock.write();
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start knowledge base setting update: {error}"))?;
        if enabled {
            let folder_count: i64 = transaction
                .query_row("SELECT COUNT(*) FROM knowledge_base_folders", [], |row| {
                    row.get(0)
                })
                .map_err(|error| format!("count knowledge base folders: {error}"))?;
            if folder_count == 0 {
                return Err("add a knowledge base folder before enabling chat use".to_string());
            }
        }
        transaction
            .execute(
                "INSERT INTO local_service_meta(key, value) VALUES('knowledge_base_enabled', ?1) \
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![if enabled { "1" } else { "0" }],
            )
            .map_err(|error| format!("save knowledge base setting: {error}"))?;
        if let Some(consent) = consent {
            save_knowledge_base_consent(&transaction, &consent)?;
        } else {
            clear_knowledge_base_consent(&transaction)?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit knowledge base setting update: {error}"))?;
        self.knowledge_agent_access.lock().clear();
        self.knowledge_base_list()
    }

    pub fn knowledge_base_remove(&self, id: &str) -> Result<KnowledgeBaseFolder, String> {
        let id = validate_record_id(id, "knowledge base folder id")?;
        let _access = self.knowledge_access_lock.write();
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start knowledge base removal: {error}"))?;
        let folder = transaction
            .query_row(
                "SELECT id, canonical_path, display_name, created_at, updated_at, last_indexed_at, document_count, indexed_bytes \
                 FROM knowledge_base_folders WHERE id=?1",
                params![id],
                row_to_knowledge_base_folder,
            )
            .optional()
            .map_err(|error| format!("read knowledge base folder: {error}"))?
            .ok_or_else(|| "knowledge base folder was not found".to_string())?;
        if knowledge_base_fts_table_exists(&transaction)
            .map_err(|error| format!("inspect knowledge base search index: {error}"))?
        {
            transaction
                .execute(
                    "DELETE FROM knowledge_base_fts WHERE folder_id=?1",
                    params![id],
                )
                .map_err(|error| format!("remove knowledge base index: {error}"))?;
        }
        transaction
            .execute(
                "DELETE FROM knowledge_base_folders WHERE id=?1",
                params![id],
            )
            .map_err(|error| format!("remove knowledge base folder: {error}"))?;
        // Any removal changes the approved source set, so it always revokes
        // prior consent rather than silently widening/retaining its scope.
        revoke_knowledge_base_consent(&transaction)?;
        transaction
            .commit()
            .map_err(|error| format!("commit knowledge base removal: {error}"))?;
        self.knowledge_agent_access.lock().clear();
        Ok(folder)
    }

    /// Rebuild one approved root.  The stored root is canonicalized again at
    /// every refresh, and every discovered file must still resolve below it.
    pub fn knowledge_base_refresh(&self, id: &str) -> Result<KnowledgeBaseIndexResult, String> {
        self.ensure_ready()?;
        let id = validate_record_id(id, "knowledge base folder id")?;
        let _access = self.knowledge_access_lock.write();
        let (mut folder, root) = {
            let connection = self.open()?;
            let folder = connection
                .query_row(
                    "SELECT id, canonical_path, display_name, created_at, updated_at, last_indexed_at, document_count, indexed_bytes \
                     FROM knowledge_base_folders WHERE id=?1",
                    params![id],
                    row_to_knowledge_base_folder,
                )
                .optional()
                .map_err(|error| format!("read knowledge base folder: {error}"))?
                .ok_or_else(|| "knowledge base folder was not found".to_string())?;
            let (root, display) = canonical_knowledge_base_root(Path::new(&folder.canonical_path))?;
            if display != folder.canonical_path {
                return Err("knowledge base folder changed unexpectedly; add it again".to_string());
            }
            (folder, root)
        };
        let (documents, skipped_files, truncated) = scan_knowledge_base(&root)?;
        let indexed_files = documents.len();
        let indexed_bytes = documents.iter().fold(0_u64, |total, document| {
            total.saturating_add(document.size_bytes)
        });
        let timestamp = now_ms();
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start knowledge base refresh: {error}"))?;
        let fts_available = !crate::storage::is_portable()
            && knowledge_base_fts_table_exists(&transaction)
                .map_err(|error| format!("inspect knowledge base search index: {error}"))?;
        if fts_available {
            transaction
                .execute(
                    "DELETE FROM knowledge_base_fts WHERE folder_id=?1",
                    params![folder.id],
                )
                .map_err(|error| format!("clear knowledge base search index: {error}"))?;
        }
        transaction
            .execute(
                "DELETE FROM knowledge_base_documents WHERE folder_id=?1",
                params![folder.id],
            )
            .map_err(|error| format!("clear knowledge base documents: {error}"))?;
        // Preparing the two insert statements once avoids reparsing them for
        // every file in a large source tree. The documents and FTS rows still
        // commit as one atomic refresh below.
        let mut document_insert = transaction
            .prepare(
                "INSERT INTO knowledge_base_documents(\
                 id, folder_id, relative_path, title, extension, size_bytes, modified_at, content_hash, content, updated_at\
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )
            .map_err(|error| format!("prepare knowledge base document insert: {error}"))?;
        let mut fts_insert = if fts_available {
            Some(
                transaction
                    .prepare(
                        "INSERT INTO knowledge_base_fts(document_id, folder_id, title, relative_path, content) \
                         VALUES(?1, ?2, ?3, ?4, ?5)",
                    )
                    .map_err(|error| format!("prepare knowledge base index insert: {error}"))?,
            )
        } else {
            None
        };
        for document in &documents {
            let document_id = Uuid::new_v4().to_string();
            let stored_content = protect_portable_local_service_text(&document.content)?;
            document_insert
                .execute(params![
                    document_id,
                    folder.id,
                    document.relative_path,
                    document.title,
                    document.extension,
                    i64::try_from(document.size_bytes).unwrap_or(i64::MAX),
                    document.modified_at,
                    document.content_hash,
                    stored_content,
                    timestamp,
                ])
                .map_err(|error| format!("store knowledge base document: {error}"))?;
            if let Some(fts_insert) = fts_insert.as_mut() {
                fts_insert
                    .execute(params![
                        document_id,
                        folder.id,
                        document.title,
                        document.relative_path,
                        document.content,
                    ])
                    .map_err(|error| format!("index knowledge base document: {error}"))?;
            }
        }
        // Statements borrow the transaction, so release them before updating
        // metadata and committing the refresh.
        drop(fts_insert);
        drop(document_insert);
        transaction
            .execute(
                "UPDATE knowledge_base_folders SET updated_at=?2, last_indexed_at=?2, document_count=?3, indexed_bytes=?4 \
                 WHERE id=?1",
                params![
                    folder.id,
                    timestamp,
                    i64::try_from(indexed_files).unwrap_or(i64::MAX),
                    i64::try_from(indexed_bytes).unwrap_or(i64::MAX),
                ],
            )
            .map_err(|error| format!("update knowledge base folder: {error}"))?;
        folder.updated_at = timestamp;
        folder.last_indexed_at = Some(timestamp);
        folder.document_count = indexed_files;
        folder.indexed_bytes = indexed_bytes;
        transaction
            .commit()
            .map_err(|error| format!("commit knowledge base refresh: {error}"))?;
        self.knowledge_fts_available
            .store(fts_available, Ordering::Release);
        Ok(KnowledgeBaseIndexResult {
            folder,
            indexed_files,
            skipped_files,
            indexed_bytes,
            truncated,
        })
    }

    pub fn knowledge_base_search(
        &self,
        query: &str,
        folder_ids: Option<Vec<String>>,
        limit: Option<usize>,
    ) -> Result<KnowledgeBaseSearchResponse, String> {
        self.ensure_ready()?;
        let _access = self.knowledge_access_lock.read();
        self.knowledge_base_search_unchecked(query, folder_ids, limit)
    }

    /// Local settings search is deliberately independent of the model-access
    /// switch: users can inspect an index before consenting to external chat.
    fn knowledge_base_search_unchecked(
        &self,
        query: &str,
        folder_ids: Option<Vec<String>>,
        limit: Option<usize>,
    ) -> Result<KnowledgeBaseSearchResponse, String> {
        let query = bounded_nonempty(
            query,
            "knowledge base search query",
            MAX_KNOWLEDGE_BASE_QUERY_CHARS,
        )?;
        let allowed_folders = validate_knowledge_base_folder_ids(folder_ids)?;
        let limit = limit.unwrap_or(8).clamp(1, MAX_KNOWLEDGE_BASE_SEARCH_LIMIT);
        if self.knowledge_fts_available.load(Ordering::Acquire) {
            match self.knowledge_base_search_fts(&query, &allowed_folders, limit) {
                Ok(response) => return Ok(response),
                Err(()) => self.knowledge_fts_available.store(false, Ordering::Release),
            }
        }
        self.knowledge_base_search_like(&query, &allowed_folders, limit)
    }

    pub fn knowledge_base_read(
        &self,
        document_id: &str,
        max_chars: Option<usize>,
    ) -> Result<KnowledgeBaseDocument, String> {
        let _access = self.knowledge_access_lock.read();
        self.knowledge_base_read_unchecked(document_id, max_chars)
    }

    /// Return the bounded text snapshot that was safely indexed, rather than
    /// reopening a mutable source path during an Agent turn.
    fn knowledge_base_read_unchecked(
        &self,
        document_id: &str,
        max_chars: Option<usize>,
    ) -> Result<KnowledgeBaseDocument, String> {
        self.ensure_ready()?;
        let document_id = validate_record_id(document_id, "knowledge base document id")?;
        let max_chars = max_chars
            .unwrap_or(MAX_KNOWLEDGE_BASE_READ_CHARS)
            .clamp(1, MAX_KNOWLEDGE_BASE_READ_CHARS);
        let connection = self.open()?;
        let (folder_id, title, relative_path, content): (String, String, String, String) =
            if crate::storage::is_portable() {
                connection.query_row(
                    "SELECT folder_id, title, relative_path, content \
                     FROM knowledge_base_documents WHERE id=?1",
                    params![document_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
            } else {
                // SQLite `substr` counts text characters. Fetch one extra
                // character so truncation remains exact without copying the
                // full indexed file in installed mode.
                let content_limit = i64::try_from(max_chars.saturating_add(1)).unwrap_or(i64::MAX);
                connection.query_row(
                    "SELECT folder_id, title, relative_path, substr(content, 1, ?2) \
                     FROM knowledge_base_documents WHERE id=?1",
                    params![document_id, content_limit],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
            }
            .optional()
            .map_err(|error| format!("read knowledge base document: {error}"))?
            .ok_or_else(|| {
                "knowledge base document was not found; refresh its folder".to_string()
            })?;
        let content = unprotect_portable_local_service_text(&content)?;
        let truncated = content.chars().count() > max_chars;
        Ok(KnowledgeBaseDocument {
            document_id,
            folder_id,
            title,
            relative_path,
            content: truncate_knowledge_base_text(&content, max_chars),
            truncated,
        })
    }

    pub fn knowledge_base_agent_begin(
        &self,
        turn_id: &str,
        provider_id: &str,
        model_id: &str,
    ) -> Result<String, String> {
        bounded_nonempty(turn_id, "knowledge base turn id", 128)?;
        let consent = validate_knowledge_base_consent(Some(KnowledgeBaseConsent {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
        }))?;
        let _access = self.knowledge_access_lock.read();
        let connection = self.open()?;
        if !knowledge_base_enabled(&connection)? {
            return Err("chat knowledge-base use is not enabled".to_string());
        }
        if knowledge_base_consent(&connection)?.as_ref() != Some(&consent) {
            return Err(
                "chat knowledge-base consent does not cover this provider or model".to_string(),
            );
        }
        let folder_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM knowledge_base_folders", [], |row| {
                row.get(0)
            })
            .map_err(|error| format!("count knowledge base folders: {error}"))?;
        if folder_count == 0 {
            return Err("chat knowledge-base use has no approved folders".to_string());
        }
        let token = Uuid::new_v4().to_string();
        let mut accesses = self.knowledge_agent_access.lock();
        prune_knowledge_agent_access(&mut accesses);
        accesses.insert(
            token.clone(),
            KnowledgeAgentAccess {
                created_at: Instant::now(),
                document_ids: HashSet::new(),
                searches: 0,
                reads: 0,
                read_chars: 0,
            },
        );
        Ok(token)
    }

    pub fn knowledge_base_agent_search(
        &self,
        access_token: &str,
        query: &str,
        limit: Option<usize>,
    ) -> Result<KnowledgeBaseSearchResponse, String> {
        let _access = self.knowledge_access_lock.read();
        {
            let mut accesses = self.knowledge_agent_access.lock();
            let grant = knowledge_agent_access_mut(&mut accesses, access_token)?;
            if grant.searches >= MAX_KNOWLEDGE_AGENT_SEARCHES_PER_TURN {
                return Err("knowledge-base search budget for this turn is exhausted".to_string());
            }
            grant.searches += 1;
        }
        let response = self.knowledge_base_search_unchecked(query, None, limit)?;
        let mut accesses = self.knowledge_agent_access.lock();
        let grant = knowledge_agent_access_mut(&mut accesses, access_token)?;
        grant
            .document_ids
            .extend(response.items.iter().map(|item| item.document_id.clone()));
        Ok(response)
    }

    pub fn knowledge_base_agent_read(
        &self,
        access_token: &str,
        document_id: &str,
        max_chars: Option<usize>,
    ) -> Result<KnowledgeBaseDocument, String> {
        let document_id = validate_record_id(document_id, "knowledge base document id")?;
        let max_chars = max_chars
            .unwrap_or(MAX_KNOWLEDGE_BASE_READ_CHARS)
            .clamp(1, MAX_KNOWLEDGE_BASE_READ_CHARS);
        let _access = self.knowledge_access_lock.read();
        {
            let mut accesses = self.knowledge_agent_access.lock();
            let grant = knowledge_agent_access_mut(&mut accesses, access_token)?;
            if !grant.document_ids.contains(&document_id) {
                return Err(
                    "knowledge-base document was not returned by this turn's search".to_string(),
                );
            }
            if grant.reads >= MAX_KNOWLEDGE_AGENT_READS_PER_TURN
                || grant.read_chars.saturating_add(max_chars)
                    > MAX_KNOWLEDGE_AGENT_READ_CHARS_PER_TURN
            {
                return Err("knowledge-base read budget for this turn is exhausted".to_string());
            }
            grant.reads += 1;
            grant.read_chars = grant.read_chars.saturating_add(max_chars);
        }
        self.knowledge_base_read_unchecked(&document_id, Some(max_chars))
    }

    fn knowledge_base_search_fts(
        &self,
        query: &str,
        allowed_folders: &HashSet<String>,
        limit: usize,
    ) -> Result<KnowledgeBaseSearchResponse, ()> {
        let connection = self.open().map_err(|_| ())?;
        let fts_query = knowledge_base_fts_query(query);
        let mut folders = allowed_folders.iter().cloned().collect::<Vec<_>>();
        folders.sort();
        let folder_filter = if folders.is_empty() {
            String::new()
        } else {
            let placeholders = (2..folders.len() + 2)
                .map(|index| format!("?{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!(" AND d.folder_id IN ({placeholders})")
        };
        let limit_index = folders.len() + 2;
        let sql = format!(
            "SELECT d.id, d.folder_id, f.display_name, d.title, d.relative_path, \
             snippet(knowledge_base_fts, 4, '', '', ' … ', 48), -bm25(knowledge_base_fts), d.modified_at \
             FROM knowledge_base_fts \
             JOIN knowledge_base_documents d ON d.id=knowledge_base_fts.document_id \
             JOIN knowledge_base_folders f ON f.id=d.folder_id \
             WHERE knowledge_base_fts MATCH ?1{folder_filter} \
             ORDER BY bm25(knowledge_base_fts), d.updated_at DESC LIMIT ?{limit_index}",
        );
        let mut values = vec![SqlValue::Text(fts_query)];
        values.extend(folders.into_iter().map(SqlValue::Text));
        values.push(SqlValue::Integer(limit.saturating_add(1) as i64));
        let mut statement = connection.prepare(&sql).map_err(|_| ())?;
        let rows = statement
            .query_map(params_from_iter(values.iter()), |row| {
                Ok(KnowledgeBaseSearchItem {
                    document_id: row.get(0)?,
                    folder_id: row.get(1)?,
                    folder_name: row.get(2)?,
                    title: row.get(3)?,
                    relative_path: row.get(4)?,
                    snippet: truncate_knowledge_base_text(
                        &row.get::<_, String>(5)?,
                        MAX_KNOWLEDGE_BASE_SNIPPET_CHARS,
                    ),
                    score: row.get(6)?,
                    modified_at: row.get(7)?,
                })
            })
            .map_err(|_| ())?;
        let mut candidates = rows.collect::<Result<Vec<_>, _>>().map_err(|_| ())?;
        let truncated = candidates.len() > limit;
        candidates.truncate(limit);
        Ok(KnowledgeBaseSearchResponse {
            items: candidates,
            backend: "fts5".to_string(),
            truncated,
        })
    }

    fn knowledge_base_search_like(
        &self,
        query: &str,
        allowed_folders: &HashSet<String>,
        limit: usize,
    ) -> Result<KnowledgeBaseSearchResponse, String> {
        if crate::storage::is_portable() {
            return self.knowledge_base_search_portable_decrypted(query, allowed_folders, limit);
        }
        let pattern = format!("%{}%", escape_like(query));
        let connection = self.open()?;
        let mut folders = allowed_folders.iter().cloned().collect::<Vec<_>>();
        folders.sort();
        let folder_filter = if folders.is_empty() {
            String::new()
        } else {
            let placeholders = (2..folders.len() + 2)
                .map(|index| format!("?{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!(" AND d.folder_id IN ({placeholders})")
        };
        let limit_index = folders.len() + 2;
        let sql = format!(
            "SELECT d.id, d.folder_id, f.display_name, d.title, d.relative_path, d.content, d.modified_at \
             FROM knowledge_base_documents d JOIN knowledge_base_folders f ON f.id=d.folder_id \
             WHERE (d.title LIKE ?1 ESCAPE '\\' OR d.relative_path LIKE ?1 ESCAPE '\\' OR d.content LIKE ?1 ESCAPE '\\'){folder_filter} \
             ORDER BY CASE WHEN d.title LIKE ?1 ESCAPE '\\' THEN 0 ELSE 1 END, d.updated_at DESC LIMIT ?{limit_index}",
        );
        let mut values = vec![SqlValue::Text(pattern)];
        values.extend(folders.into_iter().map(SqlValue::Text));
        values.push(SqlValue::Integer(limit.saturating_add(1) as i64));
        let mut statement = connection
            .prepare(&sql)
            .map_err(|error| format!("prepare knowledge base search: {error}"))?;
        let rows = statement
            .query_map(params_from_iter(values.iter()), |row| {
                let content: String = row.get(5)?;
                Ok(KnowledgeBaseSearchItem {
                    document_id: row.get(0)?,
                    folder_id: row.get(1)?,
                    folder_name: row.get(2)?,
                    title: row.get(3)?,
                    relative_path: row.get(4)?,
                    snippet: knowledge_base_excerpt(&content, query),
                    score: 0.0,
                    modified_at: row.get(6)?,
                })
            })
            .map_err(|error| format!("search knowledge base documents: {error}"))?;
        let mut candidates = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("search knowledge base documents: {error}"))?;
        let truncated = candidates.len() > limit;
        candidates.truncate(limit);
        Ok(KnowledgeBaseSearchResponse {
            items: candidates,
            backend: "like".to_string(),
            truncated,
        })
    }

    fn knowledge_base_search_portable_decrypted(
        &self,
        query: &str,
        allowed_folders: &HashSet<String>,
        limit: usize,
    ) -> Result<KnowledgeBaseSearchResponse, String> {
        let connection = self.open()?;
        let mut folders = allowed_folders.iter().cloned().collect::<Vec<_>>();
        folders.sort();
        let folder_filter = if folders.is_empty() {
            String::new()
        } else {
            let placeholders = (1..folders.len() + 1)
                .map(|index| format!("?{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!(" WHERE d.folder_id IN ({placeholders})")
        };
        let limit_index = folders.len() + 1;
        let sql = format!(
            "SELECT d.id, d.folder_id, f.display_name, d.title, d.relative_path, d.content, d.modified_at \
             FROM knowledge_base_documents d JOIN knowledge_base_folders f ON f.id=d.folder_id{folder_filter} \
             ORDER BY d.updated_at DESC LIMIT ?{limit_index}",
        );
        let mut values = folders.into_iter().map(SqlValue::Text).collect::<Vec<_>>();
        values.push(SqlValue::Integer(
            (MAX_PORTABLE_DECRYPT_SEARCH_ROWS + 1) as i64,
        ));
        let mut statement = connection
            .prepare(&sql)
            .map_err(|error| format!("prepare portable knowledge-base search: {error}"))?;
        let mut rows = statement
            .query(params_from_iter(values.iter()))
            .map_err(|error| format!("search portable knowledge-base documents: {error}"))?;
        let needle = query.to_lowercase();
        let item_cap = limit.saturating_add(1);
        let mut items = Vec::with_capacity(item_cap);
        let mut row_count = 0usize;
        while let Some(row) = rows
            .next()
            .map_err(|error| format!("search portable knowledge-base documents: {error}"))?
        {
            row_count = row_count.saturating_add(1);
            if row_count > MAX_PORTABLE_DECRYPT_SEARCH_ROWS {
                return Err("portable knowledge-base search has too many documents; select fewer folders before searching".to_string());
            }
            // Preserve the strict row cap even once the response is full, but
            // skip decryption and allocation for trailing rows that cannot
            // change the ordered `limit + 1` result.
            if items.len() >= item_cap {
                continue;
            }
            let document_id: String = row
                .get(0)
                .map_err(|error| format!("search portable knowledge-base documents: {error}"))?;
            let folder_id: String = row
                .get(1)
                .map_err(|error| format!("search portable knowledge-base documents: {error}"))?;
            let folder_name: String = row
                .get(2)
                .map_err(|error| format!("search portable knowledge-base documents: {error}"))?;
            let title: String = row
                .get(3)
                .map_err(|error| format!("search portable knowledge-base documents: {error}"))?;
            let relative_path: String = row
                .get(4)
                .map_err(|error| format!("search portable knowledge-base documents: {error}"))?;
            let stored_content: String = row
                .get(5)
                .map_err(|error| format!("search portable knowledge-base documents: {error}"))?;
            let modified_at: Option<i64> = row
                .get(6)
                .map_err(|error| format!("search portable knowledge-base documents: {error}"))?;
            let content = unprotect_portable_local_service_text(&stored_content)?;
            if !(local_service_text_contains_case_insensitive(&title, &needle)
                || local_service_text_contains_case_insensitive(&relative_path, &needle)
                || local_service_text_contains_case_insensitive(&content, &needle))
            {
                continue;
            }
            items.push(KnowledgeBaseSearchItem {
                document_id,
                folder_id,
                folder_name,
                title,
                relative_path,
                snippet: knowledge_base_excerpt(&content, query),
                score: 0.0,
                modified_at,
            });
        }
        let truncated = items.len() > limit;
        items.truncate(limit);
        Ok(KnowledgeBaseSearchResponse {
            items,
            backend: "portable-decrypted".to_string(),
            truncated,
        })
    }
}

fn row_to_knowledge_base_folder(row: &rusqlite::Row<'_>) -> rusqlite::Result<KnowledgeBaseFolder> {
    Ok(KnowledgeBaseFolder {
        id: row.get(0)?,
        canonical_path: row.get(1)?,
        display_name: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
        last_indexed_at: row.get(5)?,
        document_count: usize::try_from(row.get::<_, i64>(6)?).unwrap_or(usize::MAX),
        indexed_bytes: u64::try_from(row.get::<_, i64>(7)?).unwrap_or(u64::MAX),
    })
}

fn knowledge_base_enabled(connection: &Connection) -> Result<bool, String> {
    let value = connection
        .query_row(
            "SELECT value FROM local_service_meta WHERE key='knowledge_base_enabled'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("read knowledge base setting: {error}"))?;
    Ok(value.as_deref() == Some("1"))
}

fn knowledge_base_consent(connection: &Connection) -> Result<Option<KnowledgeBaseConsent>, String> {
    let provider_id = connection
        .query_row(
            "SELECT value FROM local_service_meta WHERE key='knowledge_base_provider_id'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("read knowledge base provider consent: {error}"))?;
    let model_id = connection
        .query_row(
            "SELECT value FROM local_service_meta WHERE key='knowledge_base_model_id'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("read knowledge base model consent: {error}"))?;
    Ok(match (provider_id, model_id) {
        (Some(provider_id), Some(model_id)) => {
            validate_knowledge_base_consent(Some(KnowledgeBaseConsent {
                provider_id,
                model_id,
            }))
            .ok()
        }
        _ => None,
    })
}

fn validate_knowledge_base_consent(
    consent: Option<KnowledgeBaseConsent>,
) -> Result<KnowledgeBaseConsent, String> {
    let consent = consent.ok_or_else(|| {
        "choose the current provider and model before enabling chat knowledge-base use".to_string()
    })?;
    let provider_id = bounded_nonempty(&consent.provider_id, "knowledge base provider id", 128)?;
    if !provider_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err("knowledge base provider id is invalid".to_string());
    }
    let model_id = bounded_nonempty(&consent.model_id, "knowledge base model id", 256)?;
    if model_id.len() > 256 || model_id.chars().any(char::is_control) {
        return Err("knowledge base model id is invalid".to_string());
    }
    Ok(KnowledgeBaseConsent {
        provider_id,
        model_id,
    })
}

fn save_knowledge_base_consent(
    connection: &Connection,
    consent: &KnowledgeBaseConsent,
) -> Result<(), String> {
    for (key, value) in [
        ("knowledge_base_provider_id", &consent.provider_id),
        ("knowledge_base_model_id", &consent.model_id),
    ] {
        connection
            .execute(
                "INSERT INTO local_service_meta(key, value) VALUES(?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            )
            .map_err(|error| format!("save knowledge base consent: {error}"))?;
    }
    Ok(())
}

fn clear_knowledge_base_consent(connection: &Connection) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM local_service_meta WHERE key IN ('knowledge_base_provider_id', 'knowledge_base_model_id')",
            [],
        )
        .map_err(|error| format!("clear knowledge base consent: {error}"))?;
    Ok(())
}

fn revoke_knowledge_base_consent(transaction: &Transaction<'_>) -> Result<(), String> {
    transaction
        .execute(
            "INSERT INTO local_service_meta(key, value) VALUES('knowledge_base_enabled', '0') \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [],
        )
        .map_err(|error| format!("revoke knowledge base setting: {error}"))?;
    transaction
        .execute(
            "DELETE FROM local_service_meta WHERE key IN ('knowledge_base_provider_id', 'knowledge_base_model_id')",
            [],
        )
        .map_err(|error| format!("clear knowledge base consent: {error}"))?;
    Ok(())
}

fn prune_knowledge_agent_access(accesses: &mut HashMap<String, KnowledgeAgentAccess>) {
    accesses.retain(|_, access| access.created_at.elapsed() <= KNOWLEDGE_AGENT_ACCESS_TTL);
}

fn knowledge_agent_access_mut<'a>(
    accesses: &'a mut HashMap<String, KnowledgeAgentAccess>,
    access_token: &str,
) -> Result<&'a mut KnowledgeAgentAccess, String> {
    prune_knowledge_agent_access(accesses);
    let access_token = validate_record_id(access_token, "knowledge base access token")?;
    accesses
        .get_mut(&access_token)
        .ok_or_else(|| "knowledge-base access grant is missing or expired".to_string())
}

fn validate_knowledge_base_folder_ids(ids: Option<Vec<String>>) -> Result<HashSet<String>, String> {
    let ids = ids.unwrap_or_default();
    if ids.len() > MAX_KNOWLEDGE_BASE_FOLDERS {
        return Err(format!(
            "knowledge base search accepts at most {MAX_KNOWLEDGE_BASE_FOLDERS} folders"
        ));
    }
    ids.into_iter()
        .map(|id| validate_record_id(&id, "knowledge base folder id"))
        .collect()
}

fn canonical_knowledge_base_root(path: &Path) -> Result<(PathBuf, String), String> {
    if !path.is_absolute() {
        return Err("knowledge base folder must be absolute".to_string());
    }
    let canonical = fs::canonicalize(path)
        .map_err(|_| "knowledge base folder is not accessible".to_string())?;
    ensure_plain_directory(&canonical)
        .map_err(|_| "knowledge base folder must be a regular non-link directory".to_string())?;
    let display = path_for_display(&canonical);
    if display.chars().count() > 4_096 {
        return Err("knowledge base folder path is too long".to_string());
    }
    Ok((canonical, display))
}

fn scan_knowledge_base(
    root: &Path,
) -> Result<(Vec<ScannedKnowledgeDocument>, usize, bool), String> {
    let mut documents = Vec::new();
    let mut skipped_files = 0_usize;
    let mut inspected_files = 0_usize;
    let mut failed_files = 0_usize;
    let mut traversal_errors = 0_usize;
    let mut total_bytes = 0_u64;
    let mut truncated = false;
    let iterator = WalkDir::new(root)
        .follow_links(false)
        .max_depth(MAX_KNOWLEDGE_BASE_DEPTH)
        .into_iter()
        .filter_entry(|entry| !knowledge_base_ignored_directory(entry));
    for entry in iterator {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.depth() == 0 => {
                return Err("knowledge base folder became inaccessible during indexing".to_string())
            }
            Err(_) => {
                traversal_errors = traversal_errors.saturating_add(1);
                skipped_files = skipped_files.saturating_add(1);
                continue;
            }
        };
        if entry.path() == root || !entry.file_type().is_file() {
            continue;
        }
        if inspected_files >= MAX_KNOWLEDGE_BASE_FILES {
            truncated = true;
            break;
        }
        inspected_files = inspected_files.saturating_add(1);
        match scan_knowledge_base_file(root, entry.path(), total_bytes) {
            Ok(ScannedKnowledgeFile::Indexed(document)) => {
                total_bytes = total_bytes.saturating_add(document.size_bytes);
                documents.push(document);
            }
            Ok(ScannedKnowledgeFile::Skipped) => {
                skipped_files = skipped_files.saturating_add(1);
            }
            Err(_) => {
                failed_files = failed_files.saturating_add(1);
                skipped_files = skipped_files.saturating_add(1);
            }
            Ok(ScannedKnowledgeFile::TotalLimitExceeded) => {
                skipped_files = skipped_files.saturating_add(1);
                truncated = true;
            }
        }
        if total_bytes >= MAX_KNOWLEDGE_BASE_TOTAL_BYTES {
            truncated = true;
            break;
        }
    }
    // An actually empty folder (or one containing only deliberately
    // unsupported files) is a valid empty index. By contrast, replacing an
    // existing index with nothing when every candidate failed an I/O/security
    // check would silently destroy the last known-good local search data.
    if documents.is_empty()
        && ((inspected_files > 0 && failed_files == inspected_files)
            || (inspected_files == 0 && traversal_errors > 0))
    {
        return Err("knowledge base scan failed before any document could be indexed".to_string());
    }
    Ok((documents, skipped_files, truncated))
}

fn knowledge_base_ignored_directory(entry: &walkdir::DirEntry) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_dir() {
        return false;
    }
    let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
    matches!(
        name.as_str(),
        ".git"
            | ".hg"
            | ".svn"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | ".next"
            | ".turbo"
            | "coverage"
    )
}

enum ScannedKnowledgeFile {
    Indexed(ScannedKnowledgeDocument),
    Skipped,
    TotalLimitExceeded,
}

/// Reject linked path components before opening the leaf. The later no-follow
/// open and post-open canonical check close the remaining replacement window.
fn reject_knowledge_base_path_links(root: &Path, candidate: &Path) -> Result<(), String> {
    let relative = candidate
        .strip_prefix(root)
        .map_err(|_| "knowledge base file escaped its approved folder".to_string())?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            return Err("knowledge base file path is invalid".to_string());
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|_| "knowledge base file is not accessible".to_string())?;
        if is_link_or_reparse(&metadata) {
            return Err(
                "knowledge base file must not traverse a link or reparse point".to_string(),
            );
        }
    }
    Ok(())
}

#[cfg(windows)]
fn open_knowledge_base_file_no_follow(path: &Path) -> Result<fs::File, String> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| "knowledge base file is not accessible".to_string())
}

#[cfg(unix)]
fn open_knowledge_base_file_no_follow(path: &Path) -> Result<fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| "knowledge base file is not accessible".to_string())
}

#[cfg(all(not(windows), not(unix)))]
fn open_knowledge_base_file_no_follow(path: &Path) -> Result<fs::File, String> {
    fs::File::open(path).map_err(|_| "knowledge base file is not accessible".to_string())
}

fn scan_knowledge_base_file(
    root: &Path,
    path: &Path,
    indexed_bytes: u64,
) -> Result<ScannedKnowledgeFile, String> {
    // Avoid the comparatively expensive no-follow open and canonicalization
    // path for files that are unambiguously ineligible by their discovered
    // name. `read_knowledge_base_file` repeats this check on the canonical
    // path as the authoritative security boundary.
    if !knowledge_base_supported_file(path) || knowledge_base_sensitive_file(path) {
        return Ok(ScannedKnowledgeFile::Skipped);
    }
    let content = match read_knowledge_base_file(root, path)? {
        Some(content) => content,
        None => return Ok(ScannedKnowledgeFile::Skipped),
    };
    let size_bytes = u64::try_from(content.len()).unwrap_or(u64::MAX);
    if indexed_bytes.saturating_add(size_bytes) > MAX_KNOWLEDGE_BASE_TOTAL_BYTES {
        return Ok(ScannedKnowledgeFile::TotalLimitExceeded);
    }
    let canonical =
        fs::canonicalize(path).map_err(|_| "knowledge base file is not accessible".to_string())?;
    let relative = canonical
        .strip_prefix(root)
        .map_err(|_| "knowledge base file escaped its approved folder".to_string())?;
    let relative_path = knowledge_base_relative_path(relative)?;
    let title = canonical
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| "knowledge base file has no name".to_string())?;
    let extension = knowledge_base_extension(&canonical).unwrap_or_default();
    let metadata = fs::metadata(&canonical)
        .map_err(|_| "knowledge base file is not accessible".to_string())?;
    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|value| i64::try_from(value.as_millis()).ok());
    Ok(ScannedKnowledgeFile::Indexed(ScannedKnowledgeDocument {
        relative_path,
        title,
        extension,
        size_bytes,
        modified_at,
        content_hash: sha256_hex(content.as_bytes()),
        content,
    }))
}

fn read_knowledge_base_file(root: &Path, candidate: &Path) -> Result<Option<String>, String> {
    reject_knowledge_base_path_links(root, candidate)?;
    let metadata = fs::symlink_metadata(candidate)
        .map_err(|_| "knowledge base file is not accessible".to_string())?;
    if !metadata.is_file() || is_link_or_reparse(&metadata) {
        return Err("knowledge base file must be a regular non-link file".to_string());
    }
    if metadata.len() > MAX_KNOWLEDGE_BASE_FILE_BYTES {
        return Ok(None);
    }
    let canonical = fs::canonicalize(candidate)
        .map_err(|_| "knowledge base file is not accessible".to_string())?;
    if !canonical.starts_with(root) {
        return Err("knowledge base file escaped its approved folder".to_string());
    }
    let canonical_metadata = fs::symlink_metadata(&canonical)
        .map_err(|_| "knowledge base file is not accessible".to_string())?;
    if !canonical_metadata.is_file() || is_link_or_reparse(&canonical_metadata) {
        return Err("knowledge base file must be a regular non-link file".to_string());
    }
    if !knowledge_base_supported_file(&canonical) || knowledge_base_sensitive_file(&canonical) {
        return Ok(None);
    }
    // The final component is opened without following links and all bytes are
    // read through that stable handle. This prevents a concurrent replacement
    // from redirecting an Agent-visible source outside the approved root or
    // from turning a small file into an unbounded allocation.
    let file = open_knowledge_base_file_no_follow(candidate)?;
    let opened_metadata = file
        .metadata()
        .map_err(|_| "knowledge base file is not accessible".to_string())?;
    if !opened_metadata.is_file()
        || is_link_or_reparse(&opened_metadata)
        || opened_metadata.len() != metadata.len()
        || opened_metadata.len() > MAX_KNOWLEDGE_BASE_FILE_BYTES
    {
        return Err("knowledge base file changed while opening".to_string());
    }
    let resolved_after_open = fs::canonicalize(candidate)
        .map_err(|_| "knowledge base file is not accessible".to_string())?;
    if resolved_after_open != canonical || !resolved_after_open.starts_with(root) {
        return Err("knowledge base file changed while opening".to_string());
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(opened_metadata.len().min(MAX_KNOWLEDGE_BASE_FILE_BYTES)).unwrap_or(0),
    );
    file.take(MAX_KNOWLEDGE_BASE_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| "read knowledge base file failed".to_string())?;
    if bytes.len() as u64 > MAX_KNOWLEDGE_BASE_FILE_BYTES
        || bytes.len() as u64 != opened_metadata.len()
    {
        return Err("knowledge base file changed while reading".to_string());
    }
    if bytes.contains(&0) {
        return Ok(None);
    }
    Ok(String::from_utf8(bytes).ok())
}

fn knowledge_base_supported_file(path: &Path) -> bool {
    matches!(
        knowledge_base_extension(path).as_deref(),
        Some(
            "md" | "mdx"
                | "txt"
                | "rst"
                | "json"
                | "jsonl"
                | "yaml"
                | "yml"
                | "toml"
                | "csv"
                | "tsv"
                | "rs"
                | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "py"
                | "go"
                | "java"
                | "kt"
                | "c"
                | "h"
                | "cpp"
                | "hpp"
                | "cs"
                | "php"
                | "rb"
                | "swift"
                | "sql"
                | "html"
                | "css"
                | "scss"
                | "vue"
                | "svelte"
                | "sh"
                | "ps1"
                | "bat"
                | "cmd"
                | "xml"
                | "ini"
                | "cfg"
                | "conf"
                | "log"
        )
    )
}

fn knowledge_base_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
}

fn knowledge_base_sensitive_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    name == ".env"
        || name.starts_with(".env.")
        || name.contains("credential")
        || name.contains("secret")
        || name.contains("token")
        || matches!(
            name.as_str(),
            "id_rsa" | "id_dsa" | "id_ecdsa" | "id_ed25519" | ".npmrc" | ".pypirc" | "known_hosts"
        )
        || matches!(
            knowledge_base_extension(path).as_deref(),
            Some("pem" | "key" | "p12" | "pfx" | "kdbx" | "der" | "crt" | "cer")
        )
}

fn knowledge_base_relative_path(path: &Path) -> Result<String, String> {
    let mut segments = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(segment) => {
                let segment = segment
                    .to_str()
                    .ok_or_else(|| "knowledge base path is not valid UTF-8".to_string())?;
                if segment.is_empty() {
                    return Err("knowledge base path is invalid".to_string());
                }
                segments.push(segment);
            }
            _ => return Err("knowledge base path is invalid".to_string()),
        }
    }
    if segments.is_empty() {
        return Err("knowledge base path is invalid".to_string());
    }
    Ok(segments.join("/"))
}

fn knowledge_base_fts_query(query: &str) -> String {
    let terms = query
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    if terms.is_empty() {
        format!("\"{}\"", query.replace('"', "\"\""))
    } else {
        terms.join(" AND ")
    }
}

fn truncate_knowledge_base_text(value: &str, maximum_chars: usize) -> String {
    let mut characters = value.chars();
    let mut output = String::new();
    for _ in 0..maximum_chars {
        let Some(character) = characters.next() else {
            return output;
        };
        output.push(character);
    }
    if characters.next().is_some() {
        output.push_str("\n[truncated]");
    }
    output
}

fn knowledge_base_excerpt(content: &str, query: &str) -> String {
    let prefix_chars = content
        .find(query)
        .map(|index| content[..index].chars().count())
        .unwrap_or(0)
        .saturating_sub(180);
    let mut excerpt = content
        .chars()
        .skip(prefix_chars)
        .take(MAX_KNOWLEDGE_BASE_SNIPPET_CHARS)
        .collect::<String>();
    if prefix_chars > 0 {
        excerpt.insert_str(0, "… ");
    }
    if content.chars().count() > prefix_chars.saturating_add(MAX_KNOWLEDGE_BASE_SNIPPET_CHARS) {
        excerpt.push_str(" …");
    }
    excerpt
}

// ---------------------------------------------------------------------------
// Cron

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJob {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub schedule: String,
    pub payload: Value,
    pub enabled: bool,
    pub next_run_at: Option<i64>,
    pub last_run_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
struct StoredCronJob {
    id: String,
    name: String,
    kind: String,
    schedule: String,
    protected_payload: String,
    enabled: bool,
    next_run_at: Option<i64>,
    last_run_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CronUpsertInput {
    pub id: Option<String>,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub schedule: String,
    pub payload: Value,
    pub enabled: bool,
}

/// The only authority accepted for a dangerous Cron action. It is created
/// immediately after a Windows-native dialog succeeds, is not serializable or
/// cloneable, and is consumed by the one matching service call.
#[derive(Debug)]
struct NativeCronApproval {
    operation: CronDangerousOperation,
    target_digest: [u8; 32],
    issued_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CronDangerousOperation {
    Save,
    Enable,
    RunNow,
}

impl CronDangerousOperation {
    fn label(self) -> &'static str {
        match self {
            Self::Save => "save",
            Self::Enable => "enable",
            Self::RunNow => "run now",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronRun {
    pub id: String,
    pub job_id: String,
    pub status: String,
    pub scheduled_for: Option<i64>,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    /// Result bodies are never retained. These flags preserve the only
    /// renderer-visible history signal without keeping arbitrary output text.
    pub has_output: bool,
    pub has_error: bool,
}

#[cfg(test)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronCompleteInput {
    pub run_id: String,
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronDispatch {
    pub run_id: String,
    pub job_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronHttpResult {
    pub status: Option<u16>,
    pub body: String,
    pub truncated: bool,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronRunNowResponse {
    pub run: CronRun,
    pub dispatch: Option<CronDispatch>,
    pub http: Option<CronHttpResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronDueClaimResponse {
    pub claimed_at: i64,
    pub runs: Vec<CronRunNowResponse>,
}

/// Renderer-safe Cron job metadata.
///
/// A Cron payload can contain a prompt, a shell command, or HTTP credentials.
/// It stays native-only after it has been saved, so neither this type nor any
/// other renderer-facing Cron response serializes it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJobSummary {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub schedule: String,
    pub enabled: bool,
    pub next_run_at: Option<i64>,
    pub last_run_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub payload_redacted: bool,
}

impl From<CronJob> for CronJobSummary {
    fn from(job: CronJob) -> Self {
        Self {
            id: job.id,
            name: job.name,
            kind: job.kind,
            schedule: job.schedule,
            enabled: job.enabled,
            next_run_at: job.next_run_at,
            last_run_at: job.last_run_at,
            created_at: job.created_at,
            updated_at: job.updated_at,
            payload_redacted: true,
        }
    }
}

/// Renderer-safe status for one Cron invocation. Output and errors can include
/// arbitrary process or HTTP response data, so only their presence is retained
/// and returned to the renderer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronRunSummary {
    pub id: String,
    pub job_id: String,
    pub status: String,
    pub scheduled_for: Option<i64>,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub has_output: bool,
    pub has_error: bool,
}

impl From<CronRun> for CronRunSummary {
    fn from(run: CronRun) -> Self {
        Self {
            id: run.id,
            job_id: run.job_id,
            status: run.status,
            scheduled_for: run.scheduled_for,
            started_at: run.started_at,
            completed_at: run.completed_at,
            has_output: run.has_output,
            has_error: run.has_error,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronDispatchSummary {
    pub run_id: String,
    pub job_id: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronHttpSummary {
    pub status: Option<u16>,
    pub truncated: bool,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronRunNowSummary {
    pub run: CronRunSummary,
    pub dispatch: Option<CronDispatchSummary>,
    pub http: Option<CronHttpSummary>,
}

impl From<CronRunNowResponse> for CronRunNowSummary {
    fn from(response: CronRunNowResponse) -> Self {
        Self {
            run: response.run.into(),
            dispatch: response.dispatch.map(|dispatch| CronDispatchSummary {
                run_id: dispatch.run_id,
                job_id: dispatch.job_id,
                kind: dispatch.kind,
            }),
            http: response.http.map(|http| CronHttpSummary {
                status: http.status,
                truncated: http.truncated,
                success: http.success,
            }),
        }
    }
}

/// Payload for the native scheduler event. It contains aggregate state only:
/// Cron payloads, HTTP responses, process output, and error details never
/// cross IPC. The database retains only completion metadata and result flags.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronSchedulerUpdate {
    pub checked_at: i64,
    pub status: String,
    pub claimed: usize,
    /// Number of claimed executions which are currently running. This is
    /// deliberately a state count, rather than the former ambiguous queue
    /// metric: automatic work has no in-memory dispatch queue.
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
}

impl CronSchedulerUpdate {
    pub fn from_claim(response: &CronDueClaimResponse) -> Self {
        let mut completed = 0;
        let mut failed = 0;
        let mut running = 0;
        for response in &response.runs {
            match response.run.status.as_str() {
                "completed" => completed += 1,
                "failed" => failed += 1,
                "running" => running += 1,
                _ => {}
            }
        }
        Self {
            checked_at: response.claimed_at,
            status: "ok".to_string(),
            claimed: response.runs.len(),
            running,
            completed,
            failed,
        }
    }

    /// Build the same redacted aggregate update for one background worker
    /// completion. A completion is reported independently so the scheduler
    /// loop never needs to await the native executor.
    pub fn from_execution(response: &CronRunNowResponse) -> Self {
        let (running, completed, failed) = match response.run.status.as_str() {
            "completed" => (0, 1, 0),
            "failed" => (0, 0, 1),
            "running" => (1, 0, 0),
            _ => (0, 0, 0),
        };
        Self {
            checked_at: now_ms(),
            status: "ok".to_string(),
            // The claim was reported when the worker was dispatched. This is
            // a completion-only update, so clients do not double-count it.
            claimed: 0,
            running,
            completed,
            failed,
        }
    }

    /// A claimed run could not be handed back by a native worker. Keep the
    /// event aggregate-only; detailed failure text is not retained.
    pub fn failed_execution() -> Self {
        Self {
            checked_at: now_ms(),
            status: "error".to_string(),
            claimed: 0,
            running: 0,
            completed: 0,
            failed: 1,
        }
    }

    pub fn failed_check() -> Self {
        Self {
            checked_at: now_ms(),
            status: "error".to_string(),
            claimed: 0,
            running: 0,
            completed: 0,
            failed: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PromptCronPayload {
    prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workdir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ShellCronPayload {
    command: String,
    workdir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HttpCronPayload {
    url: String,
    #[serde(default = "default_http_method")]
    method: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(default = "default_http_timeout_ms")]
    timeout_ms: u64,
}

fn default_http_method() -> String {
    "GET".to_string()
}

fn default_http_timeout_ms() -> u64 {
    15_000
}

impl LocalServices {
    pub fn cron_list(&self) -> Result<Vec<CronJob>, String> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT id, name, kind, schedule, payload_json, enabled, next_run_at, last_run_at, created_at, updated_at \
                 FROM cron_jobs ORDER BY name COLLATE NOCASE, id LIMIT ?1",
            )
            .map_err(|error| format!("list Cron jobs: {error}"))?;
        let rows = statement
            .query_map(params![MAX_CRON_JOBS + 1], row_to_stored_cron_job)
            .map_err(|error| format!("list Cron jobs: {error}"))?;
        let stored = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("list Cron jobs: {error}"))?;
        if stored.len() > MAX_CRON_JOBS as usize {
            return Err("Cron job database exceeds its configured limit".to_string());
        }
        stored.into_iter().map(decode_cron_job).collect()
    }

    /// Ask the operating system for a confirmation of the exact currently
    /// stored job. The returned capability is private, short-lived, and later
    /// compared to a fresh native read before the side effect occurs.
    fn native_cron_approval_for_existing_job(
        &self,
        job_id: &str,
        operation: CronDangerousOperation,
    ) -> Result<Option<NativeCronApproval>, String> {
        let job = self.cron_get(job_id)?;
        if !cron_kind_is_dangerous(&job.kind) {
            return Ok(None);
        }
        let description = cron_native_confirmation_description(
            operation,
            &job.kind,
            &job.name,
            &job.schedule,
            &job.payload,
        )?;
        issue_native_cron_approval(operation, &job, description).map(Some)
    }

    fn cron_upsert(
        &self,
        input: CronUpsertInput,
        approval: Option<NativeCronApproval>,
    ) -> Result<CronJob, String> {
        let name = bounded_nonempty(&input.name, "Cron job name", MAX_CRON_NAME_CHARS)?;
        let kind = validate_cron_kind(&input.kind)?;
        validate_schedule(&input.schedule)?;
        let payload = normalize_cron_payload(&kind, input.payload.clone())?;
        require_native_cron_approval(&kind, CronDangerousOperation::Save, &input, approval)?;
        let protected_payload = encode_cron_payload(&payload)?;
        // Dangerous payloads can cause a native side effect later without any
        // further renderer interaction. Saving or changing either one must
        // never enable it implicitly: a separate, target-bound Enable approval
        // is required after the protected record has been committed.
        let enabled = force_dangerous_cron_disabled(&kind, input.enabled);
        let timestamp = now_ms();
        let next_run_at = enabled
            .then(|| next_schedule_at(&input.schedule, timestamp))
            .transpose()?;
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start Cron job update: {error}"))?;

        let (id, created_at) = if let Some(id) = input.id.as_deref() {
            let id = validate_record_id(id, "Cron job id")?;
            let created_at = transaction
                .query_row(
                    "SELECT created_at FROM cron_jobs WHERE id=?1",
                    params![id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| format!("read Cron job: {error}"))?
                .ok_or_else(|| "Cron job was not found".to_string())?;
            (id, created_at)
        } else {
            let count: i64 = transaction
                .query_row("SELECT COUNT(*) FROM cron_jobs", [], |row| row.get(0))
                .map_err(|error| format!("count Cron jobs: {error}"))?;
            if count >= MAX_CRON_JOBS {
                return Err(format!("Cron has reached its {MAX_CRON_JOBS} job limit"));
            }
            (Uuid::new_v4().to_string(), timestamp)
        };
        transaction
            .execute(
                "INSERT INTO cron_jobs(id, name, kind, schedule, payload_json, enabled, next_run_at, last_run_at, created_at, updated_at) \
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9) \
                 ON CONFLICT(id) DO UPDATE SET name=excluded.name, kind=excluded.kind, schedule=excluded.schedule, \
                 payload_json=excluded.payload_json, enabled=excluded.enabled, next_run_at=excluded.next_run_at, \
                 updated_at=excluded.updated_at",
                params![
                    id,
                    name,
                    kind,
                    input.schedule,
                    protected_payload,
                    enabled,
                    next_run_at,
                    created_at,
                    timestamp
                ],
            )
            .map_err(|error| format!("save Cron job: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit Cron job: {error}"))?;
        self.cron_get(&id)
    }

    /// Toggle a persisted Cron job without returning or replacing its
    /// protected payload. Dangerous enables decode it only in native memory
    /// to bind the one-time confirmation, never to expose it to the renderer.
    fn cron_set_enabled(
        &self,
        id: &str,
        enabled: bool,
        approval: Option<NativeCronApproval>,
    ) -> Result<CronJob, String> {
        let id = validate_record_id(id, "Cron job id")?;
        let timestamp = now_ms();
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start Cron enable update: {error}"))?;
        let mut stored = transaction
            .query_row(
                "SELECT id, name, kind, schedule, payload_json, enabled, next_run_at, last_run_at, created_at, updated_at \
                 FROM cron_jobs WHERE id=?1",
                params![id],
                row_to_stored_cron_job,
            )
            .optional()
            .map_err(|error| format!("read Cron job: {error}"))?
            .ok_or_else(|| "Cron job was not found".to_string())?;

        if enabled && cron_kind_is_dangerous(&stored.kind) {
            // Bind this confirmation to the complete native record before
            // changing it. A concurrent payload update makes a previously
            // issued approval unusable instead of authorizing a new target.
            let current_job = decode_cron_job(stored.clone())?;
            require_native_cron_approval(
                &stored.kind,
                CronDangerousOperation::Enable,
                &current_job,
                approval,
            )?;
        }

        if stored.enabled == enabled {
            return decode_cron_job(stored);
        }

        let next_run_at = enabled
            .then(|| next_schedule_at(&stored.schedule, timestamp))
            .transpose()?;
        transaction
            .execute(
                "UPDATE cron_jobs SET enabled=?2, next_run_at=?3, updated_at=?4 WHERE id=?1",
                params![id, enabled, next_run_at, timestamp],
            )
            .map_err(|error| format!("update Cron enabled state: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit Cron enabled state: {error}"))?;
        stored.enabled = enabled;
        stored.next_run_at = next_run_at;
        stored.updated_at = timestamp;
        decode_cron_job(stored)
    }

    pub fn cron_get(&self, id: &str) -> Result<CronJob, String> {
        let id = validate_record_id(id, "Cron job id")?;
        let stored = self
            .open()?
            .query_row(
                "SELECT id, name, kind, schedule, payload_json, enabled, next_run_at, last_run_at, created_at, updated_at \
                 FROM cron_jobs WHERE id=?1",
                params![id],
                row_to_stored_cron_job,
            )
            .optional()
            .map_err(|error| format!("read Cron job: {error}"))?
            .ok_or_else(|| "Cron job was not found".to_string())?;
        decode_cron_job(stored)
    }

    pub fn cron_delete(&self, id: &str) -> Result<CronJob, String> {
        let id = validate_record_id(id, "Cron job id")?;
        let mut connection = self.open()?;
        // A previous process can leave an active row behind. Expire it before
        // deciding whether deletion is safe; recovery never executes the job
        // again, particularly important for Shell and HTTP payloads.
        recover_expired_cron_runs(&connection, now_ms())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start Cron job delete: {error}"))?;
        let stored = transaction
            .query_row(
                "SELECT id, name, kind, schedule, payload_json, enabled, next_run_at, last_run_at, created_at, updated_at \
                 FROM cron_jobs WHERE id=?1",
                params![id],
                row_to_stored_cron_job,
            )
            .optional()
            .map_err(|error| format!("read Cron job: {error}"))?
            .ok_or_else(|| "Cron job was not found".to_string())?;
        let active_runs: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM cron_runs WHERE job_id=?1 AND status IN ('dispatched', 'running')",
                params![id],
                |row| row.get(0),
            )
            .map_err(|error| format!("check Cron job runs: {error}"))?;
        if active_runs > 0 {
            return Err("Cron job has an unfinished run".to_string());
        }
        transaction
            .execute("DELETE FROM cron_jobs WHERE id=?1", params![id])
            .map_err(|error| format!("delete Cron job: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit Cron job delete: {error}"))?;
        decode_cron_job(stored)
    }

    pub fn cron_runs(&self, job_id: &str, limit: Option<usize>) -> Result<Vec<CronRun>, String> {
        let job_id = validate_record_id(job_id, "Cron job id")?;
        let limit = limit.unwrap_or(50).clamp(1, MAX_CRON_RUN_LIST_LIMIT);
        let connection = self.open()?;
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM cron_jobs WHERE id=?1)",
                params![job_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("read Cron job: {error}"))?;
        if !exists {
            return Err("Cron job was not found".to_string());
        }
        let mut statement = connection
            .prepare(
                "SELECT id, job_id, status, scheduled_for, started_at, completed_at, has_output, has_error \
                 FROM cron_runs WHERE job_id=?1 ORDER BY started_at DESC, id DESC LIMIT ?2",
            )
            .map_err(|error| format!("list Cron runs: {error}"))?;
        let rows = statement
            .query_map(params![job_id, limit as i64], row_to_cron_run)
            .map_err(|error| format!("list Cron runs: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("list Cron runs: {error}"))
    }

    fn create_cron_run(
        &self,
        job_id: &str,
        scheduled_for: Option<i64>,
        approval: Option<NativeCronApproval>,
    ) -> Result<(CronJob, CronRun), String> {
        let job_id = validate_record_id(job_id, "Cron job id")?;
        let mut connection = self.open()?;
        // Do not let an abandoned lease permanently block a later manual run;
        // the same recovery is performed before scheduler claims and deletion.
        recover_expired_cron_runs(&connection, now_ms())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start Cron run: {error}"))?;
        let stored = transaction
            .query_row(
                "SELECT id, name, kind, schedule, payload_json, enabled, next_run_at, last_run_at, created_at, updated_at \
                 FROM cron_jobs WHERE id=?1",
                params![job_id],
                row_to_stored_cron_job,
            )
            .optional()
            .map_err(|error| format!("read Cron job: {error}"))?
            .ok_or_else(|| "Cron job was not found".to_string())?;
        let job = decode_cron_job(stored)?;
        // For a manual Shell or HTTP run, consume the native capability while
        // this IMMEDIATE transaction still owns the exact job record. That
        // closes the read/confirm/run race: an edit after the dialog makes
        // the binding mismatch before any run row is created.
        require_native_cron_approval(&job.kind, CronDangerousOperation::RunNow, &job, approval)?;
        // This remains in the IMMEDIATE transaction so parallel Run now
        // commands cannot both observe an idle job and create concurrent
        // high-cost executions for it.
        let active_runs: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM cron_runs WHERE job_id=?1 AND status IN ('dispatched', 'running')",
                params![&job.id],
                |row| row.get(0),
            )
            .map_err(|error| format!("check Cron job runs: {error}"))?;
        if active_runs > 0 {
            return Err("Cron job has an unfinished run".to_string());
        }
        let run = insert_cron_run(&transaction, &job, scheduled_for, now_ms())?;
        prune_cron_runs(&transaction, &job.id)?;
        transaction
            .commit()
            .map_err(|error| format!("commit Cron run: {error}"))?;
        Ok((job, run))
    }

    /// Atomically create `running` rows and advance only the due jobs that a
    /// caller has already reserved capacity to execute. No executor is started
    /// from this transaction, which keeps claiming crash-recoverable.
    fn claim_due_cron_runs(
        &self,
        limit: usize,
        timestamp: i64,
    ) -> Result<Vec<(CronJob, CronRun)>, String> {
        let limit = limit.clamp(1, MAX_CRON_CLAIM_LIMIT);
        let mut connection = self.open()?;
        // This also recovers an expired lease while the app stays open after a
        // worker or executor failure. The scheduled occurrence remains
        // failed; it is intentionally not put back on the due queue.
        recover_expired_cron_runs(&connection, timestamp)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start Cron due claim: {error}"))?;
        let stored_jobs = {
            let mut statement = transaction
                .prepare(
                    "SELECT id, name, kind, schedule, payload_json, enabled, next_run_at, last_run_at, created_at, updated_at \
                     FROM cron_jobs WHERE enabled=1 AND next_run_at IS NOT NULL AND next_run_at<=?1 \
                     ORDER BY next_run_at ASC, id ASC LIMIT ?2",
                )
                .map_err(|error| format!("scan due Cron jobs: {error}"))?;
            let rows = statement
                .query_map(params![timestamp, limit as i64], row_to_stored_cron_job)
                .map_err(|error| format!("scan due Cron jobs: {error}"))?;
            let collected = rows
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("scan due Cron jobs: {error}"))?;
            collected
        };
        let mut claims = Vec::with_capacity(stored_jobs.len());
        for stored in stored_jobs {
            let due_at = stored.next_run_at;
            let job = decode_cron_job(stored)?;
            let run = insert_cron_run(&transaction, &job, due_at, timestamp)?;
            let next_run_at = next_schedule_at(&job.schedule, timestamp)?;
            transaction
                .execute(
                    "UPDATE cron_jobs SET last_run_at=?2, next_run_at=?3, updated_at=?2 \
                     WHERE id=?1 AND enabled=1 AND next_run_at<=?2",
                    params![job.id, timestamp, next_run_at],
                )
                .map_err(|error| format!("advance due Cron job: {error}"))?;
            prune_cron_runs(&transaction, &job.id)?;
            claims.push((job, run));
        }
        transaction
            .commit()
            .map_err(|error| format!("commit Cron due claim: {error}"))?;
        Ok(claims)
    }

    #[cfg(test)]
    pub fn cron_complete(&self, input: CronCompleteInput) -> Result<CronRun, String> {
        let run_id = validate_record_id(&input.run_id, "Cron run id")?;
        if input
            .output
            .as_ref()
            .is_some_and(|output| output.len() > MAX_CRON_RUN_OUTPUT_BYTES)
        {
            return Err(format!(
                "Cron run output exceeds the {MAX_CRON_RUN_OUTPUT_BYTES} byte limit"
            ));
        }
        if input
            .error
            .as_ref()
            .is_some_and(|error| error.len() > MAX_CRON_RUN_OUTPUT_BYTES)
        {
            return Err(format!(
                "Cron run error exceeds the {MAX_CRON_RUN_OUTPUT_BYTES} byte limit"
            ));
        }
        let has_output = input.output.is_some();
        let has_error = !input.success || input.error.is_some();
        let success = input.success;
        // Discard arbitrary result text before opening SQLite. A failing
        // completion remains visible only through the durable presence flag.
        drop(input);
        self.complete_cron_run(&run_id, success, has_output, has_error)
    }

    fn complete_cron_run(
        &self,
        run_id: &str,
        success: bool,
        has_output: bool,
        has_error: bool,
    ) -> Result<CronRun, String> {
        let connection = self.open()?;
        let changed = connection
            .execute(
                "UPDATE cron_runs SET status=?2, completed_at=?3, lease_expires_at=NULL, \
                 has_output=?4, has_error=?5, output=NULL, error=NULL \
                 WHERE id=?1 AND status IN ('dispatched', 'running')",
                params![
                    run_id,
                    if success { "completed" } else { "failed" },
                    now_ms(),
                    has_output,
                    has_error
                ],
            )
            .map_err(|error| format!("complete Cron run: {error}"))?;
        if changed == 0 {
            return Err("Cron run was not found or is already complete".to_string());
        }
        connection
            .query_row(
                "SELECT id, job_id, status, scheduled_for, started_at, completed_at, has_output, has_error \
                 FROM cron_runs WHERE id=?1",
                params![run_id],
                row_to_cron_run,
            )
            .map_err(|error| format!("read completed Cron run: {error}"))
    }

    /// Execute one previously claimed Cron run. The caller owns its scheduler
    /// permit for the whole future, while this method owns terminal persistence
    /// (including clearing the run lease).
    pub(crate) async fn execute_claimed_cron(
        &self,
        job: CronJob,
        run: CronRun,
    ) -> Result<CronRunNowResponse, String> {
        // Wave 2: HTTP, Shell, and Prompt all execute entirely in native code.
        // Renderer-facing summaries still omit payload, command, prompt text,
        // HTTP bodies, and process output; only completion flags cross IPC.
        match job.kind.as_str() {
            "http" => {
                let execution = execute_http_job(&job.payload).await;
                let (success, has_output, has_error, http) = match execution {
                    Ok(http) => {
                        let success = http.success;
                        (success, true, !success, http)
                    }
                    Err(()) => (
                        false,
                        false,
                        true,
                        CronHttpResult {
                            status: None,
                            body: String::new(),
                            truncated: false,
                            success: false,
                        },
                    ),
                };
                let completed = self.complete_cron_run(&run.id, success, has_output, has_error)?;
                Ok(CronRunNowResponse {
                    run: completed,
                    dispatch: None,
                    http: Some(http),
                })
            }
            "shell" => {
                let (success, has_output, has_error) =
                    match execute_shell_job_in_worker(job.payload.clone()).await {
                        Ok((_output, success)) => (success, true, !success),
                        Err(_message) => (false, false, true),
                    };
                let completed = self.complete_cron_run(&run.id, success, has_output, has_error)?;
                Ok(CronRunNowResponse {
                    run: completed,
                    dispatch: None,
                    http: None,
                })
            }
            "prompt" => {
                let (success, has_output, has_error) =
                    match execute_prompt_job(&self.data_root, &job.payload).await {
                        Ok((_output, success)) => (success, true, !success),
                        Err(_message) => (false, false, true),
                    };
                let completed = self.complete_cron_run(&run.id, success, has_output, has_error)?;
                Ok(CronRunNowResponse {
                    run: completed,
                    dispatch: None,
                    http: None,
                })
            }
            _other => {
                let completed = self.complete_cron_run(&run.id, false, false, true)?;
                Ok(CronRunNowResponse {
                    run: completed,
                    dispatch: None,
                    http: None,
                })
            }
        }
    }

    async fn cron_run_now(
        &self,
        job_id: &str,
        approval: Option<NativeCronApproval>,
    ) -> Result<CronRunNowResponse, String> {
        self.ensure_ready()?;
        // Reserve before persisting a `running` row. Unlike automatic work,
        // this command is user-triggered, but it must still share the same
        // fixed native execution budget as the scheduler.
        let execution_slot = self.cron_execution_pool.try_reserve_slot()?;
        let (job, run) = self.create_cron_run(job_id, None, approval)?;
        let result = self.execute_claimed_cron(job, run).await;
        drop(execution_slot);
        result
    }

    /// Legacy test harness for native execution. Production automatic work is
    /// claimed exclusively by `cron_claim_due_for_scheduler` after it reserves
    /// a scheduler permit, and the WebView never receives this operation.
    #[cfg(test)]
    async fn cron_due_claim(
        self: Arc<Self>,
        limit: Option<usize>,
    ) -> Result<CronDueClaimResponse, String> {
        let claimed_at = now_ms();
        let claims =
            self.claim_due_cron_runs(limit.unwrap_or(CRON_WORKER_POOL_SIZE), claimed_at)?;
        let mut pending = claims.into_iter().enumerate();
        let mut workers = tokio::task::JoinSet::new();
        let mut runs = Vec::new();
        let mut first_error = None;

        for _ in 0..CRON_WORKER_POOL_SIZE {
            let Some((index, (job, run))) = pending.next() else {
                break;
            };
            let services = Arc::clone(&self);
            workers.spawn(async move { (index, services.execute_claimed_cron(job, run).await) });
        }

        // Do not return on the first failed worker: every already-claimed run
        // must get a completion attempt rather than being abandoned as running.
        while let Some(joined) = workers.join_next().await {
            match joined {
                Ok((index, Ok(response))) => runs.push((index, response)),
                Ok((_, Err(error))) if first_error.is_none() => first_error = Some(error),
                Ok((_, Err(_))) => {}
                Err(error) if first_error.is_none() => {
                    first_error = Some(format!("Cron worker join failed: {error}"));
                }
                Err(_) => {}
            }

            if let Some((index, (job, run))) = pending.next() {
                let services = Arc::clone(&self);
                workers
                    .spawn(async move { (index, services.execute_claimed_cron(job, run).await) });
            }
        }

        if let Some(error) = first_error {
            return Err(error);
        }

        runs.sort_by_key(|(index, _)| *index);
        Ok(CronDueClaimResponse {
            claimed_at,
            runs: runs.into_iter().map(|(_, response)| response).collect(),
        })
    }

    /// Claim only the work that a caller has already reserved execution slots
    /// for. The returned runs are deliberately still `running`; a scheduler
    /// worker receives each claim immediately after this atomic transaction.
    pub(crate) fn cron_claim_due_for_scheduler(
        &self,
        limit: usize,
    ) -> Result<(CronDueClaimResponse, Vec<(CronJob, CronRun)>), String> {
        let claimed_at = now_ms();
        let claims = self.claim_due_cron_runs(limit, claimed_at)?;
        let response = CronDueClaimResponse {
            claimed_at,
            runs: claims
                .iter()
                .map(|(_, run)| CronRunNowResponse {
                    run: run.clone(),
                    dispatch: None,
                    http: None,
                })
                .collect(),
        };
        Ok((response, claims))
    }
}

fn row_to_stored_cron_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredCronJob> {
    Ok(StoredCronJob {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        schedule: row.get(3)?,
        protected_payload: row.get(4)?,
        enabled: row.get(5)?,
        next_run_at: row.get(6)?,
        last_run_at: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn decode_cron_job(stored: StoredCronJob) -> Result<CronJob, String> {
    Ok(CronJob {
        id: stored.id,
        name: stored.name,
        kind: stored.kind,
        schedule: stored.schedule,
        payload: decode_cron_payload(&stored.protected_payload)?,
        enabled: stored.enabled,
        next_run_at: stored.next_run_at,
        last_run_at: stored.last_run_at,
        created_at: stored.created_at,
        updated_at: stored.updated_at,
    })
}

fn row_to_cron_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<CronRun> {
    Ok(CronRun {
        id: row.get(0)?,
        job_id: row.get(1)?,
        status: row.get(2)?,
        scheduled_for: row.get(3)?,
        started_at: row.get(4)?,
        completed_at: row.get(5)?,
        has_output: row.get(6)?,
        has_error: row.get(7)?,
    })
}

fn insert_cron_run(
    transaction: &Transaction<'_>,
    job: &CronJob,
    scheduled_for: Option<i64>,
    timestamp: i64,
) -> Result<CronRun, String> {
    let lease_expires_at = timestamp.saturating_add(CRON_RUN_LEASE_MS);
    let run = CronRun {
        id: Uuid::new_v4().to_string(),
        job_id: job.id.clone(),
        // Prompt, Shell, and HTTP are claimed as running and completed by the
        // native executor in the same turn. "dispatched" remains a valid status
        // for legacy rows and incomplete historical runs.
        status: "running".to_string(),
        scheduled_for,
        started_at: timestamp,
        completed_at: None,
        has_output: false,
        has_error: false,
    };
    transaction
        .execute(
            "INSERT INTO cron_runs(id, job_id, status, scheduled_for, started_at, lease_expires_at) \
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                run.id,
                run.job_id,
                run.status,
                run.scheduled_for,
                run.started_at,
                lease_expires_at
            ],
        )
        .map_err(|error| format!("create Cron run: {error}"))?;
    Ok(run)
}

fn prune_cron_runs(transaction: &Transaction<'_>, job_id: &str) -> Result<(), String> {
    transaction
        .execute(
            "DELETE FROM cron_runs WHERE job_id=?1 AND status IN ('completed', 'failed') \
             AND id NOT IN (SELECT id FROM cron_runs WHERE job_id=?1 ORDER BY started_at DESC, id DESC LIMIT ?2)",
            params![job_id, MAX_CRON_RUNS_PER_JOB as i64],
        )
        .map_err(|error| format!("prune Cron runs: {error}"))?;
    Ok(())
}

fn validate_cron_kind(kind: &str) -> Result<String, String> {
    match kind.trim() {
        "prompt" => Ok("prompt".to_string()),
        "http" => Ok("http".to_string()),
        "shell" => Ok("shell".to_string()),
        _ => Err("Cron type must be prompt, http, or shell".to_string()),
    }
}

fn cron_kind_is_dangerous(kind: &str) -> bool {
    // A Prompt Cron reads a configured provider credential and can create a
    // paid external request. Treat it as a native-confirmation operation just
    // like Shell and HTTP work; renderer-provided booleans can never authorize
    // persistence, activation, or an immediate run.
    matches!(kind, "prompt" | "shell" | "http")
}

fn cron_approval_digest<T: Serialize>(
    operation: CronDangerousOperation,
    target: &T,
) -> Result<[u8; 32], String> {
    let encoded = serde_json::to_vec(target)
        .map_err(|_| "serialize native Cron confirmation target".to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(operation.label().as_bytes());
    hasher.update([0_u8]);
    hasher.update(encoded);
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 32];
    bytes.copy_from_slice(&digest);
    Ok(bytes)
}

/// Consume a private native approval only for its exact operation and target.
/// It is intentionally not a renderer-supplied flag or a reusable token.
fn require_native_cron_approval<T: Serialize>(
    kind: &str,
    operation: CronDangerousOperation,
    target: &T,
    approval: Option<NativeCronApproval>,
) -> Result<(), String> {
    if !cron_kind_is_dangerous(kind) {
        return Ok(());
    }
    let approval = approval.ok_or_else(|| {
        format!(
            "a native user confirmation is required before {} this {kind} Cron target",
            operation.label()
        )
    })?;
    if approval.operation != operation {
        return Err("native Cron confirmation is bound to a different operation".to_string());
    }
    if approval.issued_at.elapsed() > NATIVE_CRON_APPROVAL_TTL {
        return Err(
            "native Cron confirmation expired; confirm the current target again".to_string(),
        );
    }
    if approval.target_digest != cron_approval_digest(operation, target)? {
        return Err(
            "native Cron confirmation does not match the current target; confirm it again"
                .to_string(),
        );
    }
    Ok(())
}

fn issue_native_cron_approval<T: Serialize>(
    operation: CronDangerousOperation,
    target: &T,
    description: String,
) -> Result<NativeCronApproval, String> {
    show_native_cron_confirmation(operation, description)?;
    Ok(NativeCronApproval {
        operation,
        target_digest: cron_approval_digest(operation, target)?,
        issued_at: Instant::now(),
    })
}

#[cfg(test)]
fn test_native_cron_approval<T: Serialize>(
    operation: CronDangerousOperation,
    target: &T,
) -> NativeCronApproval {
    NativeCronApproval {
        operation,
        target_digest: cron_approval_digest(operation, target)
            .expect("test Cron approval target must serialize"),
        issued_at: Instant::now(),
    }
}

#[cfg(all(windows, not(test)))]
fn show_native_cron_confirmation(
    operation: CronDangerousOperation,
    description: String,
) -> Result<(), String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDOK, MB_ICONWARNING, MB_OKCANCEL,
    };

    let title = format!("Confirm NovaVei Cron {}", operation.label());
    let title = title
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let description = description
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // `MessageBoxW` is the Windows-owned modal confirmation surface. Nothing
    // about its result crosses IPC; it is consumed immediately into the
    // private, target-bound approval capability above.
    let response = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            description.as_ptr(),
            title.as_ptr(),
            MB_OKCANCEL | MB_ICONWARNING,
        )
    };
    if response == IDOK {
        Ok(())
    } else {
        Err("user_cancelled".to_string())
    }
}

#[cfg(any(not(windows), test))]
fn show_native_cron_confirmation(
    _operation: CronDangerousOperation,
    _description: String,
) -> Result<(), String> {
    Err("native Cron confirmation is unsupported on this platform".to_string())
}

fn native_cron_dialog_preview(value: &str, max_chars: usize) -> String {
    let mut preview = String::new();
    for character in value.chars().take(max_chars) {
        // A native approval must be auditable. Escaping a literal backslash
        // as well as invisible characters distinguishes `\\n` in a command
        // from an actual line feed, so a later Shell statement cannot hide
        // behind a visually harmless comment in the Windows dialog.
        match character {
            '\\' => preview.push_str("\\\\"),
            '\n' => preview.push_str("\\n"),
            '\r' => preview.push_str("\\r"),
            '\t' => preview.push_str("\\t"),
            character if native_cron_dialog_requires_escape(character) => {
                preview.push_str(&format!("\\u{{{:04X}}}", character as u32));
            }
            character => preview.push(character),
        }
    }
    if value.chars().nth(max_chars).is_some() {
        preview.push_str(" … (truncated)");
    }
    preview
}

fn native_cron_dialog_requires_escape(character: char) -> bool {
    character.is_control()
        // Bidirectional marks and zero-width / formatting characters do not
        // have to be `is_control()` to make a privileged command preview
        // visually misleading. The displayed Unicode escape remains exact.
        || matches!(
            character,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{070F}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
        )
}

fn native_cron_header_names_preview(headers: &BTreeMap<String, String>) -> String {
    if headers.is_empty() {
        return "(none)".to_string();
    }
    headers
        .keys()
        .map(|name| native_cron_dialog_preview(name, MAX_CRON_HEADER_NAME_BYTES))
        .collect::<Vec<_>>()
        .join(", ")
}

fn cron_native_confirmation_description(
    operation: CronDangerousOperation,
    kind: &str,
    name: &str,
    schedule: &str,
    payload: &Value,
) -> Result<String, String> {
    let action = match operation {
        CronDangerousOperation::Save => "Save",
        CronDangerousOperation::Enable => "Enable",
        CronDangerousOperation::RunNow => "Run now",
    };
    let save_notice = match operation {
        CronDangerousOperation::Save => {
            "\n\nFor safety, this Cron job will remain disabled after saving. Enabling it requires a separate native confirmation."
        }
        CronDangerousOperation::Enable | CronDangerousOperation::RunNow => "",
    };
    let name = native_cron_dialog_preview(name, MAX_CRON_NAME_CHARS);
    let schedule = native_cron_dialog_preview(schedule, 256);
    match kind {
        "prompt" => {
            let payload = serde_json::from_value::<PromptCronPayload>(payload.clone())
                .map_err(|_| "stored Prompt Cron payload is invalid".to_string())?;
            let prompt_digest = sha256_hex(payload.prompt.as_bytes());
            let workdir = payload
                .workdir
                .as_deref()
                .map(|value| native_cron_dialog_preview(value, 512))
                .unwrap_or_else(|| "(default workspace)".to_string());
            let provider = payload
                .provider_id
                .as_deref()
                .map(|value| native_cron_dialog_preview(value, 128))
                .unwrap_or_else(|| "(default provider)".to_string());
            let model = payload
                .model
                .as_deref()
                .map(|value| native_cron_dialog_preview(value, 256))
                .unwrap_or_else(|| "(provider default)".to_string());
            Ok(format!(
                "{action} this Prompt Cron job?\n\nName: {name}\nSchedule: {schedule}\nWorkdir: {workdir}\nProvider: {provider}\nModel: {model}\nPrompt: {} UTF-8 bytes (text hidden)\nPrompt SHA-256: {prompt_digest}\n\nWhen it runs, NovaVei sends this exact saved prompt using the selected locally configured provider. It can consume provider quota or incur charges. The prompt text and provider credentials are never displayed. This native confirmation is bound to the exact saved Cron target.{save_notice}",
                payload.prompt.len(),
            ))
        }
        "shell" => {
            let payload = serde_json::from_value::<ShellCronPayload>(payload.clone())
                .map_err(|_| "stored Shell Cron payload is invalid".to_string())?;
            let command_digest = sha256_hex(payload.command.as_bytes());
            Ok(format!(
                "{action} this Shell Cron job?\n\nName: {name}\nSchedule: {schedule}\nWorkdir: {}\n\nIt runs the following command with your current Windows user permissions:\n\n{}\n\nExact command SHA-256: {command_digest}\n\nChoose OK only if this exact command, schedule, and workdir are expected.{save_notice}",
                native_cron_dialog_preview(&payload.workdir, 512),
                native_cron_dialog_preview(&payload.command, MAX_CRON_COMMAND_BYTES),
            ))
        }
        "http" => {
            let payload = serde_json::from_value::<HttpCronPayload>(payload.clone())
                .map_err(|_| "stored HTTP Cron payload is invalid".to_string())?;
            let request_digest = serde_json::to_vec(&payload)
                .map(sha256_hex)
                .map_err(|_| "serialize HTTP Cron confirmation request".to_string())?;
            let body_bytes = payload.body.as_deref().map_or(0, str::len);
            Ok(format!(
                "{action} this HTTP Cron job?\n\nName: {name}\nSchedule: {schedule}\nMethod: {}\nTarget: {}\nHeader fields (values hidden): {}\nBody: {body_bytes} UTF-8 bytes (text hidden)\nExact request SHA-256: {request_digest}\n\nWhen it runs, NovaVei sends this exact saved request from this computer. Header values and body text stay hidden to avoid disclosing credentials; compare the visible fields and request SHA-256 with the reviewed request before choosing OK.{save_notice}",
                native_cron_dialog_preview(&payload.method, 16),
                native_cron_dialog_preview(&payload.url, MAX_CRON_URL_CHARS),
                native_cron_header_names_preview(&payload.headers),
            ))
        }
        _ => Err("native confirmation requested for a non-dangerous Cron type".to_string()),
    }
}

fn native_cron_approval_for_upsert(
    input: &CronUpsertInput,
) -> Result<Option<NativeCronApproval>, String> {
    let kind = validate_cron_kind(&input.kind)?;
    if !cron_kind_is_dangerous(&kind) {
        return Ok(None);
    }
    let name = bounded_nonempty(&input.name, "Cron job name", MAX_CRON_NAME_CHARS)?;
    validate_schedule(&input.schedule)?;
    if let Some(id) = input.id.as_deref() {
        validate_record_id(id, "Cron job id")?;
    }
    let payload = normalize_cron_payload(&kind, input.payload.clone())?;
    let description = cron_native_confirmation_description(
        CronDangerousOperation::Save,
        &kind,
        &name,
        &input.schedule,
        &payload,
    )?;
    issue_native_cron_approval(CronDangerousOperation::Save, input, description).map(Some)
}

fn force_dangerous_cron_disabled(kind: &str, requested_enabled: bool) -> bool {
    if cron_kind_is_dangerous(kind) {
        false
    } else {
        requested_enabled
    }
}

fn normalize_cron_payload(kind: &str, payload: Value) -> Result<Value, String> {
    match kind {
        "prompt" => {
            let mut payload = serde_json::from_value::<PromptCronPayload>(payload)
                .map_err(|_| "prompt Cron payload is invalid".to_string())?;
            payload.prompt = bounded_bytes(&payload.prompt, "Cron prompt", MAX_CRON_PROMPT_BYTES)?;
            payload.workdir = canonical_optional_workdir(payload.workdir.as_deref())?;
            payload.provider_id = payload
                .provider_id
                .as_deref()
                .map(|value| bounded_nonempty(value, "provider id", 128))
                .transpose()?;
            payload.model = payload
                .model
                .as_deref()
                .map(|value| bounded_nonempty(value, "model", 256))
                .transpose()?;
            serde_json::to_value(payload).map_err(|_| "serialize prompt Cron payload".to_string())
        }
        "shell" => {
            let mut payload = serde_json::from_value::<ShellCronPayload>(payload)
                .map_err(|_| "shell Cron payload is invalid".to_string())?;
            payload.command =
                bounded_bytes(&payload.command, "Cron command", MAX_CRON_COMMAND_BYTES)?;
            payload.workdir = canonical_workdir(&payload.workdir)?;
            serde_json::to_value(payload).map_err(|_| "serialize shell Cron payload".to_string())
        }
        "http" => {
            let mut payload = serde_json::from_value::<HttpCronPayload>(payload)
                .map_err(|_| "HTTP Cron payload is invalid".to_string())?;
            if payload.url.chars().count() > MAX_CRON_URL_CHARS {
                return Err(format!(
                    "HTTP Cron URL exceeds {MAX_CRON_URL_CHARS} characters"
                ));
            }
            let url =
                Url::parse(&payload.url).map_err(|_| "HTTP Cron URL is invalid".to_string())?;
            validate_public_https_cron_target(&url)?;
            if !url.username().is_empty() || url.password().is_some() {
                return Err("HTTP Cron URL cannot contain credentials; use headers".to_string());
            }
            if url.fragment().is_some() {
                return Err("HTTP Cron URL cannot contain a fragment".to_string());
            }
            payload.url = url.to_string();
            payload.method = payload.method.trim().to_ascii_uppercase();
            let method = Method::from_bytes(payload.method.as_bytes())
                .map_err(|_| "HTTP Cron method is invalid".to_string())?;
            if !matches!(
                method,
                Method::GET
                    | Method::POST
                    | Method::PUT
                    | Method::PATCH
                    | Method::DELETE
                    | Method::HEAD
            ) {
                return Err("HTTP Cron method is not allowed".to_string());
            }
            if payload.headers.len() > MAX_CRON_HEADERS {
                return Err(format!(
                    "HTTP Cron has more than {MAX_CRON_HEADERS} headers"
                ));
            }
            for (name, value) in &payload.headers {
                if name.len() > MAX_CRON_HEADER_NAME_BYTES
                    || value.len() > MAX_CRON_HEADER_VALUE_BYTES
                {
                    return Err("HTTP Cron header exceeds its size limit".to_string());
                }
                let parsed_name = HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| "HTTP Cron contains an invalid header name".to_string())?;
                HeaderValue::from_str(value)
                    .map_err(|_| "HTTP Cron contains an invalid header value".to_string())?;
                if matches!(
                    parsed_name.as_str(),
                    "host" | "content-length" | "connection" | "transfer-encoding"
                ) {
                    return Err("HTTP Cron contains a reserved transport header".to_string());
                }
            }
            if payload
                .body
                .as_ref()
                .is_some_and(|body| body.len() > MAX_CRON_HTTP_BODY_BYTES)
            {
                return Err(format!(
                    "HTTP Cron body exceeds the {MAX_CRON_HTTP_BODY_BYTES} byte limit"
                ));
            }
            if !(1_000..=30_000).contains(&payload.timeout_ms) {
                return Err(
                    "HTTP Cron timeout must be between 1000 and 30000 milliseconds".to_string(),
                );
            }
            serde_json::to_value(payload).map_err(|_| "serialize HTTP Cron payload".to_string())
        }
        _ => Err("Cron type is invalid".to_string()),
    }
}

/// Validate only the text form of a Cron target before it is stored. Without
/// DNS resolution or per-connection address pinning, DNS rebinding must still
/// be handled at the HTTP connection layer.
fn validate_public_https_cron_target(url: &Url) -> Result<(), String> {
    if url.scheme() != "https" {
        return Err("HTTP Cron URL must use HTTPS".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "HTTP Cron URL must include a public host".to_string())?;
    if is_disallowed_cron_hostname(host) {
        return Err(
            "HTTP Cron URL must not target localhost, private, loopback, link-local, or unspecified network addresses"
                .to_string(),
        );
    }
    Ok(())
}

fn is_disallowed_cron_hostname(host: &str) -> bool {
    let normalized_host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if matches!(
        normalized_host.as_str(),
        "localhost" | "localhost.localdomain" | "ip6-localhost" | "ip6-loopback"
    ) || [
        ".localhost",
        ".local",
        ".localdomain",
        ".lan",
        ".home",
        ".internal",
    ]
    .iter()
    .any(|suffix| normalized_host.ends_with(suffix))
    {
        return true;
    }

    if let Ok(address) = normalized_host.parse::<Ipv4Addr>() {
        return is_disallowed_cron_ipv4(address);
    }
    if let Ok(address) = normalized_host.parse::<Ipv6Addr>() {
        return is_disallowed_cron_ipv6(address);
    }
    // A single-label name can be completed through a local DNS search suffix,
    // so it cannot be safely recognized as a public network target here.
    !normalized_host.contains('.')
}

fn is_disallowed_cron_ipv4(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    // "This" network (RFC 1122) is never a public Cron destination. Rust's
    // `Ipv4Addr::is_unspecified` only covers 0.0.0.0, so explicitly reject
    // the whole 0.0.0.0/8 range as well.
    octets[0] == 0
        || address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_unspecified()
        || address.is_broadcast()
        // CGNAT / shared address space (RFC 6598)
        || octets[0] == 100 && (64..=127).contains(&octets[1])
        // Documentation (RFC 5737)
        || matches!(
            (octets[0], octets[1], octets[2]),
            (192, 0, 2) | (198, 51, 100) | (203, 0, 113)
        )
        // Benchmarking (RFC 2544)
        || octets[0] == 198 && matches!(octets[1], 18 | 19)
        // Multicast and reserved
        || octets[0] >= 224
}

fn is_disallowed_cron_ipv6(address: Ipv6Addr) -> bool {
    address.is_loopback()
        || address.is_unspecified()
        || address.is_unicast_link_local()
        || address.is_unique_local()
        || address.is_multicast()
        // Deprecated site-local fec0::/10
        || {
            let segments = address.segments();
            (segments[0] & 0xffc0) == 0xfec0
        }
        // Documentation 2001:db8::/32
        || {
            let segments = address.segments();
            segments[0] == 0x2001 && segments[1] == 0x0db8
        }
        || address
            .to_ipv4_mapped()
            .is_some_and(is_disallowed_cron_ipv4)
}

fn cron_socket_addr_is_disallowed(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(v4) => is_disallowed_cron_ipv4(v4),
        std::net::IpAddr::V6(v6) => is_disallowed_cron_ipv6(v6),
    }
}

/// Resolve HTTPS Cron destinations and pin every request to verified public
/// socket addresses while preserving the original hostname for TLS SNI.
fn plan_http_cron_connection(
    url: &Url,
    resolve: impl Fn(&str) -> Result<Vec<std::net::SocketAddr>, String>,
) -> Result<Vec<std::net::SocketAddr>, String> {
    validate_public_https_cron_target(url)?;
    let host = url
        .host_str()
        .ok_or_else(|| "HTTP Cron URL must include a public host".to_string())?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "HTTP Cron URL must include a port".to_string())?;
    // Literal IPs are validated by hostname policy; DNS names need every answer.
    if host.parse::<std::net::IpAddr>().is_ok() {
        let address: std::net::SocketAddr = format!("{host}:{port}")
            .parse()
            .map_err(|_| "HTTP Cron URL host is invalid".to_string())?;
        if cron_socket_addr_is_disallowed(address.ip()) {
            return Err(
                "HTTP Cron URL must not target localhost, private, loopback, link-local, or unspecified network addresses"
                    .to_string(),
            );
        }
        return Ok(vec![address]);
    }
    let resolved = resolve(host)?;
    if resolved.is_empty() {
        return Err("HTTP Cron host could not be resolved".to_string());
    }
    for address in &resolved {
        if cron_socket_addr_is_disallowed(address.ip()) {
            return Err("HTTP Cron host resolved to a non-public network address".to_string());
        }
    }
    Ok(resolved)
}

fn encode_cron_payload(payload: &Value) -> Result<String, String> {
    let serialized = serde_json::to_string(payload)
        .map_err(|_| "serialize protected Cron payload".to_string())?;
    if crate::storage::is_portable() {
        return protect_portable_local_service_text(&serialized);
    }
    let mut wrapper = HashMap::new();
    wrapper.insert("cron".to_string(), json!({"credential": serialized}));
    let protected = protect_settings(&wrapper)
        .map_err(|_| "protect Cron payload with the current Windows account".to_string())?;
    serde_json::to_string(
        protected
            .get("cron")
            .ok_or_else(|| "protect Cron payload".to_string())?,
    )
    .map_err(|_| "serialize protected Cron payload".to_string())
}

fn decode_cron_payload(encoded: &str) -> Result<Value, String> {
    if crate::storage::is_portable() && is_portable_local_service_text(encoded) {
        let serialized = unprotect_portable_local_service_text(encoded)?;
        return serde_json::from_str(&serialized)
            .map_err(|_| "stored Cron payload is invalid".to_string());
    }
    // Very early LocalServices builds persisted a raw JSON payload. Accept it
    // only during the locked-down portable migration, which immediately
    // rewrites it as one authenticated encrypted field.
    if crate::storage::is_portable() {
        if let Ok(raw) = serde_json::from_str::<Value>(encoded) {
            if raw.get("credential").is_none() {
                return Ok(raw);
            }
        }
    }
    let wrapped: Value =
        serde_json::from_str(encoded).map_err(|_| "stored Cron payload is invalid".to_string())?;
    let mut protected = HashMap::new();
    protected.insert("cron".to_string(), wrapped);
    let plain = unprotect_settings(&protected).map_err(|_| {
        "stored Cron payload cannot be unlocked by this Windows account".to_string()
    })?;
    let serialized = plain
        .get("cron")
        .and_then(|value| value.get("credential"))
        .and_then(Value::as_str)
        .ok_or_else(|| "stored Cron payload is invalid".to_string())?;
    serde_json::from_str(serialized).map_err(|_| "stored Cron payload is invalid".to_string())
}

fn validate_schedule(schedule: &str) -> Result<(), String> {
    if schedule == "hourly" {
        return Ok(());
    }
    let parts = schedule.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        ["daily", hour, minute] => {
            parse_schedule_time(hour, minute)?;
            Ok(())
        }
        ["weekly", day, hour, minute] => {
            if day.len() != 1 || !matches!(day.parse::<u32>(), Ok(1..=7)) {
                return Err("weekly schedule day must be 1-7 (Monday-Sunday)".to_string());
            }
            parse_schedule_time(hour, minute)?;
            Ok(())
        }
        _ => Err("schedule must be hourly, daily:HH:MM, or weekly:D:HH:MM".to_string()),
    }
}

fn parse_schedule_time(hour: &str, minute: &str) -> Result<(u32, u32), String> {
    if hour.len() != 2
        || minute.len() != 2
        || !hour.bytes().all(|byte| byte.is_ascii_digit())
        || !minute.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("schedule time must use two-digit HH:MM".to_string());
    }
    let hour = hour
        .parse::<u32>()
        .map_err(|_| "schedule hour is invalid".to_string())?;
    let minute = minute
        .parse::<u32>()
        .map_err(|_| "schedule minute is invalid".to_string())?;
    if hour > 23 || minute > 59 {
        return Err("schedule time is outside 00:00-23:59".to_string());
    }
    Ok((hour, minute))
}

fn next_schedule_at(schedule: &str, after_ms: i64) -> Result<i64, String> {
    validate_schedule(schedule)?;
    if schedule == "hourly" {
        const HOUR_MS: i64 = 60 * 60 * 1_000;
        return Ok(after_ms - after_ms.rem_euclid(HOUR_MS) + HOUR_MS);
    }
    let after = match Local.timestamp_millis_opt(after_ms) {
        LocalResult::Single(value) => value,
        LocalResult::Ambiguous(first, second) => first.min(second),
        LocalResult::None => return Err("schedule reference time is invalid".to_string()),
    };
    let parts = schedule.split(':').collect::<Vec<_>>();
    let (wanted_day, hour, minute) = match parts.as_slice() {
        ["daily", hour, minute] => (
            None,
            parse_schedule_time(hour, minute)?.0,
            parse_schedule_time(hour, minute)?.1,
        ),
        ["weekly", day, hour, minute] => (
            Some(
                day.parse::<u32>()
                    .map_err(|_| "weekly schedule day is invalid".to_string())?,
            ),
            parse_schedule_time(hour, minute)?.0,
            parse_schedule_time(hour, minute)?.1,
        ),
        _ => return Err("schedule format is invalid".to_string()),
    };
    for offset in 0..=14_u64 {
        let Some(date) = after.date_naive().checked_add_days(Days::new(offset)) else {
            break;
        };
        if wanted_day.is_some_and(|day| date.weekday().number_from_monday() != day) {
            continue;
        }
        let Some(naive) = date.and_hms_opt(hour, minute, 0) else {
            continue;
        };
        let candidates = match Local.from_local_datetime(&naive) {
            LocalResult::Single(value) => vec![value],
            LocalResult::Ambiguous(first, second) => vec![first.min(second), first.max(second)],
            LocalResult::None => Vec::new(),
        };
        if let Some(candidate) = candidates.into_iter().find(|candidate| *candidate > after) {
            return Ok(candidate.timestamp_millis());
        }
    }
    Err("could not determine the next schedule time".to_string())
}

fn build_pinned_http_cron_client(
    builder: reqwest::ClientBuilder,
    timeout: StdDuration,
    host: &str,
    pinned: &[std::net::SocketAddr],
) -> Result<reqwest::Client, ()> {
    // DNS pinning only protects the direct connection. A proxy would receive
    // the original hostname and could resolve it independently, so Cron HTTP
    // work never inherits proxy configuration from the environment.
    builder
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .resolve_to_addrs(host, pinned)
        .build()
        .map_err(|_| ())
}

async fn execute_http_job(payload: &Value) -> Result<CronHttpResult, ()> {
    // Re-run the full write-time policy so legacy rows cannot bypass method,
    // header, credential, or timeout constraints that were added later.
    let normalized = normalize_cron_payload("http", payload.clone()).map_err(|_| ())?;
    let payload = serde_json::from_value::<HttpCronPayload>(normalized).map_err(|_| ())?;
    let url = Url::parse(&payload.url).map_err(|_| ())?;
    let host = url.host_str().ok_or(())?.to_string();
    let port = url.port_or_known_default().ok_or(())?;
    let pinned = plan_http_cron_connection(&url, |hostname| {
        use std::net::ToSocketAddrs;
        format!("{hostname}:{port}")
            .to_socket_addrs()
            .map(|iter| iter.collect())
            .map_err(|_| "HTTP Cron host could not be resolved".to_string())
    })
    .map_err(|_| ())?;
    let client = build_pinned_http_cron_client(
        reqwest::Client::builder(),
        StdDuration::from_millis(payload.timeout_ms),
        &host,
        &pinned,
    )?;
    let method = Method::from_bytes(payload.method.as_bytes()).map_err(|_| ())?;
    let mut request = client.request(method, &payload.url);
    for (name, value) in payload.headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| ())?;
        let value = HeaderValue::from_str(&value).map_err(|_| ())?;
        request = request.header(name, value);
    }
    if let Some(body) = payload.body {
        request = request.body(body);
    }
    let mut response = request.send().await.map_err(|_| ())?;
    let status = response.status();
    let mut bytes = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        let remaining = MAX_CRON_HTTP_RESPONSE_BYTES.saturating_sub(bytes.len());
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        bytes.extend_from_slice(&chunk);
        if bytes.len() == MAX_CRON_HTTP_RESPONSE_BYTES {
            truncated = true;
            break;
        }
    }
    Ok(CronHttpResult {
        status: Some(status.as_u16()),
        body: String::from_utf8_lossy(&bytes).into_owned(),
        truncated,
        success: status.is_success(),
    })
}

fn bound_cron_output_text(mut text: String) -> String {
    if text.len() > MAX_CRON_RUN_OUTPUT_BYTES {
        text.truncate(MAX_CRON_RUN_OUTPUT_BYTES);
        text.push_str("\n…[truncated]");
    }
    text
}

/// Stop a scheduled shell job and every process it spawned.
///
/// `taskkill` is resolved from `%SystemRoot%\System32` rather than by bare
/// program name: `CreateProcess` searches the application directory before
/// `PATH`, so a bare name would let a `taskkill.exe` dropped beside the
/// executable receive this termination request. Keep the direct kill as the
/// fallback whenever the system helper cannot be located or started, matching
/// the Agent/terminal shell path.
fn terminate_process_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let taskkill = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("System32")
            .join("taskkill.exe");
        let mut terminated_tree = false;
        if taskkill.is_file() {
            let pid = child.id();
            let mut killer = Command::new(taskkill);
            killer
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            configure_hidden_console_process(&mut killer);
            terminated_tree = killer.status().is_ok();
        }
        if !terminated_tree {
            let _ = child.kill();
        }
    }
    #[cfg(not(windows))]
    {
        let _ = child.kill();
    }
    let _ = child.try_wait();
}

fn join_reader_bounded(
    handle: Option<std::thread::JoinHandle<Vec<u8>>>,
    timeout: StdDuration,
) -> Vec<u8> {
    let Some(handle) = handle else {
        return Vec::new();
    };
    let started = Instant::now();
    loop {
        if handle.is_finished() {
            return handle.join().unwrap_or_default();
        }
        if started.elapsed() >= timeout {
            // Dropping a still-running join handle detaches the reader; bounded
            // completion is preferred over hanging the scheduler forever.
            return Vec::new();
        }
        std::thread::sleep(StdDuration::from_millis(10));
    }
}

fn execute_shell_job(payload: &Value) -> Result<(String, bool), String> {
    let payload = serde_json::from_value::<ShellCronPayload>(payload.clone())
        .map_err(|_| "shell Cron payload is invalid".to_string())?;
    let command = payload.command.trim().to_string();
    if command.is_empty() {
        return Err("shell Cron command cannot be empty".to_string());
    }
    let workdir = PathBuf::from(canonical_workdir(&payload.workdir)?);
    if !workdir.is_dir() {
        return Err("shell Cron workdir is not a directory".to_string());
    }

    let (program, args, shell_name) = if cfg!(windows) {
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
        )
    } else {
        ("sh", vec!["-lc".to_string(), command], "sh")
    };

    let started = Instant::now();
    let mut process = Command::new(program);
    process
        .args(args)
        .current_dir(&workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Match the Agent/terminal shell path: PowerShell is a console app and
    // would otherwise flash a black window for every scheduled Shell job.
    configure_hidden_console_process(&mut process);
    let mut child = process
        .spawn()
        .map_err(|error| format!("start {shell_name}: {error}"))?;

    let stdout_handle = child.stdout.take().map(|stdout| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 8192];
            let mut reader = stdout;
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if bytes.len() < MAX_CRON_RUN_OUTPUT_BYTES {
                            let remaining = MAX_CRON_RUN_OUTPUT_BYTES - bytes.len();
                            bytes.extend_from_slice(&buffer[..count.min(remaining)]);
                        }
                    }
                    Err(_) => break,
                }
            }
            bytes
        })
    });
    let stderr_handle = child.stderr.take().map(|stderr| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 8192];
            let mut reader = stderr;
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if bytes.len() < MAX_CRON_RUN_OUTPUT_BYTES {
                            let remaining = MAX_CRON_RUN_OUTPUT_BYTES - bytes.len();
                            bytes.extend_from_slice(&buffer[..count.min(remaining)]);
                        }
                    }
                    Err(_) => break,
                }
            }
            bytes
        })
    });

    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("wait shell: {error}"))?
        {
            break status;
        }
        if started.elapsed() >= StdDuration::from_millis(MAX_CRON_SHELL_TIMEOUT_MS) {
            timed_out = true;
            terminate_process_tree(&mut child);
            break child
                .wait()
                .map_err(|error| format!("reap timed-out shell: {error}"))?;
        }
        std::thread::sleep(StdDuration::from_millis(25));
    };

    let stdout = join_reader_bounded(stdout_handle, StdDuration::from_secs(2));
    let stderr = join_reader_bounded(stderr_handle, StdDuration::from_secs(2));
    let mut combined = String::new();
    if !stdout.is_empty() {
        combined.push_str(&String::from_utf8_lossy(&stdout));
    }
    if !stderr.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&String::from_utf8_lossy(&stderr));
    }
    if timed_out {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str("shell Cron timed out");
    }
    let success = status.success() && !timed_out;
    if !success && combined.trim().is_empty() {
        combined = if timed_out {
            "shell Cron timed out".to_string()
        } else {
            format!(
                "shell Cron exited with code {}",
                status.code().unwrap_or(-1)
            )
        };
    }
    Ok((bound_cron_output_text(combined), success))
}

/// Shell execution contains synchronous process polling, process-tree cleanup,
/// and blocking reader joins. Keep all of it off Tokio's async workers so a
/// long-running Cron command cannot stall unrelated scheduled work.
async fn execute_shell_job_in_worker(payload: Value) -> Result<(String, bool), String> {
    tauri::async_runtime::spawn_blocking(move || execute_shell_job(&payload))
        .await
        .map_err(|_| "Shell Cron worker stopped unexpectedly".to_string())?
}

/// Same Windows console-window policy as the main shell tool path.
#[cfg(windows)]
fn configure_hidden_console_process(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_hidden_console_process(_command: &mut Command) {}

fn load_cron_provider_settings(data_root: &Path) -> Result<HashMap<String, Value>, String> {
    let history_path = data_root.join(HISTORY_DATABASE_FILE);
    if !history_path.exists() {
        return Err("no provider settings are available for Cron prompt jobs".to_string());
    }
    let store = HistoryStore::new(history_path);
    let raw = store
        .load_settings()
        .map_err(|_| "provider settings could not be loaded for Cron prompt jobs".to_string())?;
    unprotect_settings(&raw)
        .map_err(|_| "provider settings could not be unlocked for Cron prompt jobs".to_string())
}

fn cron_provider_is_enabled(object: &Map<String, Value>) -> bool {
    object.get("enabled").and_then(Value::as_bool) != Some(false)
}

fn cron_provider_record<'a>(
    settings: &'a HashMap<String, Value>,
    provider_id: Option<&str>,
) -> Result<&'a Map<String, Value>, String> {
    let providers = settings
        .get("providers")
        .ok_or_else(|| "no provider is configured for Cron prompt jobs".to_string())?;
    let candidates: Vec<&'a Map<String, Value>> = match providers {
        Value::Array(items) => items.iter().filter_map(Value::as_object).collect(),
        Value::Object(items) => items.values().filter_map(Value::as_object).collect(),
        _ => Vec::new(),
    };
    if candidates.is_empty() {
        return Err("no provider is configured for Cron prompt jobs".to_string());
    }
    if let Some(requested) = provider_id.map(str::trim).filter(|value| !value.is_empty()) {
        let provider = candidates
            .into_iter()
            .find(|record| {
                record
                    .get("id")
                    .or_else(|| record.get("providerId"))
                    .and_then(Value::as_str)
                    == Some(requested)
            })
            .ok_or_else(|| format!("provider is not configured: {requested}"))?;
        if !cron_provider_is_enabled(provider) {
            return Err(format!(
                "provider is disabled for Cron prompt jobs: {requested}"
            ));
        }
        return Ok(provider);
    }
    candidates
        .iter()
        .copied()
        .filter(|record| cron_provider_is_enabled(record))
        .find(|record| {
            record
                .get("default")
                .or_else(|| record.get("isDefault"))
                .and_then(Value::as_bool)
                == Some(true)
        })
        .or_else(|| {
            candidates
                .iter()
                .copied()
                .find(|record| cron_provider_is_enabled(record))
        })
        .ok_or_else(|| "no enabled provider is configured for Cron prompt jobs".to_string())
}

fn cron_provider_protocol(object: &Map<String, Value>) -> String {
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
enum CronProviderAuthFamily {
    OpenAi,
    Anthropic,
    Gemini,
}

impl CronProviderAuthFamily {
    fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
        }
    }
}

fn cron_provider_auth_family(object: &Map<String, Value>) -> CronProviderAuthFamily {
    let protocol = cron_provider_protocol(object);
    if protocol.contains("anthropic") || protocol.contains("claude") {
        CronProviderAuthFamily::Anthropic
    } else if protocol.contains("gemini") || protocol.contains("google") {
        CronProviderAuthFamily::Gemini
    } else {
        CronProviderAuthFamily::OpenAi
    }
}

/// Keep Cron's credential context equivalent to the native provider runtime:
/// the endpoint includes its path and query, while a fragment is cosmetic and
/// deliberately excluded from credential identity.
fn cron_provider_endpoint_url(object: &Map<String, Value>) -> Option<Url> {
    let family = cron_provider_auth_family(object);
    let raw = object
        .get("baseUrl")
        .or_else(|| object.get("base_url"))
        .or_else(|| object.get("endpoint"))
        .or_else(|| object.get("url"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| match family {
            CronProviderAuthFamily::Anthropic => "https://api.anthropic.com".to_string(),
            CronProviderAuthFamily::Gemini => {
                "https://generativelanguage.googleapis.com/v1beta".to_string()
            }
            CronProviderAuthFamily::OpenAi => "https://api.openai.com/v1".to_string(),
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

fn cron_provider_endpoint_origin(object: &Map<String, Value>) -> Option<String> {
    let url = cron_provider_endpoint_url(object)?;
    Some(url.origin().ascii_serialization().to_ascii_lowercase())
}

fn cron_provider_credential_endpoint(object: &Map<String, Value>) -> Option<String> {
    let mut url = cron_provider_endpoint_url(object)?;
    url.set_fragment(None);
    Some(url.to_string())
}

fn cron_provider_credential_context(
    object: &Map<String, Value>,
) -> Option<(String, CronProviderAuthFamily)> {
    Some((
        cron_provider_credential_endpoint(object)?,
        cron_provider_auth_family(object),
    ))
}

/// Legacy providers have no binding and remain compatible. Once a binding is
/// present, fail closed unless the current endpoint and auth family exactly
/// match it. This prevents a saved key from being sent to another tenant path,
/// endpoint, or protocol family through Prompt Cron.
fn cron_provider_secret_binding_matches(object: &Map<String, Value>) -> bool {
    let Some(binding) = object.get(CRON_PROVIDER_CREDENTIAL_BINDING_KEY) else {
        return true;
    };
    let Some(binding) = binding.as_object() else {
        return false;
    };
    let Some(current) = cron_provider_credential_context(object) else {
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
    // Older records were bound only to an origin. Preserve their compatibility
    // while still requiring that exact origin for any Cron credential read.
    binding.get("origin").and_then(Value::as_str)
        == cron_provider_endpoint_origin(object).as_deref()
}

fn cron_provider_api_key(object: &Map<String, Value>) -> Option<String> {
    [
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
    .map(str::to_string)
}

fn cron_provider_base_url(object: &Map<String, Value>, protocol: &str) -> Result<String, String> {
    let raw = object
        .get("baseUrl")
        .or_else(|| object.get("base_url"))
        .or_else(|| object.get("endpoint"))
        .or_else(|| object.get("url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if protocol.contains("anthropic") || protocol.contains("claude") {
                "https://api.anthropic.com".to_string()
            } else if protocol.contains("gemini") || protocol.contains("google") {
                "https://generativelanguage.googleapis.com/v1beta".to_string()
            } else {
                "https://api.openai.com/v1".to_string()
            }
        });
    let mut url = Url::parse(&raw).map_err(|_| "provider URL is invalid".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("provider URL must be an absolute http(s) URL".to_string());
    }
    let mut path = url.path().trim_end_matches('/').to_string();
    for suffix in [
        "/chat/completions",
        "/responses",
        "/messages",
        "/generateContent",
        "/streamGenerateContent",
        "/models",
    ] {
        if path.ends_with(suffix) {
            path.truncate(path.len() - suffix.len());
            break;
        }
    }
    if protocol.contains("anthropic") || protocol.contains("claude") {
        if !path.ends_with("/v1") {
            path.push_str("/v1");
        }
        path.push_str("/messages");
    } else if protocol.contains("gemini") || protocol.contains("google") {
        if !path.ends_with("/v1") && !path.ends_with("/v1beta") {
            path.push_str("/v1beta");
        }
    } else {
        if !path.ends_with("/v1") {
            path.push_str("/v1");
        }
        path.push_str("/chat/completions");
    }
    url.set_path(&path);
    Ok(url.to_string())
}

fn cron_model_id(value: &Value) -> Option<&str> {
    value
        .as_str()
        .or_else(|| {
            value.as_object().and_then(|model| {
                model
                    .get("id")
                    .or_else(|| model.get("modelId"))
                    .or_else(|| model.get("model_id"))
                    .or_else(|| model.get("name"))
                    .and_then(Value::as_str)
            })
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn cron_model_values(object: &Map<String, Value>) -> Vec<&Value> {
    let mut values = [
        "models",
        "modelList",
        "model_list",
        "activeModels",
        "active_models",
    ]
    .iter()
    .filter_map(|key| object.get(*key).and_then(Value::as_array))
    .flat_map(|models| models.iter())
    .collect::<Vec<_>>();
    // Some legacy settings store the default as a full model object instead of
    // repeating it in `models`; include those records when evaluating a
    // disabled flag or the active-model allowlist.
    values.extend(
        ["model", "defaultModel", "default_model"]
            .iter()
            .filter_map(|key| object.get(*key)),
    );
    values
}

fn cron_active_model_ids(object: &Map<String, Value>) -> Vec<&str> {
    ["activeModels", "active_models"]
        .iter()
        .filter_map(|key| object.get(*key).and_then(Value::as_array))
        .flat_map(|models| models.iter())
        .filter_map(cron_model_id)
        .collect()
}

/// `activeModels` is the persisted enabled-model set in the Settings editor.
/// Keep unknown explicitly requested ids compatible with existing provider
/// configurations, but never select a known inactive model or a model marked
/// `enabled: false` in its own record.
fn cron_model_is_disabled(object: &Map<String, Value>, model: &str) -> bool {
    let values = cron_model_values(object);
    let known = values
        .iter()
        .copied()
        .filter(|value| cron_model_id(value) == Some(model))
        .collect::<Vec<_>>();
    if known.iter().any(|value| {
        value
            .as_object()
            .and_then(|record| record.get("enabled"))
            .and_then(Value::as_bool)
            == Some(false)
    }) {
        return true;
    }
    let active = cron_active_model_ids(object);
    !known.is_empty() && !active.is_empty() && !active.contains(&model)
}

fn cron_enabled_model(object: &Map<String, Value>, model: &str) -> Result<String, String> {
    if cron_model_is_disabled(object, model) {
        return Err(format!("model is disabled for Cron prompt jobs: {model}"));
    }
    Ok(model.to_string())
}

fn cron_provider_model(
    object: &Map<String, Value>,
    requested: Option<&str>,
) -> Result<String, String> {
    if let Some(model) = requested.map(str::trim).filter(|value| !value.is_empty()) {
        return cron_enabled_model(object, model);
    }

    // Preserve the existing preferred-default order, but skip disabled values
    // rather than silently using a disabled model.
    for key in ["model", "defaultModel", "default_model"] {
        let Some(model) = object.get(key).and_then(cron_model_id) else {
            continue;
        };
        if !cron_model_is_disabled(object, model) {
            return Ok(model.to_string());
        }
    }

    for model in cron_active_model_ids(object) {
        if !cron_model_is_disabled(object, model) {
            return Ok(model.to_string());
        }
    }

    for value in cron_model_values(object) {
        let Some(model) = cron_model_id(value) else {
            continue;
        };
        if !cron_model_is_disabled(object, model) {
            return Ok(model.to_string());
        }
    }

    Err("Cron prompt job has no enabled model configured".to_string())
}

fn extract_completion_text(value: &Value) -> Option<String> {
    if let Some(text) = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
    {
        let text = text.trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    if let Some(parts) = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_array)
    {
        let joined = parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| part.as_str())
            })
            .collect::<Vec<_>>()
            .join("");
        let joined = joined.trim();
        if !joined.is_empty() {
            return Some(joined.to_string());
        }
    }
    if let Some(blocks) = value.get("content").and_then(Value::as_array) {
        let joined = blocks
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    block.get("text").and_then(Value::as_str)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");
        let joined = joined.trim();
        if !joined.is_empty() {
            return Some(joined.to_string());
        }
    }
    if let Some(candidates) = value.get("candidates").and_then(Value::as_array) {
        let joined = candidates
            .iter()
            .filter_map(|candidate| candidate.pointer("/content/parts"))
            .filter_map(Value::as_array)
            .flat_map(|parts| parts.iter())
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("");
        let joined = joined.trim();
        if !joined.is_empty() {
            return Some(joined.to_string());
        }
    }
    None
}

async fn execute_prompt_job(data_root: &Path, payload: &Value) -> Result<(String, bool), String> {
    let payload = serde_json::from_value::<PromptCronPayload>(payload.clone())
        .map_err(|_| "prompt Cron payload is invalid".to_string())?;
    let prompt = payload.prompt.trim();
    if prompt.is_empty() {
        return Err("prompt Cron text cannot be empty".to_string());
    }
    let settings = load_cron_provider_settings(data_root)?;
    let provider = cron_provider_record(&settings, payload.provider_id.as_deref())?;
    let protocol = cron_provider_protocol(provider);
    if !cron_provider_secret_binding_matches(provider) {
        return Err(
            "Cron prompt provider credentials are not valid for the configured endpoint"
                .to_string(),
        );
    }
    let model = cron_provider_model(provider, payload.model.as_deref())?;
    let api_key = cron_provider_api_key(provider)
        .ok_or_else(|| "Cron prompt provider has no credentials".to_string())?;
    let use_system_proxy = provider
        .get("useSystemProxy")
        .or_else(|| provider.get("use_system_proxy"))
        .or_else(|| provider.get("systemProxy"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut client_builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(StdDuration::from_millis(MAX_CRON_PROMPT_TIMEOUT_MS));
    if !use_system_proxy {
        client_builder = client_builder.no_proxy();
    }
    let client = client_builder
        .build()
        .map_err(|_| "Cron prompt client could not be initialized".to_string())?;

    let (url, request_body, headers) =
        if protocol.contains("anthropic") || protocol.contains("claude") {
            let url = cron_provider_base_url(provider, &protocol)?;
            let body = json!({
                "model": model,
                "max_tokens": 1024,
                "messages": [{
                    "role": "user",
                    "content": prompt,
                }],
            });
            let headers = vec![
                ("x-api-key".to_string(), api_key),
                ("anthropic-version".to_string(), "2023-06-01".to_string()),
                ("content-type".to_string(), "application/json".to_string()),
            ];
            (url, body, headers)
        } else if protocol.contains("gemini") || protocol.contains("google") {
            let mut base = cron_provider_base_url(provider, &protocol)?;
            if base.ends_with("/chat/completions") {
                base.truncate(base.len() - "/chat/completions".len());
            }
            let model_id = model.strip_prefix("models/").unwrap_or(&model);
            let url = format!(
                "{}/models/{}:generateContent",
                base.trim_end_matches('/'),
                model_id
            );
            let body = json!({
                "contents": [{
                    "role": "user",
                    "parts": [{ "text": prompt }],
                }],
            });
            let headers = vec![
                ("x-goog-api-key".to_string(), api_key),
                ("content-type".to_string(), "application/json".to_string()),
            ];
            (url, body, headers)
        } else {
            let url = cron_provider_base_url(provider, &protocol)?;
            let body = json!({
                "model": model,
                "messages": [{
                    "role": "user",
                    "content": prompt,
                }],
                "stream": false,
                "n": MAX_CRON_PROMPT_MESSAGES,
            });
            let headers = vec![
                ("authorization".to_string(), format!("Bearer {api_key}")),
                ("content-type".to_string(), "application/json".to_string()),
            ];
            (url, body, headers)
        };

    let mut request = client
        .post(&url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, "NovaVei/0.1");
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| "Cron prompt has an invalid credential header".to_string())?;
        let header_value = HeaderValue::from_str(&value)
            .map_err(|_| "Cron prompt has an invalid credential header".to_string())?;
        request = request.header(header_name, header_value);
    }
    let mut response = request
        .json(&request_body)
        .send()
        .await
        .map_err(|_| "Cron prompt request failed".to_string())?;
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Cron prompt response failed".to_string())?
    {
        let remaining = MAX_CRON_RUN_OUTPUT_BYTES.saturating_sub(bytes.len());
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            break;
        }
        bytes.extend_from_slice(&chunk);
        if bytes.len() >= MAX_CRON_RUN_OUTPUT_BYTES {
            break;
        }
    }
    if !status.is_success() {
        return Err(format!(
            "Cron prompt provider returned status {}",
            status.as_u16()
        ));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Cron prompt provider returned an invalid response".to_string())?;
    let text = extract_completion_text(&value)
        .ok_or_else(|| "Cron prompt provider returned no completion text".to_string())?;
    Ok((bound_cron_output_text(text), true))
}

/// Perform one bounded provider completion for one-click translation. The
/// caller has already resolved the provider record and model from native
/// settings; this function applies the same credential binding, redirect,
/// timeout, and response bounds as Prompt Cron without ever returning the
/// underlying credentials or provider error details to the renderer.
pub(crate) async fn translate_provider_text(
    provider: &Map<String, Value>,
    model: &str,
    system_instruction: &str,
    user_text: &str,
) -> Result<String, String> {
    if user_text.trim().is_empty() {
        return Err("translation text cannot be empty".to_string());
    }
    if user_text.chars().count() > MAX_TRANSLATION_INPUT_CHARS {
        return Err("translation text is too long".to_string());
    }
    if !cron_provider_secret_binding_matches(provider) {
        return Err(
            "translation provider credentials are not valid for the configured endpoint"
                .to_string(),
        );
    }
    let protocol = cron_provider_protocol(provider);
    let api_key = cron_provider_api_key(provider)
        .ok_or_else(|| "translation provider has no credentials".to_string())?;
    let use_system_proxy = provider
        .get("useSystemProxy")
        .or_else(|| provider.get("use_system_proxy"))
        .or_else(|| provider.get("systemProxy"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut client_builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(StdDuration::from_millis(MAX_TRANSLATION_TIMEOUT_MS));
    if !use_system_proxy {
        client_builder = client_builder.no_proxy();
    }
    let client = client_builder
        .build()
        .map_err(|_| "translation client could not be initialized".to_string())?;

    let (url, request_body, headers) =
        if protocol.contains("anthropic") || protocol.contains("claude") {
            let url = cron_provider_base_url(provider, &protocol)?;
            let body = json!({
                "model": model,
                // MCP registry descriptions can be multi-kilobyte; keep headroom
                // for CJK expansions without silently truncating the translation.
                "max_tokens": 4096,
                "system": system_instruction,
                "messages": [{
                    "role": "user",
                    "content": user_text,
                }],
            });
            let headers = vec![
                ("x-api-key".to_string(), api_key),
                ("anthropic-version".to_string(), "2023-06-01".to_string()),
                ("content-type".to_string(), "application/json".to_string()),
            ];
            (url, body, headers)
        } else if protocol.contains("gemini") || protocol.contains("google") {
            let mut base = cron_provider_base_url(provider, &protocol)?;
            if base.ends_with("/chat/completions") {
                base.truncate(base.len() - "/chat/completions".len());
            }
            let model_id = model.strip_prefix("models/").unwrap_or(model);
            let url = format!(
                "{}/models/{}:generateContent",
                base.trim_end_matches('/'),
                model_id
            );
            let body = json!({
                "system_instruction": { "parts": [{ "text": system_instruction }] },
                "contents": [{
                    "role": "user",
                    "parts": [{ "text": user_text }],
                }],
            });
            let headers = vec![
                ("x-goog-api-key".to_string(), api_key),
                ("content-type".to_string(), "application/json".to_string()),
            ];
            (url, body, headers)
        } else {
            let url = cron_provider_base_url(provider, &protocol)?;
            let body = json!({
                "model": model,
                "messages": [
                    { "role": "system", "content": system_instruction },
                    { "role": "user", "content": user_text },
                ],
                "stream": false,
                "n": 1,
            });
            let headers = vec![
                ("authorization".to_string(), format!("Bearer {api_key}")),
                ("content-type".to_string(), "application/json".to_string()),
            ];
            (url, body, headers)
        };

    let mut request = client
        .post(&url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, "NovaVei/0.1");
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| "translation has an invalid credential header".to_string())?;
        let header_value = HeaderValue::from_str(&value)
            .map_err(|_| "translation has an invalid credential header".to_string())?;
        request = request.header(header_name, header_value);
    }
    let mut response = request
        .json(&request_body)
        .send()
        .await
        .map_err(|_| "translation request failed".to_string())?;
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "translation response failed".to_string())?
    {
        let remaining = MAX_TRANSLATION_RESPONSE_BYTES.saturating_sub(bytes.len());
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            break;
        }
        bytes.extend_from_slice(&chunk);
        if bytes.len() >= MAX_TRANSLATION_RESPONSE_BYTES {
            break;
        }
    }
    if !status.is_success() {
        return Err(format!(
            "translation provider returned status {}",
            status.as_u16()
        ));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "translation provider returned an invalid response".to_string())?;
    let mut text = extract_completion_text(&value)
        .ok_or_else(|| "translation provider returned no completion text".to_string())?;
    if text.chars().count() > MAX_TRANSLATION_OUTPUT_CHARS {
        text = text.chars().take(MAX_TRANSLATION_OUTPUT_CHARS).collect();
        text.push_str("\n…[truncated]");
    }
    Ok(text)
}

// ---------------------------------------------------------------------------
// Tauri command surface

#[tauri::command(rename_all = "camelCase")]
pub fn skills_list(services: State<'_, Arc<LocalServices>>) -> Result<SkillsListResponse, String> {
    services.list_skills()
}

#[tauri::command(rename_all = "camelCase")]
pub fn skills_read(
    services: State<'_, Arc<LocalServices>>,
    name: String,
) -> Result<SkillReadResponse, String> {
    services.read_skill(&name)
}

/// Agent-only Skill discovery. Settings continues to use `skills_list` so a
/// user can manage disabled Skills without making them Agent-visible.
#[tauri::command(rename_all = "camelCase")]
pub fn agent_skills_list(
    services: State<'_, Arc<LocalServices>>,
) -> Result<SkillsListResponse, String> {
    services.list_agent_skills()
}

/// Agent-only Skill read. This command deliberately rejects disabled Skills
/// before opening their instruction file.
#[tauri::command(rename_all = "camelCase")]
pub fn agent_skills_read(
    services: State<'_, Arc<LocalServices>>,
    name: String,
) -> Result<SkillReadResponse, String> {
    services.read_agent_skill(&name)
}

#[tauri::command(rename_all = "camelCase")]
pub fn skills_enable(
    services: State<'_, Arc<LocalServices>>,
    name: String,
) -> Result<SkillEnabledResponse, String> {
    services.set_skill_enabled(&name, true)
}

#[tauri::command(rename_all = "camelCase")]
pub fn skills_disable(
    services: State<'_, Arc<LocalServices>>,
    name: String,
) -> Result<SkillEnabledResponse, String> {
    services.set_skill_enabled(&name, false)
}

/// Import one Skill directory selected by the user. Renderer-provided paths
/// are never accepted as an installation grant.
#[tauri::command(rename_all = "camelCase")]
pub async fn skills_install_pick(
    services: State<'_, Arc<LocalServices>>,
) -> Result<Option<SkillInstallResponse>, String> {
    let services = Arc::clone(services.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let selected = rfd::FileDialog::new()
            .set_title("Install a NovaVei Skill")
            .pick_folder();
        selected
            .as_deref()
            .map(|source| services.install_skill_from_directory(source))
            .transpose()
    })
    .await
    .map_err(|_| "native Skill picker did not complete".to_string())?
}

/// Browse only the fixed public ClawHub catalog. The renderer cannot select a
/// registry host, redirect target, or archive URL.
#[tauri::command(rename_all = "camelCase")]
pub async fn skills_catalog_list(
    services: State<'_, Arc<LocalServices>>,
    limit: Option<usize>,
    cursor: Option<String>,
    sort: Option<String>,
) -> Result<SkillsCatalogListResponse, String> {
    Arc::clone(services.inner())
        .skills_catalog_list(limit, cursor, sort)
        .await
}

/// Search only the fixed public ClawHub catalog, with suspicious results
/// filtered by the registry before they reach the renderer.
#[tauri::command(rename_all = "camelCase")]
pub async fn skills_catalog_search(
    services: State<'_, Arc<LocalServices>>,
    query: String,
    limit: Option<usize>,
) -> Result<SkillsCatalogListResponse, String> {
    Arc::clone(services.inner())
        .skills_catalog_search(&query, limit)
        .await
}

/// Inspect one ClawHub Skill and its exact version metadata. Owner-qualified
/// references are required for installation; bare slugs may return safe
/// disambiguation choices for catalog browsing.
#[tauri::command(rename_all = "camelCase")]
pub async fn skills_catalog_detail(
    services: State<'_, Arc<LocalServices>>,
    reference: String,
    version: Option<String>,
) -> Result<SkillsCatalogDetailResponse, String> {
    Arc::clone(services.inner())
        .skills_catalog_detail(&reference, version)
        .await
}

/// Install a previously reviewed exact ClawHub version only after native
/// security checks, per-file size/hash verification, and an explicit matching
/// confirmation token. It never follows remote archive or GitHub handoff URLs.
#[tauri::command(rename_all = "camelCase")]
pub async fn skills_catalog_install(
    services: State<'_, Arc<LocalServices>>,
    input: SkillsCatalogInstallInput,
) -> Result<SkillsCatalogInstallResponse, String> {
    Arc::clone(services.inner())
        .skills_catalog_install(input)
        .await
}

#[tauri::command(rename_all = "camelCase")]
pub fn memory_create(
    services: State<'_, Arc<LocalServices>>,
    input: MemoryCreateInput,
) -> Result<MemoryEntry, String> {
    services.memory_create(input)
}

#[tauri::command(rename_all = "camelCase")]
pub fn memory_read(
    services: State<'_, Arc<LocalServices>>,
    id: String,
    workdir: Option<String>,
) -> Result<MemoryEntry, String> {
    services.memory_get(&id, workdir.as_deref())
}

#[tauri::command(rename_all = "camelCase")]
pub fn memory_list(
    services: State<'_, Arc<LocalServices>>,
    filter: Option<MemoryFilter>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<MemoryListResponse, String> {
    services.memory_list(filter.unwrap_or_default(), limit, offset)
}

#[tauri::command(rename_all = "camelCase")]
pub fn memory_update(
    services: State<'_, Arc<LocalServices>>,
    input: MemoryUpdateInput,
) -> Result<MemoryEntry, String> {
    services.memory_update(input)
}

#[tauri::command(rename_all = "camelCase")]
pub fn memory_delete(
    services: State<'_, Arc<LocalServices>>,
    id: String,
    workdir: Option<String>,
) -> Result<MemoryEntry, String> {
    services.memory_delete(&id, workdir.as_deref())
}

#[tauri::command(rename_all = "camelCase")]
pub fn memory_search(
    services: State<'_, Arc<LocalServices>>,
    query: String,
    filter: Option<MemoryFilter>,
    limit: Option<usize>,
) -> Result<MemorySearchResponse, String> {
    services.memory_search(&query, filter.unwrap_or_default(), limit)
}

#[tauri::command(rename_all = "camelCase")]
pub fn memory_stats(
    services: State<'_, Arc<LocalServices>>,
    filter: Option<MemoryFilter>,
) -> Result<MemoryStats, String> {
    services.memory_stats(filter.unwrap_or_default())
}

#[tauri::command(rename_all = "camelCase")]
pub fn memory_clear(
    services: State<'_, Arc<LocalServices>>,
    input: MemoryClearInput,
) -> Result<MemoryClearResponse, String> {
    services.memory_clear(input)
}

#[tauri::command(rename_all = "camelCase")]
pub fn memory_organize(
    services: State<'_, Arc<LocalServices>>,
    input: MemoryOrganizeInput,
) -> Result<MemoryOrganizeResponse, String> {
    services.memory_organize(input)
}

/// Export is intentionally picker-only. The renderer cannot nominate an
/// arbitrary filesystem destination.
#[tauri::command(rename_all = "camelCase")]
pub async fn memory_export(
    services: State<'_, Arc<LocalServices>>,
    filter: Option<MemoryFilter>,
    format: String,
) -> Result<Option<MemoryExportResponse>, String> {
    let services = Arc::clone(services.inner());
    let format = format.trim().to_ascii_lowercase();
    tauri::async_runtime::spawn_blocking(move || {
        let (bytes, entries) = services.memory_export_data(filter.unwrap_or_default(), &format)?;
        let (extension, label) = match format.as_str() {
            "json" => ("json", "JSON"),
            "markdown" => ("md", "Markdown"),
            _ => return Err("memory export format must be json or markdown".to_string()),
        };
        let selected = rfd::FileDialog::new()
            .set_title("Export NovaVei Memory")
            .set_file_name(format!("novavei-memory.{extension}"))
            .add_filter(label, &[extension])
            .save_file();
        let Some(path) = selected else {
            return Ok(None);
        };
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .map_err(|error| format!("create memory export: {error}"))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("write memory export: {error}"))?;
        Ok(Some(MemoryExportResponse {
            path: path_for_display(&path),
            format,
            entries,
            bytes: bytes.len(),
        }))
    })
    .await
    .map_err(|_| "native memory export picker did not complete".to_string())?
}

/// Usage reports contain only aggregate counters and use a native save picker;
/// the renderer cannot provide a destination path or receive memory content.
#[tauri::command(rename_all = "camelCase")]
pub async fn memory_usage_export(
    services: State<'_, Arc<LocalServices>>,
    filter: Option<MemoryFilter>,
) -> Result<Option<MemoryUsageExportResponse>, String> {
    let services = Arc::clone(services.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let (bytes, stats) = services.memory_usage_report_data(filter.unwrap_or_default())?;
        let selected = rfd::FileDialog::new()
            .set_title("Export NovaVei Memory Usage Report")
            .set_file_name("novavei-memory-usage.md")
            .add_filter("Markdown", &["md"])
            .save_file();
        let Some(path) = selected else {
            return Ok(None);
        };
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .map_err(|error| format!("create memory usage report: {error}"))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("write memory usage report: {error}"))?;
        Ok(Some(MemoryUsageExportResponse {
            path: path_for_display(&path),
            format: "markdown".to_string(),
            bytes: bytes.len(),
            week_started_at: stats.week_started_at,
        }))
    })
    .await
    .map_err(|_| "native memory usage report picker did not complete".to_string())?
}

/// Register a knowledge-base root through an operating-system folder picker.
/// The renderer supplies no path, so it cannot turn an Agent request into a
/// filesystem grant.
#[tauri::command(rename_all = "camelCase")]
pub async fn knowledge_base_pick_folder(
    services: State<'_, Arc<LocalServices>>,
) -> Result<Option<KnowledgeBaseIndexResult>, String> {
    let services = Arc::clone(services.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let selected = rfd::FileDialog::new()
            .set_title("Add a NovaVei Knowledge Base Folder")
            .pick_folder();
        selected
            .as_deref()
            .map(|source| services.knowledge_base_add_folder(source))
            .transpose()
    })
    .await
    .map_err(|_| "native knowledge base picker did not complete".to_string())?
}

#[tauri::command(rename_all = "camelCase")]
pub fn knowledge_base_list(
    services: State<'_, Arc<LocalServices>>,
) -> Result<KnowledgeBaseListResponse, String> {
    services.knowledge_base_list()
}

#[tauri::command(rename_all = "camelCase")]
pub fn knowledge_base_set_enabled(
    services: State<'_, Arc<LocalServices>>,
    enabled: bool,
    consent: Option<KnowledgeBaseConsent>,
) -> Result<KnowledgeBaseListResponse, String> {
    services.knowledge_base_set_enabled(enabled, consent)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn knowledge_base_refresh(
    services: State<'_, Arc<LocalServices>>,
    folder_id: String,
) -> Result<KnowledgeBaseIndexResult, String> {
    let services = Arc::clone(services.inner());
    tauri::async_runtime::spawn_blocking(move || services.knowledge_base_refresh(&folder_id))
        .await
        .map_err(|_| "knowledge base refresh did not complete".to_string())?
}

/// Removing a root removes only NovaVei's local index and permission record;
/// it never deletes user files in the selected folder.
#[tauri::command(rename_all = "camelCase")]
pub fn knowledge_base_remove(
    services: State<'_, Arc<LocalServices>>,
    folder_id: String,
) -> Result<KnowledgeBaseFolder, String> {
    services.knowledge_base_remove(&folder_id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn knowledge_base_search(
    services: State<'_, Arc<LocalServices>>,
    query: String,
    folder_ids: Option<Vec<String>>,
    limit: Option<usize>,
) -> Result<KnowledgeBaseSearchResponse, String> {
    services.knowledge_base_search(&query, folder_ids, limit)
}

#[tauri::command(rename_all = "camelCase")]
pub fn knowledge_base_read(
    services: State<'_, Arc<LocalServices>>,
    document_id: String,
    max_chars: Option<usize>,
) -> Result<KnowledgeBaseDocument, String> {
    services.knowledge_base_read(&document_id, max_chars)
}

/// Mint an opaque, short-lived budget for the current interactive turn. It is
/// intentionally not an Agent tool: only the renderer's native run setup can
/// capture and pass it to the two bounded knowledge tools.
#[tauri::command(rename_all = "camelCase")]
pub fn knowledge_base_agent_begin(
    services: State<'_, Arc<LocalServices>>,
    turn_id: String,
    provider_id: String,
    model_id: String,
) -> Result<String, String> {
    services.knowledge_base_agent_begin(&turn_id, &provider_id, &model_id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn knowledge_base_agent_search(
    services: State<'_, Arc<LocalServices>>,
    access_token: String,
    query: String,
    limit: Option<usize>,
) -> Result<KnowledgeBaseSearchResponse, String> {
    services.knowledge_base_agent_search(&access_token, &query, limit)
}

#[tauri::command(rename_all = "camelCase")]
pub fn knowledge_base_agent_read(
    services: State<'_, Arc<LocalServices>>,
    access_token: String,
    document_id: String,
    max_chars: Option<usize>,
) -> Result<KnowledgeBaseDocument, String> {
    services.knowledge_base_agent_read(&access_token, &document_id, max_chars)
}

#[tauri::command(rename_all = "camelCase")]
pub fn cron_schedule_validate(schedule: String) -> Result<(), String> {
    validate_schedule(&schedule)
}

#[tauri::command(rename_all = "camelCase")]
pub fn cron_list(services: State<'_, Arc<LocalServices>>) -> Result<Vec<CronJobSummary>, String> {
    services
        .cron_list()
        .map(|jobs| jobs.into_iter().map(Into::into).collect())
}

/// Create or replace a Cron job. Payload values are accepted only for this
/// write operation and are never returned; editing an existing job requires
/// the user to supply its payload again.
#[tauri::command(rename_all = "camelCase")]
pub fn cron_upsert(
    services: State<'_, Arc<LocalServices>>,
    input: CronUpsertInput,
) -> Result<CronJobSummary, String> {
    let approval = native_cron_approval_for_upsert(&input)?;
    services.cron_upsert(input, approval).map(Into::into)
}

#[tauri::command(rename_all = "camelCase")]
pub fn cron_set_enabled(
    services: State<'_, Arc<LocalServices>>,
    id: String,
    enabled: bool,
) -> Result<CronJobSummary, String> {
    let approval = if enabled {
        services.native_cron_approval_for_existing_job(&id, CronDangerousOperation::Enable)?
    } else {
        None
    };
    services
        .cron_set_enabled(&id, enabled, approval)
        .map(Into::into)
}

#[tauri::command(rename_all = "camelCase")]
pub fn cron_delete(
    services: State<'_, Arc<LocalServices>>,
    id: String,
) -> Result<CronJobSummary, String> {
    services.cron_delete(&id).map(Into::into)
}

#[tauri::command(rename_all = "camelCase")]
pub fn cron_runs(
    services: State<'_, Arc<LocalServices>>,
    job_id: String,
    limit: Option<usize>,
) -> Result<Vec<CronRunSummary>, String> {
    services
        .cron_runs(&job_id, limit)
        .map(|runs| runs.into_iter().map(Into::into).collect())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn cron_run_now(
    services: State<'_, Arc<LocalServices>>,
    job_id: String,
) -> Result<CronRunNowSummary, String> {
    let services = Arc::clone(services.inner());
    let approval =
        services.native_cron_approval_for_existing_job(&job_id, CronDangerousOperation::RunNow)?;
    services
        .cron_run_now(&job_id, approval)
        .await
        .map(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "novavei-local-services-{label}-{}-{}",
                std::process::id(),
                Uuid::new_v4()
            ));
            fs::create_dir_all(&path).expect("create test directory");
            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_skill(path: &Path, name: &str, description: &str) {
        fs::create_dir_all(path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n# Instructions\n"),
        )
        .unwrap();
    }

    /// Simulate the private capability created after a native dialog succeeds.
    /// Tests that are not about approval mechanics use this deliberately rather
    /// than accidentally depending on the old renderer-bypass behavior.
    fn confirmed_test_cron_upsert(service: &LocalServices, input: CronUpsertInput) -> CronJob {
        let approval = test_native_cron_approval(CronDangerousOperation::Save, &input);
        service
            .cron_upsert(input, Some(approval))
            .expect("a test-native Save approval should persist this Cron job")
    }

    fn enabled_test_cron(service: &LocalServices, job: &CronJob) -> CronJob {
        let approval = test_native_cron_approval(CronDangerousOperation::Enable, job);
        service
            .cron_set_enabled(&job.id, true, Some(approval))
            .expect("a test-native Enable approval should activate this Cron job")
    }

    fn security_gate_from_lock_flag(locked: Arc<AtomicBool>) -> LocalServicesSecurityGate {
        Arc::new(move || {
            if locked.load(Ordering::Acquire) {
                Err("application password is required".to_string())
            } else {
                Ok(())
            }
        })
    }

    #[test]
    fn initialization_is_reentrant_and_seeds_builtins() {
        let root = TestDirectory::new("initialize");
        let service = LocalServices::for_test(root.path.clone());
        service.initialize().unwrap();
        service.initialize().unwrap();

        let skills = service.list_skills().unwrap();
        assert_eq!(skills.invalid.len(), 0);
        assert!(skills
            .skills
            .iter()
            .any(|skill| skill.name == "skills-creator" && skill.built_in));
        assert!(skills
            .skills
            .iter()
            .any(|skill| skill.name == "skills-installer" && skill.built_in));
        assert!(service.database_path().is_file());
    }

    #[test]
    fn security_gate_defers_initialization_and_blocks_knowledge_base_reads() {
        let root = TestDirectory::new("security-gate-locked");
        let locked = Arc::new(AtomicBool::new(true));
        let service = LocalServices::for_test_with_security_gate(
            root.path.clone(),
            security_gate_from_lock_flag(Arc::clone(&locked)),
        );

        assert!(!service.database_path().exists());
        assert_eq!(
            service.initialize().unwrap_err(),
            "application password is required"
        );
        assert_eq!(
            service
                .knowledge_base_search("deployment", None, Some(1))
                .unwrap_err(),
            "application password is required"
        );
        assert!(!service.database_path().exists());

        locked.store(false, Ordering::Release);
        assert!(service.knowledge_base_list().is_ok());
        assert!(service.database_path().is_file());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn security_gate_blocks_manual_cron_run_now() {
        let root = TestDirectory::new("security-gate-cron");
        let locked = Arc::new(AtomicBool::new(false));
        let service = LocalServices::for_test_with_security_gate(
            root.path.clone(),
            security_gate_from_lock_flag(Arc::clone(&locked)),
        );
        let job = confirmed_test_cron_upsert(
            &service,
            CronUpsertInput {
                id: None,
                name: "Locked manual run".to_string(),
                kind: "prompt".to_string(),
                schedule: "hourly".to_string(),
                payload: json!({"prompt": "This must not run while locked."}),
                enabled: false,
            },
        );

        locked.store(true, Ordering::Release);
        assert_eq!(
            service.cron_run_now(&job.id, None).await.unwrap_err(),
            "application password is required"
        );

        locked.store(false, Ordering::Release);
        assert!(service.cron_runs(&job.id, None).unwrap().is_empty());
    }

    #[test]
    fn portable_search_matcher_preserves_ascii_and_unicode_case_behavior() {
        for (haystack, query) in [
            ("Release NOTES", "notes"),
            ("resume.txt", "SUME"),
            ("résumé", "RÉSUMÉ"),
            ("İstanbul", "i"),
            ("Straße", "STRASSE"),
            ("中文搜索", "搜索"),
            ("no match", "z"),
        ] {
            let lowered_query = query.to_lowercase();
            assert_eq!(
                local_service_text_contains_case_insensitive(haystack, &lowered_query),
                haystack.to_lowercase().contains(&lowered_query),
                "unexpected match result for {haystack:?} / {query:?}",
            );
        }
    }

    #[test]
    fn skill_install_stages_validates_and_rejects_conflicts() {
        let root = TestDirectory::new("skill-install");
        let source_parent = TestDirectory::new("skill-source");
        let source = source_parent.path.join("focused-review");
        write_skill(&source, "focused-review", "Review one bounded change.");
        fs::create_dir(source.join("references")).unwrap();
        fs::write(source.join("references").join("checklist.md"), "check").unwrap();

        let service = LocalServices::for_test(root.path.clone());
        let installed = service.install_skill_from_directory(&source).unwrap();
        assert_eq!(installed.skill.name, "focused-review");
        assert_eq!(installed.skill.file_count, 2);
        assert!(service.install_skill_from_directory(&source).is_err());
        assert!(
            !service
                .set_skill_enabled("focused-review", false)
                .unwrap()
                .enabled
        );
        assert!(!service.read_skill("focused-review").unwrap().skill.enabled);
    }

    #[test]
    fn agent_skill_capabilities_hide_disabled_skills_but_settings_can_manage_them() {
        let root = TestDirectory::new("agent-skill-visibility");
        let source_parent = TestDirectory::new("agent-skill-source");
        let source = source_parent.path.join("disabled-review");
        write_skill(
            &source,
            "disabled-review",
            "Must not be exposed to an Agent.",
        );

        let service = LocalServices::for_test(root.path.clone());
        service.install_skill_from_directory(&source).unwrap();
        service.set_skill_enabled("disabled-review", false).unwrap();

        // Settings retains the disabled item and may inspect it to re-enable it.
        assert!(service
            .list_skills()
            .unwrap()
            .skills
            .iter()
            .any(|skill| skill.name == "disabled-review" && !skill.enabled));
        assert!(!service.read_skill("disabled-review").unwrap().skill.enabled);

        let agent_skills = service.list_agent_skills().unwrap();
        assert!(!agent_skills
            .skills
            .iter()
            .any(|skill| skill.name == "disabled-review"));

        // A disabled Skill must be rejected before its directory or
        // instruction file is examined. This makes disabling a Skill a
        // reliable containment switch even if the on-disk instructions are
        // subsequently corrupt or otherwise fail validation.
        let disabled_skill_file = service.skills_root.join("disabled-review").join("SKILL.md");
        fs::write(&disabled_skill_file, "not valid Skill instructions").unwrap();
        let disabled_error = service.read_agent_skill("disabled-review").unwrap_err();
        assert!(disabled_error.contains("disabled"));
        assert!(!disabled_error.contains("SKILL.md"));

        write_skill(
            disabled_skill_file.parent().unwrap(),
            "disabled-review",
            "Must not be exposed to an Agent.",
        );

        service.set_skill_enabled("disabled-review", true).unwrap();
        assert!(service
            .read_agent_skill("disabled-review")
            .unwrap()
            .content
            .contains("Instructions"));
    }

    #[test]
    fn cron_prompt_provider_selection_rejects_disabled_providers() {
        let settings = HashMap::from([(
            "providers".to_string(),
            json!([
                { "id": "disabled", "enabled": false, "default": true },
                { "id": "enabled", "enabled": true },
            ]),
        )]);

        assert!(cron_provider_record(&settings, Some("disabled"))
            .unwrap_err()
            .contains("disabled"));
        let default = cron_provider_record(&settings, None).unwrap();
        assert_eq!(default.get("id").and_then(Value::as_str), Some("enabled"));
    }

    #[test]
    fn portable_full_text_cleanup_removes_plaintext_shadow_tables() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE VIRTUAL TABLE memory_fts USING fts5(id UNINDEXED, title, content); \
                 CREATE VIRTUAL TABLE knowledge_base_fts USING fts5(document_id UNINDEXED, content);",
            )
            .unwrap();
        disable_portable_full_text_indexes(&connection).unwrap();
        for table in ["memory_fts", "knowledge_base_fts"] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                    params![table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(!exists, "{table} must not retain portable plaintext");
        }
    }

    #[test]
    fn cron_prompt_model_selection_rejects_disabled_models_and_skips_them_by_default() {
        let provider = serde_json::from_value::<Map<String, Value>>(json!({
            "models": [
                { "id": "explicitly-disabled", "enabled": false },
                "inactive-model",
                "enabled-model",
            ],
            "activeModels": ["enabled-model"],
            "defaultModel": "inactive-model",
        }))
        .unwrap();

        assert!(cron_provider_model(&provider, Some("explicitly-disabled"))
            .unwrap_err()
            .contains("disabled"));
        assert!(cron_provider_model(&provider, Some("inactive-model"))
            .unwrap_err()
            .contains("disabled"));
        assert_eq!(
            cron_provider_model(&provider, None).unwrap(),
            "enabled-model"
        );
    }

    #[test]
    fn cron_prompt_credentials_require_a_matching_endpoint_and_auth_family_binding() {
        let provider = serde_json::from_value::<Map<String, Value>>(json!({
            "type": "codex",
            "baseUrl": "https://gateway.example/tenant/v1#ui-fragment",
            "__credentialBinding": {
                "endpoint": "https://gateway.example/tenant/v1",
                "authFamily": "openai",
            },
        }))
        .unwrap();
        assert!(cron_provider_secret_binding_matches(&provider));

        let endpoint_mismatch = serde_json::from_value::<Map<String, Value>>(json!({
            "type": "codex",
            "baseUrl": "https://gateway.example/other-tenant/v1",
            "__credentialBinding": {
                "endpoint": "https://gateway.example/tenant/v1",
                "authFamily": "openai",
            },
        }))
        .unwrap();
        assert!(!cron_provider_secret_binding_matches(&endpoint_mismatch));

        let auth_family_mismatch = serde_json::from_value::<Map<String, Value>>(json!({
            "type": "gemini",
            "baseUrl": "https://gateway.example/tenant/v1",
            "__credentialBinding": {
                "endpoint": "https://gateway.example/tenant/v1",
                "authFamily": "openai",
            },
        }))
        .unwrap();
        assert!(!cron_provider_secret_binding_matches(&auth_family_mismatch));

        let legacy_origin_binding = serde_json::from_value::<Map<String, Value>>(json!({
            "type": "codex",
            "baseUrl": "https://gateway.example/tenant/v1",
            "__credentialBinding": {
                "origin": "https://gateway.example",
                "authFamily": "openai",
            },
        }))
        .unwrap();
        assert!(cron_provider_secret_binding_matches(&legacy_origin_binding));

        let legacy_without_binding = serde_json::from_value::<Map<String, Value>>(json!({
            "type": "codex",
            "baseUrl": "https://gateway.example/tenant/v1",
        }))
        .unwrap();
        assert!(cron_provider_secret_binding_matches(
            &legacy_without_binding
        ));

        let malformed_binding = serde_json::from_value::<Map<String, Value>>(json!({
            "type": "codex",
            "baseUrl": "https://gateway.example/tenant/v1",
            "__credentialBinding": true,
        }))
        .unwrap();
        assert!(!cron_provider_secret_binding_matches(&malformed_binding));
    }

    #[test]
    fn skill_validation_rejects_name_mismatch_and_oversized_file() {
        let source_parent = TestDirectory::new("skill-validation");
        let mismatch = source_parent.path.join("directory-name");
        write_skill(&mismatch, "different-name", "Mismatch should fail.");
        assert!(validate_skill_directory(&mismatch)
            .unwrap_err()
            .contains("exactly match"));

        let oversized = source_parent.path.join("oversized-skill");
        write_skill(&oversized, "oversized-skill", "Oversized file should fail.");
        let file = fs::File::create(oversized.join("large.bin")).unwrap();
        file.set_len(MAX_SKILL_FILE_BYTES + 1).unwrap();
        assert!(validate_skill_directory(&oversized)
            .unwrap_err()
            .contains("byte limit"));
    }

    #[test]
    fn clawhub_reference_url_keeps_owner_out_of_the_path_segments() {
        let reference = parse_catalog_reference("@steipete/weather", true).unwrap();
        let mut query = clawhub_owner_query(&reference);
        query.push(("path".to_string(), "references/checklist.md".to_string()));
        let url = clawhub_api_url(&["skills", reference.slug.as_str(), "file"], &query).unwrap();

        assert_eq!(url.path(), "/api/v1/skills/weather/file");
        let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(pairs.get("ownerHandle"), Some(&"steipete".to_string()));
        assert_eq!(
            pairs.get("path"),
            Some(&"references/checklist.md".to_string())
        );
        assert!(!url.path().contains("@steipete"));
        assert_eq!(
            url.path_segments().unwrap().collect::<Vec<_>>(),
            vec!["api", "v1", "skills", "weather", "file"]
        );
    }

    #[test]
    fn clawhub_catalog_rejects_unsafe_paths_and_requires_skill_metadata() {
        let valid = ClawHubFile {
            path: "SKILL.md".to_string(),
            size: 42,
            sha256: "a".repeat(64),
        };
        assert!(validate_catalog_files(vec![valid.clone()]).is_ok());
        let traversal = ClawHubFile {
            path: "../SKILL.md".to_string(),
            ..valid.clone()
        };
        assert!(validate_catalog_files(vec![traversal])
            .unwrap_err()
            .contains("path"));
        let no_skill_file = ClawHubFile {
            path: "notes.md".to_string(),
            ..valid.clone()
        };
        assert!(validate_catalog_files(vec![no_skill_file])
            .unwrap_err()
            .contains("SKILL.md"));
        let bad_hash = ClawHubFile {
            sha256: "not-a-sha256".to_string(),
            ..valid
        };
        assert!(validate_catalog_files(vec![bad_hash])
            .unwrap_err()
            .contains("SHA-256"));
    }

    #[test]
    fn sha256_hex_preserves_lowercase_64_character_digest_format() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn clawhub_catalog_verifies_exact_file_hashes_and_reports_redirects() {
        let bytes = b"verified Skill file";
        let file = ClawHubFile {
            path: "SKILL.md".to_string(),
            size: bytes.len() as u64,
            sha256: sha256_hex(bytes),
        };
        assert!(verify_catalog_file(&file, bytes).is_ok());
        assert!(verify_catalog_file(&file, b"changed Skill file")
            .unwrap_err()
            .contains("size"));
        let bad_hash = ClawHubFile {
            sha256: "0".repeat(64),
            ..file
        };
        assert!(verify_catalog_file(&bad_hash, bytes)
            .unwrap_err()
            .contains("SHA-256"));
        assert!(clawhub_http_error(302).contains("redirect"));
        assert!(clawhub_http_error(429).contains("429"));
    }

    #[test]
    fn clawhub_catalog_requires_clean_security_without_high_risk_scanners() {
        let clean = ClawHubSecurity {
            status: Some("clean".to_string()),
            has_warnings: true,
            scanners: Some(ClawHubScanners {
                vt: Some(ClawHubScanner {
                    status: Some("clean".to_string()),
                    verdict: Some("benign".to_string()),
                    normalized_status: Some("clean".to_string()),
                    severity: None,
                    recommendation: None,
                    summary: Some("No blocking result".to_string()),
                    risk_summary: None,
                }),
                skillspector: Some(ClawHubScanner {
                    status: Some("clean".to_string()),
                    verdict: None,
                    normalized_status: Some("clean".to_string()),
                    severity: Some("low".to_string()),
                    recommendation: Some("SAFE".to_string()),
                    summary: None,
                    risk_summary: None,
                }),
                llm: None,
            }),
        };
        assert!(catalog_security_summary(Some(&clean)).installable);

        let high_risk = ClawHubSecurity {
            scanners: Some(ClawHubScanners {
                skillspector: Some(ClawHubScanner {
                    status: Some("clean".to_string()),
                    verdict: None,
                    normalized_status: Some("clean".to_string()),
                    severity: Some("HIGH".to_string()),
                    recommendation: Some("DO_NOT_INSTALL".to_string()),
                    summary: Some("High risk".to_string()),
                    risk_summary: Some(json!({ "riskLevel": "high" })),
                }),
                ..clean.scanners.clone().unwrap()
            }),
            ..clean
        };
        let summary = catalog_security_summary(Some(&high_risk));
        assert!(!summary.installable);
        assert!(summary
            .install_block_reason
            .as_deref()
            .is_some_and(|value| value.contains("high-risk")));
    }

    #[test]
    fn clawhub_catalog_confirmation_is_bound_to_the_exact_reference_and_version() {
        let expected = catalog_install_confirmation("@steipete/weather", "1.0.0");
        assert_eq!(expected, "INSTALL_CLAWHUB_SKILL:@steipete/weather@1.0.0");
        assert_ne!(
            expected,
            catalog_install_confirmation("@steipete/weather", "1.0.1")
        );
        assert!(parse_catalog_reference("weather", true).is_err());
        assert_eq!(
            parse_catalog_reference("@steipete/weather", true)
                .unwrap()
                .owner_handle,
            Some("steipete".to_string())
        );
    }

    #[test]
    fn project_memory_requires_exact_workdir_and_like_search_falls_back() {
        let root = TestDirectory::new("memory-scope");
        let workspace = TestDirectory::new("workspace-a");
        let other_workspace = TestDirectory::new("workspace-b");
        let service = LocalServices::for_test(root.path.clone());
        service.fts_available.store(false, Ordering::Release);

        let global = service
            .memory_create(MemoryCreateInput {
                scope: "global".to_string(),
                workdir: None,
                kind: "user".to_string(),
                title: "Editor preference".to_string(),
                content: "Use compact diffs".to_string(),
            })
            .unwrap();
        let project = service
            .memory_create(MemoryCreateInput {
                scope: "project".to_string(),
                workdir: Some(workspace.path.display().to_string()),
                kind: "project".to_string(),
                title: "Build target".to_string(),
                content: "Use the shared E drive target".to_string(),
            })
            .unwrap();

        assert!(service
            .memory_get(
                &project.id,
                Some(&other_workspace.path.display().to_string())
            )
            .is_err());
        assert_eq!(service.memory_get(&global.id, None).unwrap().id, global.id);
        let search = service
            .memory_search(
                "shared E drive",
                MemoryFilter {
                    scope: None,
                    workdir: Some(workspace.path.display().to_string()),
                },
                None,
            )
            .unwrap();
        assert_eq!(search.backend, "like");
        assert_eq!(search.items.len(), 1);
        assert_eq!(search.items[0].id, project.id);
        let hidden = service
            .memory_list(
                MemoryFilter {
                    scope: None,
                    workdir: Some(other_workspace.path.display().to_string()),
                },
                None,
                None,
            )
            .unwrap();
        assert_eq!(hidden.total, 1);
    }

    #[test]
    fn memory_stats_and_organize_report_real_rows() {
        let root = TestDirectory::new("memory-organize");
        let service = LocalServices::for_test(root.path.clone());
        for _ in 0..2 {
            service
                .memory_create(MemoryCreateInput {
                    scope: "global".to_string(),
                    workdir: None,
                    kind: "reference".to_string(),
                    title: "Same".to_string(),
                    content: "Exact duplicate".to_string(),
                })
                .unwrap();
        }
        let dry_run = service
            .memory_organize(MemoryOrganizeInput {
                scope: Some("global".to_string()),
                workdir: None,
                dry_run: true,
            })
            .unwrap();
        assert_eq!(dry_run.inspected, 2);
        assert_eq!(dry_run.duplicate_entries, 1);
        assert_eq!(dry_run.removed, 0);

        let applied = service
            .memory_organize(MemoryOrganizeInput {
                scope: Some("global".to_string()),
                workdir: None,
                dry_run: false,
            })
            .unwrap();
        assert_eq!(applied.removed, 1);
        let stats = service
            .memory_stats(MemoryFilter {
                scope: Some("global".to_string()),
                workdir: None,
            })
            .unwrap();
        assert_eq!(stats.total_entries, 1);
        assert_eq!(stats.by_type[0].entries, 1);
        assert_eq!(
            stats.total_bytes,
            ("Same".len() + "Exact duplicate".len()) as u64
        );
    }

    #[test]
    fn memory_usage_counts_successful_native_operations_and_reports_capacity() {
        let root = TestDirectory::new("memory-usage");
        let workspace = TestDirectory::new("memory-usage-workspace");
        let service = LocalServices::for_test(root.path.clone());
        let global = service
            .memory_create(MemoryCreateInput {
                scope: "global".to_string(),
                workdir: None,
                kind: "user".to_string(),
                title: "Global entry".to_string(),
                content: "Searchable global content".to_string(),
            })
            .unwrap();
        service
            .memory_create(MemoryCreateInput {
                scope: "project".to_string(),
                workdir: Some(workspace.path.display().to_string()),
                kind: "project".to_string(),
                title: "Project entry".to_string(),
                content: "Searchable project content".to_string(),
            })
            .unwrap();
        service
            .memory_update(MemoryUpdateInput {
                id: global.id,
                workdir: None,
                kind: None,
                title: Some("Updated global entry".to_string()),
                content: None,
            })
            .unwrap();
        service
            .memory_search(
                "global",
                MemoryFilter {
                    scope: Some("global".to_string()),
                    workdir: None,
                },
                None,
            )
            .unwrap();
        service
            .memory_search(
                "Searchable",
                MemoryFilter {
                    scope: None,
                    workdir: Some(workspace.path.display().to_string()),
                },
                None,
            )
            .unwrap();

        let global_stats = service
            .memory_stats(MemoryFilter {
                scope: Some("global".to_string()),
                workdir: None,
            })
            .unwrap();
        assert_eq!(global_stats.weekly_searches, 1);
        assert_eq!(global_stats.weekly_writes, 2);
        assert_eq!(global_stats.capacity.used_entries, 1);
        assert_eq!(
            global_stats.capacity.max_entries,
            MAX_MEMORY_ENTRIES_PER_SCOPE as usize
        );
        assert!(global_stats.tracking_started_at >= global_stats.week_started_at);

        let combined_filter = MemoryFilter {
            scope: None,
            workdir: Some(workspace.path.display().to_string()),
        };
        let combined_stats = service.memory_stats(combined_filter.clone()).unwrap();
        assert_eq!(combined_stats.total_entries, 2);
        assert_eq!(combined_stats.weekly_searches, 2);
        assert_eq!(combined_stats.weekly_writes, 3);
        assert_eq!(
            combined_stats.capacity.max_entries,
            (MAX_MEMORY_ENTRIES_PER_SCOPE as usize) * 2
        );
        assert_eq!(combined_stats.capacity.remaining_entries, 9_998);
        let (report, report_stats) = service.memory_usage_report_data(combined_filter).unwrap();
        let report = String::from_utf8(report).unwrap();
        assert_eq!(report_stats.weekly_searches, 2);
        assert!(report.contains("Successful searches: 2"));
        assert!(report.contains("Writes count successful native memory create and update"));
        assert!(!report.contains("Searchable global content"));
    }

    #[test]
    fn memory_clear_requires_confirmation_and_deletes_only_the_exact_scope() {
        let root = TestDirectory::new("memory-clear");
        let workspace = TestDirectory::new("memory-clear-workspace");
        let service = LocalServices::for_test(root.path.clone());
        service
            .memory_create(MemoryCreateInput {
                scope: "global".to_string(),
                workdir: None,
                kind: "reference".to_string(),
                title: "Global only".to_string(),
                content: "Remove this global memory".to_string(),
            })
            .unwrap();
        let project = service
            .memory_create(MemoryCreateInput {
                scope: "project".to_string(),
                workdir: Some(workspace.path.display().to_string()),
                kind: "project".to_string(),
                title: "Project survives".to_string(),
                content: "Keep this project memory".to_string(),
            })
            .unwrap();

        assert!(service
            .memory_clear(MemoryClearInput {
                scope: "global".to_string(),
                workdir: None,
                confirmation: "CLEAR_PROJECT_MEMORY".to_string(),
            })
            .unwrap_err()
            .contains("explicit scope confirmation"));
        let cleared = service
            .memory_clear(MemoryClearInput {
                scope: "global".to_string(),
                workdir: None,
                confirmation: "CLEAR_GLOBAL_MEMORY".to_string(),
            })
            .unwrap();
        assert_eq!(cleared.removed, 1);
        assert!(cleared.reclaimed_bytes > 0);
        assert_eq!(
            service
                .memory_list(
                    MemoryFilter {
                        scope: Some("global".to_string()),
                        workdir: None,
                    },
                    None,
                    None,
                )
                .unwrap()
                .total,
            0
        );
        assert_eq!(
            service
                .memory_get(&project.id, Some(&workspace.path.display().to_string()))
                .unwrap()
                .id,
            project.id
        );
    }

    #[test]
    fn knowledge_base_indexes_only_safe_text_and_revokes_empty_root_consent() {
        let data_root = TestDirectory::new("knowledge-base-data");
        let source_root = TestDirectory::new("knowledge-base-source");
        fs::write(
            source_root.path.join("guide.md"),
            "Release handbook: use the stable deployment checklist.",
        )
        .unwrap();
        fs::create_dir_all(source_root.path.join("notes")).unwrap();
        fs::write(
            source_root.path.join("notes").join("runbook.txt"),
            "Incident runbook for service recovery.",
        )
        .unwrap();
        // Sensitive, unsupported, and ignored build-tree files must never
        // reach the local index or a model-visible search result.
        fs::write(source_root.path.join(".env"), "API_TOKEN=not-indexed").unwrap();
        fs::write(source_root.path.join("archive.pdf"), "%PDF-not-indexed").unwrap();
        fs::create_dir_all(source_root.path.join("target")).unwrap();
        fs::write(
            source_root.path.join("target").join("generated.md"),
            "not-indexed build output",
        )
        .unwrap();

        let service = LocalServices::for_test(data_root.path.clone());
        let indexed = service
            .knowledge_base_add_folder(&source_root.path)
            .unwrap();
        assert_eq!(indexed.indexed_files, 2);
        assert!(indexed.skipped_files >= 2);
        assert!(!service.knowledge_base_list().unwrap().enabled);
        assert_eq!(
            service
                .knowledge_base_search("stable deployment", None, Some(10))
                .unwrap()
                .items
                .len(),
            1
        );
        assert!(service
            .knowledge_base_agent_begin(
                "turn-before-consent",
                "reviewed-provider",
                "reviewed-model"
            )
            .is_err());

        let enabled = service
            .knowledge_base_set_enabled(
                true,
                Some(KnowledgeBaseConsent {
                    provider_id: "reviewed-provider".to_string(),
                    model_id: "reviewed-model".to_string(),
                }),
            )
            .unwrap();
        assert!(enabled.enabled);
        assert_eq!(enabled.folders.len(), 1);
        let grant = service
            .knowledge_base_agent_begin("turn-after-consent", "reviewed-provider", "reviewed-model")
            .unwrap();
        let search = service
            .knowledge_base_agent_search(&grant, "stable deployment", Some(10))
            .unwrap();
        assert_eq!(search.items.len(), 1);
        assert_eq!(search.items[0].relative_path, "guide.md");
        assert!(service
            .knowledge_base_agent_search(&grant, "not-indexed", Some(10))
            .unwrap()
            .items
            .is_empty());
        let document = service
            .knowledge_base_agent_read(&grant, &search.items[0].document_id, Some(24))
            .unwrap();
        assert!(document.content.contains("Release handbook"));
        assert!(document.truncated);

        // Adding a source changes the user's approved source set and must
        // revoke both consent and any already-issued turn grant.
        let added_root = TestDirectory::new("knowledge-base-added-source");
        fs::write(added_root.path.join("release.md"), "new source").unwrap();
        let added = service.knowledge_base_add_folder(&added_root.path).unwrap();
        assert!(!service.knowledge_base_list().unwrap().enabled);
        assert!(service
            .knowledge_base_agent_search(&grant, "stable deployment", Some(10))
            .is_err());
        service.knowledge_base_remove(&added.folder.id).unwrap();

        service.knowledge_base_remove(&indexed.folder.id).unwrap();
        let after_remove = service.knowledge_base_list().unwrap();
        assert!(after_remove.folders.is_empty());
        assert!(!after_remove.enabled);
        assert!(service
            .knowledge_base_agent_read(&grant, &search.items[0].document_id, Some(24))
            .is_err());
    }

    #[test]
    fn knowledge_base_allows_truly_empty_and_unsupported_only_folders() {
        let data_root = TestDirectory::new("knowledge-base-empty-data");
        let empty_root = TestDirectory::new("knowledge-base-empty-source");
        let service = LocalServices::for_test(data_root.path.clone());

        let empty = service.knowledge_base_add_folder(&empty_root.path).unwrap();
        assert_eq!(empty.indexed_files, 0);
        assert_eq!(empty.skipped_files, 0);

        fs::write(empty_root.path.join("archive.pdf"), "%PDF-unsupported").unwrap();
        let unsupported = service.knowledge_base_refresh(&empty.folder.id).unwrap();
        assert_eq!(unsupported.indexed_files, 0);
        assert_eq!(unsupported.skipped_files, 1);
    }

    #[cfg(windows)]
    #[test]
    fn knowledge_base_refresh_keeps_old_index_when_every_candidate_fails() {
        use std::os::windows::fs::OpenOptionsExt;

        let data_root = TestDirectory::new("knowledge-base-failed-refresh-data");
        let source_root = TestDirectory::new("knowledge-base-failed-refresh-source");
        let guide = source_root.path.join("guide.md");
        fs::write(&guide, "last known good deployment guide").unwrap();
        let service = LocalServices::for_test(data_root.path.clone());
        let indexed = service
            .knowledge_base_add_folder(&source_root.path)
            .unwrap();
        assert_eq!(indexed.indexed_files, 1);

        // Deny sharing while the refresh runs so the supported candidate is
        // discoverable but cannot pass the checked no-follow open.
        let locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&guide)
            .unwrap();
        let error = service
            .knowledge_base_refresh(&indexed.folder.id)
            .expect_err("an all-failed scan must not commit an empty replacement");
        assert!(error.contains("scan failed"));

        let folder = service
            .knowledge_base_list()
            .unwrap()
            .folders
            .into_iter()
            .find(|folder| folder.id == indexed.folder.id)
            .unwrap();
        assert_eq!(folder.document_count, 1);
        assert_eq!(
            service
                .knowledge_base_search("last known good", None, Some(10))
                .unwrap()
                .items
                .len(),
            1
        );
        drop(locked);
    }

    #[test]
    fn schedule_parser_is_strict_and_computes_future_times() {
        assert!(validate_schedule("hourly").is_ok());
        assert!(validate_schedule("daily:09:05").is_ok());
        assert!(validate_schedule("weekly:7:23:59").is_ok());
        assert!(validate_schedule("daily:9:05").is_err());
        assert!(validate_schedule("weekly:0:09:05").is_err());
        assert!(validate_schedule("0 * * * *").is_err());
        let reference = now_ms();
        assert!(next_schedule_at("hourly", reference).unwrap() > reference);
        assert!(next_schedule_at("daily:09:05", reference).unwrap() > reference);
    }

    #[test]
    fn cron_http_connection_plan_rejects_private_dns_answers() {
        let url = Url::parse("https://public.example/task").unwrap();
        let private =
            plan_http_cron_connection(&url, |_| Ok(vec!["127.0.0.1:443".parse().unwrap()]));
        assert!(private.unwrap_err().contains("non-public"));

        let mixed = plan_http_cron_connection(&url, |_| {
            Ok(vec![
                "8.8.8.8:443".parse().unwrap(),
                "10.0.0.1:443".parse().unwrap(),
            ])
        });
        assert!(mixed.unwrap_err().contains("non-public"));

        let public =
            plan_http_cron_connection(&url, |_| Ok(vec!["8.8.8.8:443".parse().unwrap()])).unwrap();
        assert_eq!(public.len(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cron_http_client_bypasses_an_explicit_proxy() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        const HOST: &str = "cron-proxy-test.invalid";
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

        let client = build_pinned_http_cron_client(
            reqwest::Client::builder()
                .proxy(reqwest::Proxy::all(format!("http://{proxy_addr}")).unwrap()),
            StdDuration::from_secs(1),
            HOST,
            &[target_addr],
        )
        .unwrap();
        let response = client
            .get(format!("http://{HOST}:{}/", target_addr.port()))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 204);
        target_task.await.unwrap();
        proxy_task.abort();
    }

    #[test]
    fn cron_execution_rejects_legacy_http_payload_policy_violations() {
        for payload in [
            json!({
                "url": "https://user:pass@example.com/task",
                "method": "GET",
                "headers": {},
                "timeoutMs": 15000,
            }),
            json!({
                "url": "https://example.com/task",
                "method": "TRACE",
                "headers": {},
                "timeoutMs": 15000,
            }),
            json!({
                "url": "https://example.com/task",
                "method": "GET",
                "headers": {"Host": "evil.example"},
                "timeoutMs": 15000,
            }),
            json!({
                "url": "https://example.com/task",
                "method": "GET",
                "headers": {},
                "timeoutMs": 50,
            }),
        ] {
            assert!(
                normalize_cron_payload("http", payload).is_err(),
                "legacy unsafe HTTP Cron payload must fail full policy"
            );
        }
    }

    #[test]
    fn cron_requires_native_bound_confirmation_and_rejects_renderer_boolean() {
        let root = TestDirectory::new("cron-dangerous-confirmation");
        let service = LocalServices::for_test(root.path.clone());

        let shell_input = serde_json::from_value::<CronUpsertInput>(json!({
            "name": "Unconfirmed shell task",
            "type": "shell",
            "schedule": "hourly",
            "payload": {
                "command": "Write-Output unsafe",
                "workdir": root.path.display().to_string(),
            },
            "enabled": false,
        }))
        .expect("deserialize shell Cron input");
        let shell_error = service
            .cron_upsert(shell_input.clone(), None)
            .expect_err("unconfirmed Shell payload must be rejected");
        assert!(shell_error.contains("native user confirmation"));

        // A confirmation is bound to the full native input. Altering even one
        // command field consumes and rejects the old approval rather than
        // authorizing a different Shell target.
        let approval = test_native_cron_approval(CronDangerousOperation::Save, &shell_input);
        let mut changed_shell_input = shell_input.clone();
        changed_shell_input.payload["command"] = json!("Write-Output changed");
        let changed_error = service
            .cron_upsert(changed_shell_input, Some(approval))
            .expect_err("an approval for one target must not authorize another");
        assert!(changed_error.contains("does not match the current target"));

        let wrong_operation = require_native_cron_approval(
            "shell",
            CronDangerousOperation::Enable,
            &shell_input,
            Some(test_native_cron_approval(
                CronDangerousOperation::Save,
                &shell_input,
            )),
        )
        .expect_err("an approval must not be replayed for a different operation");
        assert!(wrong_operation.contains("different operation"));

        let mut expired = test_native_cron_approval(CronDangerousOperation::Save, &shell_input);
        expired.issued_at = Instant::now() - NATIVE_CRON_APPROVAL_TTL - StdDuration::from_secs(1);
        let expired_error = require_native_cron_approval(
            "shell",
            CronDangerousOperation::Save,
            &shell_input,
            Some(expired),
        )
        .expect_err("an expired confirmation must be rejected");
        assert!(expired_error.contains("expired"));

        let http_input = serde_json::from_value::<CronUpsertInput>(json!({
            "name": "Unconfirmed HTTP task",
            "type": "http",
            "schedule": "hourly",
            "payload": {
                "url": "https://example.com/task",
                "method": "POST",
                "headers": {},
                "timeoutMs": 15000,
            },
            "enabled": false,
        }))
        .expect("deserialize HTTP Cron input");
        let http_error = service
            .cron_upsert(http_input, None)
            .expect_err("unconfirmed HTTP payload must be rejected");
        assert!(http_error.contains("native user confirmation"));

        let prompt_input = CronUpsertInput {
            id: None,
            name: "Unconfirmed prompt task".to_string(),
            kind: "prompt".to_string(),
            schedule: "hourly".to_string(),
            payload: json!({
                "prompt": "Use the configured provider without an approval.",
                "workdir": root.path.display().to_string(),
                "providerId": "reviewed-provider",
                "model": "reviewed-model",
            }),
            enabled: true,
        };
        let prompt_error = service
            .cron_upsert(prompt_input.clone(), None)
            .expect_err("unconfirmed Prompt payload must be rejected");
        assert!(prompt_error.contains("native user confirmation"));

        let prompt_job = confirmed_test_cron_upsert(&service, prompt_input);
        assert!(
            !prompt_job.enabled,
            "a Prompt Save approval must not authorize persistent scheduling"
        );
        let prompt_enable_error = service
            .cron_set_enabled(&prompt_job.id, true, None)
            .expect_err("unconfirmed Prompt enable must be rejected");
        assert!(prompt_enable_error.contains("native user confirmation"));

        // The former renderer field is now an unknown IPC field, so a
        // compromised WebView cannot forge a native approval by adding it.
        let mut forged_renderer_input = serde_json::to_value(&shell_input).unwrap();
        forged_renderer_input
            .as_object_mut()
            .unwrap()
            .insert("dangerousConfirmed".to_string(), Value::Bool(true));
        assert!(serde_json::from_value::<CronUpsertInput>(forged_renderer_input).is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn translate_provider_text_round_trips_an_openai_completion() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 8_192];
            let read = socket.read(&mut request).await.unwrap();
            let body = String::from_utf8_lossy(&request[..read]);
            assert!(body.contains("\"role\":\"system\""));
            assert!(body.contains("professional translator"));
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"choices\":[{\"message\":{\"content\":\"\xe4\xbd\xa0\xe5\xa5\xbd\xef\xbc\x8c\xe4\xb8\x96\xe7\x95\x8c\"}}]}",
                )
                .await
                .unwrap();
        });

        let provider = json!({
            "id": "translation-provider",
            "type": "openai",
            "baseUrl": format!("http://{addr}"),
            "apiKey": "native-secret",
            "enabled": true,
            "model": "translation-model"
        });
        let provider = serde_json::from_value::<Map<String, Value>>(provider).unwrap();
        let result = translate_provider_text(
            &provider,
            "translation-model",
            "You are a professional translator. Detect the source language and translate the user-provided text into zh.",
            "Hello world",
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert_eq!(result, "你好，世界");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn translate_provider_text_rejects_a_stale_credential_binding() {
        let provider = json!({
            "id": "translation-provider",
            "type": "openai",
            "baseUrl": "https://api.openai.com/v1",
            "apiKey": "native-secret",
            "__credentialBinding": {
                "authFamily": "anthropic",
                "endpoint": "https://api.openai.com/v1"
            }
        });
        let provider = serde_json::from_value::<Map<String, Value>>(provider).unwrap();
        let error = translate_provider_text(&provider, "model", "system", "hello")
            .await
            .unwrap_err();
        assert!(error.contains("not valid for the configured endpoint"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn translate_provider_text_bounds_its_input_before_network() {
        let provider = json!({
            "id": "translation-provider",
            "type": "openai",
            "baseUrl": "https://api.openai.com/v1",
            "apiKey": "native-secret"
        });
        let provider = serde_json::from_value::<Map<String, Value>>(provider).unwrap();
        assert_eq!(
            translate_provider_text(&provider, "model", "system", "   ")
                .await
                .unwrap_err(),
            "translation text cannot be empty"
        );
        let oversized = "x".repeat(MAX_TRANSLATION_INPUT_CHARS + 1);
        assert_eq!(
            translate_provider_text(&provider, "model", "system", &oversized)
                .await
                .unwrap_err(),
            "translation text is too long"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn prompt_cron_run_now_rejects_missing_native_approval_before_creating_a_run() {
        let root = TestDirectory::new("cron-prompt-run-now-confirmation");
        let service = LocalServices::for_test(root.path.clone());
        let input = CronUpsertInput {
            id: None,
            name: "Prompt run needs confirmation".to_string(),
            kind: "prompt".to_string(),
            schedule: "hourly".to_string(),
            payload: json!({"prompt": "Do not contact a provider in this test."}),
            enabled: false,
        };
        let job = confirmed_test_cron_upsert(&service, input);

        let error = service
            .cron_run_now(&job.id, None)
            .await
            .expect_err("unconfirmed Prompt run-now must be rejected");
        assert!(error.contains("native user confirmation"));
        assert!(
            service.cron_runs(&job.id, None).unwrap().is_empty(),
            "a rejected Prompt run-now must not create a persisted run"
        );
    }

    #[test]
    fn legacy_enabled_prompt_cron_is_disabled_before_the_native_confirmation_boundary() {
        let root = TestDirectory::new("cron-prompt-confirmation-migration");
        let service = LocalServices::for_test(root.path.clone());
        let protected_payload = encode_cron_payload(&json!({
            "prompt": "Legacy prompt that predates native confirmation.",
        }))
        .expect("encode legacy Prompt Cron payload");
        let connection = service.open().unwrap();
        connection
            .execute(
                "DELETE FROM local_service_meta WHERE key=?1",
                params![CRON_PROMPT_NATIVE_CONFIRMATION_MIGRATION_KEY],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO cron_jobs(\
                    id, name, kind, schedule, payload_json, enabled, next_run_at, last_run_at, created_at, updated_at\
                 ) VALUES(?1, ?2, 'prompt', 'hourly', ?3, 1, ?4, NULL, ?4, ?4)",
                params!["legacy-prompt", "Legacy prompt", protected_payload, now_ms()],
            )
            .unwrap();
        drop(connection);

        service.initialize().unwrap();

        let job = service.cron_get("legacy-prompt").unwrap();
        assert!(!job.enabled);
        assert_eq!(job.next_run_at, None);
        let migration_marker: String = service
            .open()
            .unwrap()
            .query_row(
                "SELECT value FROM local_service_meta WHERE key=?1",
                params![CRON_PROMPT_NATIVE_CONFIRMATION_MIGRATION_KEY],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(migration_marker, "1");
    }

    #[test]
    fn cron_native_confirmation_preview_escapes_invisible_shell_characters() {
        assert_eq!(
            native_cron_dialog_preview("one\\two\nthree\rfour\tfive", 128),
            r"one\\two\nthree\rfour\tfive"
        );

        let bidi_preview = native_cron_dialog_preview("safe\u{202E}hidden", 128);
        assert!(bidi_preview.contains(r"\u{202E}"));
        assert!(!bidi_preview.contains('\u{202E}'));
        assert!(
            !bidi_preview.chars().any(char::is_control),
            "native confirmation must not hide control characters"
        );
    }

    #[test]
    fn dangerous_cron_upserts_start_disabled_and_require_a_separate_enable_approval() {
        let root = TestDirectory::new("cron-dangerous-save-then-enable");
        let service = LocalServices::for_test(root.path.clone());
        let input = CronUpsertInput {
            id: None,
            name: "Deferred HTTP request".to_string(),
            kind: "http".to_string(),
            schedule: "hourly".to_string(),
            payload: json!({
                "url": "https://example.com/task",
                "method": "POST",
                "headers": {"x-request-purpose": "scheduled"},
                "body": "private payload",
                "timeoutMs": 15000,
            }),
            // A compromised renderer can still submit this value. It must not
            // turn a Save approval into permission to begin scheduled network
            // requests.
            enabled: true,
        };

        let saved = service
            .cron_upsert(
                input.clone(),
                Some(test_native_cron_approval(
                    CronDangerousOperation::Save,
                    &input,
                )),
            )
            .expect("a confirmed HTTP target should save");
        assert!(
            !saved.enabled,
            "dangerous Cron jobs must always save disabled"
        );
        assert!(
            service.cron_set_enabled(&saved.id, true, None).is_err(),
            "a Save approval must not authorize a later enable"
        );

        let enabled = service
            .cron_set_enabled(
                &saved.id,
                true,
                Some(test_native_cron_approval(
                    CronDangerousOperation::Enable,
                    &saved,
                )),
            )
            .expect("a separately confirmed exact record should enable");
        assert!(enabled.enabled);
    }

    #[test]
    fn cron_http_confirmation_commits_to_hidden_payload_without_disclosing_secrets() {
        let secret = "never-display-this-credential";
        let body = format!(r#"{{"token":"{secret}"}}"#);
        let body_bytes = body.len();
        let description = cron_native_confirmation_description(
            CronDangerousOperation::Save,
            "http",
            "reviewed request",
            "hourly",
            &json!({
                "url": "https://example.com/task",
                "method": "POST",
                "headers": {
                    "authorization": format!("Bearer {secret}"),
                    "x-request-id": "request-42",
                },
                "body": body,
                "timeoutMs": 15000,
            }),
        )
        .expect("HTTP confirmation description");

        assert!(description.contains("Header fields (values hidden): authorization, x-request-id"));
        assert!(description.contains(&format!("Body: {body_bytes} UTF-8 bytes (text hidden)")));
        assert!(description.contains("Exact request SHA-256:"));
        assert!(!description.contains(secret));
    }

    #[test]
    fn cron_prompt_confirmation_discloses_review_metadata_without_prompt_text_or_credentials() {
        let prompt = "Summarize this private value: test-credential-marker";
        let prompt_digest = sha256_hex(prompt.as_bytes());
        let description = cron_native_confirmation_description(
            CronDangerousOperation::Enable,
            "prompt",
            "Reviewed prompt request",
            "daily at 09:00",
            &json!({
                "prompt": prompt,
                "workdir": "C:/reviewed/workspace",
                "providerId": "provider-alpha",
                "model": "model-beta",
            }),
        )
        .expect("Prompt confirmation description");

        assert!(description.contains("Enable this Prompt Cron job?"));
        assert!(description.contains("Schedule: daily at 09:00"));
        assert!(description.contains("Workdir: C:/reviewed/workspace"));
        assert!(description.contains("Provider: provider-alpha"));
        assert!(description.contains("Model: model-beta"));
        assert!(description.contains(&format!(
            "Prompt: {} UTF-8 bytes (text hidden)",
            prompt.len()
        )));
        assert!(description.contains(&format!("Prompt SHA-256: {prompt_digest}")));
        assert!(!description.contains(prompt));
        assert!(!description.contains("test-credential-marker"));
    }

    #[test]
    fn cron_http_payload_rejects_non_public_or_non_https_targets() {
        for target in [
            "http://example.com/task",
            "https://localhost/task",
            "https://localhost.localdomain/task",
            "https://intranet/task",
            "https://127.0.0.1/task",
            "https://10.0.0.1/task",
            "https://172.16.0.1/task",
            "https://192.168.0.1/task",
            "https://[::1]/task",
            "https://[::]/task",
            "https://[fe80::1]/task",
            "https://[fd00::1]/task",
        ] {
            let error = normalize_cron_payload(
                "http",
                json!({
                    "url": target,
                    "method": "GET",
                    "headers": {},
                    "timeoutMs": 15000,
                }),
            )
            .expect_err("unsafe HTTP Cron target must be rejected");
            assert!(!error.contains(target));
        }

        assert!(normalize_cron_payload(
            "http",
            json!({
                "url": "https://example.com/task",
                "method": "GET",
                "headers": {},
                "timeoutMs": 15000,
            }),
        )
        .is_ok());
    }

    #[test]
    fn cron_ip_policy_rejects_non_global_ipv4_and_ipv6_ranges() {
        for address in [
            "0.0.0.0",
            "0.0.0.1",
            "10.0.0.1",
            "100.64.0.0",
            "100.127.255.255",
            "127.0.0.1",
            "169.254.0.1",
            "172.16.0.1",
            "192.0.2.1",
            "192.168.0.1",
            "198.18.0.0",
            "198.19.255.255",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "239.255.255.255",
            "240.0.0.0",
            "255.255.255.255",
        ] {
            assert!(
                is_disallowed_cron_ipv4(address.parse().expect("valid IPv4 test address")),
                "{address} must not be a Cron destination"
            );
        }
        assert!(!is_disallowed_cron_ipv4(
            "8.8.8.8".parse().expect("valid global IPv4 test address")
        ));

        for address in [
            "::",
            "::1",
            "fc00::1",
            "fdff:ffff::1",
            "fe80::1",
            "febf:ffff::1",
            "fec0::1",
            "feff:ffff::1",
            "ff00::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "::ffff:100.64.0.1",
        ] {
            assert!(
                is_disallowed_cron_ipv6(address.parse().expect("valid IPv6 test address")),
                "{address} must not be a Cron destination"
            );
        }
        assert!(!is_disallowed_cron_ipv6(
            "2606:4700:4700::1111"
                .parse()
                .expect("valid global IPv6 test address")
        ));
    }

    #[test]
    fn cron_renderer_summaries_never_serialize_payload_or_result_content() {
        let secret = "private-value-123";
        let job = CronJob {
            id: "job-1".to_string(),
            name: "Protected job".to_string(),
            kind: "http".to_string(),
            schedule: "hourly".to_string(),
            payload: json!({
                "url": "https://example.test/private",
                "headers": {"authorization": format!("Bearer {secret}")},
            }),
            enabled: true,
            next_run_at: Some(1),
            last_run_at: None,
            created_at: 1,
            updated_at: 1,
        };
        let job_json = serde_json::to_value(CronJobSummary::from(job)).unwrap();
        assert!(!job_json.to_string().contains(secret));
        assert_eq!(job_json["payloadRedacted"], true);
        assert!(job_json.get("payload").is_none());

        let run_response = CronRunNowResponse {
            run: CronRun {
                id: "run-1".to_string(),
                job_id: "job-1".to_string(),
                status: "completed".to_string(),
                scheduled_for: None,
                started_at: 1,
                completed_at: Some(2),
                has_output: true,
                has_error: true,
            },
            dispatch: Some(CronDispatch {
                run_id: "run-1".to_string(),
                job_id: "job-1".to_string(),
                kind: "prompt".to_string(),
                payload: json!({"prompt": secret}),
            }),
            http: Some(CronHttpResult {
                status: Some(200),
                body: format!("body={secret}"),
                truncated: false,
                success: true,
            }),
        };
        let scheduler_json =
            serde_json::to_value(CronSchedulerUpdate::from_claim(&CronDueClaimResponse {
                claimed_at: 2,
                runs: vec![run_response.clone()],
            }))
            .unwrap();
        assert!(!scheduler_json.to_string().contains(secret));
        assert_eq!(scheduler_json["status"], "ok");
        assert_eq!(scheduler_json["claimed"], 1);
        assert_eq!(scheduler_json["completed"], 1);
        assert_eq!(scheduler_json["failed"], 0);
        assert_eq!(scheduler_json["running"], 0);

        let completion_update = CronSchedulerUpdate::from_execution(&run_response);
        assert_eq!(completion_update.claimed, 0);
        assert_eq!(completion_update.running, 0);
        assert_eq!(completion_update.completed, 1);
        assert_eq!(completion_update.failed, 0);
        assert!(!serde_json::to_string(&completion_update)
            .unwrap()
            .contains(secret));

        let failed_execution_update = CronSchedulerUpdate::failed_execution();
        assert_eq!(failed_execution_update.claimed, 0);
        assert_eq!(failed_execution_update.running, 0);
        assert_eq!(failed_execution_update.completed, 0);
        assert_eq!(failed_execution_update.failed, 1);

        let run_json = serde_json::to_value(CronRunNowSummary::from(run_response)).unwrap();
        assert!(!run_json.to_string().contains(secret));
        assert!(run_json["run"]["hasOutput"].as_bool().unwrap());
        assert!(run_json["run"]["hasError"].as_bool().unwrap());
        assert!(run_json["dispatch"].get("payload").is_none());
        assert!(run_json["http"].get("body").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn cron_payload_is_protected_and_due_claim_is_atomic() {
        let root = TestDirectory::new("cron");
        let service = LocalServices::for_test(root.path.clone());
        let secret_prompt = "run with private-value-123";
        let saved = confirmed_test_cron_upsert(
            &service,
            CronUpsertInput {
                id: None,
                name: "Protected prompt".to_string(),
                kind: "prompt".to_string(),
                schedule: "hourly".to_string(),
                payload: json!({"prompt": secret_prompt}),
                enabled: true,
            },
        );
        let job = enabled_test_cron(&service, &saved);
        assert_eq!(job.payload["prompt"], secret_prompt);

        let database_bytes = fs::read(service.database_path()).unwrap();
        assert!(!database_bytes
            .windows(secret_prompt.len())
            .any(|window| window == secret_prompt.as_bytes()));

        service
            .open()
            .unwrap()
            .execute(
                "UPDATE cron_jobs SET next_run_at=?2 WHERE id=?1",
                params![job.id, now_ms() - 1_000],
            )
            .unwrap();
        let claims = service.claim_due_cron_runs(10, now_ms()).unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].1.status, "running");
        assert!(service
            .claim_due_cron_runs(10, now_ms())
            .unwrap()
            .is_empty());

        let completed = service
            .cron_complete(CronCompleteInput {
                run_id: claims[0].1.id.clone(),
                success: true,
                output: Some("ok".to_string()),
                error: None,
            })
            .unwrap();
        assert_eq!(completed.status, "completed");
        let cleared_lease: Option<i64> = service
            .open()
            .unwrap()
            .query_row(
                "SELECT lease_expires_at FROM cron_runs WHERE id=?1",
                params![completed.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cleared_lease, None, "completing a run must clear its lease");
        assert_eq!(service.cron_runs(&job.id, None).unwrap().len(), 1);

        let disabled = service.cron_set_enabled(&job.id, false, None).unwrap();
        assert!(!disabled.enabled);
        assert_eq!(disabled.payload["prompt"], secret_prompt);
        let enabled = enabled_test_cron(&service, &disabled);
        assert!(enabled.enabled);
        assert!(enabled.next_run_at.is_some());
        assert_eq!(enabled.payload["prompt"], secret_prompt);
    }

    #[cfg(windows)]
    #[test]
    fn cron_run_results_retain_only_presence_flags() {
        let root = TestDirectory::new("cron-result-retention");
        let service = LocalServices::for_test(root.path.clone());
        let job = confirmed_test_cron_upsert(
            &service,
            CronUpsertInput {
                id: None,
                name: "Result retention job".to_string(),
                kind: "prompt".to_string(),
                schedule: "hourly".to_string(),
                payload: json!({"prompt": "Do not persist the result body."}),
                enabled: false,
            },
        );
        let (_, run) = service
            .create_cron_run(
                &job.id,
                None,
                Some(test_native_cron_approval(
                    CronDangerousOperation::RunNow,
                    &job,
                )),
            )
            .unwrap();
        let output_secret = "cron-output-secret-123";
        let error_secret = "cron-error-secret-456";
        let completed = service
            .cron_complete(CronCompleteInput {
                run_id: run.id.clone(),
                success: false,
                output: Some(output_secret.to_string()),
                error: Some(error_secret.to_string()),
            })
            .unwrap();
        assert!(completed.has_output);
        assert!(completed.has_error);

        let retained: (Option<String>, Option<String>, bool, bool) = service
            .open()
            .unwrap()
            .query_row(
                "SELECT output, error, has_output, has_error FROM cron_runs WHERE id=?1",
                params![&run.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(retained, (None, None, true, true));

        let summary = serde_json::to_string(&CronRunSummary::from(completed)).unwrap();
        assert!(!summary.contains(output_secret));
        assert!(!summary.contains(error_secret));

        let database_paths = [
            service.database_path().to_path_buf(),
            PathBuf::from(format!("{}-wal", service.database_path().display())),
        ];
        for database_path in database_paths {
            if database_path.exists() {
                let bytes = fs::read(&database_path).unwrap();
                assert!(
                    !bytes
                        .windows(output_secret.len())
                        .any(|window| window == output_secret.as_bytes()),
                    "output must not enter {}",
                    database_path.display()
                );
                assert!(
                    !bytes
                        .windows(error_secret.len())
                        .any(|window| window == error_secret.as_bytes()),
                    "error detail must not enter {}",
                    database_path.display()
                );
            }
        }
    }

    #[test]
    fn cron_run_result_retention_migrates_legacy_plaintext_rows() {
        let root = TestDirectory::new("cron-result-retention-migration");
        let database_path = root.path.join(DATABASE_FILE);
        let connection = Connection::open(&database_path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE cron_jobs (\
                    id TEXT PRIMARY KEY NOT NULL,\
                    name TEXT NOT NULL,\
                    kind TEXT NOT NULL,\
                    schedule TEXT NOT NULL,\
                    payload_json TEXT NOT NULL,\
                    enabled INTEGER NOT NULL,\
                    next_run_at INTEGER,\
                    last_run_at INTEGER,\
                    created_at INTEGER NOT NULL,\
                    updated_at INTEGER NOT NULL\
                 );\
                 CREATE TABLE cron_runs (\
                    id TEXT PRIMARY KEY NOT NULL,\
                    job_id TEXT NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,\
                    status TEXT NOT NULL,\
                    scheduled_for INTEGER,\
                    started_at INTEGER NOT NULL,\
                    completed_at INTEGER,\
                    output TEXT,\
                    error TEXT\
                 );",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO cron_jobs(\
                    id, name, kind, schedule, payload_json, enabled, next_run_at, last_run_at, created_at, updated_at\
                 ) VALUES(?1, ?2, 'prompt', 'hourly', '{}', 0, NULL, NULL, 1, 1)",
                params!["legacy-job", "Legacy result job"],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO cron_runs(\
                    id, job_id, status, scheduled_for, started_at, completed_at, output, error\
                 ) VALUES(?1, ?2, 'completed', NULL, 1, 2, ?3, ?4)",
                params![
                    "legacy-run",
                    "legacy-job",
                    "legacy-output-secret",
                    "legacy-error-secret"
                ],
            )
            .unwrap();
        drop(connection);

        let service = LocalServices::for_test(root.path.clone());
        let runs = service.cron_runs("legacy-job", None).unwrap();
        assert_eq!(runs.len(), 1);
        assert!(runs[0].has_output);
        assert!(runs[0].has_error);

        let retained: (Option<String>, Option<String>, bool, bool) = service
            .open()
            .unwrap()
            .query_row(
                "SELECT output, error, has_output, has_error FROM cron_runs WHERE id='legacy-run'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(retained, (None, None, true, true));
    }

    #[cfg(windows)]
    #[test]
    fn cron_startup_recovers_expired_http_lease_without_retrying_it() {
        let root = TestDirectory::new("cron-expired-lease");
        let service = LocalServices::for_test(root.path.clone());
        let input = CronUpsertInput {
            id: None,
            name: "Expired HTTP run".to_string(),
            kind: "http".to_string(),
            schedule: "hourly".to_string(),
            payload: json!({
                "url": "https://example.com/task",
                "method": "POST",
                "headers": {},
                "timeoutMs": 15000,
            }),
            enabled: true,
        };
        let job = service
            .cron_upsert(
                input.clone(),
                Some(test_native_cron_approval(
                    CronDangerousOperation::Save,
                    &input,
                )),
            )
            .unwrap();
        assert!(!job.enabled, "HTTP Cron writes must start disabled");
        let approval = test_native_cron_approval(CronDangerousOperation::Enable, &job);
        let job = service
            .cron_set_enabled(&job.id, true, Some(approval))
            .expect("a separately confirmed HTTP Cron should enable");
        service
            .open()
            .unwrap()
            .execute(
                "UPDATE cron_jobs SET next_run_at=?2 WHERE id=?1",
                params![job.id, now_ms() - 1_000],
            )
            .unwrap();

        // Claim only: a crash here is the interval this regression covers.
        // Do not call the HTTP executor, so this test cannot make a request.
        let claims = service.claim_due_cron_runs(10, now_ms()).unwrap();
        assert_eq!(claims.len(), 1);
        let run = &claims[0].1;
        let lease_expires_at: Option<i64> = service
            .open()
            .unwrap()
            .query_row(
                "SELECT lease_expires_at FROM cron_runs WHERE id=?1",
                params![run.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(lease_expires_at.is_some_and(|lease| lease > run.started_at));
        service
            .open()
            .unwrap()
            .execute(
                "UPDATE cron_runs SET lease_expires_at=?2 WHERE id=?1",
                params![run.id, now_ms() - 1],
            )
            .unwrap();
        drop(service);

        // Construction runs startup recovery. The occurrence is recorded as
        // failed and must not be put back on the due queue for another HTTP
        // request.
        let restarted = LocalServices::for_test(root.path.clone());
        let runs = restarted.cron_runs(&job.id, None).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "failed");
        assert!(runs[0].completed_at.is_some());
        assert!(runs[0].has_error);
        assert!(restarted
            .claim_due_cron_runs(10, now_ms())
            .unwrap()
            .is_empty());

        // The terminal recovered record must no longer prevent removal.
        assert_eq!(restarted.cron_delete(&job.id).unwrap().id, job.id);
    }

    #[cfg(windows)]
    #[test]
    fn cron_due_claim_recovers_an_expired_lease_without_reclaiming_the_run() {
        let root = TestDirectory::new("cron-live-lease-recovery");
        let service = LocalServices::for_test(root.path.clone());
        let saved = confirmed_test_cron_upsert(
            &service,
            CronUpsertInput {
                id: None,
                name: "Expired active run".to_string(),
                kind: "prompt".to_string(),
                schedule: "hourly".to_string(),
                payload: json!({"prompt": "This must not execute twice."}),
                enabled: true,
            },
        );
        let job = enabled_test_cron(&service, &saved);
        service
            .open()
            .unwrap()
            .execute(
                "UPDATE cron_jobs SET next_run_at=?2 WHERE id=?1",
                params![&job.id, now_ms() - 1_000],
            )
            .unwrap();

        let claims = service.claim_due_cron_runs(1, now_ms()).unwrap();
        assert_eq!(claims.len(), 1);
        let run_id = claims[0].1.id.clone();
        service
            .open()
            .unwrap()
            .execute(
                "UPDATE cron_runs SET lease_expires_at=?2 WHERE id=?1",
                params![&run_id, now_ms() - 1],
            )
            .unwrap();

        // A later tick first expires the abandoned active row. It must not
        // create another occurrence because the original job was advanced in
        // the claim transaction.
        assert!(service.claim_due_cron_runs(1, now_ms()).unwrap().is_empty());
        let runs = service.cron_runs(&job.id, None).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, run_id);
        assert_eq!(runs[0].status, "failed");
        assert!(runs[0].completed_at.is_some());
        assert!(runs[0].has_error);
        let recovered_lease: Option<i64> = service
            .open()
            .unwrap()
            .query_row(
                "SELECT lease_expires_at FROM cron_runs WHERE id=?1",
                params![&run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(recovered_lease, None);
    }

    #[cfg(windows)]
    #[test]
    fn cron_startup_marks_interrupted_runs_failed_and_scrubs_legacy_result_content() {
        let root = TestDirectory::new("cron-restart-recovery");
        let service = LocalServices::for_test(root.path.clone());
        let job = confirmed_test_cron_upsert(
            &service,
            CronUpsertInput {
                id: None,
                name: "Restart recovery job".to_string(),
                kind: "prompt".to_string(),
                schedule: "hourly".to_string(),
                payload: json!({"prompt": "Do not run again after restart."}),
                enabled: false,
            },
        );
        let future_lease = now_ms() + CRON_RUN_LEASE_MS;
        let connection = service.open().unwrap();
        connection
            .execute(
                "INSERT INTO cron_runs(\
                    id, job_id, status, scheduled_for, started_at, lease_expires_at, completed_at, output, error\
                 ) VALUES(?1, ?2, 'running', NULL, ?3, ?4, NULL, ?5, ?6)",
                params![
                    "restart-running",
                    &job.id,
                    10_i64,
                    future_lease,
                    "running output",
                    "running error"
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO cron_runs(\
                    id, job_id, status, scheduled_for, started_at, lease_expires_at, completed_at, output, error\
                 ) VALUES(?1, ?2, 'dispatched', NULL, ?3, ?4, NULL, ?5, ?6)",
                params![
                    "restart-dispatched",
                    &job.id,
                    20_i64,
                    future_lease,
                    "dispatched output",
                    "dispatched error"
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO cron_runs(\
                    id, job_id, status, scheduled_for, started_at, lease_expires_at, completed_at, output, error\
                 ) VALUES(?1, ?2, 'completed', NULL, ?3, NULL, ?4, ?5, ?6)",
                params![
                    "restart-completed",
                    &job.id,
                    30_i64,
                    40_i64,
                    "completed output",
                    "completed error"
                ],
            )
            .unwrap();
        drop(connection);
        drop(service);

        let restarted = LocalServices::for_test(root.path.clone());
        let recovered_runs = restarted.cron_runs(&job.id, None).unwrap();
        assert_eq!(recovered_runs.len(), 3, "restart must not create a rerun");

        let running_run = recovered_runs
            .iter()
            .find(|run| run.id == "restart-running")
            .expect("persisted running run");
        assert_eq!(running_run.status, "failed");
        assert!(running_run.completed_at.is_some());
        assert!(running_run.has_output);
        assert!(running_run.has_error);

        let dispatched_run = recovered_runs
            .iter()
            .find(|run| run.id == "restart-dispatched")
            .expect("persisted dispatched run");
        assert_eq!(dispatched_run.status, "failed");
        assert!(dispatched_run.completed_at.is_some());
        assert!(dispatched_run.has_output);
        assert!(dispatched_run.has_error);

        let completed_run = recovered_runs
            .iter()
            .find(|run| run.id == "restart-completed")
            .expect("persisted completed run");
        assert_eq!(completed_run.status, "completed");
        assert_eq!(completed_run.completed_at, Some(40));
        assert!(completed_run.has_output);
        assert!(completed_run.has_error);

        let connection = restarted.open().unwrap();
        let mut statement = connection
            .prepare("SELECT output, error, has_output, has_error FROM cron_runs ORDER BY id")
            .unwrap();
        let retained_result_columns: Vec<(Option<String>, Option<String>, bool, bool)> = statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(retained_result_columns.len(), 3);
        assert!(retained_result_columns
            .iter()
            .all(|(output, error, has_output, has_error)| {
                output.is_none() && error.is_none() && *has_output && *has_error
            }));
        drop(statement);
        drop(connection);

        let first_recovery_timestamps = (running_run.completed_at, dispatched_run.completed_at);
        restarted.initialize().unwrap();
        let runs_after_reinitialize = restarted.cron_runs(&job.id, None).unwrap();
        assert_eq!(runs_after_reinitialize.len(), 3, "recovery is idempotent");
        assert_eq!(
            runs_after_reinitialize
                .iter()
                .find(|run| run.id == "restart-running")
                .and_then(|run| run.completed_at),
            first_recovery_timestamps.0
        );
        assert_eq!(
            runs_after_reinitialize
                .iter()
                .find(|run| run.id == "restart-dispatched")
                .and_then(|run| run.completed_at),
            first_recovery_timestamps.1
        );

        assert_eq!(restarted.cron_delete(&job.id).unwrap().id, job.id);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cron_run_now_respects_scheduler_execution_capacity_before_creating_a_run() {
        let root = TestDirectory::new("cron-run-now-capacity");
        let service = LocalServices::for_test(root.path.clone());
        let job = confirmed_test_cron_upsert(
            &service,
            CronUpsertInput {
                id: None,
                name: "Manual run capacity".to_string(),
                kind: "prompt".to_string(),
                schedule: "hourly".to_string(),
                payload: json!({"prompt": "This must not start while Cron is full."}),
                enabled: true,
            },
        );

        // This is the same pool used by the automatic scheduler before it
        // claims due work. Holding every slot must reject a manual run without
        // leaving a persisted active row behind.
        let scheduler_slots = service
            .cron_execution_pool()
            .reserve_available_slots(CRON_WORKER_POOL_SIZE);
        assert_eq!(scheduler_slots.len(), CRON_WORKER_POOL_SIZE);

        let error = match service.cron_run_now(&job.id, None).await {
            Ok(_) => panic!("manual Cron run unexpectedly bypassed execution capacity"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            "Cron execution capacity is exhausted; wait for an active run to finish"
        );
        assert!(service.cron_runs(&job.id, None).unwrap().is_empty());

        drop(scheduler_slots);
        assert!(service.cron_execution_pool().try_reserve_slot().is_ok());
    }

    #[test]
    fn cron_run_creation_rejects_concurrent_active_job_atomically() {
        let root = TestDirectory::new("cron-run-now-single-job");
        let service = Arc::new(LocalServices::for_test(root.path.clone()));
        let job = confirmed_test_cron_upsert(
            service.as_ref(),
            CronUpsertInput {
                id: None,
                name: "One active run at a time".to_string(),
                kind: "prompt".to_string(),
                schedule: "hourly".to_string(),
                payload: json!({"prompt": "The database transaction serializes this run."}),
                enabled: true,
            },
        );

        let start = Arc::new(std::sync::Barrier::new(3));
        let first_service = Arc::clone(&service);
        let first_job_id = job.id.clone();
        let first_job = job.clone();
        let first_start = Arc::clone(&start);
        let first = std::thread::spawn(move || {
            first_start.wait();
            first_service.create_cron_run(
                &first_job_id,
                None,
                Some(test_native_cron_approval(
                    CronDangerousOperation::RunNow,
                    &first_job,
                )),
            )
        });
        let second_service = Arc::clone(&service);
        let second_job_id = job.id.clone();
        let second_job = job.clone();
        let second_start = Arc::clone(&start);
        let second = std::thread::spawn(move || {
            second_start.wait();
            second_service.create_cron_run(
                &second_job_id,
                None,
                Some(test_native_cron_approval(
                    CronDangerousOperation::RunNow,
                    &second_job,
                )),
            )
        });

        start.wait();
        let outcomes = [first.join().unwrap(), second.join().unwrap()];
        assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
        let errors = outcomes
            .iter()
            .filter_map(|outcome| outcome.as_ref().err())
            .collect::<Vec<_>>();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].as_str(), "Cron job has an unfinished run");

        let runs = service.cron_runs(&job.id, None).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "running");
        service
            .complete_cron_run(&runs[0].id, false, false, true)
            .unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn scheduler_claim_only_reserves_ten_available_worker_slots_and_leaves_backlog_due() {
        let root = TestDirectory::new("cron-scheduler-slot-capacity");
        let service = LocalServices::for_test(root.path.clone());
        let due_at = now_ms() - 1_000;
        let mut jobs = Vec::new();

        assert_eq!(CRON_WORKER_POOL_SIZE, 10);

        for index in 0..=CRON_WORKER_POOL_SIZE {
            let saved = confirmed_test_cron_upsert(
                &service,
                CronUpsertInput {
                    id: None,
                    name: format!("Scheduled prompt {index}"),
                    kind: "prompt".to_string(),
                    schedule: "hourly".to_string(),
                    payload: json!({"prompt": format!("slot test {index}")}),
                    enabled: true,
                },
            );
            let job = enabled_test_cron(&service, &saved);
            service
                .open()
                .unwrap()
                .execute(
                    "UPDATE cron_jobs SET next_run_at=?2 WHERE id=?1",
                    params![&job.id, due_at],
                )
                .unwrap();
            jobs.push(job);
        }

        let (response, claims) = service
            .cron_claim_due_for_scheduler(CRON_WORKER_POOL_SIZE)
            .unwrap();
        assert_eq!(claims.len(), CRON_WORKER_POOL_SIZE);
        assert_eq!(response.runs.len(), CRON_WORKER_POOL_SIZE);
        assert!(response
            .runs
            .iter()
            .all(|response| response.run.status == "running"));
        let claim_update = CronSchedulerUpdate::from_claim(&response);
        assert_eq!(claim_update.claimed, CRON_WORKER_POOL_SIZE);
        assert_eq!(claim_update.running, CRON_WORKER_POOL_SIZE);
        assert_eq!(claim_update.completed, 0);
        assert_eq!(claim_update.failed, 0);

        let claimed_job_ids = claims
            .iter()
            .map(|(job, _)| job.id.as_str())
            .collect::<HashSet<_>>();
        let unclaimed = jobs
            .iter()
            .find(|job| !claimed_job_ids.contains(job.id.as_str()))
            .expect("one due job must remain unclaimed when all worker slots are reserved");
        let unclaimed_id = unclaimed.id.clone();
        assert_eq!(
            service.cron_get(&unclaimed.id).unwrap().next_run_at,
            Some(due_at)
        );

        let active_count = service
            .open()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM cron_runs WHERE status IN ('running', 'dispatched')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(active_count, CRON_WORKER_POOL_SIZE as i64);

        for (_, run) in claims {
            service
                .complete_cron_run(&run.id, false, false, true)
                .unwrap();
        }

        // This represents the next 30-second scheduler tick after the first
        // ten worker futures have completed and released their permits. Until
        // then the unclaimed job remains due, rather than becoming `running`.
        let (after_release, after_release_claims) = service
            .cron_claim_due_for_scheduler(CRON_WORKER_POOL_SIZE)
            .unwrap();
        assert_eq!(after_release_claims.len(), 1);
        assert_eq!(after_release.runs.len(), 1);
        assert_eq!(after_release_claims[0].0.id, unclaimed_id);
        assert_eq!(after_release_claims[0].1.status, "running");
        service
            .complete_cron_run(&after_release_claims[0].1.id, false, false, true)
            .unwrap();
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "current_thread")]
    async fn cron_due_claim_runs_shell_jobs_through_a_bounded_worker_pool() {
        let root = TestDirectory::new("cron-shell-worker-pool");
        let service = Arc::new(LocalServices::for_test(root.path.clone()));
        let due_at = now_ms() - 1_000;
        let job_count = CRON_WORKER_POOL_SIZE;

        for index in 0..job_count {
            let input = CronUpsertInput {
                id: None,
                name: format!("Parallel shell {index}"),
                kind: "shell".to_string(),
                schedule: "hourly".to_string(),
                payload: json!({
                    "command": format!(
                        "Start-Sleep -Milliseconds 1200; Write-Output 'worker-{index}'"
                    ),
                    "workdir": root.path.display().to_string(),
                }),
                enabled: true,
            };
            let job = service
                .cron_upsert(
                    input.clone(),
                    Some(test_native_cron_approval(
                        CronDangerousOperation::Save,
                        &input,
                    )),
                )
                .unwrap();
            let approval = test_native_cron_approval(CronDangerousOperation::Enable, &job);
            let job = service
                .cron_set_enabled(&job.id, true, Some(approval))
                .expect("explicit confirmation enables the test Shell Cron");
            service
                .open()
                .unwrap()
                .execute(
                    "UPDATE cron_jobs SET next_run_at=?2 WHERE id=?1",
                    params![&job.id, due_at],
                )
                .unwrap();
        }

        let started = Instant::now();
        let claimed = Arc::clone(&service)
            .cron_due_claim(Some(job_count))
            .await
            .unwrap();
        let elapsed = started.elapsed();

        assert_eq!(claimed.runs.len(), job_count);
        assert!(claimed
            .runs
            .iter()
            .all(|response| response.run.status == "completed"));
        // Ten 1.2s commands must use the ten-slot pool. A serial executor takes
        // roughly twelve seconds before process
        // startup overhead, while this leaves room for slower Windows hosts.
        assert!(
            elapsed < StdDuration::from_millis(4_500),
            "Cron worker pool was unexpectedly serial: {elapsed:?}"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn cron_due_claim_executes_shell_natively_without_exposing_payload() {
        let root = TestDirectory::new("cron-shell-execute");
        let service = Arc::new(LocalServices::for_test(root.path.clone()));
        let secret_token = "private-shell-token-xyz";
        let workdir = root.path.display().to_string();
        let input = CronUpsertInput {
            id: None,
            name: "Shell job".to_string(),
            kind: "shell".to_string(),
            schedule: "hourly".to_string(),
            payload: json!({
                "command": format!("Write-Output '{secret_token}'"),
                "workdir": workdir,
            }),
            enabled: true,
        };
        let job = service
            .cron_upsert(
                input.clone(),
                Some(test_native_cron_approval(
                    CronDangerousOperation::Save,
                    &input,
                )),
            )
            .unwrap();
        assert!(!job.enabled, "Shell Cron writes must start disabled");
        assert!(service.cron_set_enabled(&job.id, true, None).is_err());
        let approval = test_native_cron_approval(CronDangerousOperation::Enable, &job);
        let job = service
            .cron_set_enabled(&job.id, true, Some(approval))
            .expect("an explicit follow-up confirmation enables the Shell Cron");
        assert!(job.enabled);
        service
            .open()
            .unwrap()
            .execute(
                "UPDATE cron_jobs SET next_run_at=?2 WHERE id=?1",
                params![job.id, now_ms() - 1_000],
            )
            .unwrap();

        let claimed = Arc::clone(&service).cron_due_claim(Some(10)).await.unwrap();
        assert_eq!(claimed.runs.len(), 1);
        assert_eq!(claimed.runs[0].run.status, "completed");
        assert!(claimed.runs[0].dispatch.is_none());
        assert!(claimed.runs[0].run.has_output);
        assert!(!claimed.runs[0].run.has_error);

        let update = CronSchedulerUpdate::from_claim(&claimed);
        assert_eq!(update.claimed, 1);
        assert_eq!(update.completed, 1);
        assert_eq!(update.failed, 0);
        assert_eq!(update.running, 0);
        assert!(!serde_json::to_string(&update)
            .unwrap()
            .contains(secret_token));
        assert!(
            !serde_json::to_value(CronRunNowSummary::from(claimed.runs[0].clone()))
                .unwrap()
                .to_string()
                .contains(secret_token)
        );
        assert!(Arc::clone(&service)
            .cron_due_claim(Some(10))
            .await
            .unwrap()
            .runs
            .is_empty());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn cron_due_claim_marks_prompt_failed_without_provider_settings() {
        let root = TestDirectory::new("cron-prompt-no-provider");
        let service = Arc::new(LocalServices::for_test(root.path.clone()));
        let secret_prompt = "private scheduled prompt";
        let saved = confirmed_test_cron_upsert(
            service.as_ref(),
            CronUpsertInput {
                id: None,
                name: "Prompt job".to_string(),
                kind: "prompt".to_string(),
                schedule: "hourly".to_string(),
                payload: json!({"prompt": secret_prompt}),
                enabled: true,
            },
        );
        let job = enabled_test_cron(service.as_ref(), &saved);
        service
            .open()
            .unwrap()
            .execute(
                "UPDATE cron_jobs SET next_run_at=?2 WHERE id=?1",
                params![job.id, now_ms() - 1_000],
            )
            .unwrap();

        let claimed = Arc::clone(&service).cron_due_claim(Some(10)).await.unwrap();
        assert_eq!(claimed.runs.len(), 1);
        assert_eq!(claimed.runs[0].run.status, "failed");
        assert!(claimed.runs[0].dispatch.is_none());
        assert!(claimed.runs[0].run.has_error);
        assert!(!claimed.runs[0].run.has_output);

        let update = CronSchedulerUpdate::from_claim(&claimed);
        assert_eq!(update.claimed, 1);
        assert_eq!(update.completed, 0);
        assert_eq!(update.failed, 1);
        assert_eq!(update.running, 0);
        assert!(!serde_json::to_string(&update)
            .unwrap()
            .contains(secret_prompt));
    }
}
