//! Tauri application library — state, commands, and event bridge.

mod builds;
mod commands;
mod devserver;
mod explainer;
mod haven;
mod history;
mod meaning_candidates;
mod meaning_memory;
mod memory_recall;
mod operations;
mod qa_profile;
mod semantic_runtime;
mod state;
mod store_activation;
mod wizard_joe;
mod word_shapes;

use tauri::{Emitter, Manager};
use tracing::{info, warn};
use std::sync::{Arc, atomic::{AtomicU8, Ordering}};

/// Quit waits for observed cleanup; failed cleanup keeps the native window and
/// its recovery controls available. A repeated quit can retry the retained owner.
#[derive(Default)]
struct ExitGate(AtomicU8);
impl ExitGate {
    fn begin(&self) -> bool { self.0.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst).is_ok() }
    fn completed(&self) -> bool { self.0.load(Ordering::SeqCst) == 2 }
    fn finish(&self, success: bool) { self.0.store(if success { 2 } else { 0 }, Ordering::SeqCst); }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let exit_gate = Arc::new(ExitGate::default());
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Resolve/refuse QA native+browser aliases before store mutation or
            // creation of any webview with the normal production data store.
            let browser_isolation = qa_profile::discover()?;
            let app_handle = app.handle().clone();
            let state = tauri::async_runtime::block_on(AppState::initialize())?;
            let bus = state.event_bus.clone();
            let persistence = state.persistence.clone();
            let db_path = persistence.path().display().to_string();
            app.manage(state);

            // Subscribe before the task is scheduled. Only committed data reaches
            // the UI; lost notifications reconcile against the durable cursor.
            let mut rx = bus.subscribe_committed();
            let health_bus = bus.clone();
            let health_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            let _ = app_handle.emit("committed-event", &ev);
                            // Presentation/legacy observers receive the same
                            // committed event; the app owner consumes envelopes.
                            let _ = app_handle.emit("control-event", &ev.event);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(n, "UI notifications lagged; durable replay required");
                            let _ = app_handle.emit("event-reconcile-required", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            // Coverage failures are control-plane health, never fabricated
            // committed data. This remains visible when the writer cannot append.
            tauri::async_runtime::spawn(async move {
                let mut ticker = tokio::time::interval(std::time::Duration::from_millis(500));
                let mut previous = None;
                loop {
                    ticker.tick().await;
                    let health = health_bus.health();
                    let fingerprint = (health.healthy, health.error.clone(), health.generation);
                    if previous.as_ref() != Some(&fingerprint) {
                        let _ = health_handle.emit("event-health", &health);
                        previous = Some(fingerprint);
                    }
                }
            });

            info!(db = %db_path, "Bomb Code backend ready (SQLite thread memory)");
            let config = app.config().app.windows.first().ok_or("missing main window configuration")?;
            let mut window = tauri::WebviewWindowBuilder::from_config(app, config)?;
            if let Some(isolation) = browser_isolation {
                #[cfg(target_os = "macos")]
                { window = window.data_store_identifier(isolation.identifier); }
                #[cfg(not(target_os = "macos"))]
                { window = window.data_directory(isolation.directory); }
            }
            window.build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            wizard_joe::joe_status,
            wizard_joe::joe_analyze,
            wizard_joe::joe_cdiss_example,
            wizard_joe::joe_word_shape_replay,
            word_shapes::word_shape_dictionary,
            memory_recall::memory_recall,
            meaning_memory::meaning_memory,
            builds::get_build_concurrency,
            builds::set_build_concurrency,
            builds::list_builds,
            builds::create_build,
            builds::preview_build,
            builds::approve_build_plan,
            builds::accept_build,
            builds::cancel_build,
            builds::retry_build_cleanup,
            history::history_scan,
            history::history_stats,
            history::history_search,
            history::history_read,
            history::history_prepare,
            history::history_continue_native,
            history::history_import,
            history::history_open_original,
            commands::discover_environment,
            commands::list_backends,
            commands::get_config,
            commands::save_config,
            commands::capture_baseline,
            commands::get_runtime_status,
            commands::set_last_cwd,
            commands::create_project_folder,
            commands::get_auth_status,
            commands::backend_auth_status,
            commands::open_backend_login,
            commands::start_grok_login,
            commands::start_grok_login_oauth,
            commands::grok_login_status,
            commands::submit_grok_login_code,
            commands::open_grok_login_url,
            commands::cancel_grok_login,
            commands::logout_grok,
            commands::start_session,
            commands::start_mock_session,
            commands::list_sessions,
            commands::list_threads,
            commands::get_session,
            commands::get_session_transcript,
            commands::get_event_snapshot,
            commands::release_event_snapshot,
            commands::replay_events,
            commands::event_health,
            commands::send_prompt,
            commands::cancel_session,
            commands::remove_session,
            commands::set_plan_mode,
            commands::set_always_approve,
            commands::set_approval_mode,
            commands::add_session_allow_rule,
            commands::explainer_focus,
            commands::set_explainer_enabled,
            commands::set_explainer_provider,
            commands::respond_approval,
            commands::get_pending_approvals,
            commands::rename_thread,
            commands::land_thread,
            commands::sync_thread,
            commands::list_projects,
            commands::add_project,
            commands::remove_project,
            commands::list_worktrees,
            commands::create_worktree,
            commands::remove_worktree,
            commands::worktree_diff,
            commands::prune_worktrees,
            commands::list_permission_presets,
            commands::evaluate_permission,
            commands::list_extensions,
            commands::add_mcp,
            commands::remove_mcp,
            commands::toggle_mcp,
            commands::list_mcp_servers,
            commands::get_mcp_server,
            commands::add_mcp_server,
            commands::update_mcp_server,
            commands::remove_mcp_server,
            commands::doctor_mcp_server,
            commands::list_mcp_tools,
            commands::list_mcp_catalog,
            commands::set_mcp_credential,
            commands::list_mcp_credentials,
            commands::remove_mcp_credential,
            commands::suggest_mcp_for_project,
            commands::preview_session_mcp,
            commands::add_skill,
            commands::remove_skill,
            commands::extensions_doctor,
            commands::memory_list,
            commands::memory_add,
            commands::memory_remove,
            commands::memory_flush,
            commands::memory_digest,
            commands::remember,
            commands::project_scope,
            commands::scheduler_list,
            commands::scheduler_add,
            commands::scheduler_cancel,
            commands::scheduler_pause,
            commands::scheduler_resume,
            commands::diff_current,
            commands::diff_capture_before,
            commands::diff_capture_after,
            commands::export_session_markdown,
            commands::list_persisted_sessions,
            commands::persistence_checkpoint,
            commands::shutdown_all,
            commands::detect_dev_server,
            commands::start_dev_server,
            commands::stop_dev_server,
            commands::dev_server_status,
            commands::open_dev_server,
            commands::reveal_project,
            commands::haven_status,
            commands::haven_get_config,
            commands::haven_set_config,
            commands::haven_list_jobs,
            commands::haven_start_shell,
            commands::haven_job_log,
            commands::haven_remove_job,
            commands::haven_list_files,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |handle, event| match event {
            tauri::RunEvent::WindowEvent { label, event: tauri::WindowEvent::CloseRequested { api, .. }, .. }
                if label == "main" && !exit_gate.completed() => {
                api.prevent_close();
                handle.exit(0);
            }
            tauri::RunEvent::ExitRequested { api, code, .. } if !exit_gate.completed() => {
                api.prevent_exit();
                if !exit_gate.begin() { return; }
                let handle = handle.clone();
                let gate = exit_gate.clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle.state::<AppState>();
                    match state.shutdown_for_exit().await {
                        Ok(()) => { gate.finish(true); handle.exit(code.unwrap_or(0)); }
                        Err(error) => {
                            gate.finish(false);
                            warn!(error = %error, "quit cleanup unresolved; application retained");
                            state.event_bus.emit_error(None, format!("Quit could not verify cleanup. The application and recovery controls remain open. Retry Stop or Quit after inspecting the unresolved session. {error}"));
                        }
                    }
                });
            }
            _ => {}
        });
}

pub use state::AppState;

#[cfg(test)]
mod exit_tests {
    use super::ExitGate;
    #[test]
    fn duplicate_quit_does_not_duplicate_cleanup_and_failure_can_retry() {
        let gate = ExitGate::default();
        assert!(gate.begin());
        assert!(!gate.begin());
        assert!(!gate.completed());
        gate.finish(false);
        assert!(gate.begin());
        gate.finish(true);
        assert!(gate.completed());
        assert!(!gate.begin());
    }
}
