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
use grok_mcp::{
    AddMcpRequest, DoctorReport, McpCatalogEntry, McpCredential, McpServerConfigExt, McpToolInfo,
    UpdateMcpRequest,
};
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

// ── Auth / Grok login ────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_auth_status() -> Result<grok_cli_wrapper::AuthStatus, String> {
    Ok(grok_cli_wrapper::GrokCli::auth_status())
}

/// Opens Grok OAuth in the browser (`grok login --oauth`) and waits for completion.
#[tauri::command]
pub async fn login_with_grok(
    state: State<'_, AppState>,
) -> Result<grok_cli_wrapper::LoginResult, String> {
    state
        .grok_cli
        .login_oauth(std::time::Duration::from_secs(300))
        .await
        .map_err(err)
}

/// Device-code login (prints/opens URL with user_code). Best for GUI.
#[tauri::command]
pub async fn login_with_device(
    state: State<'_, AppState>,
) -> Result<grok_cli_wrapper::LoginResult, String> {
    state
        .grok_cli
        .login_device(std::time::Duration::from_secs(300))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn logout_grok(state: State<'_, AppState>) -> Result<grok_cli_wrapper::AuthStatus, String> {
    state.grok_cli.logout().await.map_err(err)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub grok_binary: String,
    pub grok_binary_exists: bool,
    pub grok_version: Option<String>,
    pub home_dir: String,
    pub config_path: String,
    pub worktrees_dir: String,
    pub default_cwd: String,
    pub session_count: usize,
    pub mcp_count: usize,
    pub xai_api_key_present: bool,
    pub ready: bool,
    pub message: String,
}

#[tauri::command]
pub async fn get_runtime_status(state: State<'_, AppState>) -> Result<RuntimeStatus, String> {
    let binary = state.grok_cli.grok_path.clone();
    let exists = binary.is_file();
    let version = if exists {
        state.grok_cli.version().await.ok()
    } else {
        None
    };
    let default_cwd = std::env::var("HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::current_dir())
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    // Prefer last used cwd from persistence.
    let default_cwd = state
        .persistence
        .get_kv("last_cwd")
        .ok()
        .flatten()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or(default_cwd);

    let mcp_count = state.mcp.list().await.len();
    let session_count = state.registry.session_count();
    let xai = std::env::var("XAI_API_KEY")
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let auth = grok_cli_wrapper::GrokCli::auth_status();

    let (ready, message) = if !exists {
        (
            false,
            "Grok Build CLI not found. Install it, then restart the panel.".into(),
        )
    } else if version.is_none() {
        (
            false,
            format!(
                "Found {} but `grok version` failed. Check permissions.",
                binary.display()
            ),
        )
    } else if !auth.logged_in && !xai {
        (
            false,
            "Not signed in — use Log in with Grok.".into(),
        )
    } else {
        let who = auth
            .email
            .clone()
            .unwrap_or_else(|| "Grok".into());
        (
            true,
            format!(
                "Ready · {who} · {}",
                version.as_deref().unwrap_or("?")
            ),
        )
    };

    Ok(RuntimeStatus {
        grok_binary: binary.display().to_string(),
        grok_binary_exists: exists,
        grok_version: version,
        home_dir: state.paths.home_dir.display().to_string(),
        config_path: state.paths.config_file.display().to_string(),
        worktrees_dir: state.paths.worktrees_dir.display().to_string(),
        default_cwd: default_cwd.display().to_string(),
        session_count,
        mcp_count,
        xai_api_key_present: xai,
        ready,
        message,
    })
}

#[tauri::command]
pub async fn set_last_cwd(state: State<'_, AppState>, cwd: String) -> Result<(), String> {
    let path = PathBuf::from(&cwd);
    if !path.is_absolute() || !path.is_dir() {
        return Err("cwd must be an absolute existing directory".into());
    }
    state.persistence.set_kv("last_cwd", &cwd).map_err(err)
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
    mut opts: SpawnOptions,
) -> Result<SessionIdResponse, String> {
    // Resolve MCP attachments via McpManager (names + auto + high-risk approval).
    if !opts.mcp_server_names.is_empty() || opts.include_auto_mcp {
        let payload = state
            .mcp
            .session_mcp_payload(
                &opts.mcp_server_names,
                &opts.approved_high_risk_mcp,
                opts.include_auto_mcp,
            )
            .await
            .map_err(err)?;
        if !payload.is_empty() {
            opts.mcp_servers = payload;
        }
    }
    let id = state.registry.spawn_agent(&cwd, opts).await.map_err(err)?;
    let _ = state.persistence.set_kv("last_cwd", &cwd);
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
    let _ = state.persistence.set_kv("last_cwd", &cwd);
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

// ── Phase 3: Extensions / MCP / Memory / Scheduler ───────────────────────

#[tauri::command]
pub async fn list_extensions(state: State<'_, AppState>) -> Result<Vec<ExtensionEntry>, String> {
    Ok(state.extensions.list_all().await)
}

/// Legacy simple add — prefers full `add_mcp_server` for catalog/security.
#[tauri::command]
pub async fn add_mcp(
    state: State<'_, AppState>,
    name: String,
    command: String,
    args: Vec<String>,
    enabled: bool,
) -> Result<(), String> {
    state
        .mcp
        .add(AddMcpRequest {
            name,
            kind: Some("custom".into()),
            transport: Some("stdio".into()),
            command: Some(command),
            args: Some(args),
            url: None,
            env: None,
            enabled: Some(enabled),
            scope: None,
            description: None,
            allowed_paths: None,
            read_only: None,
            auto_attach: None,
            requires_approval: Some(true),
            from_catalog: None,
            headers: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            rate_limit_per_min: None,
            credential_keys: None,
        })
        .await
        .map_err(err)?;
    Ok(())
}

#[tauri::command]
pub async fn remove_mcp(state: State<'_, AppState>, name: String) -> Result<(), String> {
    state.mcp.remove(&name).await.map_err(err)
}

#[tauri::command]
pub async fn toggle_mcp(
    state: State<'_, AppState>,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    state.mcp.set_enabled(&name, enabled).await.map_err(err)
}

// ── Full MCP manager surface ─────────────────────────────────────────────

#[tauri::command]
pub async fn list_mcp_servers(
    state: State<'_, AppState>,
) -> Result<Vec<McpServerConfigExt>, String> {
    Ok(state.mcp.list().await)
}

#[tauri::command]
pub async fn get_mcp_server(
    state: State<'_, AppState>,
    name: String,
) -> Result<McpServerConfigExt, String> {
    state.mcp.get(&name).await.map_err(err)
}

#[tauri::command]
pub async fn add_mcp_server(
    state: State<'_, AppState>,
    request: AddMcpRequest,
) -> Result<McpServerConfigExt, String> {
    state.mcp.add(request).await.map_err(err)
}

#[tauri::command]
pub async fn update_mcp_server(
    state: State<'_, AppState>,
    request: UpdateMcpRequest,
) -> Result<McpServerConfigExt, String> {
    state.mcp.update(request).await.map_err(err)
}

#[tauri::command]
pub async fn remove_mcp_server(state: State<'_, AppState>, name: String) -> Result<(), String> {
    state.mcp.remove(&name).await.map_err(err)
}

#[tauri::command]
pub async fn doctor_mcp_server(
    state: State<'_, AppState>,
    name: Option<String>,
) -> Result<Vec<DoctorReport>, String> {
    state
        .mcp
        .doctor(name.as_deref())
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn list_mcp_tools(
    state: State<'_, AppState>,
    name: Option<String>,
) -> Result<Vec<McpToolInfo>, String> {
    state.mcp.list_tools(name.as_deref()).await.map_err(err)
}

#[tauri::command]
pub async fn list_mcp_catalog() -> Result<Vec<McpCatalogEntry>, String> {
    Ok(grok_mcp::builtin_catalog())
}

#[tauri::command]
pub async fn set_mcp_credential(
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), String> {
    state.mcp.set_credential(&key, &value).await.map_err(err)
}

#[tauri::command]
pub async fn list_mcp_credentials(
    state: State<'_, AppState>,
) -> Result<Vec<McpCredential>, String> {
    state.mcp.list_credentials_masked().await.map_err(err)
}

#[tauri::command]
pub async fn suggest_mcp_for_project(
    state: State<'_, AppState>,
    git_remote: Option<String>,
    branch: Option<String>,
) -> Result<Vec<String>, String> {
    Ok(state
        .mcp
        .suggest_for_project(git_remote.as_deref(), branch.as_deref())
        .await)
}

#[tauri::command]
pub async fn preview_session_mcp(
    state: State<'_, AppState>,
    names: Vec<String>,
    approved_high_risk: Vec<String>,
    include_auto: bool,
) -> Result<Vec<serde_json::Value>, String> {
    state
        .mcp
        .session_mcp_payload(&names, &approved_high_risk, include_auto)
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

// ── Dev server / live preview ────────────────────────────────────────────

fn resolve_preview_cwd(state: &AppState, cwd: Option<String>, session_id: Option<String>) -> Result<PathBuf, String> {
    if let Some(id) = session_id {
        let uuid = Uuid::parse_str(&id).map_err(err)?;
        let snap = state.registry.get_snapshot(uuid).map_err(err)?;
        return Ok(PathBuf::from(snap.metadata.cwd));
    }
    if let Some(c) = cwd {
        let p = PathBuf::from(c);
        if p.is_dir() {
            return Ok(p);
        }
        return Err(format!("cwd is not a directory: {}", p.display()));
    }
    if let Ok(Some(last)) = state.persistence.get_kv("last_cwd") {
        let p = PathBuf::from(last);
        if p.is_dir() {
            return Ok(p);
        }
    }
    Err("No project path — select a session or set cwd".into())
}

#[tauri::command]
pub async fn detect_dev_server(
    state: State<'_, AppState>,
    cwd: Option<String>,
    session_id: Option<String>,
) -> Result<crate::devserver::DetectedProject, String> {
    let path = resolve_preview_cwd(&state, cwd, session_id)?;
    crate::devserver::DevServerManager::detect(&path)
}

#[tauri::command]
pub async fn start_dev_server(
    state: State<'_, AppState>,
    cwd: Option<String>,
    session_id: Option<String>,
    open_browser: Option<bool>,
) -> Result<crate::devserver::DevServerStatus, String> {
    let path = resolve_preview_cwd(&state, cwd, session_id)?;
    let _ = state.persistence.set_kv("last_cwd", &path.display().to_string());
    state
        .dev_server
        .start(&path, open_browser.unwrap_or(true))
        .await
}

#[tauri::command]
pub async fn stop_dev_server(
    state: State<'_, AppState>,
) -> Result<crate::devserver::DevServerStatus, String> {
    Ok(state.dev_server.stop().await)
}

#[tauri::command]
pub async fn dev_server_status(
    state: State<'_, AppState>,
) -> Result<crate::devserver::DevServerStatus, String> {
    Ok(state.dev_server.status().await)
}

#[tauri::command]
pub async fn open_dev_server(state: State<'_, AppState>) -> Result<String, String> {
    state.dev_server.open_in_browser().await
}

#[tauri::command]
pub async fn reveal_project(
    state: State<'_, AppState>,
    cwd: Option<String>,
    session_id: Option<String>,
) -> Result<(), String> {
    let path = resolve_preview_cwd(&state, cwd, session_id)?;
    crate::devserver::DevServerManager::reveal_project(&path).await
}
