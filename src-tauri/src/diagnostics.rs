//! Privacy-preserving native diagnostics.
//!
//! This module is intentionally narrower than a general application logger.
//! It accepts only fixed event names and stores no raw error text. Diagnostic
//! exports contain aggregate history health plus allowlisted event metadata.

use crate::history_store::{HistoryDiagnosticsSummary, HistoryStore};
use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tracing_appender::non_blocking::WorkerGuard;

const DIAGNOSTICS_DIRECTORY_NAME: &str = "diagnostics";
const DIAGNOSTICS_LOG_PREFIX: &str = "novavei";
const DIAGNOSTICS_LOG_RETENTION_DAYS: usize = 7;
const MAX_DIAGNOSTIC_LOG_FILE_BYTES: u64 = 256 * 1024;
const MAX_EXPORTED_LOG_EVENTS: usize = 200;

static DIAGNOSTICS_LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();
static DIAGNOSTICS_INITIALIZED: OnceLock<()> = OnceLock::new();

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsExportResponse {
    pub bytes: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsReport {
    schema_version: u8,
    generated_at: String,
    application: DiagnosticsApplicationMetadata,
    storage: DiagnosticsStorageMetadata,
    history: HistoryDiagnosticsSummary,
    events: Vec<DiagnosticsEvent>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsApplicationMetadata {
    version: String,
    operating_system: String,
    architecture: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsStorageMetadata {
    state: String,
    error_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsEvent {
    timestamp: String,
    level: String,
    component: String,
    event: String,
    outcome: String,
    detail: String,
}

pub fn application_data_dir() -> PathBuf {
    crate::storage::application_data_dir()
}

fn diagnostics_log_dir() -> PathBuf {
    application_data_dir().join(DIAGNOSTICS_DIRECTORY_NAME)
}

pub fn initialize() -> Result<(), String> {
    if DIAGNOSTICS_INITIALIZED.get().is_some() {
        return Ok(());
    }
    let log_directory = diagnostics_log_dir();
    fs::create_dir_all(&log_directory)
        .map_err(|error| format!("create diagnostics directory: {error}"))?;
    prune_log_files(&log_directory)?;

    let appender = tracing_appender::rolling::daily(&log_directory, DIAGNOSTICS_LOG_PREFIX);
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_ansi(false)
        .with_current_span(false)
        .with_span_list(false)
        .with_writer(writer)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|error| format!("initialize diagnostics subscriber: {error}"))?;
    DIAGNOSTICS_LOG_GUARD
        .set(guard)
        .map_err(|_| "retain diagnostics log writer".to_string())?;
    let _ = DIAGNOSTICS_INITIALIZED.set(());
    record_event("app", "native_host_started", "success", None);
    Ok(())
}

/// Write only static diagnostic event names. Dynamic error text is never
/// retained, so copied provider responses and local filesystem details cannot
/// reach an exported diagnostics package.
pub fn record_event(
    component: &'static str,
    event: &'static str,
    outcome: &'static str,
    detail: Option<&str>,
) {
    let detail = detail.map(sanitize_detail).unwrap_or_default();
    tracing::info!(component, event, outcome, detail, "diagnostic_event");
}

fn sanitize_detail(value: &str) -> String {
    match value.trim() {
        "user_cancelled"
        | "storage_unavailable"
        | "unsupported_platform"
        | "initialization_failed" => value.trim().to_string(),
        _ => "redacted".to_string(),
    }
}

pub fn export_diagnostics(
    history: &HistoryStore,
    storage_state: &str,
    storage_error_count: usize,
) -> Result<Option<DiagnosticsExportResponse>, String> {
    let report = build_report(history, storage_state, storage_error_count)?;
    let bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("serialize diagnostics export: {error}"))?;
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
    let selected = rfd::FileDialog::new()
        .set_title("Export NovaVei Diagnostics")
        .set_file_name(format!("novavei-diagnostics-{timestamp}.json"))
        .add_filter("JSON", &["json"])
        .save_file();
    let Some(path) = selected else {
        record_event("diagnostics", "export", "skipped", Some("user_cancelled"));
        return Ok(None);
    };
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|error| format!("create diagnostics export: {error}"))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("write diagnostics export: {error}"))?;
    record_event("diagnostics", "export", "success", None);
    Ok(Some(DiagnosticsExportResponse { bytes: bytes.len() }))
}

fn build_report(
    history: &HistoryStore,
    storage_state: &str,
    storage_error_count: usize,
) -> Result<DiagnosticsReport, String> {
    Ok(DiagnosticsReport {
        schema_version: 1,
        generated_at: Utc::now().to_rfc3339(),
        application: DiagnosticsApplicationMetadata {
            version: env!("CARGO_PKG_VERSION").to_string(),
            operating_system: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
        },
        storage: DiagnosticsStorageMetadata {
            state: sanitize_storage_state(storage_state),
            error_count: storage_error_count,
        },
        history: history.diagnostics_summary()?,
        events: read_recent_events(&diagnostics_log_dir())?,
    })
}

fn sanitize_storage_state(value: &str) -> String {
    match value {
        "ready" | "degraded" => value.to_string(),
        _ => "unknown".to_string(),
    }
}

fn prune_log_files(directory: &Path) -> Result<(), String> {
    let mut files = fs::read_dir(directory)
        .map_err(|error| format!("read diagnostics directory: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(DIAGNOSTICS_LOG_PREFIX)
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|entry| entry.file_name());
    let excess = files.len().saturating_sub(DIAGNOSTICS_LOG_RETENTION_DAYS);
    for entry in files.into_iter().take(excess) {
        fs::remove_file(entry.path())
            .map_err(|error| format!("remove expired diagnostics log: {error}"))?;
    }
    Ok(())
}

fn read_recent_events(directory: &Path) -> Result<Vec<DiagnosticsEvent>, String> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut files = fs::read_dir(directory)
        .map_err(|error| format!("read diagnostics events: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(DIAGNOSTICS_LOG_PREFIX)
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|entry| entry.file_name());
    let mut events = Vec::new();
    for entry in files.into_iter().rev().take(DIAGNOSTICS_LOG_RETENTION_DAYS) {
        let mut contents = String::new();
        fs::File::open(entry.path())
            .map_err(|error| format!("open diagnostics log: {error}"))?
            .take(MAX_DIAGNOSTIC_LOG_FILE_BYTES)
            .read_to_string(&mut contents)
            .map_err(|error| format!("read diagnostics log: {error}"))?;
        for line in contents.lines() {
            if events.len() >= MAX_EXPORTED_LOG_EVENTS {
                break;
            }
            if let Some(event) = parse_sanitized_event(line) {
                events.push(event);
            }
        }
    }
    events.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
    events.truncate(MAX_EXPORTED_LOG_EVENTS);
    Ok(events)
}

fn parse_sanitized_event(line: &str) -> Option<DiagnosticsEvent> {
    let record = serde_json::from_str::<Value>(line).ok()?;
    let fields = record.get("fields")?.as_object()?;
    let component = allowed_component(fields.get("component")?.as_str()?)?;
    let event = allowed_event(fields.get("event")?.as_str()?)?;
    let outcome = allowed_outcome(fields.get("outcome")?.as_str()?)?;
    let detail = fields
        .get("detail")
        .and_then(Value::as_str)
        .map(sanitize_detail)
        .unwrap_or_default();
    Some(DiagnosticsEvent {
        timestamp: bounded_timestamp(record.get("timestamp").and_then(Value::as_str)?),
        level: allowed_level(record.get("level").and_then(Value::as_str)?).to_string(),
        component: component.to_string(),
        event: event.to_string(),
        outcome: outcome.to_string(),
        detail,
    })
}

fn allowed_component(value: &str) -> Option<&'static str> {
    match value {
        "app" => Some("app"),
        "storage" => Some("storage"),
        "proxy" => Some("proxy"),
        "mcp" => Some("mcp"),
        "cron" => Some("cron"),
        "diagnostics" => Some("diagnostics"),
        _ => None,
    }
}

fn allowed_event(value: &str) -> Option<&'static str> {
    match value {
        "native_host_started" => Some("native_host_started"),
        "settings_unlock_failed" => Some("settings_unlock_failed"),
        "recovery_required" => Some("recovery_required"),
        "secure_compaction_failed" => Some("secure_compaction_failed"),
        "initial_persist_failed" => Some("initial_persist_failed"),
        "listener_conversion_failed" => Some("listener_conversion_failed"),
        "server_stopped" => Some("server_stopped"),
        "scheduler_due_check_failed" => Some("scheduler_due_check_failed"),
        "export" => Some("export"),
        _ => None,
    }
}

fn allowed_outcome(value: &str) -> Option<&'static str> {
    match value {
        "success" => Some("success"),
        "failure" => Some("failure"),
        "skipped" => Some("skipped"),
        _ => None,
    }
}

fn allowed_level(value: &str) -> &'static str {
    match value {
        "INFO" => "INFO",
        "WARN" => "WARN",
        "ERROR" => "ERROR",
        _ => "INFO",
    }
}

fn bounded_timestamp(value: &str) -> String {
    value.chars().take(64).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_redaction_never_returns_raw_dynamic_detail() {
        let detail = sanitize_detail("Bearer test-secret-value C:\\private\\notes.txt");

        assert_eq!(detail, "redacted");
    }

    #[test]
    fn diagnostic_event_parser_drops_unallowlisted_fields() {
        let raw = r#"{"timestamp":"2026-07-25T00:00:00Z","level":"INFO","fields":{"component":"storage","event":"settings_unlock_failed","outcome":"failure","detail":"test-secret-value"},"secret":"leak"}"#;

        let event = parse_sanitized_event(raw).expect("safe diagnostic event");

        assert_eq!(event.component, "storage");
        assert_eq!(event.event, "settings_unlock_failed");
        assert_eq!(event.detail, "redacted");
        let serialized = serde_json::to_string(&event).unwrap();
        assert!(!serialized.contains("test-secret-value"));
        assert!(!serialized.contains("leak"));
    }

    #[test]
    fn diagnostic_event_parser_rejects_unknown_event_names() {
        let raw = r#"{"timestamp":"2026-07-25T00:00:00Z","level":"INFO","fields":{"component":"storage","event":"secret_exfiltration","outcome":"failure"}}"#;

        assert!(parse_sanitized_event(raw).is_none());
    }

    #[test]
    fn log_pruning_keeps_the_bounded_retention_window() {
        let directory = std::env::temp_dir().join(format!(
            "novavei-diagnostics-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&directory).unwrap();
        for index in 0..=DIAGNOSTICS_LOG_RETENTION_DAYS {
            fs::write(
                directory.join(format!("novavei.2026-07-{:02}", index + 1)),
                "{}\n",
            )
            .unwrap();
        }

        prune_log_files(&directory).unwrap();

        let retained = fs::read_dir(&directory).unwrap().count();
        assert_eq!(retained, DIAGNOSTICS_LOG_RETENTION_DAYS);
        let _ = fs::remove_dir_all(directory);
    }
}
