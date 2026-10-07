//! Concurrent session registry with ACP-first spawn path.

use std::path::Path;
use std::sync::Arc;
use std::{collections::{HashMap, HashSet}, sync::Mutex};
use grok_worktree::{WorkspaceCoordinator, WorkspaceLease};

use chrono::Utc;
use dashmap::DashMap;
use serde_json::json;
use tracing::{info, warn};
use uuid::Uuid;

use grok_acp::{AcpClient, AcpClientConfig, AcpSpawnOptions, ApprovalMode, BrainMode, ConnectOpts};
use grok_cli_wrapper::{GrokCli, HeadlessSpawnOptions};
use grok_config::{descriptor, resolve_backend, Backend, GrokConfig, ResolvedBackend};
use grok_events::{EventBus, SessionStatus};

use crate::error::{CoreError, Result};
use crate::handle::{AgentHandle, AgentHandleSnapshot, SessionMetadata};
use crate::options::{AgentMode, SpawnOptions};

pub struct SessionRegistry {
    sessions: Arc<DashMap<Uuid, AgentHandle>>,
    starting: Arc<DashMap<Uuid, ()>>,
    admitted: Arc<Mutex<HashSet<Uuid>>>,
    workspace_leases: Mutex<HashMap<Uuid, WorkspaceLease>>,
    mode_updates: Mutex<HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>,
    cleanup_updates: Mutex<HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>,
    event_bus: Arc<EventBus>,
    config: Arc<tokio::sync::RwLock<GrokConfig>>,
    grok_cli: Arc<GrokCli>,
}

struct AdmissionSlot { slots: Arc<Mutex<HashSet<Uuid>>>, id: Uuid, committed: bool }
impl AdmissionSlot {
    fn commit(&mut self) { self.committed = true; }
}
impl Drop for AdmissionSlot {
    fn drop(&mut self) { if !self.committed { if let Ok(mut slots) = self.slots.lock() { slots.remove(&self.id); } } }
}

/// Everything the deferred ACP connect needs, detached from `&self` so it can
/// run in a background task while the UI already shows the thread.
struct PendingConnect {
    client_cfg: AcpClientConfig,
    acp_opts: AcpSpawnOptions,
    connect_opts: ConnectOpts,
}

/// Run the ACP handshake and fill in (or fail) the placeholder session entry.
async fn connect_and_fill(
    sessions: Arc<DashMap<Uuid, AgentHandle>>,
    starting: Arc<DashMap<Uuid, ()>>,
    event_bus: Arc<EventBus>,
    id: Uuid,
    pending: PendingConnect,
) -> Result<()> {
    struct Connecting { starting: Arc<DashMap<Uuid, ()>>, id: Uuid }
    impl Drop for Connecting { fn drop(&mut self) { self.starting.remove(&self.id); } }
    let _connecting = Connecting { starting, id };
    match AcpClient::connect_with(
        pending.client_cfg,
        &pending.acp_opts,
        Some(event_bus.clone()),
        id,
        pending.connect_opts,
    )
    .await
    {
        Ok(client) => {
            let acp_session_id = client.session_id().await;
            let brain_mode = client.brain_mode().await;
            // Never hold a DashMap guard across an await.
            if let Some(mut entry) = sessions.get_mut(&id) {
                entry.metadata.acp_session_id = acp_session_id;
                entry.metadata.brain_mode = brain_mode;
                entry.metadata.status = SessionStatus::Idle;
                entry.acp_client = Some(client);
                entry.touch();
            } else {
                // Session was removed while starting — kill the orphan.
                let _ = client.shutdown().await;
            }
            Ok(())
        }
        Err(e) => {
            if let Some(mut entry) = sessions.get_mut(&id) {
                entry.metadata.status = SessionStatus::Failed;
                entry.touch();
            }
            event_bus.emit_error(Some(id), format!("session start failed: {e}"));
            event_bus.emit_status(id, SessionStatus::Failed).await;
            Err(e.into())
        }
    }
}

impl SessionRegistry {
    pub fn new(
        event_bus: Arc<EventBus>,
        config: Arc<tokio::sync::RwLock<GrokConfig>>,
        grok_cli: Arc<GrokCli>,
    ) -> Arc<Self> {
        let registry = Arc::new(Self {
            sessions: Arc::new(DashMap::new()),
            starting: Arc::new(DashMap::new()),
            admitted: Arc::new(Mutex::new(HashSet::new())),
            workspace_leases: Mutex::new(HashMap::new()),
            mode_updates: Mutex::new(HashMap::new()),
            cleanup_updates: Mutex::new(HashMap::new()),
            event_bus: event_bus.clone(),
            config,
            grok_cli,
        });
        // Mirror status events into live metadata. The ACP client reports
        // turn completion (Idle) only on the event bus; without this the
        // thread list — which reads metadata — shows "running" forever
        // after the first prompt.
        if tokio::runtime::Handle::try_current().is_ok() {
            let sessions = registry.sessions.clone();
            let mut rx = event_bus.subscribe();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(grok_events::ControlEvent::SessionStatusChanged {
                            session_id,
                            status,
                            ..
                        }) => {
                            if let Some(mut entry) = sessions.get_mut(&session_id) {
                                entry.metadata.status = status;
                                entry.metadata.last_activity = Utc::now();
                            }
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }
        registry
    }

    /// Start a session. The thread appears (status `Starting`) immediately;
    /// the ACP handshake completes in the background and flips it to
    /// `Idle`/`Failed` via status events.
    pub async fn spawn_agent(&self, cwd: &str, opts: SpawnOptions) -> Result<Uuid> {
        let id = Uuid::new_v4();
        self.spawn_agent_preallocated(id, cwd, opts, ConnectOpts::default())
            .await?;
        Ok(id)
    }

    /// Spawn with a caller-chosen id — used when the caller needs the id
    /// before spawning (e.g. to name the thread's worktree after it).
    /// `connect_opts` carries first-prompt injections (memory context).
    pub async fn spawn_agent_preallocated(
        &self,
        id: Uuid,
        cwd: &str,
        opts: SpawnOptions,
        connect_opts: ConnectOpts,
    ) -> Result<()> {
        self.spawn_agent_with_id(id, cwd, opts, None, connect_opts, true)
            .await
    }

    /// Host-only reviewed-role inheritance. This capability is never decoded
    /// from a frontend owner ID, session metadata or provider request.
    pub async fn spawn_build_role(&self, id: Uuid, cwd: &str, opts: SpawnOptions, owner: &WorkspaceLease) -> Result<()> {
        self.spawn_agent_owned(id, cwd, opts, None, ConnectOpts::default(), true, Some(owner)).await
    }

    /// Set the display label (smart thread name).
    pub fn set_label(&self, id: Uuid, label: &str) -> Result<()> {
        let mut entry = self
            .sessions
            .get_mut(&id)
            .ok_or(CoreError::SessionNotFound(id))?;
        entry.metadata.label = Some(label.to_string());
        Ok(())
    }

    /// Re-attach a live ACP process to an existing thread id (after reboot / update).
    /// Tries session/load for full brain; else history-only + transcript inject.
    pub async fn resume_session(
        &self,
        id: Uuid,
        cwd: &str,
        opts: SpawnOptions,
        created_at: Option<chrono::DateTime<Utc>>,
        connect_opts: ConnectOpts,
    ) -> Result<BrainMode> {
        if self.sessions.contains_key(&id) {
            let mode = if let Some(c) = self.sessions.get(&id).and_then(|h| h.acp_client.clone()) {
                c.brain_mode().await
            } else {
                BrainMode::Fresh
            };
            return Ok(mode);
        }
        // Resume is blocking: the caller sends a prompt right after, so the
        // client must be live before we return.
        self.spawn_agent_with_id(id, cwd, opts, created_at, connect_opts, false)
            .await?;
        let mode = self
            .sessions
            .get(&id)
            .and_then(|h| h.acp_client.clone())
            .map(|c| async move { c.brain_mode().await });
        let brain = if let Some(f) = mode {
            f.await
        } else {
            BrainMode::HistoryOnly
        };
        info!(%id, cwd, ?brain, "session resumed from disk");
        Ok(brain)
    }

    async fn spawn_agent_with_id(
        &self,
        id: Uuid,
        cwd: &str,
        opts: SpawnOptions,
        created_at: Option<chrono::DateTime<Utc>>,
        connect_opts: ConnectOpts,
        background: bool,
    ) -> Result<()> {
        self.spawn_agent_owned(id, cwd, opts, created_at, connect_opts, background, None).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn spawn_agent_owned(&self, id: Uuid, cwd: &str, opts: SpawnOptions, created_at: Option<chrono::DateTime<Utc>>, connect_opts: ConnectOpts, background: bool, owner: Option<&WorkspaceLease>) -> Result<()> {
        opts.validate().map_err(CoreError::InvalidOptions)?;

        let cwd_path = Path::new(cwd);
        if !cwd_path.is_absolute() {
            return Err(CoreError::InvalidOptions(format!(
                "cwd must be absolute: {cwd}"
            )));
        }
        if !cwd_path.exists() {
            return Err(CoreError::InvalidOptions(format!(
                "cwd does not exist: {cwd}"
            )));
        }

        let cfg = self.config.read().await;
        let max = cfg.max_concurrent_sessions;
        let mut slot = {
            let mut slots = self.admitted.lock().map_err(|_| CoreError::Internal("admission poisoned".into()))?;
            if slots.contains(&id) { return Err(CoreError::InvalidOptions(format!("session {id} already admitted"))); }
            if slots.len() >= max { return Err(CoreError::MaxSessions(max)); }
            slots.insert(id);
            AdmissionSlot { slots: self.admitted.clone(), id, committed: false }
        };
        let workspace = WorkspaceCoordinator::shared().session(cwd_path, owner).await
            .map_err(|e| CoreError::InvalidOptions(e.to_string()))?;

        if opts.always_approve {
            warn!("spawning with always_approve=true — elevated trust mode");
        }

        let backend = opts.backend;
        let model = opts.model.clone().unwrap_or_else(|| cfg.model_for(backend));
        // Mock threads need no binary; headless resolves via grok_cli.
        let needs_binary =
            matches!(opts.mode, AgentMode::Acp) && !model.eq_ignore_ascii_case("mock");
        let resolved: Option<ResolvedBackend> = if needs_binary {
            Some(resolve_backend(backend, &cfg)?)
        } else {
            None
        };
        let backend_env_cfg = cfg
            .backend_config(backend)
            .map(|c| c.env.clone())
            .unwrap_or_default();
        // Deny rules: global config deny list + per-spawn deny list, enforced
        // by the ACP client ahead of any approval.
        let mut deny_patterns = cfg.permissions.deny.clone();
        deny_patterns.extend(opts.permission_deny.iter().cloned());
        // Allow rules: auto-approve matching requests in any mode (deny wins).
        // These were previously built in config and silently dropped here.
        let mut allow_patterns = cfg.permissions.allow.clone();
        allow_patterns.extend(opts.permission_allow.iter().cloned());
        drop(cfg);
        let approval_mode = opts.resolved_mode();

        let now = Utc::now();
        let mut metadata = SessionMetadata {
            id,
            acp_session_id: None,
            cwd: cwd.to_string(),
            worktree: opts.worktree.clone(),
            project_root: opts.project_root.clone(),
            model: model.clone(),
            backend,
            mode: opts.mode,
            status: SessionStatus::Starting,
            approval_mode,
            plan_mode: approval_mode == ApprovalMode::Plan,
            read_only: opts.read_only,
            always_approve: approval_mode == ApprovalMode::Yolo,
            sandbox_profile: opts.sandbox_profile.clone(),
            mcp_servers: opts.mcp_server_names.clone(),
            approved_high_risk_mcp: opts.approved_high_risk_mcp.clone(),
            created_at: created_at.unwrap_or(now),
            last_activity: now,
            label: None,
            brain_mode: BrainMode::Fresh,
        };

        let handle = match opts.mode {
            AgentMode::Acp => {
                // Offline / mock threads from memory
                if model.eq_ignore_ascii_case("mock") {
                    let client = AcpClient::mock_for_tests(
                        &format!("mock-{id}"),
                        Some(self.event_bus.clone()),
                    );
                    client.set_approval_mode(approval_mode).await?;
                    metadata.acp_session_id = Some(format!("mock-{id}"));
                    metadata.status = SessionStatus::Idle;
                    metadata.label = Some("mock".into());
                    metadata.brain_mode = if connect_opts.transcript_context.is_some() {
                        BrainMode::HistoryOnly
                    } else {
                        BrainMode::Fresh
                    };
                    AgentHandle {
                        metadata,
                        child: None,
                        acp_client: Some(client),
                    }
                } else {
                    let acp_opts = AcpSpawnOptions {
                        model: Some(model),
                        rules: if opts.rules.is_empty() {
                            None
                        } else {
                            Some(json!(opts.rules))
                        },
                        mcp_servers: opts.mcp_servers.clone(),
                        plan_mode: metadata.plan_mode,
                        always_approve: metadata.always_approve,
                        approval_mode,
                        read_only: opts.read_only,
                        sandbox_profile: opts.sandbox_profile.clone(),
                        extra_env: Vec::new(),
                        deny_patterns,
                        allow_patterns,
                    };
                    let resolved = resolved.expect("resolved backend for live ACP spawn");
                    let desc = descriptor(backend);

                    // Forward backend API-key/env vars from the panel process,
                    // then apply per-backend config env on top.
                    let mut env: Vec<(String, String)> = Vec::new();
                    for key in desc.env_passthrough {
                        if let Ok(v) = std::env::var(key) {
                            if !v.is_empty() {
                                env.push(((*key).to_string(), v));
                            }
                        }
                    }
                    for (k, v) in backend_env_cfg {
                        env.retain(|(ek, _)| ek != &k);
                        env.push((k, v));
                    }
                    // Some claude adapter versions require the var to be defined;
                    // empty means "use the CLI login".
                    if backend == Backend::Claude
                        && !env.iter().any(|(k, _)| k == "ANTHROPIC_API_KEY")
                    {
                        env.push(("ANTHROPIC_API_KEY".into(), String::new()));
                    }

                    let mut client_cfg = AcpClientConfig::new(&resolved.program, cwd_path);
                    client_cfg.args = resolved.args.clone();
                    client_cfg.env = env;
                    client_cfg.auth_preference = desc
                        .auth_preference
                        .iter()
                        .map(|s| (*s).to_string())
                        .collect();
                    client_cfg.skip_auth_when_unadvertised = desc.skip_auth_when_unadvertised;
                    client_cfg.backend_label = backend.key().to_string();

                    // Insert a placeholder and announce the thread NOW — the
                    // handshake (spawn + initialize + auth + session/new) can
                    // take many seconds and the UI must not sit blank. The
                    // entry API also makes concurrent resume/start of the same
                    // id spawn exactly one process.
                    match self.sessions.entry(id) {
                        dashmap::mapref::entry::Entry::Occupied(_) => {
                            return Err(CoreError::InvalidOptions(format!(
                                "session {id} is already starting"
                            )));
                        }
                        dashmap::mapref::entry::Entry::Vacant(v) => {
                            v.insert(AgentHandle {
                                metadata: metadata.clone(),
                                child: None,
                                acp_client: None,
                            });
                        }
                    }
                    self.event_bus.emit_session_created(id, cwd, "acp").await;
                    self.event_bus
                        .emit_status(id, SessionStatus::Starting)
                        .await;

                    let pending = PendingConnect {
                        client_cfg,
                        acp_opts,
                        connect_opts,
                    };
                    self.starting.insert(id, ());
                    self.workspace_leases.lock().map_err(|_| CoreError::Internal("workspace admission poisoned".into()))?.insert(id, workspace);
                    slot.commit();
                    if background {
                        let starting = self.starting.clone();
                        let sessions = self.sessions.clone();
                        let bus = self.event_bus.clone();
                        tokio::spawn(async move {
                            let _ = connect_and_fill(sessions, starting, bus, id, pending).await;
                        });
                    } else {
                        // Blocking resume propagates failure while retaining
                        // the placeholder/lease until explicit cleanup.
                        connect_and_fill(
                            self.sessions.clone(),
                            self.starting.clone(),
                            self.event_bus.clone(),
                            id,
                            pending,
                        )
                        .await?;
                    }
                    info!(%id, cwd, background, "ACP session spawn initiated");
                    return Ok(());
                }
            }
            AgentMode::Headless => {
                grok_acp::ensure_native_policy_supported(opts.sandbox_profile.as_deref(), opts.read_only,
                    approval_mode == ApprovalMode::Plan, !deny_patterns.is_empty(), "Grok headless")?;
                if !opts.rules.is_empty() {
                    return Err(CoreError::InvalidOptions("headless policy capability unavailable: rules are metadata and cannot enforce native tool restrictions".into()));
                }
                let prompt = opts.prompt.clone().unwrap_or_default();
                let headless = HeadlessSpawnOptions {
                    model: Some(model),
                    worktree: opts.worktree.clone(),
                    always_approve: opts.always_approve,
                    plan_mode: metadata.plan_mode,
                    rules: opts.rules.clone(),
                    sandbox_profile: opts.sandbox_profile.clone(),
                    timeout_secs: None,
                };
                let child = self
                    .grok_cli
                    .spawn_headless(cwd_path, &prompt, &headless)
                    .await?;
                metadata.status = SessionStatus::Running;
                AgentHandle {
                    metadata,
                    child: Some(tokio::sync::Mutex::new(child)),
                    acp_client: None,
                }
            }
        };

        let mode_str = match opts.mode {
            AgentMode::Acp => "acp",
            AgentMode::Headless => "headless",
        };
        let status = handle.metadata.status;
        self.sessions.insert(id, handle);
        self.workspace_leases.lock().map_err(|_| CoreError::Internal("workspace admission poisoned".into()))?.insert(id, workspace);
        slot.commit();
        self.event_bus.emit_session_created(id, cwd, mode_str).await;
        self.event_bus.emit_status(id, status).await;

        info!(%id, mode = mode_str, cwd, "session spawned");
        Ok(())
    }

    /// Spawn a mock ACP session for tests / offline UI development.
    pub async fn spawn_mock(&self, cwd: &str) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let opts = SpawnOptions {
            model: Some("mock".into()),
            mode: AgentMode::Acp,
            ..Default::default()
        };
        self.spawn_agent_with_id(id, cwd, opts, None, ConnectOpts::default(), false)
            .await?;
        Ok(id)
    }

    pub fn is_live(&self, id: Uuid) -> bool {
        self.sessions.contains_key(&id)
    }

    pub async fn brain_mode(&self, id: Uuid) -> Option<BrainMode> {
        let c = self.sessions.get(&id)?.acp_client.clone()?;
        Some(c.brain_mode().await)
    }

    pub fn list_sessions(&self) -> Vec<SessionMetadata> {
        self.sessions
            .iter()
            .map(|e| e.value().metadata.clone())
            .collect()
    }

    pub fn get_snapshot(&self, id: Uuid) -> Result<AgentHandleSnapshot> {
        self.sessions
            .get(&id)
            .map(|h| h.snapshot())
            .ok_or(CoreError::SessionNotFound(id))
    }

    pub async fn send_prompt(&self, id: Uuid, prompt: &str) -> Result<()> {
        self.send_prompt_inner(id, prompt, false).await
    }

    /// Auditor/verifier prompts use their role instructions while native Plan
    /// mode and permission gates remain unchanged.
    pub async fn send_review_prompt(&self, id: Uuid, prompt: &str) -> Result<()> {
        self.send_prompt_inner(id, prompt, true).await
    }

    async fn send_prompt_inner(&self, id: Uuid, prompt: &str, review: bool) -> Result<()> {
        let client = {
            let mut entry = self
                .sessions
                .get_mut(&id)
                .ok_or(CoreError::SessionNotFound(id))?;
            if entry.acp_client.is_none()
                && matches!(entry.metadata.status, SessionStatus::Starting)
            {
                return Err(CoreError::InvalidOptions(
                    "session is still starting — wait for it to become idle".into(),
                ));
            }
            entry.touch();
            entry.metadata.status = SessionStatus::Running;
            entry.acp_client.clone().ok_or(CoreError::NotAcp)?
        };
        self.event_bus.emit_status(id, SessionStatus::Running).await;
        if review { client.send_review_prompt(prompt).await?; } else { client.send_prompt(prompt).await?; }
        Ok(())
    }

    pub async fn cancel_session(&self, id: Uuid) -> Result<()> {
        if !self.sessions.contains_key(&id) { return Err(CoreError::SessionNotFound(id)); }
        let lock = self.cleanup_updates.lock().map_err(|_| CoreError::Internal("cleanup gate poisoned".into()))?
            .entry(id).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone();
        let _cleanup = lock.lock().await;
        if !self.cleanup_updates.lock().map_err(|_| CoreError::Internal("cleanup gate poisoned".into()))?
            .get(&id).is_some_and(|current| Arc::ptr_eq(current, &lock)) {
            return Err(CoreError::SessionNotFound(id));
        }
        self.cancel_session_inner(id).await
    }

    async fn cancel_session_inner(&self, id: Uuid) -> Result<()> {
        let (acp, has_child) = {
            let mut entry = self
                .sessions
                .get_mut(&id)
                .ok_or(CoreError::SessionNotFound(id))?;
            entry.metadata.status = SessionStatus::Cancelling;
            (entry.acp_client.clone(), entry.child.is_some())
        };

        if let Some(client) = acp {
            client.cancel().await?;
        }
        if has_child {
            let child = self.sessions.get_mut(&id).and_then(|mut entry| entry.child.take());
            if let Some(child) = child {
                let result = child.lock().await.kill().await;
                if let Err(error) = result {
                    if let Some(mut entry) = self.sessions.get_mut(&id) { entry.child = Some(child); }
                    return Err(error.into());
                }
            }
        }

        if let Some(mut entry) = self.sessions.get_mut(&id) {
            entry.metadata.status = SessionStatus::Cancelled;
            entry.touch();
        }
        self.event_bus.emit_session_cancelled(id).await;
        Ok(())
    }

    pub async fn set_plan_mode(&self, id: Uuid, enabled: bool) -> Result<()> {
        self.set_approval_mode(id, if enabled { ApprovalMode::Plan } else { ApprovalMode::Ask }).await
    }

    /// Switch a live session's approval stance (composer pills).
    pub async fn set_approval_mode(&self, id: Uuid, mode: ApprovalMode) -> Result<()> {
        let lock = {
            let mut locks = self.mode_updates.lock().map_err(|_| CoreError::Internal("mode transition gate poisoned".into()))?;
            if !self.sessions.contains_key(&id) { return Err(CoreError::SessionNotFound(id)); }
            locks.entry(id).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
        };
        let expected_client = self.sessions.get(&id).ok_or(CoreError::SessionNotFound(id))?
            .acp_client.clone().ok_or(CoreError::NotAcp)?;
        let _transition = lock.lock().await;
        let client = {
            let entry = self
                .sessions
                .get(&id)
                .ok_or(CoreError::SessionNotFound(id))?;
            if entry.metadata.read_only && mode != ApprovalMode::Plan { return Err(CoreError::InvalidOptions("immutable read-only role cannot elevate permissions".into())); }
            let client = entry.acp_client.clone().ok_or(CoreError::NotAcp)?;
            if !Arc::ptr_eq(&client, &expected_client) {
                return Err(CoreError::InvalidOptions("session generation changed before mode transition".into()));
            }
            client
        };
        {
            // Client-side gating is authoritative; the agent-side mode is
            // best-effort (most adapters don't advertise an auto mode).
            client.set_approval_mode(mode).await?;
            let applied = client.approval_mode().await;
            let mut entry = self.sessions.get_mut(&id).ok_or(CoreError::SessionNotFound(id))?;
            if !entry.acp_client.as_ref().is_some_and(|current| Arc::ptr_eq(current, &client)) {
                return Err(CoreError::InvalidOptions("session generation changed during mode transition".into()));
            }
            entry.metadata.approval_mode = applied;
            entry.metadata.plan_mode = applied == ApprovalMode::Plan;
            entry.metadata.always_approve = applied == ApprovalMode::Yolo;
            entry.touch();
            drop(entry);
            let wanted = match mode {
                ApprovalMode::Plan => "plan",
                ApprovalMode::Auto => "auto",
                ApprovalMode::Yolo => "always_approve",
                ApprovalMode::Ask => "default",
            };
            if let Err(e) = client.set_mode(wanted).await {
                tracing::warn!(error = %e, ?mode, "agent-side mode not applied (client gate still enforces it)");
            }
        }
        Ok(())
    }

    /// Record an "always allow this" rule for the rest of the session.
    pub async fn add_session_allow_rule(&self, id: Uuid, pattern: String) -> Result<()> {
        let client = self
            .sessions
            .get(&id)
            .ok_or(CoreError::SessionNotFound(id))?
            .acp_client
            .clone()
            .ok_or(CoreError::NotAcp)?;
        client.add_session_allow_rule(pattern).await;
        Ok(())
    }

    pub async fn set_always_approve(&self, id: Uuid, enabled: bool) -> Result<()> {
        self.set_approval_mode(id, if enabled { ApprovalMode::Yolo } else { ApprovalMode::Ask }).await
    }

    pub async fn respond_approval(
        &self,
        id: Uuid,
        request_id: &str,
        option_id: Option<&str>,
    ) -> Result<()> {
        let client = self
            .sessions
            .get(&id)
            .ok_or(CoreError::SessionNotFound(id))?
            .acp_client
            .clone()
            .ok_or(CoreError::NotAcp)?;
        client.respond_approval(request_id, option_id).await?;
        if let Some(mut entry) = self.sessions.get_mut(&id) {
            entry.metadata.status = SessionStatus::Running;
            entry.touch();
        }
        Ok(())
    }

    pub async fn remove_session(&self, id: Uuid) -> Result<()> {
        if !self.sessions.contains_key(&id) && !self.starting.contains_key(&id) { return Ok(()); }
        let lock = self.cleanup_updates.lock().map_err(|_| CoreError::Internal("cleanup gate poisoned".into()))?
            .entry(id).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone();
        let _cleanup = lock.lock().await;
        if !self.cleanup_updates.lock().map_err(|_| CoreError::Internal("cleanup gate poisoned".into()))?
            .get(&id).is_some_and(|current| Arc::ptr_eq(current, &lock)) { return Ok(()); }
        // A cancelled Starting placeholder can still own a detached handshake.
        // Retain it until handshake cleanup is observable, or return a retryable error.
        tokio::time::timeout(std::time::Duration::from_secs(120), async {
            while self.starting.contains_key(&id) { tokio::time::sleep(std::time::Duration::from_millis(25)).await; }
        }).await.map_err(|_| CoreError::Internal("native startup still pending; retry cleanup".into()))?;
        let cancelled = if self.sessions.contains_key(&id) { self.cancel_session_inner(id).await } else { Ok(()) };
        // cancel() only sends session/cancel — the grok child (and any MCP
        // servers it spawned) keeps running unless we kill it.
        let client = self.sessions.get(&id).and_then(|handle| handle.acp_client.clone());
        if let Some(client) = client {
            // A stopped transport can reject session/cancel; observed process
            // shutdown is the cleanup boundary. A failed shutdown retains all
            // handles and leases, independently of the cancel response.
            client.shutdown().await?;
        } else {
            cancelled?;
        }
        self.sessions.remove(&id);
        self.workspace_leases.lock().map_err(|_| CoreError::Internal("workspace admission poisoned".into()))?.remove(&id);
        self.admitted.lock().map_err(|_| CoreError::Internal("admission poisoned".into()))?.remove(&id);
        self.mode_updates.lock().map_err(|_| CoreError::Internal("mode transition gate poisoned".into()))?.remove(&id);
        self.cleanup_updates.lock().map_err(|_| CoreError::Internal("cleanup gate poisoned".into()))?.remove(&id);
        Ok(())
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub async fn shutdown_all(&self) {
        let ids: Vec<Uuid> = self.sessions.iter().map(|e| *e.key()).collect();
        for id in ids {
            if let Err(e) = self.remove_session(id).await {
                warn!(%id, error = %e, "shutdown unresolved; session admission retained");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grok_cli_wrapper::GrokCli;
    use grok_events::shared_bus;
    use std::path::PathBuf;

    fn test_registry() -> Arc<SessionRegistry> {
        let bus = shared_bus();
        let cfg = Arc::new(tokio::sync::RwLock::new(GrokConfig::default()));
        let cli = Arc::new(GrokCli::new(PathBuf::from("/bin/true")));
        SessionRegistry::new(bus, cfg, cli)
    }

    #[tokio::test]
    async fn mock_session_lifecycle() {
        let reg = test_registry();
        let cwd = tempfile::tempdir().unwrap();
        let id = reg.spawn_mock(cwd.path().to_str().unwrap()).await.unwrap();
        assert_eq!(reg.session_count(), 1);
        let snap = reg.get_snapshot(id).unwrap();
        assert_eq!(snap.metadata.cwd, cwd.path().to_str().unwrap());
        assert_eq!(snap.metadata.mode, AgentMode::Acp);
        reg.cancel_session(id).await.unwrap();
        assert_eq!(
            reg.get_snapshot(id).unwrap().metadata.status,
            SessionStatus::Cancelled
        );
        reg.remove_session(id).await.unwrap();
        assert_eq!(reg.session_count(), 0);
    }

    #[tokio::test]
    async fn removal_waits_for_detached_startup_ownership() {
        let reg = test_registry();
        let cwd = tempfile::tempdir().unwrap();
        let id = reg.spawn_mock(cwd.path().to_str().unwrap()).await.unwrap();
        reg.starting.insert(id, ());
        let removing = reg.clone();
        let task = tokio::spawn(async move { removing.remove_session(id).await });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(reg.is_live(id));
        assert!(!task.is_finished());
        reg.starting.remove(&id);
        task.await.unwrap().unwrap();
        assert!(!reg.is_live(id));
    }

    #[tokio::test]
    async fn concurrent_cleanup_waits_and_cannot_release_owner_early() {
        let reg = test_registry();
        let cwd = tempfile::tempdir().unwrap();
        let id = reg.spawn_mock(cwd.path().to_str().unwrap()).await.unwrap();
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        reg.cleanup_updates.lock().unwrap().insert(id, gate.clone());
        let held = gate.lock().await;
        let removing = reg.clone();
        let task = tokio::spawn(async move { removing.remove_session(id).await });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(!task.is_finished());
        assert!(WorkspaceCoordinator::shared().session(cwd.path(), None).await.is_err());
        drop(held);
        task.await.unwrap().unwrap();
        assert!(WorkspaceCoordinator::shared().session(cwd.path(), None).await.is_ok());
    }

    #[tokio::test]
    async fn list_sessions() {
        let reg = test_registry();
        let a = tempfile::tempdir().unwrap(); let b = tempfile::tempdir().unwrap();
        let _a = reg.spawn_mock(a.path().to_str().unwrap()).await.unwrap();
        let _b = reg.spawn_mock(b.path().to_str().unwrap()).await.unwrap();
        assert_eq!(reg.list_sessions().len(), 2);
    }

    #[test]
    fn spawn_options_validate_headless() {
        let mut opts = SpawnOptions {
            mode: AgentMode::Headless,
            prompt: None,
            ..Default::default()
        };
        assert!(opts.validate().is_err());
        opts.prompt = Some("do thing".into());
        assert!(opts.validate().is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn headless_unsupported_policy_is_rejected_before_marker_worker_launch() {
        use std::os::unix::fs::PermissionsExt;
        let cwd = tempfile::tempdir().unwrap();
        let program = cwd.path().join("native-worker");
        std::fs::write(&program, "#!/bin/sh\nprintf launched > \"$0.marker\"\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = Arc::new(tokio::sync::RwLock::new(GrokConfig::default()));
        config.write().await.permissions.deny.clear();
        let reg = SessionRegistry::new(shared_bus(), config.clone(), Arc::new(GrokCli::new(&program)));
        let base = SpawnOptions { mode:AgentMode::Headless, prompt:Some("generated offline fixture".into()),
            approval_mode:Some(ApprovalMode::Ask), plan_mode:false,
            sandbox_profile:Some("unrestricted".into()), ..Default::default() };
        let policies = [
            SpawnOptions { approval_mode:Some(ApprovalMode::Plan), ..base.clone() },
            SpawnOptions { sandbox_profile:Some("workspace".into()), ..base.clone() },
            SpawnOptions { sandbox_profile:Some("strict".into()), ..base.clone() },
            SpawnOptions { sandbox_profile:Some("read-only".into()), ..base.clone() },
            SpawnOptions { read_only:true, ..base.clone() },
            SpawnOptions { permission_deny:vec!["Write(*)".into()], ..base.clone() },
            SpawnOptions { rules:vec!["Never mutate files".into()], ..base.clone() },
        ];
        for options in policies {
            assert!(reg.spawn_agent(cwd.path().to_str().unwrap(), options).await.is_err());
            assert_eq!(reg.session_count(), 0);
            assert!(reg.admitted.lock().unwrap().is_empty());
            assert!(!program.with_file_name("native-worker.marker").exists());
        }
        config.write().await.permissions.deny.push("Bash(*)".into());
        assert!(reg.spawn_agent(cwd.path().to_str().unwrap(), base.clone()).await.is_err());
        assert!(!program.with_file_name("native-worker.marker").exists());
        config.write().await.permissions.deny.clear();
        let id = reg.spawn_agent(cwd.path().to_str().unwrap(), base).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !program.with_file_name("native-worker.marker").exists() { tokio::task::yield_now().await; }
        }).await.unwrap();
        reg.remove_session(id).await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_admission_counts_pending_and_retains_cancelled_owner() {
        let reg = test_registry();
        reg.config.write().await.max_concurrent_sessions = 1;
        let a = tempfile::tempdir().unwrap(); let b = tempfile::tempdir().unwrap();
        let (left, right) = tokio::join!(reg.spawn_mock(a.path().to_str().unwrap()), reg.spawn_mock(b.path().to_str().unwrap()));
        assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
        let id = left.or(right).unwrap();
        reg.cancel_session(id).await.unwrap();
        assert!(reg.spawn_mock(b.path().to_str().unwrap()).await.is_err());
        reg.remove_session(id).await.unwrap();
        assert!(reg.spawn_mock(b.path().to_str().unwrap()).await.is_ok());
    }

    #[tokio::test]
    async fn failed_preflight_releases_slot_and_alias_cannot_start_second_owner() {
        let reg = test_registry(); reg.config.write().await.max_concurrent_sessions = 1;
        let a = tempfile::tempdir().unwrap();
        let bad = SpawnOptions { model: Some("mock".into()), sandbox_profile: Some("misspelled".into()), ..Default::default() };
        assert!(reg.spawn_agent(a.path().to_str().unwrap(), bad).await.is_err());
        let id = reg.spawn_mock(a.path().to_str().unwrap()).await.unwrap();
        reg.config.write().await.max_concurrent_sessions = 2;
        assert!(reg.spawn_mock(a.path().join(".").to_str().unwrap()).await.is_err());
        reg.cancel_session(id).await.unwrap();
        assert!(reg.spawn_mock(a.path().to_str().unwrap()).await.is_err());
        reg.remove_session(id).await.unwrap();
        assert!(reg.spawn_mock(a.path().to_str().unwrap()).await.is_ok());
    }

    #[tokio::test]
    async fn immutable_role_capability_and_mcp_cannot_be_forged_by_options() {
        let reg = test_registry(); let a = tempfile::tempdir().unwrap();
        let owner = WorkspaceCoordinator::shared().build(a.path().into(), a.path().into(), vec!["src".into()]).unwrap();
        let mut opts: SpawnOptions = serde_json::from_value(json!({"model":"mock", "workspaceOwner":"fake", "projectRoot":a.path(),"isolateWorktree":false})).unwrap();
        assert!(reg.spawn_agent(a.path().to_str().unwrap(), opts.clone()).await.is_err());
        opts.read_only = true;
        let id = Uuid::new_v4();
        reg.spawn_build_role(id, a.path().to_str().unwrap(), opts.clone(), &owner).await.unwrap();
        assert!(reg.set_always_approve(id, true).await.is_err());
        assert!(reg.set_plan_mode(id, false).await.is_err());
        assert!(reg.set_approval_mode(id, ApprovalMode::Ask).await.is_err());
        reg.remove_session(id).await.unwrap();
        opts.mcp_servers = vec![json!({"name":"filesystem"})];
        assert!(reg.spawn_build_role(Uuid::new_v4(), a.path().to_str().unwrap(), opts, &owner).await.is_err());
    }

    #[tokio::test]
    async fn legacy_mode_commands_use_the_authoritative_policy_owner() {
        let reg = test_registry(); let a = tempfile::tempdir().unwrap();
        let id = reg.spawn_mock(a.path().to_str().unwrap()).await.unwrap();
        let client = reg.sessions.get(&id).unwrap().acp_client.clone().unwrap();
        assert_eq!(client.approval_mode().await, ApprovalMode::Plan);
        reg.set_always_approve(id,true).await.unwrap();
        assert_eq!(client.approval_mode().await,ApprovalMode::Yolo);
        reg.set_plan_mode(id,true).await.unwrap();
        assert_eq!(client.approval_mode().await,ApprovalMode::Plan);
        let metadata = reg.get_snapshot(id).unwrap().metadata;
        assert!(metadata.plan_mode && !metadata.always_approve);
        assert_eq!(metadata.approval_mode,ApprovalMode::Plan);
        reg.set_always_approve(id,false).await.unwrap();
        assert_eq!(client.approval_mode().await,ApprovalMode::Ask);
        assert!(!reg.get_snapshot(id).unwrap().metadata.plan_mode);
    }
}
