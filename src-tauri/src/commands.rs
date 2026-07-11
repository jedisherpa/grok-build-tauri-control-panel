//! Tauri invoke command surface for the control panel.

use std::path::PathBuf;

use chrono::Utc;
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use grok_config::{DiscoveryReport, GrokConfig};
use grok_control_core::{AgentHandleSnapshot, SessionStatus, SpawnOptions};
use grok_diff::{DiffCapture, DiffEngine, DiffSummary};
use grok_extensions::ExtensionEntry;
use grok_memory::MemoryEntry;
use grok_permissions::{builtin_presets, PermissionController, PermissionDecision, PermissionPreset};
use grok_persistence::SessionRecord;
use grok_scheduler::{ScheduleKind, ScheduledJob};
use grok_worktree::{CreateWorktreeRequest, WorktreeInfo};

use crate::state::AppState;

fn err(e: impl ToString) -> String {
    e.to_string()
}

// ── Phase 0: Discovery & Config ──────────────────────────────────────────

#[tauri::command]
pub async fn discover_environment() -> Result<DiscoveryReport, String> {
    grok_config::discover_environment().map_err(err)
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<GrokConfig, String> {
    Ok(state.config.read().await.clone())
}

#[tauri::command]
pub async fn save_config(state: State<'_, AppState>, config: GrokConfig) -> Result<(), String> {
    {
        let mut cfg = state.config.write().await;
        *cfg = config;
        cfg.save(&state.paths.config_file).map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn capture_baseline(
    state: State<'_, AppState>,
) -> Result<grok_cli_wrapper::BaselineSnapshot, String> {
    Ok(state.grok_cli.capture_baseline().await)
}

// ── Phase 1: Sessions ────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SessionIdResponse {
    pub id: String,
}

#[tauri::command]
pub async fn start_session(
    state: State<'_, AppState>,
    cwd: String,
    opts: SpawnOptions,
) -> Result<SessionIdResponse, String> {
    let id = state.registry.spawn_agent(&cwd, opts).await.map_err(err)?;
    persist_session(&state, id).await;
    Ok(SessionIdResponse {
        id: id.to_string(),
    })
}

#[tauri::command]
pub async fn start_mock_session(
    state: State<'_, AppState>,
    cwd: String,
) -> Result<SessionIdResponse, String> {
    let id = state.registry.spawn_mock(&cwd).await.map_err(err)?;
    persist_session(&state, id).await;
    Ok(SessionIdResponse {
        id: id.to_string(),
    })
}

#[tauri::command]
pub async fn list_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<grok_control_core::SessionMetadata>, String> {
    Ok(state.registry.list_sessions())
}

#[tauri::command]
pub async fn get_session(
    state: State<'_, AppState>,
    id: String,
) -> Result<AgentHandleSnapshot, String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    state.registry.get_snapshot(id).map_err(err)
}

#[tauri::command]
pub async fn send_prompt(
    state: State<'_, AppState>,
    id: String,
    prompt: String,
) -> Result<(), String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    state.registry.send_prompt(id, &prompt).await.map_err(err)?;
    let _ = state.persistence.append_transcript(&grok_persistence::TranscriptChunk {
        session_id: id,
        seq: Utc::now().timestamp_millis() as u64,
        kind: "prompt".into(),
        payload: prompt,
        at: Utc::now(),
    });
    Ok(())
}

#[tauri::command]
pub async fn cancel_session(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    state.registry.cancel_session(id).await.map_err(err)
}

#[tauri::command]
pub async fn remove_session(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    state.registry.remove_session(id).await.map_err(err)?;
    let _ = state.persistence.delete_session(id);
    Ok(())
}

#[tauri::command]
pub async fn set_plan_mode(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    state
        .registry
        .set_plan_mode(id, enabled)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn respond_approval(
    state: State<'_, AppState>,
    id: String,
    request_id: String,
    approved: bool,
) -> Result<(), String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    state
        .registry
        .respond_approval(id, &request_id, approved)
        .await
        .map_err(err)
}

// ── Phase 2: Worktrees & Permissions ─────────────────────────────────────

#[tauri::command]
pub async fn list_worktrees(
    state: State<'_, AppState>,
    repo: String,
) -> Result<Vec<WorktreeInfo>, String> {
    state
        .worktrees
        .list(PathBuf::from(repo).as_path())
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn create_worktree(
    state: State<'_, AppState>,
    repo: String,
    name: String,
    base_ref: Option<String>,
) -> Result<WorktreeInfo, String> {
    state
        .worktrees
        .create(
            PathBuf::from(repo).as_path(),
            CreateWorktreeRequest {
                name,
                base_ref,
                prefer_grok_cli: true,
            },
        )
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn remove_worktree(
    state: State<'_, AppState>,
    repo: String,
    name: String,
    force: bool,
) -> Result<(), String> {
    state
        .worktrees
        .remove(PathBuf::from(repo).as_path(), &name, force)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn worktree_diff(
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    state
        .worktrees
        .diff(PathBuf::from(path).as_path())
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn list_permission_presets() -> Result<Vec<PermissionPreset>, String> {
    Ok(builtin_presets())
}

#[derive(Debug, Serialize)]
pub struct PermissionEvalResult {
    pub decision: PermissionDecision,
}

#[tauri::command]
pub async fn evaluate_permission(
    state: State<'_, AppState>,
    tool: String,
    detail: String,
    preset: Option<String>,
) -> Result<PermissionEvalResult, String> {
    let cfg = state.config.read().await;
    let mut ctl = if let Some(name) = preset {
        let presets = builtin_presets();
        let p = presets
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| format!("unknown preset: {name}"))?;
        PermissionController::with_preset(p)
    } else {
        PermissionController::from_defaults(&cfg.permissions, cfg.sandbox_profile)
    };
    // silence mut if unused later
    let _ = &mut ctl;
    Ok(PermissionEvalResult {
        decision: ctl.evaluate(&tool, &detail),
    })
}

// ── Phase 3: Extensions / Memory / Scheduler ─────────────────────────────

#[tauri::command]
pub async fn list_extensions(state: State<'_, AppState>) -> Result<Vec<ExtensionEntry>, String> {
    Ok(state.extensions.list_all().await)
}

#[tauri::command]
pub async fn add_mcp(
    state: State<'_, AppState>,
    name: String,
    command: String,
    args: Vec<String>,
    enabled: bool,
) -> Result<(), String> {
    state
        .extensions
        .add_mcp(name, command, args, enabled)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn remove_mcp(state: State<'_, AppState>, name: String) -> Result<(), String> {
    state.extensions.remove_mcp(&name).await.map_err(err)
}

#[tauri::command]
pub async fn toggle_mcp(
    state: State<'_, AppState>,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    state
        .extensions
        .toggle_mcp(&name, enabled)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn add_skill(
    state: State<'_, AppState>,
    name: String,
    path: Option<String>,
    description: Option<String>,
    enabled: bool,
) -> Result<(), String> {
    state
        .extensions
        .add_skill(name, path.map(PathBuf::from), description, enabled)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn remove_skill(state: State<'_, AppState>, name: String) -> Result<(), String> {
    state.extensions.remove_skill(&name).await.map_err(err)
}

#[tauri::command]
pub async fn extensions_doctor(state: State<'_, AppState>) -> Result<String, String> {
    state.extensions.doctor().await.map_err(err)
}

#[tauri::command]
pub async fn memory_list(
    state: State<'_, AppState>,
    scope: Option<String>,
) -> Result<Vec<MemoryEntry>, String> {
    Ok(state.memory.list(scope.as_deref()).await)
}

#[tauri::command]
pub async fn memory_add(
    state: State<'_, AppState>,
    scope: String,
    content: String,
    tags: Vec<String>,
) -> Result<MemoryEntry, String> {
    state.memory.add(scope, content, tags).await.map_err(err)
}

#[tauri::command]
pub async fn memory_remove(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.memory.remove(&id).await.map_err(err)
}

#[tauri::command]
pub async fn memory_flush(state: State<'_, AppState>, scope: String) -> Result<String, String> {
    state.memory.flush_markdown(&scope).await.map_err(err)
}

#[tauri::command]
pub async fn memory_dream(
    state: State<'_, AppState>,
    scope: String,
    max_chars: Option<usize>,
) -> Result<String, String> {
    state
        .memory
        .dream(&scope, max_chars.unwrap_or(4000))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn scheduler_list(state: State<'_, AppState>) -> Result<Vec<ScheduledJob>, String> {
    Ok(state.scheduler.list().await)
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerAddRequest {
    pub name: String,
    pub prompt: String,
    pub interval_secs: Option<u64>,
    pub cron: Option<String>,
    pub once_delay_secs: Option<u64>,
    pub cwd: Option<String>,
    pub max_runs: Option<u64>,
}

#[tauri::command]
pub async fn scheduler_add(
    state: State<'_, AppState>,
    request: SchedulerAddRequest,
) -> Result<ScheduledJob, String> {
    let schedule = if let Some(expr) = request.cron {
        ScheduleKind::Cron { expr }
    } else if let Some(d) = request.once_delay_secs {
        ScheduleKind::Once { delay_secs: d }
    } else {
        ScheduleKind::Interval {
            secs: request.interval_secs.unwrap_or(3600),
        }
    };
    state
        .scheduler
        .add(
            request.name,
            request.prompt,
            schedule,
            request.cwd,
            request.max_runs,
        )
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn scheduler_cancel(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.scheduler.cancel(&id).await.map_err(err)
}

#[tauri::command]
pub async fn scheduler_pause(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.scheduler.pause(&id).await.map_err(err)
}

#[tauri::command]
pub async fn scheduler_resume(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.scheduler.resume(&id).await.map_err(err)
}

// ── Phase 4: Diff, Export, Recovery ──────────────────────────────────────

#[tauri::command]
pub async fn diff_current(cwd: String) -> Result<DiffSummary, String> {
    DiffEngine::current_summary(PathBuf::from(cwd).as_path())
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn diff_capture_before(cwd: String) -> Result<DiffCapture, String> {
    DiffEngine::capture_before(PathBuf::from(cwd).as_path())
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn diff_capture_after(capture: DiffCapture) -> Result<DiffCapture, String> {
    DiffEngine::capture_after(capture).await.map_err(err)
}

#[tauri::command]
pub async fn export_session_markdown(
    state: State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    state.persistence.export_markdown(id).map_err(err)
}

#[tauri::command]
pub async fn list_persisted_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<SessionRecord>, String> {
    state.persistence.list_sessions().map_err(err)
}

#[tauri::command]
pub async fn persistence_checkpoint(state: State<'_, AppState>) -> Result<(), String> {
    state.persistence.checkpoint().map_err(err)
}

#[tauri::command]
pub async fn shutdown_all(state: State<'_, AppState>) -> Result<(), String> {
    state.registry.shutdown_all().await;
    state.persistence.checkpoint().map_err(err)?;
    Ok(())
}

async fn persist_session(state: &AppState, id: Uuid) {
    if let Ok(snap) = state.registry.get_snapshot(id) {
        let mode = match snap.metadata.mode {
            grok_control_core::AgentMode::Acp => "acp",
            grok_control_core::AgentMode::Headless => "headless",
        };
        let status = format!("{:?}", snap.metadata.status).to_lowercase();
        let metadata_json = serde_json::to_string(&snap).unwrap_or_else(|_| "{}".into());
        let rec = SessionRecord {
            id,
            cwd: snap.metadata.cwd.clone(),
            mode: mode.into(),
            model: snap.metadata.model.clone(),
            status,
            worktree: snap.metadata.worktree.clone(),
            acp_session_id: snap.metadata.acp_session_id.clone(),
            metadata_json,
            created_at: snap.metadata.created_at,
            updated_at: Utc::now(),
        };
        let _ = state.persistence.upsert_session(&rec);
    }
}

// Keep SessionStatus import used for docs / future
#[allow(dead_code)]
fn _status_idle() -> SessionStatus {
    SessionStatus::Idle
}
