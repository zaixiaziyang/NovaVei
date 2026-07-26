//! NovaVei Tauri host.
//!
//! Pi itself runs in the embedded WebView. The native side only exposes local
//! capabilities and a transport hook for streaming events. There is no second
//! desktop process or sidecar agent.

mod backend;
mod browser;
mod diagnostics;
mod history_store;
mod local_services;
mod mcp_registry;
mod mcp_runtime;
mod path_display;
mod proxy;
mod secret_store;
mod storage;
mod subagent_store;
mod worktree_runtime;

#[cfg(feature = "desktop")]
use std::sync::Arc;
#[cfg(feature = "desktop")]
use std::time::Duration;

#[cfg(feature = "desktop")]
use tauri::Emitter;
#[cfg(feature = "desktop")]
use tauri::WebviewWindowBuilder;
#[cfg(feature = "desktop")]
use tokio::sync::OwnedSemaphorePermit;

#[cfg(feature = "desktop")]
const CRON_SCHEDULER_EVENT: &str = "cron:scheduler-update";
#[cfg(feature = "desktop")]
const CRON_SCHEDULER_INTERVAL: Duration = Duration::from_secs(30);
const CRON_SCHEDULER_CLAIM_LIMIT: usize = 10;

/// A bounded native worker pool for due Cron work. It deliberately has no
/// in-memory backlog: a task is claimed only after its execution slot has
/// been reserved, keeping `running` leases truthful and recoverable.
#[cfg(feature = "desktop")]
struct CronWorkerPool {
    app_handle: tauri::AppHandle,
    services: Arc<local_services::LocalServices>,
    execution_pool: local_services::CronExecutionPool,
}

#[cfg(feature = "desktop")]
impl CronWorkerPool {
    fn new(app_handle: tauri::AppHandle, services: Arc<local_services::LocalServices>) -> Self {
        Self {
            app_handle,
            execution_pool: services.cron_execution_pool(),
            services,
        }
    }

    /// Reserve only immediately executable slots before touching the due-job
    /// transaction. Extra due jobs stay due for the next 30-second check.
    fn reserve_slots(&self, limit: usize) -> Vec<OwnedSemaphorePermit> {
        self.execution_pool.reserve_available_slots(limit)
    }

    fn dispatch(
        &self,
        job: local_services::CronJob,
        run: local_services::CronRun,
        permit: OwnedSemaphorePermit,
    ) {
        let app_handle = self.app_handle.clone();
        let services = Arc::clone(&self.services);
        tauri::async_runtime::spawn(async move {
            // Retain the permit for the complete native execution. Dropping it
            // before publishing the terminal update makes a slot available to
            // the next scheduler tick without blocking that tick on this work.
            // RAII also releases it if the executor future is cancelled.
            let permit = permit;
            let update = match services.execute_claimed_cron(job, run).await {
                Ok(response) => local_services::CronSchedulerUpdate::from_execution(&response),
                Err(_) => {
                    diagnostics::record_event(
                        "cron",
                        "scheduler_execution_failed",
                        "failure",
                        None,
                    );
                    local_services::CronSchedulerUpdate::failed_execution()
                }
            };
            drop(permit);
            let _ = app_handle.emit(CRON_SCHEDULER_EVENT, update);
        });
    }
}

#[cfg(feature = "desktop")]
fn start_cron_scheduler(
    app_handle: tauri::AppHandle,
    services: Arc<local_services::LocalServices>,
) {
    let worker_pool = CronWorkerPool::new(app_handle.clone(), Arc::clone(&services));
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(CRON_SCHEDULER_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            // Tokio's first interval tick completes immediately, so overdue
            // jobs are checked once during startup and then at a bounded rate.
            interval.tick().await;
            let permits = worker_pool.reserve_slots(CRON_SCHEDULER_CLAIM_LIMIT);
            if permits.is_empty() {
                // Workers are still busy. Do not claim a run into a memory
                // queue; it remains due and will be reconsidered next tick.
                let _ = app_handle.emit(
                    CRON_SCHEDULER_EVENT,
                    local_services::CronSchedulerUpdate::from_claim(
                        &local_services::CronDueClaimResponse {
                            claimed_at: chrono::Utc::now().timestamp_millis(),
                            runs: Vec::new(),
                        },
                    ),
                );
                continue;
            }

            let claim_limit = permits.len();
            let claim_services = Arc::clone(&services);
            match tauri::async_runtime::spawn_blocking(move || {
                claim_services.cron_claim_due_for_scheduler(claim_limit)
            })
            .await
            {
                Ok(Ok((response, claims))) => {
                    // Publish the redacted claim/running counts before a fast
                    // worker can publish its terminal result, preserving event
                    // order without awaiting any execution.
                    let _ = app_handle.emit(
                        CRON_SCHEDULER_EVENT,
                        local_services::CronSchedulerUpdate::from_claim(&response),
                    );
                    for ((job, run), permit) in claims.into_iter().zip(permits) {
                        worker_pool.dispatch(job, run, permit);
                    }
                }
                Ok(Err(_)) | Err(_) => {
                    diagnostics::record_event(
                        "cron",
                        "scheduler_due_check_failed",
                        "failure",
                        None,
                    );
                    let _ = app_handle.emit(
                        CRON_SCHEDULER_EVENT,
                        local_services::CronSchedulerUpdate::failed_check(),
                    );
                }
            }
        }
    });
}

#[cfg(feature = "desktop")]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(error) = storage::initialize() {
        // There is no trustworthy data root for diagnostics yet. Fail closed
        // instead of allowing a linked portable directory to redirect writes.
        // A windowed executable has no visible stderr, so the reason must be
        // surfaced through a native dialog (read-only USB media is the most
        // common trigger for a portable package).
        report_fatal_startup_error(&error);
        return;
    }
    // A second process sharing the same data root would race the SQLite WAL
    // files and the WebView2 profile. Hold an exclusive lock file inside the
    // resolved root for the process lifetime; a portable copy launched twice
    // (or an installed build started twice) fails closed with a clear message.
    let _instance_lock = match storage::acquire_instance_lock() {
        Ok(lock) => lock,
        Err(error) => {
            report_fatal_startup_error(&error);
            return;
        }
    };
    // Diagnostics initialization is intentionally best-effort: the desktop
    // shell must remain usable even when a local log directory is unavailable.
    let _ = diagnostics::initialize();
    let state = Arc::new(backend::AppState::new());
    // Local services deliberately retain initialization failures and retry on
    // later requests instead of preventing the desktop shell from starting.
    let local_services = Arc::new(local_services::LocalServices::new());
    // MCP clients are native-owned: the renderer can select a configured
    // server but never provides its command, URL, headers, or environment.
    let mcp_runtime = Arc::new(mcp_runtime::McpRuntimeManager::new());
    let resolver_state = state.clone();
    let resolver: proxy::CredentialResolver = Arc::new(move |provider_id| {
        backend::provider_proxy_config(&resolver_state, provider_id).map(|config| {
            proxy::ProviderCredentials {
                headers: config.headers,
                upstream_base_url: config.upstream_base_url,
                use_system_proxy: config.use_system_proxy,
            }
        })
    });
    // The provider proxy is an optional transport dependency. Its runtime
    // retains a failed startup as an unavailable state rather than panicking,
    // so the desktop shell can start and proxy-dependent calls can return a
    // stable error (or retry through the explicit runtime command).
    let proxy_state = proxy::start_proxy_runtime(resolver);
    let scheduler_services = Arc::clone(&local_services);

    tauri::Builder::default()
        .manage(state)
        .manage(local_services)
        .manage(mcp_runtime)
        .manage(proxy_state)
        .setup(move |app| {
            // The configured main window is created here rather than by the
            // declarative auto-create path so a portable executable can keep
            // WebView2's localStorage, cache, and profile beside itself.
            let main_config = app
                .config()
                .app
                .windows
                .first()
                .ok_or("main window configuration is missing")?;
            let builder = WebviewWindowBuilder::from_config(app.handle(), main_config)?;
            let builder = if storage::is_portable() {
                builder.data_directory(storage::application_data_dir().join("webview-main"))
            } else {
                builder
            };
            builder.build()?;

            // Stored Cron jobs can run shell, HTTP, or prompt work. Do not
            // execute any of them while a removable drive is still locked.
            // Portable scheduling stays deliberately paused for this process;
            // it requires an explicit future re-enable flow after unlock.
            if !secret_store::portable_storage_needs_unlock() && !storage::is_portable() {
                start_cron_scheduler(app.handle().clone(), Arc::clone(&scheduler_services));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Host / session compatibility
            backend::system_info,
            backend::storage_mode_status,
            backend::storage_mode_set,
            backend::diagnostics_export,
            backend::storage_recovery_status,
            backend::portable_storage_status,
            backend::portable_storage_unlock,
            backend::portable_storage_recover,
            backend::app_health,
            // Native child WebView browser. The external page's own webview
            // receives no app capability; only the main renderer can invoke
            // this compact controller surface.
            backend::browser_open,
            backend::browser_layout,
            backend::browser_back,
            backend::browser_reload,
            backend::browser_status,
            backend::browser_agent_navigate,
            backend::browser_agent_snapshot,
            backend::browser_agent_click,
            backend::browser_agent_type,
            backend::sessions_list,
            backend::sessions_create,
            backend::workspace_pick,
            backend::workspace_reveal,
            backend::workspace_paths_status,
            backend::sessions_relocate_workspace,
            backend::sessions_relocate_workspace_cancel,
            backend::workspace_register_project,
            backend::workspace_pick_files,
            backend::composer_pick_attachments,
            backend::composer_stage_pasted_image,
            backend::composer_media_load,
            backend::composer_media_discard,
            backend::sessions_get,
            backend::sessions_is_blank,
            backend::session_goal_get,
            backend::session_goal_set,
            backend::session_goal_progress_update,
            backend::history_context_load,
            backend::history_context_compaction_source_load,
            backend::session_context_compaction_set,
            backend::session_context_compaction_clear,
            backend::sessions_send,
            backend::providers_list,
            backend::provider_test,
            backend::provider_models_fetch,
            backend::provider_draft_prepare,
            backend::provider_draft_test,
            backend::provider_draft_models_fetch,
            backend::provider_import_preview,
            backend::provider_import_apply,
            // Embedded Pi transport and permission broker
            backend::full_permission_confirm,
            backend::agent_run,
            backend::agent_cancel,
            backend::agent_permission,
            backend::agent_emit_event,
            backend::agent_event,
            // Subagent lifecycle and isolated child read capabilities. The
            // model still receives the user-approved delegated task itself
            // through the embedded runtime, not through durable task storage.
            backend::subagent_task_start,
            backend::subagent_tasks_list,
            backend::subagent_task_get,
            backend::subagent_task_finish,
            backend::subagent_task_cancel,
            // A worktree child can change only its native-provisioned detached
            // checkout. Patch review and native confirmation remain separate
            // renderer-visible commands; nothing applies automatically.
            backend::worktree_task_start,
            backend::worktree_task_finish,
            backend::worktree_task_review_get,
            backend::worktree_task_apply,
            backend::worktree_task_discard,
            // Settings persistence
            backend::settings_load_all,
            backend::provider_runtime_config,
            backend::proxy_transport_info,
            backend::settings_save_providers,
            backend::settings_save_system,
            backend::settings_save_projects,
            backend::settings_save_mcp,
            backend::settings_save_agents,
            backend::settings_save_ssh,
            backend::settings_save_remote,
            backend::settings_save_memory,
            // Native-only MCP runtime. Server configuration is resolved from
            // protected settings rather than renderer-supplied payloads.
            backend::mcp_list_tools,
            backend::mcp_call_tool,
            backend::mcp_runtime_status,
            backend::mcp_test_server,
            backend::mcp_stop_server,
            backend::mcp_restart_server,
            backend::memory_agent_create,
            // Read-only public MCP Registry discovery. Imported records still
            // enter the existing protected settings flow disabled by default.
            mcp_registry::mcp_registry_list,
            mcp_registry::mcp_registry_get,
            mcp_registry::mcp_registry_remote_draft,
            // Local Skills, durable Memory, and native Cron services
            local_services::skills_list,
            local_services::skills_read,
            local_services::agent_skills_list,
            local_services::agent_skills_read,
            local_services::skills_enable,
            local_services::skills_disable,
            local_services::skills_install_pick,
            local_services::skills_catalog_list,
            local_services::skills_catalog_search,
            local_services::skills_catalog_detail,
            local_services::skills_catalog_install,
            local_services::memory_create,
            local_services::memory_read,
            local_services::memory_list,
            local_services::memory_update,
            local_services::memory_delete,
            local_services::memory_search,
            local_services::memory_stats,
            local_services::memory_clear,
            local_services::memory_organize,
            local_services::memory_export,
            local_services::memory_usage_export,
            local_services::knowledge_base_pick_folder,
            local_services::knowledge_base_list,
            local_services::knowledge_base_set_enabled,
            local_services::knowledge_base_refresh,
            local_services::knowledge_base_remove,
            local_services::knowledge_base_search,
            local_services::knowledge_base_read,
            local_services::knowledge_base_agent_begin,
            local_services::knowledge_base_agent_search,
            local_services::knowledge_base_agent_read,
            local_services::cron_schedule_validate,
            local_services::cron_list,
            local_services::cron_upsert,
            local_services::cron_set_enabled,
            local_services::cron_delete,
            local_services::cron_runs,
            local_services::cron_run_now,
            // Chat history persistence
            backend::chat_history_list,
            backend::chat_history_workdirs,
            backend::chat_history_get,
            backend::chat_history_get_active_segment,
            backend::chat_history_search,
            backend::session_metadata_search,
            backend::chat_history_trace_get,
            backend::chat_history_upsert,
            backend::chat_history_upsert_active_segment,
            backend::chat_history_append_segment,
            backend::chat_history_rename,
            backend::chat_history_set_pinned,
            backend::chat_history_set_model,
            backend::chat_history_set_archived,
            backend::chat_history_bulk_set_archived,
            backend::chat_history_branch,
            backend::chat_history_delete,
            backend::chat_history_bulk_delete,
            backend::chat_history_share_get,
            backend::chat_history_share_set,
            // Workspace and shell tools
            backend::fs_read_text,
            backend::fs_read_global_text,
            backend::fs_read_editable_text,
            backend::fs_path_status,
            backend::fs_grep,
            backend::fs_write_text,
            backend::fs_edit_text,
            backend::fs_delete,
            backend::fs_list,
            backend::fs_roots,
            backend::shell_run,
            backend::shell_cancel,
            backend::workspace_capability_issue,
            // Local provider proxy metadata and bounded recovery controls.
            proxy::proxy_runtime_status,
            proxy::proxy_runtime_retry,
        ])
        .run(tauri::generate_context!())
        .expect("error while running NovaVei");
}

/// Startup failures happen before any WebView exists, and a
/// `windows_subsystem = "windows"` executable has no console. Without a
/// native dialog a portable user on read-only media sees the process exit
/// silently. stderr is still written for terminal launches and CI.
#[cfg(feature = "desktop")]
fn report_fatal_startup_error(error: &str) {
    eprintln!("NovaVei startup failed: {error}");
    #[cfg(windows)]
    {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("NovaVei 无法启动")
            .set_description(format!(
                "NovaVei 无法启动：{error}\n\n如果这是便携版，请确认其所在文件夹可写（未写保护、非只读介质），且没有另一个 NovaVei 正在使用同一数据目录。"
            ))
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_execution_pool_uses_ten_permits_and_reclaims_a_slot_only_after_release() {
        assert_eq!(CRON_SCHEDULER_CLAIM_LIMIT, 10);
        assert_eq!(
            CRON_SCHEDULER_CLAIM_LIMIT,
            local_services::CRON_WORKER_POOL_SIZE
        );

        let execution_pool = local_services::CronExecutionPool::new();
        let mut reserved = execution_pool.reserve_available_slots(CRON_SCHEDULER_CLAIM_LIMIT);
        assert_eq!(reserved.len(), 10);
        assert!(execution_pool.reserve_available_slots(1).is_empty());

        drop(reserved.pop());
        let after_release = execution_pool.reserve_available_slots(1);
        assert_eq!(after_release.len(), 1);
    }
}
