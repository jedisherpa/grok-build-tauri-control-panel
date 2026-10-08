//! Tauri invoke command surface for the control panel.

use std::path::PathBuf;

use chrono::Utc;
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use grok_config::{DiscoveryReport, GrokConfig};
use grok_control_core::{AgentHandleSnapshot, SpawnOptions};
use grok_diff::{DiffCapture, DiffEngine, DiffSummary};
use grok_extensions::ExtensionEntry;
use grok_mcp::{
    AddMcpRequest, DoctorReport, McpCatalogEntry, McpCredential, McpServerConfigExt, McpToolInfo,
    UpdateMcpRequest,
};
use grok_memory::MemoryEntry;
use grok_permissions::{builtin_presets, PermissionController, PermissionDecision, PermissionPreset};
use grok_events::ControlEvent;
use grok_persistence::{SessionRecord, ThreadDto, TranscriptEntry};
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendInfo {
    pub id: String,
    pub display_name: String,
    pub available: bool,
    /// "binary:/abs/path" | "npx" | null when unavailable.
    pub via: Option<String>,
    /// Why the backend is unavailable, when it is.
    pub reason: Option<String>,
    pub default_model: String,
    pub models: Vec<String>,
    pub supports_headless: bool,
}

#[tauri::command]
pub async fn list_backends(state: State<'_, AppState>) -> Result<Vec<BackendInfo>, String> {
    let cfg = state.config.read().await.clone();
    // Grok's model ids move fast — ask the CLI for the live catalog so the
    // pickers never offer an id that fails every `-m` call.
    let live_grok_models = state.grok_cli.list_models().await.unwrap_or_default();
    Ok(grok_config::Backend::ALL
        .iter()
        .map(|&b| {
            let desc = grok_config::descriptor(b);
            let (available, via, reason) = match grok_config::resolve_backend(b, &cfg) {
                Ok(r) => {
                    let via = match r.via {
                        grok_config::LaunchVia::Binary => {
                            format!("binary:{}", r.program.display())
                        }
                        grok_config::LaunchVia::Npx => "npx".to_string(),
                    };
                    (true, Some(via), None)
                }
                Err(e) => (false, None, Some(e.to_string())),
            };
            let (default_model, models) = if b == grok_config::Backend::Grok
                && !live_grok_models.is_empty()
            {
                let default = live_grok_models
                    .iter()
                    .find(|(_, d)| *d)
                    .map(|(m, _)| m.clone())
                    .unwrap_or_else(|| cfg.model_for(b));
                (
                    default,
                    live_grok_models.iter().map(|(m, _)| m.clone()).collect(),
                )
            } else {
                (cfg.model_for(b), cfg.models_for(b))
            };
            BackendInfo {
                id: b.key().to_string(),
                display_name: desc.display_name.to_string(),
                available,
                via,
                reason,
                default_model,
                models,
                supports_headless: desc.supports_headless,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn save_config(state: State<'_, AppState>, config: GrokConfig) -> Result<(), String> {
    let target="panel_config".into();
    crate::operations::recorded(&state.event_bus,"save_config",target,async {
    {
        let mut cfg = state.config.write().await;
        config.save(&state.paths.config_file).map_err(err)?;
        *cfg = config;
    }
    Ok(())
    }).await
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

/// Sign-in state for every backend: which services can actually run right now.
#[tauri::command]
pub async fn backend_auth_status(
    state: State<'_, AppState>,
) -> Result<Vec<grok_cli_wrapper::BackendAuth>, String> {
    let cfg = state.config.read().await.clone();
    Ok(grok_cli_wrapper::backend_auth::all(&cfg).await)
}

/// Hand a backend's sign-in (or sign-out) off to a real terminal window.
///
/// `claude auth login` and `codex login` drive a browser flow and expect a TTY,
/// so we cannot run them headless the way we drive grok's device-code flow.
/// The panel polls `backend_auth_status` afterwards to notice the result.
#[tauri::command]
pub async fn open_backend_login(state: State<'_, AppState>, backend: String, logout: bool) -> Result<(), String> {
    let cfg = state.config.read().await.clone();
    let cmd = grok_cli_wrapper::backend_auth::configured_auth_command(
        &backend, if logout { "logout" } else { "login" }, &cfg)
    .ok_or_else(|| format!("no way to sign in to {backend}: install its CLI, or npx"))?;

    crate::operations::recorded(&state.event_bus,"backend_auth_handoff",serde_json::json!({"backend":backend,"logout":logout}).to_string(),async {spawn_in_terminal(&cmd).map_err(|e|format!("could not open a terminal: {e}"))}).await
}

/// Open `cmd` in the platform's terminal. The command string is built by
/// `backend_auth` from a fixed table, never from user input.
fn spawn_in_terminal(cmd: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        // `osascript` keeps the window open and focused so the user can see the
        // browser prompt and any error the CLI prints.
        let script = format!(
            r#"tell application "Terminal"
                 activate
                 do script "{cmd}"
               end tell"#
        );
        std::process::Command::new("osascript")
            .args(["-e", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cmd;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "terminal hand-off is only wired up for macOS; run the command yourself",
        ))
    }
}

/// Start interactive login (device-code). Returns immediately with URL + confirm code.
#[tauri::command]
pub async fn start_grok_login(
    state: State<'_, AppState>,
) -> Result<grok_cli_wrapper::LoginSessionState, String> {
    crate::operations::recorded(&state.event_bus,"start_device_login","grok".into(),async {state.login.start_device_login().await.map_err(err)}).await
}

/// Fallback OAuth browser login start.
#[tauri::command]
pub async fn start_grok_login_oauth(
    state: State<'_, AppState>,
) -> Result<grok_cli_wrapper::LoginSessionState, String> {
    crate::operations::recorded(&state.event_bus,"start_oauth_login","grok".into(),async {state.login.start_oauth_login().await.map_err(err)}).await
}

/// Poll login session (phase, confirm code, logged-in status).
#[tauri::command]
pub async fn grok_login_status(
    state: State<'_, AppState>,
) -> Result<grok_cli_wrapper::LoginSessionState, String> {
    Ok(state.login.state().await)
}

/// Paste a verification code from the browser into the running login process.
#[tauri::command]
pub async fn submit_grok_login_code(
    state: State<'_, AppState>,
    code: String,
) -> Result<grok_cli_wrapper::LoginSessionState, String> {
    crate::operations::recorded(&state.event_bus,"submit_login_code","grok".into(),async {state.login.submit_code(&code).await.map_err(err)}).await
}

#[tauri::command]
pub async fn open_grok_login_url(
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    crate::operations::recorded(&state.event_bus,"open_login_url","grok".into(),async {state.login.open_login_url().await.map_err(err)}).await
}

#[tauri::command]
pub async fn cancel_grok_login(state: State<'_, AppState>) -> Result<(), String> {
    state.login.cancel().await;
    Ok(())
}

#[tauri::command]
pub async fn logout_grok(state: State<'_, AppState>) -> Result<grok_cli_wrapper::AuthStatus, String> {
    state.login.cancel().await;
    crate::operations::recorded(&state.event_bus,"logout","grok".into(),async {state.grok_cli.logout().await.map_err(err)}).await
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
    pub haven: crate::haven::HavenStatus,
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

    let haven = state.haven.last_status().await;
    let message = if haven.connected {
        format!("{message} · {}", haven.label)
    } else if haven.configured {
        format!("{message} · haven offline")
    } else {
        message
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
        haven,
    })
}

// ── Haven (Hetzner process + temp store) ─────────────────────────────────

#[tauri::command]
pub async fn haven_status(state: State<'_, AppState>) -> Result<crate::haven::HavenStatus, String> {
    Ok(state.haven.connect_and_status().await)
}

#[tauri::command]
pub async fn haven_get_config(
    state: State<'_, AppState>,
) -> Result<crate::haven::HavenConfig, String> {
    let mut cfg = state.haven.config().await;
    // Never return full token to UI logs — mask middle (char-safe: byte
    // slicing panics on multibyte tokens).
    let chars: Vec<char> = cfg.auth_token.chars().collect();
    if chars.len() > 12 {
        let head: String = chars[..6].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        cfg.auth_token = format!("{head}…{tail}");
    }
    Ok(cfg)
}

#[tauri::command]
pub async fn haven_set_config(
    state: State<'_, AppState>,
    mut config: crate::haven::HavenConfig,
) -> Result<crate::haven::HavenStatus, String> {
    let target="haven_config".into();
    crate::operations::recorded(&state.event_bus,"haven_set_config",target,async {
    // If UI sent a masked token, keep existing secret.
    let existing = state.haven.config().await;
    if config.auth_token.contains('…') || config.auth_token.contains("...") {
        config.auth_token = existing.auth_token;
    }
    // A bearer token over plaintext http is readable by anyone on the path —
    // but private/tailnet hosts (Tailscale, LAN, localhost) are fine.
    if config.base_url.starts_with("http://")
        && !config.auth_token.is_empty()
        && !is_private_host(&config.base_url)
        && !config.allow_insecure_http
    {
        return Err(
            "haven base_url must be https for public hosts (plain http is allowed for \
             localhost, LAN, and Tailscale addresses — or tick 'allow insecure http' \
             to accept the risk)"
                .into(),
        );
    }
    state.haven.set_config(config).await?;
    Ok(state.haven.connect_and_status().await)
    }).await
}

/// True for hosts where plaintext http is acceptable: loopback, RFC1918 LAN,
/// Tailscale CGNAT range (100.64/10), .local, and MagicDNS .ts.net names.
fn is_private_host(base_url: &str) -> bool {
    let host = base_url
        .trim_start_matches("http://")
        .split(['/', ':'])
        .next()
        .unwrap_or("");
    if host == "localhost" || host.ends_with(".local") || host.ends_with(".ts.net") {
        return true;
    }
    let octets: Vec<u8> = host.split('.').filter_map(|p| p.parse().ok()).collect();
    if octets.len() != 4 {
        return false;
    }
    match octets[..] {
        [127, ..] => true,
        [10, ..] => true,
        [192, 168, ..] => true,
        [172, b, ..] if (16..=31).contains(&b) => true,
        [100, b, ..] if (64..=127).contains(&b) => true,
        _ => false,
    }
}

#[tauri::command]
pub async fn haven_list_jobs(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    state.haven.list_jobs().await
}

#[tauri::command]
pub async fn haven_start_shell(
    state: State<'_, AppState>,
    name: String,
    command: String,
    cwd: Option<String>,
    keep_alive: Option<bool>,
) -> Result<serde_json::Value, String> {
    let target=serde_json::json!({"name":name,"cwd":cwd,"keep_alive":keep_alive.unwrap_or(false)}).to_string();
    crate::operations::recorded(&state.event_bus,"haven_start_shell",target,async {state.haven.start_shell(name,command,cwd,keep_alive.unwrap_or(false)).await}).await
}

#[tauri::command]
pub async fn haven_job_log(
    state: State<'_, AppState>,
    id: String,
    bytes: Option<u64>,
) -> Result<String, String> {
    state.haven.job_log(id, bytes.unwrap_or(64_000)).await
}

#[tauri::command]
pub async fn haven_remove_job(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    crate::operations::recorded(&state.event_bus,"haven_remove_job",serde_json::json!({"job_id":id}).to_string(),async {state.haven.remove_job(id).await}).await
}

#[tauri::command]
pub async fn haven_list_files(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    state.haven.list_files().await
}

#[tauri::command]
pub async fn set_last_cwd(state: State<'_, AppState>, cwd: String) -> Result<(), String> {
    let path = PathBuf::from(&cwd);
    if !path.is_absolute() || !path.is_dir() {
        return Err("cwd must be an absolute existing directory".into());
    }
    state.persistence.set_kv("last_cwd", &cwd).map_err(err)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolderResult {
    pub path: String,
    pub name: String,
    pub created: bool,
}

/// Create a new project folder under a parent directory (default: ~/Projects or home).
#[tauri::command]
pub async fn create_project_folder(
    state: State<'_, AppState>,
    name: String,
    parent: Option<String>,
) -> Result<CreateFolderResult, String> {
    let target=serde_json::json!({"name":name,"parent":parent}).to_string();
    crate::operations::recorded(&state.event_bus,"create_project_folder",target,async {
    let slug = sanitize_folder_name(&name)?;
    let parent_dir = resolve_projects_parent(parent)?;
    std::fs::create_dir_all(&parent_dir).map_err(err)?;
    let path = parent_dir.join(&slug);
    let created = if path.exists() {
        if !path.is_dir() {
            return Err(format!("path exists and is not a directory: {}", path.display()));
        }
        false
    } else {
        std::fs::create_dir_all(&path).map_err(err)?;
        true
    };
    let path_str = path.display().to_string();
    let _ = state.persistence.set_kv("last_cwd", &path_str);
    Ok(CreateFolderResult {
        path: path_str,
        name: slug,
        created,
    })
    }).await
}

fn sanitize_folder_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("folder name is empty".into());
    }
    // Keep letters, numbers, dash, underscore, space -> dash
    let mut out = String::new();
    for c in trimmed.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c.to_ascii_lowercase());
        } else if (c.is_whitespace() || c == '/' || c == '\\')
            && !out.ends_with('-')
            && !out.is_empty()
        {
            out.push('-');
        }
        // drop other punctuation
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        return Err("folder name has no usable characters".into());
    }
    if out == "." || out == ".." {
        return Err("invalid folder name".into());
    }
    if out.len() > 80 {
        return Err("folder name too long".into());
    }
    Ok(out)
}

fn resolve_projects_parent(parent: Option<String>) -> Result<PathBuf, String> {
    if let Some(p) = parent {
        let path = PathBuf::from(p);
        if !path.is_absolute() {
            return Err("parent must be absolute".into());
        }
        return Ok(path);
    }
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "HOME not set".to_string())?;
    // Prefer existing project roots
    for candidate in ["Projects", "projects", "Code", "code", "Developer", "dev"] {
        let p = home.join(candidate);
        if p.is_dir() {
            return Ok(p);
        }
    }
    // Default: ~/Projects (create on demand by caller)
    Ok(home.join("Projects"))
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
    opts.validate().map_err(err)?;
    if opts.read_only || opts.resolved_mode() == grok_control_core::ApprovalMode::Plan {
        if !opts.mcp_server_names.is_empty() {
            return Err("Selected external MCP tools cannot enforce Plan/read-only mutation limits. Start an explicitly reviewed Ask session for those integrations.".into());
        }
        opts.include_auto_mcp = false;
    }
    // Resolve MCP attachments via McpManager (names + auto + high-risk approval).
    let mut mcp_skipped = Vec::new();
    if !opts.mcp_server_names.is_empty() || opts.include_auto_mcp {
        let resolution = state
            .mcp
            .session_mcp_payload(
                &opts.mcp_server_names,
                &opts.approved_high_risk_mcp,
                opts.include_auto_mcp,
            )
            .await
            .map_err(err)?;
        opts.mcp_servers = resolution.payload;
        opts.mcp_server_names = resolution.attached_names;
        mcp_skipped = resolution.skipped;
    }
    // Thread-per-worktree isolation: give this thread its own checkout so
    // parallel threads on the same project can't overwrite each other.
    let id = Uuid::new_v4();
    let mut isolation_note: Option<String> = None;
    let requested_cwd = cwd.clone();
    let mut spawn_cwd = cwd.clone();
    let mut isolation_admission = None;
    if opts.isolate_worktree && opts.mode == grok_control_core::AgentMode::Acp {
        if grok_worktree::is_git_repo(std::path::Path::new(&cwd)).await {
            // Admission precedes topology mutation. Retain the canonical
            // project owner through child admission so an active Build cannot
            // cause an unreported orphan checkout between create and spawn.
            isolation_admission = Some(grok_worktree::WorkspaceCoordinator::shared()
                .session(std::path::Path::new(&cwd), None).await.map_err(err)?);
            let short = &id.to_string()[..8];
            let target=serde_json::json!({"session_id":id,"repository":cwd,"name":format!("t-{short}")}).to_string();
            match crate::operations::recorded(&state.event_bus,"isolate_session_worktree",target,async {
                state.worktrees.create(std::path::Path::new(&cwd),CreateWorktreeRequest{name:format!("t-{short}"),base_ref:None,prefer_grok_cli:false}).await.map_err(err)
            }).await {
                Ok(wt) => {
                    spawn_cwd = wt.path.display().to_string();
                    opts.worktree = Some(wt.name.clone());
                    opts.project_root = Some(requested_cwd.clone());
                    isolation_note = Some(format!(
                        "🌱 isolated worktree · branch {} · {}",
                        wt.branch.as_deref().unwrap_or("?"),
                        wt.path.display()
                    ));
                }
                Err(e) => return Err(format!("Requested worktree isolation failed: {e}. No agent was started in the shared project folder.")),
            }
        } else {
            isolation_note =
                Some("not a git repo — thread works directly in the folder".into());
        }
    }

    // Durable memory rides along: global notes + this project's notes are
    // injected with the thread's first prompt.
    let memory_context = build_memory_context(&state, &requested_cwd).await;
    let connect_opts = grok_control_core::ConnectOpts {
        resume_acp_session_id: None,
        transcript_context: None,
        memory_context,
    };
    state
        .registry
        .spawn_agent_preallocated(id, &spawn_cwd, opts, connect_opts)
        .await
        .map_err(err)?;
    drop(isolation_admission);
    if let Some(note) = isolation_note {thread_note(&state,id,"worktree",&note)?;}
    for skipped in &mcp_skipped {thread_note(&state,id,"mcp",&format!("⚠ MCP `{}` skipped: {}",skipped.name,skipped.reason))?;}
    let _ = state.persistence.set_kv("last_cwd", &cwd);
    persist_session(&state, id).await?;
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
    persist_session(&state, id).await?;
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

/// Live + SQLite-restored threads for the UI thread list.
#[tauri::command]
pub async fn list_threads(state: State<'_, AppState>) -> Result<Vec<ThreadDto>, String> {
    Ok(build_thread_list(&state))
}

#[tauri::command]
pub async fn get_session(
    state: State<'_, AppState>,
    id: String,
) -> Result<AgentHandleSnapshot, String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    state.registry.get_snapshot(id).map_err(err)
}

/// Load transcript history from SQLite (works for live and restored threads).
#[tauri::command]
pub async fn get_session_transcript(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<TranscriptEntry>, String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    state.persistence.transcript_entries(id).map_err(err)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Existing desktop IPC contract.
pub async fn send_prompt(
    state: State<'_, AppState>,
    id: String,
    prompt: String,
    backend: Option<String>,
    model: Option<String>,
    approval_mode: Option<String>,
    plan_mode: Option<bool>,
    always_approve: Option<bool>,
    client_submission_id: Option<String>,
) -> Result<(), String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    if state.builds.is_managed(id).await {
        return Err("Reviewed-build sessions use their Builds controls and individual native tool approvals.".into());
    }

    let want_backend = backend.as_deref().and_then(grok_config::Backend::from_key);
    let want_model = model.filter(|m| {
        let t = m.trim();
        !t.is_empty() && !t.eq_ignore_ascii_case("default")
    });

    // Stop ended the owned native runner. This new user Send deliberately
    // reconnects the same conversation; no prior effectful prompt is replayed.
    if state.registry.requires_reconnect(id) {
        persist_session_checked(&state, id)?;
        state.registry.remove_session(id).await.map_err(err)?;
    }

    // Switching backend/model mid-thread: restart the thread under the new
    // agent. Cross-agent session/load can't work, so the resume ladder lands
    // on history-only and injects the prior transcript as context.
    if state.registry.is_live(id) {
        let snap = state.registry.get_snapshot(id).map_err(err)?;
        let cur = &snap.metadata;
        let backend_changed = want_backend.is_some_and(|b| b != cur.backend);
        let model_changed = want_model
            .as_deref()
            .is_some_and(|m| !m.eq_ignore_ascii_case(&cur.model) && cur.model != "mock");
        if backend_changed || (model_changed && cur.mode == grok_control_core::AgentMode::Acp) {
            let label = format!(
                "🔀 switching to {} · {} — prior chat carries over as context",
                want_backend.unwrap_or(cur.backend).key(),
                want_model.clone().unwrap_or_else(|| cur.model.clone()),
            );
            persist_session(&state, id).await?;
            state.registry.remove_session(id).await.map_err(err)?;
            // The continuation commits its own runtime before this note.

            resume_saved_session(
                &state,
                id,
                want_backend,
                want_model.clone(),
                approval_mode.clone(),
                plan_mode,
                always_approve,
            )
            .await?;
            thread_note(&state,id,"thread",&label)?;
        }
    }

    // Saved threads after reboot have history in SQLite but no live ACP process.
    // Auto-resume so "Send" picks up the same thread id + transcript.
    if !state.registry.is_live(id) {
        resume_saved_session(
            &state,
            id,
            want_backend,
            want_model,
            approval_mode.clone(),
            plan_mode,
            always_approve,
        )
        .await?;
    }

    let submission=client_submission_id.map(|value|Uuid::parse_str(&value)).transpose().map_err(err)?;
    state.registry.send_user_prompt(id,&prompt,submission).await.map_err(err)?;
    persist_session(&state,id).await?;
    // Optional title generation starts only after durable prompt admission.
    let snap=state.registry.get_snapshot(id).map_err(err)?;
    if snap.metadata.label.is_none() {
        let bus=state.registry.session_event_bus(id).map_err(err)?;
        let runtime=bus.runtime_id().ok_or("session has no runtime identity")?;
        let slug=prompt_slug(&prompt);
        if !slug.is_empty() {
            state.registry.set_label_for_runtime(id,runtime,&slug).map_err(err)?;
            persist_session(&state,id).await?;
            emit_thread_label(&state,id,&slug)?;
        }
        let explainer=state.explainer.clone();let registry=state.registry.clone();
        tauri::async_runtime::spawn(async move {
            if let Ok(title)=explainer.generate_title_on(&bus,&prompt).await {
                if title.is_empty(){return;}
                let expected=if slug.is_empty(){None}else{Some(slug.as_str())};
                if registry.set_label_if_current(id,runtime,expected,&title).is_ok() {
                    let _=bus.emit_checked(ControlEvent::Raw{session_id:Some(id),payload:serde_json::json!({"channel":"thread","kind":"label","label":title})});
                }
            }
        });
    }
    Ok(())
}

/// Bring a SQLite thread back online under the same id.
/// Ladder: session/load → session/resume → session/new + transcript inject.
/// Overrides switch the thread to a different backend/model; a backend switch
/// drops the prior ACP session id (it belongs to another agent) so the ladder
/// goes straight to history-only transcript injection.
#[derive(serde::Deserialize)]
#[serde(default, rename_all="camelCase")]
struct SavedLaunchAuthority {
    #[serde(alias="sandbox_profile")] sandbox_profile:Option<String>,
    #[serde(alias="permission_allow")] permission_allow:Vec<String>,
    #[serde(alias="permission_deny")] permission_deny:Vec<String>,
    rules:Vec<String>,
    #[serde(alias="trust_repo")] trust_repo:bool,
    #[serde(alias="read_only")] read_only:bool,
    #[serde(alias="approval_mode")] approval_mode:Option<grok_control_core::ApprovalMode>,
    #[serde(alias="plan_mode")] plan_mode:bool,
    #[serde(alias="always_approve")] always_approve:bool,
}
impl Default for SavedLaunchAuthority {
    fn default()->Self { Self {sandbox_profile:Some("workspace".into()),permission_allow:vec![],permission_deny:vec![],rules:vec![],trust_repo:false,read_only:false,approval_mode:None,plan_mode:true,always_approve:false} }
}
fn saved_launch_authority(json:&str)->Result<SavedLaunchAuthority,String> {
    let value:serde_json::Value=serde_json::from_str(json).map_err(|error|format!("cannot resume: saved launch authority is invalid — {error}"))?;
    let metadata=value.get("metadata").unwrap_or(&value).clone();
    serde_json::from_value(metadata).map_err(|error|format!("cannot resume: saved launch authority is invalid — {error}"))
}

pub(crate) async fn resume_saved_session(
    state: &AppState,
    id: Uuid,
    override_backend: Option<grok_config::Backend>,
    override_model: Option<String>,
    approval_mode: Option<String>,
    plan_mode: Option<bool>,
    always_approve: Option<bool>,
) -> Result<(), String> {
    let rec = state
        .persistence
        .get_session(id)
        .map_err(|e| format!("cannot resume thread — {e}"))?;

    if rec.cwd.trim().is_empty() {
        return Err("cannot resume: saved thread has no project path".into());
    }
    if !PathBuf::from(&rec.cwd).is_dir() {
        return Err(format!(
            "cannot resume: project path missing — {}",
            rec.cwd
        ));
    }

    let mut opts = SpawnOptions {
        mode: if rec.mode.eq_ignore_ascii_case("headless") {
            grok_control_core::AgentMode::Headless
        } else {
            grok_control_core::AgentMode::Acp
        },
        ..Default::default()
    };
    let recorded_backend = extract_backend_from_meta(&rec.metadata_json);
    opts.backend = override_backend.unwrap_or(recorded_backend);
    let backend_switched = opts.backend != recorded_backend;
    opts.model = override_model.or_else(|| {
        if rec.model.is_empty() || backend_switched {
            // Old model id belongs to the other vendor; let config pick.
            None
        } else {
            Some(rec.model.clone())
        }
    });
    opts.worktree = rec.worktree.clone();
    // Preserve the project link so Land/Sync keep working after a restart.
    // Never re-isolate on resume: the stored cwd already IS the worktree.
    opts.isolate_worktree = false;
    let saved_authority = saved_launch_authority(&rec.metadata_json)?;
    opts.read_only = saved_authority.read_only;
    opts.sandbox_profile = saved_authority.sandbox_profile.clone();
    opts.permission_allow = saved_authority.permission_allow.clone();
    opts.permission_deny = saved_authority.permission_deny.clone();
    opts.rules = saved_authority.rules.clone();
    opts.trust_repo = saved_authority.trust_repo;
    opts.project_root = extract_meta_string(&rec.metadata_json, "projectRoot")
        .or_else(|| extract_meta_string(&rec.metadata_json, "project_root"));
    // Honor the caller's current stance. An explicit approval_mode wins;
    // otherwise fall back to the legacy booleans (default: plan on, yolo off).
    opts.approval_mode = approval_mode.as_deref().and_then(|m| match m {
        "plan" => Some(grok_control_core::ApprovalMode::Plan),
        "auto" => Some(grok_control_core::ApprovalMode::Auto),
        "yolo" | "always_approve" => Some(grok_control_core::ApprovalMode::Yolo),
        "ask" | "default" => Some(grok_control_core::ApprovalMode::Ask),
        _ => None,
    });
    if opts.approval_mode.is_none() && plan_mode.is_none() && always_approve.is_none() {
        opts.approval_mode = saved_authority.approval_mode.or(Some(if saved_authority.always_approve {grok_control_core::ApprovalMode::Yolo} else if saved_authority.plan_mode {grok_control_core::ApprovalMode::Plan}else{grok_control_core::ApprovalMode::Ask}));
    }
    opts.always_approve = always_approve.unwrap_or(saved_authority.always_approve);
    opts.plan_mode = if opts.always_approve {
        false
    } else {
        plan_mode.unwrap_or(saved_authority.plan_mode)
    };
    opts.mcp_server_names = extract_mcp_from_meta(&rec.metadata_json);
    // Re-apply the high-risk approvals granted when the thread was created —
    // resuming must not silently drop approved servers (e.g. playwright).
    opts.approved_high_risk_mcp = extract_approved_mcp_from_meta(&rec.metadata_json);
    if matches!(opts.mode, grok_control_core::AgentMode::Headless) {
        opts.mode = grok_control_core::AgentMode::Acp;
    }

    let mut resume_notes=Vec::new();
    if !opts.mcp_server_names.is_empty() {
        let resolution = state
            .mcp
            .session_mcp_payload(&opts.mcp_server_names, &opts.approved_high_risk_mcp, false)
            .await
            .map_err(|e| format!("MCP resolution failed on resume: {e}"))?;
        opts.mcp_servers = resolution.payload;
        opts.mcp_server_names = resolution.attached_names;
        for s in &resolution.skipped {
            resume_notes.push(format!("⚠ MCP `{}` skipped on resume: {}",s.name,s.reason));
        }
    }

    let transcript_context = build_transcript_context(state, id);
    let memory_root = opts
        .project_root
        .clone()
        .unwrap_or_else(|| rec.cwd.clone());
    let connect_opts = grok_control_core::ConnectOpts {
        // A prior ACP session id from another agent can't be loaded/resumed.
        resume_acp_session_id: if backend_switched {
            None
        } else {
            rec.acp_session_id.clone().filter(|s| !s.is_empty())
        },
        transcript_context: transcript_context.clone(),
        memory_context: build_memory_context(state, &memory_root).await,
    };

    let brain = state
        .registry
        .resume_session(id, &rec.cwd, opts, Some(rec.created_at), connect_opts)
        .await
        .map_err(err)?;

    let msg = match brain {
        grok_control_core::BrainMode::FullBrain => {
            "🧠 full brain — agent reloaded prior ACP session (true continuity)"
        }
        grok_control_core::BrainMode::HistoryOnly => {
            "📜 history-only — new ACP process; prior transcript will be injected on next send"
        }
        grok_control_core::BrainMode::Fresh => {
            "agent resumed — fresh ACP session (no prior agent id / history pack)"
        }
    };
    persist_session(state,id).await?;
    for note in resume_notes {thread_note(state,id,"mcp",&note)?;}
    thread_note(state,id,"thread",msg)?;
    Ok(())
}

/// Cheap instant label: first significant words of the prompt.
fn prompt_slug(prompt: &str) -> String {
    const STOP: &[&str] = &[
        "a", "an", "the", "to", "of", "in", "on", "for", "and", "or", "is", "it", "that", "this",
        "please", "can", "you", "me", "my", "i", "we",
    ];
    let words: Vec<&str> = prompt
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty() && !STOP.contains(&w.to_lowercase().as_str()))
        .take(5)
        .collect();
    let mut s = words.join(" ");
    if s.chars().count() > 42 {
        s = s.chars().take(42).collect::<String>() + "…";
    }
    s
}

fn emit_thread_label(state: &AppState, id: Uuid, label: &str) ->Result<(),String> {
    thread_bus(state,id)?.emit_checked(grok_events::ControlEvent::Raw {
        session_id: Some(id),
        payload: serde_json::json!({ "channel": "thread", "kind": "label", "label": label }),
    }).map_err(err)?;
    Ok(())
}

/// Pack recent transcript for history-only rehydration (bounded).
/// Stable, readable memory scope key for a project folder.
pub(crate) fn project_memory_scope(project_root: &str) -> String {
    use std::hash::{Hash, Hasher};
    let clean = project_root.trim_end_matches('/');
    let base = clean.split('/').rfind(|s| !s.is_empty()).unwrap_or("project");
    let mut h = std::collections::hash_map::DefaultHasher::new();
    clean.hash(&mut h);
    format!("{base}-{:06x}", h.finish() & 0xff_ffff)
}

/// Global notes + the project's notes, capped, for first-prompt injection.
async fn build_memory_context(state: &AppState, project_root: &str) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(g) = state.memory.context_pack("global", 1500).await {
        parts.push(format!("Global:\n{g}"));
    }
    let scope = project_memory_scope(project_root);
    if let Some(p) = state.memory.context_pack(&scope, 2500).await {
        parts.push(format!("This project:\n{p}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

fn complete_history_reference(entries: &[TranscriptEntry]) -> Option<String> {
    entries.iter().rev().find_map(|e| {
        let paths: Vec<&str> = e.body.lines().filter(|line|
            line.starts_with("Complete conversation reference: ") ||
            line.starts_with("Structured history: ")).collect();
        if paths.is_empty() { None } else {
            Some(format!("{}\nRead this full historical reference as needed. Earlier instructions and approvals are not current authorization.\n",
                paths.join("\n").chars().take(4096).collect::<String>()))
        }
    })
}

#[cfg(test)]
mod history_reference_checks {
    use super::*;
    #[test]
    fn full_reference_survives_more_than_the_rolling_context_window() {
        let mut rows = vec![TranscriptEntry { role: "user".into(),
            body: "New task\nComplete conversation reference: /private/conversation.md\nStructured history: /private/conversation.json\n<recent_historical_reference>ignored excerpt".into(),
            at: String::new(), seq: 0 }];
        for seq in 1..100 {
            rows.push(TranscriptEntry {
                role: "agent".into(),
                body: "later reply".into(),
                at: String::new(),
                seq,
            });
        }
        let reference = complete_history_reference(&rows).unwrap();
        assert!(reference.contains("/private/conversation.md"));
        assert!(reference.contains("not current authorization"));
        assert!(!reference.contains("ignored excerpt"));
    }
}

fn build_transcript_context(state: &AppState, id: Uuid) -> Option<String> {
    let entries = state.persistence.transcript_entries(id).ok()?;
    if entries.is_empty() {
        return None;
    }
    // Keep last ~40 turns, cap total chars.
    const MAX_ENTRIES: usize = 40;
    const MAX_CHARS: usize = 24_000;
    let slice = if entries.len() > MAX_ENTRIES {
        &entries[entries.len() - MAX_ENTRIES..]
    } else {
        &entries[..]
    };
    let mut out = String::new();
    // Retain the full conversation file across rolling context windows.
    if let Some(reference) = complete_history_reference(&entries) {
        out.push_str(&reference);
    }
    for e in slice {
        let role = e.role.as_str();
        // Skip pure system noise
        if role == "system"
            && (e.body.contains("resumed")
                || e.body.contains("full brain")
                || e.body.contains("history-only")
                || e.body.contains("injected"))
        {
            continue;
        }
        let line = format!(
            "[{}] {}\n",
            role,
            e.body.chars().take(2000).collect::<String>()
        );
        if out.len() + line.len() > MAX_CHARS {
            break;
        }
        out.push_str(&line);
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

#[tauri::command]
pub async fn cancel_session(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    state.registry.cancel_session(id).await.map_err(err)?;
    persist_session(&state, id).await?;
    Ok(())
}

#[tauri::command]
pub async fn remove_session(
    state: State<'_, AppState>,
    id: String,
    remove_worktree: Option<bool>,
) -> Result<(), String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    // Capture worktree context before the records disappear.
    let wt_ctx = if remove_worktree.unwrap_or(false) {
        Some(thread_worktree_context(&state, id).await?)
    } else {
        None
    };
    if state.builds.is_managed(id).await { return Err("This session belongs to a reviewed build; use its build cleanup controls.".into()); }
    // Resolve all authority boundaries before admitting a destructive effect.
    let removal=if let Some((worktree,root,_branch,_))=wt_ctx {
        let worktree=std::fs::canonicalize(&worktree).map_err(err)?;
        let root=std::fs::canonicalize(&root).map_err(err)?;
        let managed=std::fs::canonicalize(state.worktrees.worktrees_root()).map_err(err)?;
        if worktree==root||!worktree.starts_with(&managed){return Err("Requested worktree is outside the managed root; thread evidence was retained.".into());}
        Some((worktree,root))
    }else{None};
    let target=serde_json::json!({"session_id":id,"worktree":removal.as_ref().map(|(worktree,_)|worktree.display().to_string())}).to_string();
    crate::operations::recorded(&state.event_bus,"remove_session",target,async {
        if state.registry.is_live(id){state.registry.remove_session(id).await.map_err(err)?;}
        if let Some((worktree,root))=removal {state.worktrees.remove(&root,&worktree.display().to_string(),true).await.map_err(err)?;}
        state.event_bus.emit_checked(ControlEvent::SessionRemoved{session_id:id,at:Utc::now()}).map_err(err)?;
        Ok(())
    }).await?;
    Ok(())
}

#[tauri::command]
pub async fn set_plan_mode(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    if state.builds.is_managed(id).await {
        return Err("Reviewed-build sessions use their Builds controls and individual native tool approvals.".into());
    }
    state
        .registry
        .set_plan_mode(id, enabled)
        .await
        .map_err(err)
}

// ── Explainer (right-panel ELI12 narrator) ───────────────────────────────

#[tauri::command]
pub async fn explainer_focus(
    state: State<'_, AppState>,
    id: Option<String>,
) -> Result<(), String> {
    let uuid = match id.as_deref().filter(|s| !s.is_empty()) {
        Some(s) => Some(Uuid::parse_str(s).map_err(err)?),
        None => None,
    };
    state.explainer.set_focus(uuid).await;
    Ok(())
}

#[tauri::command]
pub async fn set_explainer_provider(
    state: State<'_, AppState>,
    backend: Option<String>,
    model: Option<String>,
) -> Result<(), String> {
    let target="explainer_configuration".into();
    crate::operations::recorded(&state.event_bus,"set_explainer_provider",target,async {
    state
        .explainer
        .set_provider(backend.clone(), model.clone())
        .await;
    {
        let mut cfg = state.config.write().await;
        if backend.is_some() {
            cfg.explainer_backend = backend;
        }
        if model.is_some() {
            cfg.explainer_model = model;
        }
        cfg.save(&state.paths.config_file).map_err(err)?;
    }
    Ok(())
    }).await
}

#[tauri::command]
pub async fn set_explainer_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<bool, String> {
    let target=serde_json::json!({"enabled":enabled}).to_string();
    crate::operations::recorded(&state.event_bus,"set_explainer_enabled",target,async {
    state.explainer.set_enabled(enabled);
    {
        let mut cfg = state.config.write().await;
        cfg.explainer_enabled = enabled;
        cfg.save(&state.paths.config_file).map_err(err)?;
    }
    Ok(state.explainer.enabled())
    }).await
}

/// Set a live session's approval stance: plan | ask | auto | yolo.
#[tauri::command]
pub async fn set_approval_mode(
    state: State<'_, AppState>,
    id: String,
    mode: String,
) -> Result<(), String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    if state.builds.is_managed(id).await {
        return Err("Reviewed-build sessions use their Builds controls and individual native tool approvals.".into());
    }
    let mode = match mode.to_lowercase().as_str() {
        "plan" => grok_control_core::ApprovalMode::Plan,
        "auto" => grok_control_core::ApprovalMode::Auto,
        "yolo" | "always_approve" => grok_control_core::ApprovalMode::Yolo,
        "ask" | "default" => grok_control_core::ApprovalMode::Ask,
        other => return Err(format!("unknown approval mode: {other}")),
    };
    state
        .registry
        .set_approval_mode(id, mode)
        .await
        .map_err(err)?;
    persist_session(&state, id).await?;
    Ok(())
}

/// "Always allow this" from an approval card — auto-approves matching
/// requests for the rest of the session.
#[tauri::command]
pub async fn add_session_allow_rule(
    state: State<'_, AppState>,
    id: String,
    pattern: String,
) -> Result<(), String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    if state.builds.is_managed(id).await {
        return Err("Reviewed-build sessions use their Builds controls and individual native tool approvals.".into());
    }
    let pattern = pattern.trim().to_string();
    if pattern.is_empty() {
        return Err("empty rule".into());
    }
    state
        .registry
        .add_session_allow_rule(id, pattern.clone())
        .await
        .map_err(err)?;
    thread_note(&state,id,"policy",&format!("✓ always allowing `{pattern}` for the rest of this session"))?;
    Ok(())
}

#[tauri::command]
pub async fn set_always_approve(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    if state.builds.is_managed(id).await {
        return Err("Reviewed-build sessions use their Builds controls and individual native tool approvals.".into());
    }
    state
        .registry
        .set_always_approve(id, enabled)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn respond_approval(
    state: State<'_, AppState>, id:String,request_id:String,option_id:Option<String>,
    runtime_id:String,host_epoch:u64,
)->Result<(),String> {
    let id=Uuid::parse_str(&id).map_err(err)?;
    let runtime=Uuid::parse_str(&runtime_id).map_err(err)?;
    state.registry.respond_approval_for_runtime(id,runtime,host_epoch,&request_id,option_id.as_deref()).await.map_err(err)
}
#[tauri::command]
pub async fn get_pending_approvals(state:State<'_,AppState>,id:String)->Result<Vec<grok_acp::LiveApproval>,String> {
    let id=Uuid::parse_str(&id).map_err(err)?;
    if !state.registry.is_live(id){return Ok(Vec::new());}
    state.registry.pending_approvals(id).await.map_err(err)
}

/// Rename a thread (manual override of the smart name). Works for live and
/// saved threads; manual names are never overwritten by the auto-titler
/// (which only fires when a thread has no label at its first prompt).
#[tauri::command]
pub async fn rename_thread(
    state: State<'_, AppState>,
    id: String,
    label: String,
) -> Result<(), String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    let label = label.trim();
    if label.is_empty() {
        return Err("name cannot be empty".into());
    }
    let label: String = label.chars().take(60).collect();

    if state.registry.is_live(id) {
        state.registry.set_label(id, &label).map_err(err)?;
        persist_session(&state, id).await?;
    } else {
        // Saved thread: patch the label inside the persisted metadata.
        let mut rec = state.persistence.get_session(id).map_err(err)?;
        rec.metadata_json = renamed_saved_metadata(&rec.metadata_json, &label)?;
        rec.updated_at = Utc::now();
        state.event_bus.emit_checked(ControlEvent::SessionMetadataUpdated {session_id:id,metadata_json:serde_json::to_string(&rec).map_err(err)?,at:Utc::now()}).map_err(err)?;
    }
    emit_thread_label(&state, id, &label)?;
    Ok(())
}

fn renamed_saved_metadata(source: &str, label: &str) -> Result<String, String> {
    let mut value: serde_json::Value = serde_json::from_str(source)
        .map_err(|error| format!("saved metadata is invalid; original preserved: {error}"))?;
    let root = value.as_object_mut().ok_or("saved metadata must be an object; original preserved")?;
    match root.get_mut("metadata") {
        Some(metadata) => {
            metadata.as_object_mut().ok_or("saved metadata envelope is invalid; original preserved")?
                .insert("label".into(), serde_json::Value::String(label.into()));
        }
        None => { root.insert("label".into(), serde_json::Value::String(label.into())); }
    }
    serde_json::to_string(&value).map_err(err)
}

// ── Projects (persisted folder list for the sidebar) ─────────────────────

#[tauri::command]
pub async fn list_projects(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(state
        .persistence
        .get_kv("projects")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default())
}

#[tauri::command]
pub async fn add_project(state: State<'_, AppState>, path: String) -> Result<Vec<String>, String> {
    let path = path.trim().trim_end_matches('/').to_string();
    if path.is_empty() || !PathBuf::from(&path).is_dir() {
        return Err(format!("not a folder: {path}"));
    }
    let mut list = list_projects(state.clone()).await?;
    if !list.contains(&path) {
        list.push(path);
        list.sort();
        state
            .persistence
            .set_kv("projects", &serde_json::to_string(&list).map_err(err)?)
            .map_err(err)?;
    }
    Ok(list)
}

#[tauri::command]
pub async fn remove_project(
    state: State<'_, AppState>,
    path: String,
) -> Result<Vec<String>, String> {
    let mut list = list_projects(state.clone()).await?;
    list.retain(|p| p != &path);
    state
        .persistence
        .set_kv("projects", &serde_json::to_string(&list).map_err(err)?)
        .map_err(err)?;
    Ok(list)
}

// ── Thread land / sync (worktree merge flow) ─────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMergeResult {
    /// landed | needs_sync | synced | conflicts
    pub status: String,
    pub files: Vec<String>,
    pub branch: String,
    pub target_branch: String,
}

/// Worktree path + project root + branch for a thread, or a friendly error.
async fn thread_worktree_context(
    state: &AppState,
    id: Uuid,
) -> Result<(PathBuf, PathBuf, String, String), String> {
    // Live metadata first; fall back to the persisted record.
    let (cwd, project_root, label) = match state.registry.get_snapshot(id) {
        Ok(snap) => (
            snap.metadata.cwd.clone(),
            snap.metadata.project_root.clone(),
            snap.metadata.label.clone().unwrap_or_default(),
        ),
        Err(_) => {
            let rec = state.persistence.get_session(id).map_err(err)?;
            let root = serde_json::from_str::<serde_json::Value>(&rec.metadata_json)
                .ok()
                .and_then(|v| {
                    v.pointer("/metadata/projectRoot")
                        .or_else(|| v.pointer("/metadata/project_root"))
                        .and_then(|p| p.as_str())
                        .map(String::from)
                });
            (rec.cwd, root, String::new())
        }
    };
    let project_root = project_root
        .filter(|p| !p.is_empty())
        .ok_or("this thread has no isolated worktree (it works directly in the project folder)")?;
    let worktree = PathBuf::from(&cwd);
    let root = PathBuf::from(&project_root);
    let branch = state
        .worktrees
        .current_branch(&worktree)
        .await
        .map_err(err)?;
    Ok((worktree, root, branch, label))
}

/// Merge the thread's worktree branch back into the project's current branch.
#[tauri::command]
pub async fn land_thread(
    state: State<'_, AppState>,
    id: String,
) -> Result<ThreadMergeResult, String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    let (worktree, root, branch, label) = thread_worktree_context(&state, id).await?;
    let title = if label.is_empty() { branch.clone() } else { label.clone() };

    // Untracked files in the project must not block Land (feature-deep left
    // games/ in the project while the agent still needed Land from a worktree).
    if !state
        .worktrees
        .is_tracked_clean(&root)
        .await
        .map_err(err)?
    {
        return Err(format!(
            "the project folder has uncommitted edits to tracked files — commit or stash them in {} first, then Land again (untracked files are OK)",
            root.display()
        ));
    }
    let target_branch = state.worktrees.current_branch(&root).await.map_err(err)?;

    let target=serde_json::json!({"session_id":id,"repository":root,"worktree":worktree,"source_branch":branch,"target_branch":target_branch}).to_string();
    let outcome=crate::operations::recorded(&state.event_bus,"land_thread",target,async {
        let outcome=state.worktrees.merge_clean(&root,&branch,&format!("land thread: {title}"),&[&root,&worktree]).await.map_err(err)?;
        if matches!(outcome,grok_worktree::MergeOutcome::Conflicts{..}) {state.worktrees.merge_abort(&root).await;}
        Ok(outcome)
    }).await?;
    match outcome {
        grok_worktree::MergeOutcome::Merged => {
            let msg = format!("⬆ landed into {target_branch} ✓");
            thread_note(&state,id,"worktree",&msg)?;
            Ok(ThreadMergeResult {
                status: "landed".into(),
                files: vec![],
                branch,
                target_branch,
            })
        }
        grok_worktree::MergeOutcome::Conflicts { files } => {
            // Never leave the user's main checkout mid-merge.
            let msg = format!(
                "⚠ landing hit conflicts in {} — run Sync so this thread's agent can resolve them, then land again",
                files.join(", ")
            );
            thread_note(&state,id,"worktree",&msg)?;
            Ok(ThreadMergeResult {
                status: "needs_sync".into(),
                files,
                branch,
                target_branch,
            })
        }
    }
}

/// Merge the project's current branch INTO the thread's worktree. Conflicts
/// stay in the worktree where the thread's own agent can resolve them.
#[tauri::command]
pub async fn sync_thread(
    state: State<'_, AppState>,
    id: String,
) -> Result<ThreadMergeResult, String> {
    let id = Uuid::parse_str(&id).map_err(err)?;
    let (worktree, root, branch, label) = thread_worktree_context(&state, id).await?;
    let _title = if label.is_empty() { branch.clone() } else { label.clone() };
    let target_branch = state.worktrees.current_branch(&root).await.map_err(err)?;

    let target=serde_json::json!({"session_id":id,"repository":root,"worktree":worktree,"source_branch":target_branch,"target_branch":branch}).to_string();
    let outcome=crate::operations::recorded(&state.event_bus,"sync_thread",target,async {
        state.worktrees.merge_clean(&worktree,&target_branch,&format!("sync from {target_branch}"),&[&root,&worktree]).await.map_err(err)
    }).await?;
    match outcome {
        grok_worktree::MergeOutcome::Merged => {
            let msg = format!("⟳ synced from {target_branch} ✓");
            thread_note(&state,id,"worktree",&msg)?;
            Ok(ThreadMergeResult {
                status: "synced".into(),
                files: vec![],
                branch,
                target_branch,
            })
        }
        grok_worktree::MergeOutcome::Conflicts { files } => {
            let msg = format!(
                "⚠ merge conflicts from {target_branch} left in this worktree: {} — ask this thread's agent to resolve and commit them",
                files.join(", ")
            );
            thread_note(&state,id,"worktree",&msg)?;
            Ok(ThreadMergeResult {
                status: "conflicts".into(),
                files,
                branch,
                target_branch,
            })
        }
    }
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
pub async fn create_worktree(state:State<'_,AppState>,repo:String,name:String,base_ref:Option<String>)->Result<WorktreeInfo,String> {
    let target=serde_json::json!({"repository":repo,"name":name,"base_ref":base_ref}).to_string();
    crate::operations::recorded(&state.event_bus,"create_worktree",target,async {
        state.worktrees.create(PathBuf::from(repo).as_path(),CreateWorktreeRequest{name,base_ref,prefer_grok_cli:false}).await.map_err(err)
    }).await
}
#[tauri::command]
pub async fn remove_worktree(state:State<'_,AppState>,repo:String,name:String,force:bool)->Result<(),String> {
    let target=serde_json::json!({"repository":repo,"name":name,"force":force}).to_string();
    crate::operations::recorded(&state.event_bus,"remove_worktree",target,async {state.worktrees.remove(PathBuf::from(repo).as_path(),&name,force).await.map_err(err)}).await
}
#[tauri::command]
pub async fn prune_worktrees(state:State<'_,AppState>,repo:String)->Result<String,String> {
    let target=serde_json::json!({"repository":repo}).to_string();
    crate::operations::recorded(&state.event_bus,"prune_worktrees",target,async {state.worktrees.prune(PathBuf::from(repo).as_path()).await.map_err(err)}).await
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
    let ctl = if let Some(name) = preset {
        let presets = builtin_presets();
        let p = presets
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| format!("unknown preset: {name}"))?;
        PermissionController::with_preset(p)
    } else {
        PermissionController::from_defaults(&cfg.permissions, cfg.sandbox_profile)
    };
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
    let target=serde_json::json!({"name":name}).to_string();
    crate::operations::recorded(&state.event_bus,"add_mcp",target,async {
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
    }).await
}

#[tauri::command]
pub async fn remove_mcp(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let target=serde_json::json!({"name":name}).to_string();
    crate::operations::recorded(&state.event_bus,"remove_mcp",target,async {
    state.mcp.remove(&name).await.map_err(err)
    }).await
}

#[tauri::command]
pub async fn toggle_mcp(
    state: State<'_, AppState>,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    let target=serde_json::json!({"name":name,"enabled":enabled}).to_string();
    crate::operations::recorded(&state.event_bus,"toggle_mcp",target,async {
    state.mcp.set_enabled(&name, enabled).await.map_err(err)
    }).await
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
    let target=serde_json::json!({"name":request.name}).to_string();
    crate::operations::recorded(&state.event_bus,"add_mcp_server",target,async {
    state.mcp.add(request).await.map_err(err)
    }).await
}

#[tauri::command]
pub async fn update_mcp_server(
    state: State<'_, AppState>,
    request: UpdateMcpRequest,
) -> Result<McpServerConfigExt, String> {
    let target=serde_json::json!({"name":request.name}).to_string();
    crate::operations::recorded(&state.event_bus,"update_mcp_server",target,async {
    state.mcp.update(request).await.map_err(err)
    }).await
}

#[tauri::command]
pub async fn remove_mcp_server(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let target=serde_json::json!({"name":name}).to_string();
    crate::operations::recorded(&state.event_bus,"remove_mcp_server",target,async {
    state.mcp.remove(&name).await.map_err(err)
    }).await
}

#[tauri::command]
pub async fn doctor_mcp_server(
    state: State<'_, AppState>,
    name: Option<String>,
) -> Result<Vec<DoctorReport>, String> {
    let target=serde_json::json!({"name":name}).to_string();
    crate::operations::recorded(&state.event_bus,"doctor_mcp_server",target,async {
    state
        .mcp
        .doctor(name.as_deref())
        .await
        .map_err(err)
    }).await
}

#[tauri::command]
pub async fn list_mcp_tools(
    state: State<'_, AppState>,
    name: Option<String>,
) -> Result<Vec<McpToolInfo>, String> {
    let target=serde_json::json!({"name":name}).to_string();
    crate::operations::recorded(&state.event_bus,"list_mcp_tools",target,async {
    state.mcp.list_tools(name.as_deref()).await.map_err(err)
    }).await
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
    let target=serde_json::json!({"credential_key":key}).to_string();
    crate::operations::recorded(&state.event_bus,"set_mcp_credential",target,async {
    state.mcp.set_credential(&key, &value).await.map_err(err)
    }).await
}

#[tauri::command]
pub async fn list_mcp_credentials(
    state: State<'_, AppState>,
) -> Result<Vec<McpCredential>, String> {
    state.mcp.list_credentials_masked().await.map_err(err)
}

#[tauri::command]
pub async fn remove_mcp_credential(
    state: State<'_, AppState>,
    key: String,
) -> Result<(), String> {
    let target=serde_json::json!({"credential_key":key}).to_string();
    crate::operations::recorded(&state.event_bus,"remove_mcp_credential",target,async {
    state.mcp.credentials().remove(&key).map_err(err)
    }).await
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
) -> Result<serde_json::Value, String> {
    let res = state
        .mcp
        .session_mcp_payload(&names, &approved_high_risk, include_auto)
        .await
        .map_err(err)?;
    // Webview gets masked secrets only.
    let masked: Vec<serde_json::Value> = res
        .payload
        .iter()
        .map(grok_mcp::mask_payload_for_preview)
        .collect();
    Ok(serde_json::json!({
        "servers": masked,
        "attached": res.attached_names,
        "skipped": res.skipped,
    }))
}

#[tauri::command]
pub async fn add_skill(
    state: State<'_, AppState>,
    name: String,
    path: Option<String>,
    description: Option<String>,
    enabled: bool,
) -> Result<(), String> {
    let target=serde_json::json!({"name":name,"path":path,"enabled":enabled}).to_string();
    crate::operations::recorded(&state.event_bus,"add_skill",target,async {
    state
        .extensions
        .add_skill(name, path.map(PathBuf::from), description, enabled)
        .await
        .map_err(err)
    }).await
}

#[tauri::command]
pub async fn remove_skill(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let target=serde_json::json!({"name":name}).to_string();
    crate::operations::recorded(&state.event_bus,"remove_skill",target,async {
    state.extensions.remove_skill(&name).await.map_err(err)
    }).await
}

#[tauri::command]
pub async fn extensions_doctor(state: State<'_, AppState>) -> Result<String, String> {
    let target="extensions".into();
    crate::operations::recorded(&state.event_bus,"extensions_doctor",target,async {
    state.extensions.doctor().await.map_err(err)
    }).await
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
    let target=serde_json::json!({"scope":scope,"tags":tags}).to_string();
    crate::operations::recorded(&state.event_bus,"memory_add",target,async {state.memory.add(scope,content,tags).await.map_err(err)}).await
}

#[tauri::command]
pub async fn memory_remove(state: State<'_, AppState>, id: String) -> Result<(), String> {
    crate::operations::recorded(&state.event_bus,"memory_remove",serde_json::json!({"memory_id":id}).to_string(),async {state.memory.remove(&id).await.map_err(err)}).await
}

#[tauri::command]
pub async fn memory_flush(state: State<'_, AppState>, scope: String) -> Result<String, String> {
    crate::operations::recorded(&state.event_bus,"memory_flush",serde_json::json!({"scope":scope}).to_string(),async {state.memory.flush_markdown(&scope).await.map_err(err)}).await
}

/// Digest: LLM-summarize a scope's notes into one compact entry (tagged
/// `digest`). Originals stay — the user deletes what's superseded.
#[tauri::command]
pub async fn memory_digest(
    state: State<'_, AppState>,
    scope: String,
) -> Result<MemoryEntry, String> {
    let target=serde_json::json!({"scope":scope}).to_string();
    crate::operations::recorded(&state.event_bus,"memory_digest",target,async {
    let pack = state
        .memory
        .context_pack(&scope, 6000)
        .await
        .ok_or("no notes in this scope to digest")?;
    let summary = state
        .explainer
        .summarize(&format!(
            "Condense these project notes into at most 8 short bullet points, \
             merging duplicates and dropping anything obsolete. Keep concrete \
             facts (commands, versions, decisions). Output only the bullets.\n\n{pack}"
        ))
        .await?;
    state
        .memory
        .add(&scope, summary, vec!["digest".into()])
        .await
        .map_err(err)
    }).await
}

/// Scope key the given project folder maps to (for the Memory view).
#[tauri::command]
pub async fn project_scope(path: String) -> Result<String, String> {
    if path.trim().is_empty() {
        return Ok("global".into());
    }
    Ok(project_memory_scope(&path))
}

/// Pin a piece of transcript text into the project's durable memory.
#[tauri::command]
pub async fn remember(
    state: State<'_, AppState>,
    id: String,
    content: String,
) -> Result<MemoryEntry, String> {
    let target=serde_json::json!({"session_id":id}).to_string();
    crate::operations::recorded(&state.event_bus,"remember",target,async {
    let id = Uuid::parse_str(&id).map_err(err)?;
    let content = content.trim();
    if content.is_empty() {
        return Err("nothing to remember".into());
    }
    let content: String = content.chars().take(600).collect();
    // Scope by the thread's project (fall back to its cwd, then global).
    let root = state
        .registry
        .get_snapshot(id)
        .ok()
        .map(|s| s.metadata.project_root.clone().unwrap_or(s.metadata.cwd))
        .or_else(|| state.persistence.get_session(id).ok().map(|r| r.cwd))
        .filter(|s| !s.is_empty());
    let scope = root
        .map(|r| project_memory_scope(&r))
        .unwrap_or_else(|| "global".into());
    state
        .memory
        .add(&scope, content, vec!["remembered".into()])
        .await
        .map_err(err)
    }).await
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
    /// When false/omitted, the job is atomically admitted as paused so
    /// routines never auto-fire from the UI without an explicit enable.
    #[serde(default)]
    pub enable: bool,
}

fn require_scheduler_cwd(cwd: Option<String>) -> Result<String, String> {
    let cwd = cwd
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "working directory (cwd) is required".to_string())?;
    if !PathBuf::from(&cwd).is_absolute() {
        return Err("working directory must be an absolute path".into());
    }
    Ok(cwd)
}

#[tauri::command]
pub async fn scheduler_add(
    state: State<'_, AppState>,
    request: SchedulerAddRequest,
) -> Result<ScheduledJob, String> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err("job name is required".into());
    }
    let prompt = request.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err("prompt is required".into());
    }
    let cwd = require_scheduler_cwd(request.cwd)?;
    let schedule = if let Some(expr) = request.cron.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        ScheduleKind::Cron { expr }
    } else if let Some(d) = request.once_delay_secs {
        ScheduleKind::Once { delay_secs: d }
    } else {
        ScheduleKind::Interval {
            secs: request.interval_secs.unwrap_or(3600),
        }
    };
    let job = if request.enable {
        state.scheduler.add(name,prompt,schedule,Some(cwd),request.max_runs).await
    } else {
        state.scheduler.add_paused(name,prompt,schedule,Some(cwd),request.max_runs).await
    }.map_err(err)?;
    Ok(job)
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
    let scheduler = state.scheduler.stop_all().await;
    let retained=state.scheduler.retained_session_ids().await;
    let registry = state.registry.shutdown_preserving(retained).await;
    state.persistence.release_all_snapshots();
    let checkpoint = state.persistence.checkpoint();
    let errors = [scheduler.err().map(err),registry.err().map(err),checkpoint.err().map(err)].into_iter().flatten().collect::<Vec<_>>();
    if errors.is_empty() {Ok(())} else {Err(errors.join("; "))}
}

fn session_record(snap: &AgentHandleSnapshot) -> Result<SessionRecord, String> {
    Ok(SessionRecord {
        id:snap.metadata.id, cwd:snap.metadata.cwd.clone(), mode:match snap.metadata.mode { grok_control_core::AgentMode::Acp=>"acp", grok_control_core::AgentMode::Headless=>"headless" }.into(),
        model:snap.metadata.model.clone(), status:format!("{:?}",snap.metadata.status).to_lowercase(),
        worktree:snap.metadata.worktree.clone(), acp_session_id:snap.metadata.acp_session_id.clone(),
        metadata_json:serde_json::to_string(snap).map_err(err)?, created_at:snap.metadata.created_at,
        updated_at:Utc::now(), message_count:0,
    })
}
fn persist_session_checked(state: &AppState, id: Uuid) -> Result<(), String> {
    let bus=state.registry.session_event_bus(id).map_err(err)?;
    let snap=state.registry.get_snapshot(id).map_err(err)?;
    if bus.runtime_id()!=snap.metadata.runtime_id {return Err("session runtime changed while saving metadata".into());}
    bus.emit_checked(ControlEvent::SessionMetadataUpdated {session_id:id,metadata_json:serde_json::to_string(&session_record(&snap)?).map_err(err)?,at:Utc::now()}).map_err(err)?;
    Ok(())
}
pub(crate) async fn persist_session(state: &AppState, id: Uuid) -> Result<(), String> {
    persist_session_checked(state,id)
}
fn thread_bus(state:&AppState,id:Uuid)->Result<std::sync::Arc<grok_events::EventBus>,String> {
    if state.registry.is_live(id) {state.registry.session_event_bus(id).map_err(err)} else {Ok(state.event_bus.clone())}
}
fn thread_note(state:&AppState,id:Uuid,stream:&str,line:&str)->Result<(),String> {
    thread_bus(state,id)?.emit_checked(ControlEvent::Raw {session_id:Some(id),payload:serde_json::json!({"channel":"term","stream":stream,"line":line})}).map_err(err)?;
    Ok(())
}

#[derive(Serialize)]
pub struct EventSnapshotResponse {
    #[serde(flatten)] snapshot:grok_persistence::EventSnapshot,
    health:grok_events::EventHealth,
}
#[tauri::command]
pub async fn get_event_snapshot(state:State<'_,AppState>,cursor:Option<grok_persistence::SnapshotCursor>,session_id:Option<String>,limit:Option<usize>)->Result<EventSnapshotResponse,String> {
    let id=session_id.map(|id|Uuid::parse_str(&id)).transpose().map_err(err)?;
    let db=state.persistence.clone();let bus=state.event_bus.clone();
    tauri::async_runtime::spawn_blocking(move||{
        bus.with_read_boundary(|health|db.event_snapshot_page(cursor,id,limit.unwrap_or(256)).map(|snapshot|EventSnapshotResponse{snapshot,health})).map_err(err)?.map_err(err)
    }).await.map_err(err)?
}
#[tauri::command]
pub async fn release_event_snapshot(state:State<'_,AppState>,cursor:grok_persistence::SnapshotCursor)->Result<(),String> {
    state.persistence.release_snapshot(cursor);Ok(())
}
#[tauri::command]
pub async fn replay_events(state:State<'_,AppState>,cursor:grok_persistence::EventCursor,limit:Option<usize>)->Result<grok_persistence::ReplayPage,String> {
    let db=state.persistence.clone();
    tauri::async_runtime::spawn_blocking(move||db.replay_events(cursor,limit.unwrap_or(256))).await.map_err(err)?.map_err(err)
}
#[tauri::command]
pub async fn event_health(state:State<'_,AppState>)->Result<grok_events::EventHealth,String> {Ok(state.event_bus.health())}

fn build_thread_list(state: &AppState) -> Vec<ThreadDto> {
    let live = state.registry.list_sessions();
    let mut live_ids = std::collections::HashSet::new();
    let mut out: Vec<ThreadDto> = Vec::new();

    for m in live {
        live_ids.insert(m.id);
        let mode = match m.mode {
            grok_control_core::AgentMode::Acp => "acp",
            grok_control_core::AgentMode::Headless => "headless",
        };
        let status = format!("{:?}", m.status).to_lowercase();
        let msg_count = state
            .persistence
            .get_session(m.id)
            .map(|r| r.message_count)
            .unwrap_or(0);
        out.push(ThreadDto {
            id: m.id.to_string(),
            cwd: m.cwd,
            mode: mode.into(),
            model: m.model,
            backend: m.backend.key().to_string(),
            status,
            live: true,
            message_count: msg_count,
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.last_activity.to_rfc3339(),
            worktree: m.worktree,
            mcp_servers: m.mcp_servers,
            label: m.label,
            approval_mode: Some(
                serde_json::to_value(m.approval_mode)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| "ask".into()),
            ),
            project_root: m.project_root,
            brain_mode: Some(m.brain_mode.as_str().into()),
        });
    }

    if let Ok(saved) = state.persistence.list_sessions() {
        for rec in saved {
            if live_ids.contains(&rec.id) {
                continue;
            }
            // After reboot ACP is gone — never show stale "running".
            let status = match rec.status.to_lowercase().as_str() {
                "running" | "starting" | "cancelling" | "waitingapproval" => "saved".into(),
                other => other.to_string(),
            };
            let mcp = extract_mcp_from_meta(&rec.metadata_json);
            let backend = extract_backend_from_meta(&rec.metadata_json);
            let label = extract_meta_string(&rec.metadata_json, "label");
            let project_root = extract_meta_string(&rec.metadata_json, "projectRoot")
                .or_else(|| extract_meta_string(&rec.metadata_json, "project_root"));
            out.push(ThreadDto {
                id: rec.id.to_string(),
                cwd: rec.cwd,
                mode: rec.mode,
                model: rec.model,
                backend: backend.key().to_string(),
                status,
                live: false,
                message_count: rec.message_count,
                created_at: rec.created_at.to_rfc3339(),
                updated_at: rec.updated_at.to_rfc3339(),
                worktree: rec.worktree,
                mcp_servers: mcp,
                label,
                approval_mode: extract_meta_string(&rec.metadata_json, "approvalMode")
                    .or_else(|| extract_meta_string(&rec.metadata_json, "approval_mode")),
                project_root,
                brain_mode: None,
            });
        }
    }

    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out
}

fn extract_backend_from_meta(json: &str) -> grok_config::Backend {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| {
            v.pointer("/metadata/backend")
                .and_then(|b| b.as_str())
                .and_then(grok_config::Backend::from_key)
        })
        .unwrap_or_default()
}

fn extract_approved_mcp_from_meta(json: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| {
            v.pointer("/metadata/approvedHighRiskMcp")
                .or_else(|| v.pointer("/metadata/approved_high_risk_mcp"))
                .cloned()
        })
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn extract_meta_string(json: &str, key: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| {
            v.pointer(&format!("/metadata/{key}"))
                .and_then(|s| s.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from)
        })
}

fn extract_mcp_from_meta(json: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| {
            v.pointer("/metadata/mcpServers")
                .or_else(|| v.pointer("/metadata/mcp_servers"))
                .cloned()
        })
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

// ── Dev server / live preview ────────────────────────────────────────────

fn resolve_preview_cwd(state: &AppState, cwd: Option<String>, session_id: Option<String>) -> Result<PathBuf, String> {
    // A thread's own directory wins (it may be a worktree). Live threads carry
    // it in the registry; a saved thread has no live snapshot but its cwd is
    // still on disk — falling straight through to "session not found" made
    // preview unusable for any thread whose agent was not currently running.
    if let Some(id) = session_id.as_deref() {
        if let Ok(uuid) = Uuid::parse_str(id) {
            let from_thread = state
                .registry
                .get_snapshot(uuid)
                .map(|snap| snap.metadata.cwd)
                .ok()
                .or_else(|| state.persistence.get_session(uuid).map(|rec| rec.cwd).ok());
            if let Some(dir) = from_thread {
                let p = PathBuf::from(dir);
                if p.is_dir() {
                    return Ok(p);
                }
                // A worktree that has since been pruned: fall back to the
                // project cwd rather than dead-ending.
                tracing::warn!(path = %p.display(), "thread cwd is gone; falling back to project cwd");
            }
        }
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
    let target=serde_json::json!({"session_id":session_id,"cwd":cwd,"open_browser":open_browser}).to_string();
    crate::operations::recorded(&state.event_bus,"start_dev_server",target,async {
    let path = resolve_preview_cwd(&state, cwd, session_id)?;
    let _ = state.persistence.set_kv("last_cwd", &path.display().to_string());
    state
        .dev_server
        .start(&path, open_browser.unwrap_or(true))
        .await
    }).await
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
    let target="current_preview".into();
    crate::operations::recorded(&state.event_bus,"open_dev_server",target,async {
    state.dev_server.open_in_browser().await
    }).await
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

#[cfg(test)]
mod saved_launch_authority_tests {
    use super::*;
    #[test]
    fn saved_rename_preserves_authority_shape_and_rejects_corruption() {
        for source in [
            r#"{"label":"old","readOnly":true,"permissionDeny":["Write(*)"]}"#,
            r#"{"metadata":{"label":"old","readOnly":true,"permissionDeny":["Write(*)"]},"status":"idle"}"#,
        ] {
            let old: serde_json::Value = serde_json::from_str(source).unwrap();
            let changed: serde_json::Value = serde_json::from_str(&renamed_saved_metadata(source,"manual").unwrap()).unwrap();
            let mut expected = old;
            if expected.get("metadata").is_some() { expected["metadata"]["label"] = serde_json::json!("manual"); }
            else { expected["label"] = serde_json::json!("manual"); }
            assert_eq!(changed,expected);
        }
        for source in ["corrupt preserved bytes", "[]", r#"{"metadata":null}"#] {
            assert!(renamed_saved_metadata(source,"manual").is_err());
        }
    }
    #[test]
    fn reconnect_preserves_launch_ceiling_and_legacy_imports_default_to_plan() {
        let saved = saved_launch_authority(r#"{"metadata":{"sandboxProfile":"unrestricted","permissionAllow":["Read(*)"],"permissionDeny":["Write(*)"],"rules":["stay read only"],"readOnly":true,"trustRepo":true,"approvalMode":"ask","planMode":false,"alwaysApprove":false,"acpSessionId":"native-kept"}}"#).unwrap();
        assert_eq!(saved.sandbox_profile.as_deref(), Some("unrestricted"));
        assert!(saved.read_only && saved.trust_repo);
        assert_eq!(saved.permission_deny, ["Write(*)"]);
        assert_eq!(saved.permission_allow, ["Read(*)"]);
        assert_eq!(saved.rules, ["stay read only"]);
        assert_eq!(saved.approval_mode, Some(grok_control_core::ApprovalMode::Ask));
        let imported = saved_launch_authority(r#"{"source":"old-import","foreignMetadata":true}"#).unwrap();
        assert!(imported.plan_mode && !imported.always_approve);
        assert_eq!(imported.sandbox_profile.as_deref(),Some("workspace"));
        assert!(saved_launch_authority(r#"{"metadata":{"readOnly":"false"}}"#).is_err());
        assert!(saved_launch_authority("corrupt preserved bytes").is_err());
    }
}
