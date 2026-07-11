//! Tauri application library — state, commands, and event bridge.

mod commands;
mod state;

use tauri::{Emitter, Manager};
use tracing::info;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let app_handle = app.handle().clone();
            let state = tauri::async_runtime::block_on(AppState::initialize())?;
            let bus = state.event_bus.clone();
            app.manage(state);

            // Forward backend events to the frontend
            tauri::async_runtime::spawn(async move {
                let mut rx = bus.subscribe();
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            let _ = app_handle.emit("control-event", &ev);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(n, "event bus lagged");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            info!("Grok Build Control Panel backend ready");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::discover_environment,
            commands::get_config,
            commands::save_config,
            commands::capture_baseline,
            commands::start_session,
            commands::start_mock_session,
            commands::list_sessions,
            commands::get_session,
            commands::send_prompt,
            commands::cancel_session,
            commands::remove_session,
            commands::set_plan_mode,
            commands::respond_approval,
            commands::list_worktrees,
            commands::create_worktree,
            commands::remove_worktree,
            commands::worktree_diff,
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
            commands::suggest_mcp_for_project,
            commands::preview_session_mcp,
            commands::add_skill,
            commands::remove_skill,
            commands::extensions_doctor,
            commands::memory_list,
            commands::memory_add,
            commands::memory_remove,
            commands::memory_flush,
            commands::memory_dream,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

pub use state::AppState;
