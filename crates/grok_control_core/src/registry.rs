//! Concurrent session registry with ACP-first spawn path.

use std::path::Path;
use std::sync::{Arc,atomic::{AtomicBool,Ordering}};
use std::{collections::{HashMap, HashSet}, sync::Mutex};
use grok_worktree::{WorkspaceCoordinator, WorkspaceLease};

use chrono::Utc;
use dashmap::DashMap;
use serde_json::json;
use tracing::{info, warn};
use uuid::Uuid;

use grok_acp::{AcpClient, AcpClientConfig, AcpSpawnOptions, ApprovalMode, BrainMode, ConnectOpts};
use grok_cli_wrapper::{GrokCli, HeadlessSpawnOptions};
use grok_cli_wrapper::process::{ProcessHandle, ProcessConfig, ProcessOutcome, ProcessEnd};
use grok_config::{descriptor, resolve_backend, Backend, GrokConfig, ResolvedBackend};
use grok_events::{ControlEvent, EventBus, SessionStatus};

use crate::error::{CoreError, Result};
use crate::handle::{AgentHandle, AgentHandleSnapshot, SessionMetadata};
use crate::options::{AgentMode, SpawnOptions};

pub struct SessionRegistry {
    sessions: Arc<DashMap<Uuid, AgentHandle>>,
    starting: Arc<DashMap<Uuid, ()>>,
    admitted: Arc<Mutex<HashSet<Uuid>>>,
    shutting_down: AtomicBool,
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
            let scope_current=sessions.get(&id).is_some_and(|entry|entry.event_bus.runtime_id()==event_bus.runtime_id());
            if !scope_current {client.shutdown().await?;return Err(grok_events::EventError::StaleRuntime.into());}
            let acp_session_id = client.session_id().await;
            let brain_mode = client.brain_mode().await;
            let stop_requested = sessions.get(&id).is_some_and(|entry| matches!(entry.metadata.status, SessionStatus::Cancelling | SessionStatus::Cancelled));
            let stop_result = if stop_requested { Some(client.cancel().await) } else { None };
            // Never hold a DashMap guard across an await.
            if let Some(mut entry) = sessions.get_mut(&id) {
                if entry.event_bus.runtime_id()!=event_bus.runtime_id() {drop(entry);client.shutdown().await?;return Err(grok_events::EventError::StaleRuntime.into());}
                entry.metadata.acp_session_id = acp_session_id;
                entry.metadata.brain_mode = brain_mode;
                entry.metadata.status = if stop_requested {
                    if stop_result.as_ref().is_some_and(|result| result.is_ok()) {SessionStatus::Cancelled} else {SessionStatus::Cancelling}
                } else {SessionStatus::Idle};
                entry.acp_client = Some(client);
                entry.touch();
            } else {
                // Session was removed while starting — kill the orphan.
                client.shutdown().await?;
            }
            Ok(())
        }
        Err(e) => {
            if let Some(mut entry) = sessions.get_mut(&id) {
                if entry.event_bus.runtime_id()!=event_bus.runtime_id() {return Err(e.into());}
                if let grok_acp::AcpError::StartupCleanup { process, .. } = &e { entry.child = Some(process.clone()); }
                entry.metadata.status = SessionStatus::Failed;
                entry.touch();
            }
            event_bus.emit_error(Some(id), format!("session start failed: {e}"));
            event_bus.emit_status(id, SessionStatus::Failed).await;
            Err(e.into())
        }
    }
}

fn metadata_record_json(metadata:&SessionMetadata)->Result<String> {
    let snapshot=serde_json::to_string(&json!({"metadata":metadata})).map_err(|error|CoreError::Internal(error.to_string()))?;
    serde_json::to_string(&json!({"id":metadata.id,"cwd":metadata.cwd,"mode":match metadata.mode {AgentMode::Acp=>"acp",AgentMode::Headless=>"headless"},"model":metadata.model,"status":metadata.status,"worktree":metadata.worktree,"acpSessionId":metadata.acp_session_id,"metadataJson":snapshot,"createdAt":metadata.created_at,"updatedAt":metadata.last_activity,"messageCount":0})).map_err(|error|CoreError::Internal(error.to_string()))
}

fn outcome_status(outcome: &ProcessOutcome) -> SessionStatus {
    if !outcome.cleanup_complete { return SessionStatus::Failed; }
    match outcome.end { ProcessEnd::Success => SessionStatus::Completed, ProcessEnd::Cancelled => SessionStatus::Cancelled, _ => SessionStatus::Failed }
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
            shutting_down: AtomicBool::new(false),
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
            let mut rx = event_bus.subscribe_committed();
            let mirror_bus = event_bus.clone();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(envelope) => {
                            if let ControlEvent::SessionStatusChanged{session_id,status,..}=envelope.event {
                                if let Some(mut entry)=sessions.get_mut(&session_id) {
                                    if entry.event_bus.runtime_id()!=envelope.origin.runtime_id {continue;}
                                    // A queued event may predate a later commit in this runtime.
                                    entry.metadata.status = if mirror_bus.ensure_healthy().is_err() { SessionStatus::Failed } else {
                                        mirror_bus.current_status(session_id).filter(|(runtime,_)| Some(*runtime)==entry.event_bus.runtime_id()).map(|(_,current)| current).unwrap_or(status)
                                    };
                                    entry.metadata.last_activity=Utc::now();
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            // Reconcile from committed state, never from a missed raw stream.
                            let unhealthy = mirror_bus.ensure_healthy().is_err();
                            for mut entry in sessions.iter_mut() {
                                if unhealthy { entry.metadata.status=SessionStatus::Failed; }
                                else if let Some((runtime,status))=mirror_bus.current_status(*entry.key()) {
                                    if Some(runtime)==entry.event_bus.runtime_id() { entry.metadata.status=status; }
                                }
                            }
                        }
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
    pub fn set_label_for_runtime(&self,id:Uuid,expected_runtime:Uuid,label:&str)->Result<()> {
        self.set_label_checked(id,Some(expected_runtime),None,label)
    }
    pub fn set_label_if_current(&self,id:Uuid,expected_runtime:Uuid,expected_label:Option<&str>,label:&str)->Result<()> {
        self.set_label_checked(id,Some(expected_runtime),Some(expected_label),label)
    }
    pub fn set_label(&self,id:Uuid,label:&str)->Result<()> {self.set_label_checked(id,None,None,label)}
    fn set_label_checked(&self,id:Uuid,expected_runtime:Option<Uuid>,expected_label:Option<Option<&str>>,label:&str)->Result<()> {
        let mut entry=self.sessions.get_mut(&id).ok_or(CoreError::SessionNotFound(id))?;
        if expected_runtime.is_some_and(|expected|entry.event_bus.runtime_id()!=Some(expected)){return Err(grok_events::EventError::StaleRuntime.into());}
        if expected_label.is_some_and(|expected|entry.metadata.label.as_deref()!=expected){return Err(CoreError::InvalidOptions("label changed before title update".into()));}
        let mut metadata=entry.metadata.clone();metadata.label=Some(label.into());metadata.last_activity=Utc::now();
        entry.event_bus.emit_checked(ControlEvent::SessionMetadataUpdated{session_id:id,metadata_json:metadata_record_json(&metadata)?,at:Utc::now()})?;
        entry.metadata=metadata;Ok(())
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
            if self.shutting_down.load(Ordering::Acquire) {return Err(CoreError::InvalidOptions("app is stopping; session admission refused".into()));}
            if slots.contains(&id) { return Err(CoreError::InvalidOptions(format!("session {id} already admitted"))); }
            if slots.len() >= max { return Err(CoreError::MaxSessions(max)); }
            slots.insert(id);
            AdmissionSlot { slots: self.admitted.clone(), id, committed: false }
        };
        let workspace = WorkspaceCoordinator::shared().session(cwd_path, owner).await
            .map_err(|e| CoreError::InvalidOptions(e.to_string()))?;
        if self.shutting_down.load(Ordering::Acquire) {return Err(CoreError::InvalidOptions("app is stopping; pending admission refused".into()));}

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

        if opts.mode==AgentMode::Headless {
                grok_acp::ensure_native_policy_supported(opts.sandbox_profile.as_deref(), opts.read_only,
                    approval_mode == ApprovalMode::Plan, !deny_patterns.is_empty(), "Grok headless")?;
                if !opts.rules.is_empty() {
                    return Err(CoreError::InvalidOptions("headless policy capability unavailable: rules are metadata and cannot enforce native tool restrictions".into()));
                }
        }
        if opts.mode == AgentMode::Headless || !model.eq_ignore_ascii_case("mock") { self.event_bus.ensure_durable()?; }
        let runtime_bus = self.event_bus.register_runtime(id)?;
        runtime_bus.emit_checked(ControlEvent::SessionCreated {session_id:id,cwd:cwd.into(),mode:match opts.mode {AgentMode::Acp=>"acp",AgentMode::Headless=>"headless"}.into(),at:Utc::now()})?;
        runtime_bus.emit_checked(ControlEvent::SessionStatusChanged {session_id:id,status:SessionStatus::Starting,at:Utc::now()})?;
        let now = Utc::now();
        let mut metadata = SessionMetadata {
            id,
            runtime_id: runtime_bus.runtime_id(),
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
            permission_allow: opts.permission_allow.clone(),
            permission_deny: opts.permission_deny.clone(),
            rules: opts.rules.clone(),
            trust_repo: opts.trust_repo,
            process_outcome: None,
            mcp_servers: opts.mcp_server_names.clone(),
            approved_high_risk_mcp: opts.approved_high_risk_mcp.clone(),
            created_at: created_at.unwrap_or(now),
            last_activity: now,
            label: None,
            brain_mode: BrainMode::Fresh,
        };

        runtime_bus.emit_checked(ControlEvent::SessionMetadataUpdated{session_id:id,metadata_json:metadata_record_json(&metadata)?,at:Utc::now()})?;
        let mut headless_operation=None;
        let handle = match opts.mode {
            AgentMode::Acp => {
                // Offline / mock threads from memory
                if model.eq_ignore_ascii_case("mock") {
                    let client = AcpClient::mock_for_session(
                        id, &format!("mock-{id}"),
                        Some(runtime_bus.clone()),
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
                        event_bus: runtime_bus.clone(),
                        process_receipt_committed: false,
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
                                event_bus: runtime_bus.clone(),
                                process_receipt_committed: false,
                                metadata: metadata.clone(),
                                child: None,
                                acp_client: None,
                            });
                        }
                    }


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
                        let bus = runtime_bus.clone();
                        tokio::spawn(async move {
                            let _ = connect_and_fill(sessions, starting, bus, id, pending).await;
                        });
                    } else {
                        // Blocking resume propagates failure while retaining
                        // the placeholder/lease until explicit cleanup.
                        connect_and_fill(
                            self.sessions.clone(),
                            self.starting.clone(),
                            runtime_bus.clone(),
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
                let operation_id=Uuid::new_v4();headless_operation=Some(operation_id);
                runtime_bus.emit_checked(ControlEvent::UserMessage {session_id:id,operation_id,text:prompt.clone(),at:Utc::now()})?;
                runtime_bus.ensure_durable()?;
                let (child,proof) = self
                    .grok_cli
                    .spawn_headless(cwd_path, &prompt, &headless)
                    .await?;
                metadata.status = SessionStatus::Running;
                AgentHandle {
                    event_bus: runtime_bus.clone(),
                    process_receipt_committed: false,
                    metadata,
                    child: Some(ProcessHandle::adopt_attested(child,proof, ProcessConfig::default(), vec![])?),
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
        runtime_bus.emit_checked(ControlEvent::SessionStatusChanged {session_id:id,status,at:Utc::now()})?;
        if let Some(process) = self.sessions.get(&id).and_then(|entry| entry.child.clone()) {
            let sessions = self.sessions.clone(); let bus = runtime_bus.clone();
            let cleanup_gate=self.cleanup_updates.lock().map_err(|_|CoreError::Internal("cleanup gate poisoned".into()))?.entry(id).or_insert_with(||Arc::new(tokio::sync::Mutex::new(()))).clone();
            tokio::spawn(async move {
                let outcome = process.wait_outcome().await;
                // Wait for physical settlement first. Only publication shares Stop's gate.
                let _cleanup=cleanup_gate.lock().await;
                let Some(mut entry)=sessions.get_mut(&id) else {return;};
                if entry.event_bus.runtime_id()!=bus.runtime_id() || !entry.child.as_ref().is_some_and(|current|current.same_process(&process)) {return;}
                let status=if matches!(entry.metadata.status,SessionStatus::Cancelling|SessionStatus::Cancelled) {entry.metadata.status}else{outcome_status(&outcome)};
                let mut metadata=entry.metadata.clone();metadata.process_outcome=Some(outcome.clone());metadata.status=status;metadata.last_activity=Utc::now();
                let publication = (|| -> Result<()> {
                    let text=outcome.output.text();
                    if !text.is_empty() {bus.emit_checked(ControlEvent::AgentMessage{session_id:id,text,at:Utc::now()})?;}
                    if let Some(error)=&outcome.error {bus.emit_checked(ControlEvent::Error{session_id:Some(id),message:error.clone(),at:Utc::now()})?;}
                    if let Some(operation_id)=headless_operation {bus.emit_checked(ControlEvent::HostOperationOutcome{session_id:Some(id),operation_id,kind:"submission".into(),target:"conversation".into(),result:if outcome.end==ProcessEnd::Success && outcome.cleanup_complete {"completed"}else{"uncertain"}.into(),at:Utc::now()})?;}
                    bus.emit_checked(ControlEvent::SessionMetadataUpdated{session_id:id,metadata_json:metadata_record_json(&metadata)?,at:Utc::now()})?;
                    bus.emit_checked(ControlEvent::SessionStatusChanged{session_id:id,status,at:Utc::now()})?;Ok(())
                })();
                entry.process_receipt_committed=publication.is_ok();
                entry.metadata.process_outcome=Some(outcome);
                entry.metadata.status=if publication.is_ok(){status}else{SessionStatus::Failed};entry.touch();
            });
        }

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

    pub fn requires_reconnect(&self, id: Uuid) -> bool {
        self.sessions.get(&id).and_then(|entry| entry.acp_client.clone())
            .is_some_and(|client| client.runner_stopped())
    }
    pub async fn wait_headless(&self, id: Uuid) -> Result<ProcessOutcome> {
        let process = self.sessions.get(&id).ok_or(CoreError::SessionNotFound(id))?
            .child.clone().ok_or(CoreError::NotHeadless)?;
        let bus=self.scoped_bus(id)?;let outcome=process.wait_outcome().await;
        tokio::time::timeout(std::time::Duration::from_secs(10),async {
            loop {
                bus.ensure_healthy()?;
                let entry=self.sessions.get(&id).ok_or(CoreError::SessionNotFound(id))?;
                if entry.event_bus.runtime_id()!=bus.runtime_id(){return Err(grok_events::EventError::StaleRuntime.into());}
                if entry.process_receipt_committed && entry.metadata.process_outcome.is_some(){return Ok::<(),CoreError>(());}
                drop(entry);tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        }).await.map_err(|_|CoreError::Internal("completion publication unresolved; retain worker owner".into()))??;
        bus.ensure_healthy()?;
        Ok(outcome)
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

    pub async fn send_user_prompt(&self,id:Uuid,prompt:&str,client_submission_id:Option<Uuid>)->Result<()> {
        self.send_prompt_admitted(id,prompt,false,client_submission_id).await
    }

    pub async fn send_prompt(&self, id: Uuid, prompt: &str) -> Result<()> {
        self.send_prompt_inner(id, prompt, false).await
    }

    /// Auditor/verifier prompts use their role instructions while native Plan
    /// mode and permission gates remain unchanged.
    pub async fn send_review_prompt(&self, id: Uuid, prompt: &str) -> Result<()> {
        self.send_prompt_inner(id, prompt, true).await
    }

    async fn send_prompt_inner(&self,id:Uuid,prompt:&str,review:bool)->Result<()> {
        self.send_prompt_admitted(id,prompt,review,None).await
    }
    async fn send_prompt_admitted(&self, id: Uuid, prompt: &str, review: bool,client_submission_id:Option<Uuid>) -> Result<()> {
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
            let client = entry.acp_client.clone().ok_or(CoreError::NotAcp)?;
            if client.runner_stopped() { return Err(CoreError::InvalidOptions("native runner stopped; explicitly resume this conversation before sending a new prompt".into())); }
            entry.touch();
            client
        };
        let bus=self.scoped_bus(id)?;
        bus.ensure_healthy()?;
        // Host operation identity is prepared here; ACP commits it after serialized admission.
        client.send_recorded_prompt(prompt,review,Uuid::new_v4(),client_submission_id).await?;
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
        if self.starting.contains_key(&id) {
            if let Some(mut entry) = self.sessions.get_mut(&id) {entry.metadata.status = SessionStatus::Cancelling;}
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                while self.starting.contains_key(&id) {tokio::time::sleep(std::time::Duration::from_millis(20)).await;}
            }).await.map_err(|_| CoreError::Internal("native startup cleanup still pending; Stop has not been confirmed, retry cleanup".into()))?;
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

        let mut errors=Vec::new();
        if let Some(client) = acp {
            if let Err(error)=client.cancel_without_status().await {errors.push(error.to_string());}
        }
        if has_child {
            let process = self.sessions.get(&id).and_then(|entry| entry.child.clone());
            if let Some(process) = process {
                let outcome = process.cancel().await;
                if let Some(mut entry) = self.sessions.get_mut(&id) {
                    entry.metadata.process_outcome=Some(outcome.clone());entry.process_receipt_committed=false;entry.touch();
                    let receipt=(||->Result<()> {entry.event_bus.emit_checked(ControlEvent::SessionMetadataUpdated{session_id:id,metadata_json:metadata_record_json(&entry.metadata)?,at:Utc::now()})?;Ok(())})();
                    if let Err(error)=receipt {errors.push(format!("native process receipt unresolved: {error}"));}
                    else {entry.process_receipt_committed=true;}
                }
                if !outcome.cleanup_complete {
                    errors.push(format!("native cleanup unresolved: {}", outcome.error.unwrap_or_default()));
                }
            }
        }

        if !errors.is_empty(){return Err(CoreError::Internal(format!("Stop cleanup or durable evidence unresolved: {}",errors.join("; "))));}
        let published=self.scoped_bus(id)?.emit_checked(ControlEvent::SessionCancelled {session_id:id,at:Utc::now()});
        if let Err(error)=published {errors.push(error.to_string());}
        if !errors.is_empty(){return Err(CoreError::Internal(format!("Stop cleanup or durable evidence unresolved: {}",errors.join("; "))));}
        if let Some(mut entry) = self.sessions.get_mut(&id) {entry.metadata.status=SessionStatus::Cancelled;entry.touch();}
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
        let expected_runtime=self.sessions.get(&id).ok_or(CoreError::SessionNotFound(id))?.event_bus.runtime_id();
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
            let mut updated=entry.metadata.clone();
            updated.approval_mode = applied;
            updated.plan_mode = applied == ApprovalMode::Plan;
            updated.always_approve = applied == ApprovalMode::Yolo;
            if let Err(error)=entry.event_bus.emit_checked(ControlEvent::SessionMetadataUpdated{session_id:id,metadata_json:metadata_record_json(&updated)?,at:Utc::now()}) {
                // The policy operation was already acknowledged. Keep its actual
                // authority visible locally, while the failed receipt remains explicit.
                if entry.acp_client.as_ref().is_some_and(|current|Arc::ptr_eq(current,&client))
                    && entry.event_bus.runtime_id()==expected_runtime {
                    updated.status=SessionStatus::Failed;
                    entry.metadata=updated;
                    entry.touch();
                }
                return Err(error.into());
            }
            entry.metadata=updated;
            entry.touch();
            drop(entry);
            let wanted = match mode {
                ApprovalMode::Plan => "plan",
                ApprovalMode::Auto => "auto",
                ApprovalMode::Yolo => "always_approve",
                ApprovalMode::Ask => "default",
            };
            if let Err(e) = client.set_mode(wanted).await {
                if matches!(&e,grok_acp::AcpError::Protocol(message) if message.starts_with("agent does not advertise a '")) {
                    tracing::warn!(error = %e, ?mode, "agent-side mode unavailable (client gate still enforces it)");
                } else {return Err(e.into());}
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
        client.add_session_allow_rule(pattern).await?;
        Ok(())
    }

    pub async fn set_always_approve(&self, id: Uuid, enabled: bool) -> Result<()> {
        self.set_approval_mode(id, if enabled { ApprovalMode::Yolo } else { ApprovalMode::Ask }).await
    }

    pub async fn pending_approvals(&self,id:Uuid)->Result<Vec<grok_acp::LiveApproval>> {
        let client=self.sessions.get(&id).ok_or(CoreError::SessionNotFound(id))?.acp_client.clone().ok_or(CoreError::NotAcp)?;
        Ok(client.pending_approvals().await?)
    }
    pub async fn respond_approval_for_runtime(&self,id:Uuid,runtime_id:Uuid,host_epoch:u64,request_id:&str,option_id:Option<&str>)->Result<()> {
        let client={let entry=self.sessions.get(&id).ok_or(CoreError::SessionNotFound(id))?;
            if entry.event_bus.runtime_id()!=Some(runtime_id){return Err(grok_events::EventError::StaleRuntime.into());}
            entry.acp_client.clone().ok_or(CoreError::NotAcp)?};
        client.respond_approval_scoped(runtime_id,host_epoch,request_id,option_id).await?;Ok(())
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
        }
        cancelled?;
        let bus=self.scoped_bus(id)?;
        bus.retire_runtime(bus.runtime_id().ok_or_else(||CoreError::Internal("runtime scope missing".into()))?)?;
        self.sessions.remove(&id);
        self.workspace_leases.lock().map_err(|_| CoreError::Internal("workspace admission poisoned".into()))?.remove(&id);
        self.admitted.lock().map_err(|_| CoreError::Internal("admission poisoned".into()))?.remove(&id);
        self.mode_updates.lock().map_err(|_| CoreError::Internal("mode transition gate poisoned".into()))?.remove(&id);
        self.cleanup_updates.lock().map_err(|_| CoreError::Internal("cleanup gate poisoned".into()))?.remove(&id);
        Ok(())
    }

    /// Capture this runner's producer once, before starting asynchronous work.
    pub fn session_event_bus(&self, id:Uuid)->Result<Arc<EventBus>> {
        self.scoped_bus(id)
    }
    pub fn scoped_bus(&self, id:Uuid)->Result<Arc<EventBus>> {
        Ok(self.sessions.get(&id).ok_or(CoreError::SessionNotFound(id))?.event_bus.clone())
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn fence_admission(&self) -> Result<()> {
        let _slots=self.admitted.lock().map_err(|_|CoreError::Internal("admission poisoned".into()))?;
        self.shutting_down.store(true,Ordering::Release);
        Ok(())
    }
    pub async fn shutdown_all(self: &Arc<Self>) -> Result<()> {
        self.shutdown_preserving(HashSet::new()).await
    }
    pub async fn shutdown_preserving(self: &Arc<Self>, retained:HashSet<Uuid>) -> Result<()> {
        // Admission can precede publication of the Starting/headless handle.
        // Cleanup must include those admitted owners, not only visible sessions.
        let ids: Vec<Uuid> = self.admitted.lock().map_err(|_|CoreError::Internal("admission poisoned".into()))?.iter().copied().collect();
        let mut tasks=Vec::new();
        for id in ids {let registry=self.clone();let preserve=retained.contains(&id);tasks.push(tokio::spawn(async move{
            while !registry.sessions.contains_key(&id) {
                if !registry.admitted.lock().map_err(|_|"admission poisoned".to_string())?.contains(&id) {return Ok(());}
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            if let Some(mut entry)=registry.sessions.get_mut(&id) {
                if matches!(entry.metadata.status,SessionStatus::Starting) {entry.metadata.status=SessionStatus::Cancelling;}
            }
            if preserve {registry.cancel_session(id).await} else {registry.remove_session(id).await}.map_err(|e|format!("{id}: {e}"))
        }));}
        let cleanup=tokio::time::timeout(std::time::Duration::from_secs(31),async {
            let mut errors=Vec::new();
            for task in tasks {match task.await {Ok(Ok(()))=>{},Ok(Err(error))=>errors.push(error),Err(error)=>errors.push(error.to_string())}}
            errors
        }).await.map_err(|_|CoreError::Internal("shutdown deadline exceeded; cleanup tasks and session owners remain retained".into()))?;
        if cleanup.is_empty() {Ok(())} else {Err(CoreError::Internal(format!("shutdown cleanup unresolved: {}",cleanup.join("; "))))}
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

    struct RejectProcessReceipt {store:Arc<grok_persistence::Persistence>}
    impl grok_events::EventSink for RejectProcessReceipt {
        fn identity(&self)->grok_events::StoreIdentity {grok_events::EventSink::identity(self.store.as_ref())}
        fn commit(&self,origin:&grok_events::EventOrigin,event:&ControlEvent)->grok_events::Result<grok_events::CommittedEvent> {
            if let ControlEvent::SessionMetadataUpdated{metadata_json,..}=event {
                let record:serde_json::Value=serde_json::from_str(metadata_json).unwrap();
                let snapshot:serde_json::Value=serde_json::from_str(record["metadataJson"].as_str().unwrap()).unwrap();
                if !snapshot["metadata"]["processOutcome"].is_null() {return Err(grok_events::EventError::Sink("generated failure saving observed process receipt".into()));}
            }
            grok_events::EventSink::commit(self.store.as_ref(),origin,event)
        }
    }

    struct RejectModeMetadata {store:Arc<grok_persistence::Persistence>,armed:Arc<std::sync::atomic::AtomicBool>}
    impl grok_events::EventSink for RejectModeMetadata {
        fn identity(&self)->grok_events::StoreIdentity {grok_events::EventSink::identity(self.store.as_ref())}
        fn commit(&self,origin:&grok_events::EventOrigin,event:&ControlEvent)->grok_events::Result<grok_events::CommittedEvent> {
            if self.armed.load(Ordering::Acquire) && matches!(event,ControlEvent::SessionMetadataUpdated{..}) {
                return Err(grok_events::EventError::Sink("generated secondary mode receipt failure".into()));
            }
            grok_events::EventSink::commit(self.store.as_ref(),origin,event)
        }
    }

    #[tokio::test]
    async fn acknowledged_policy_with_failed_snapshot_retains_actual_mode_and_explicit_uncertainty() {
        let cwd=tempfile::tempdir().unwrap();let store=Arc::new(grok_persistence::Persistence::open(cwd.path().join("mode.sqlite")).unwrap());
        let armed=Arc::new(std::sync::atomic::AtomicBool::new(false));let bus=shared_bus();
        bus.install_sink(Arc::new(RejectModeMetadata{store:store.clone(),armed:armed.clone()})).unwrap();
        let reg=SessionRegistry::new(bus.clone(),Arc::new(tokio::sync::RwLock::new(GrokConfig::default())),Arc::new(GrokCli::new("/usr/bin/true")));
        let id=reg.spawn_mock(cwd.path().to_str().unwrap()).await.unwrap();let client=reg.sessions.get(&id).unwrap().acp_client.clone().unwrap();
        assert_eq!(client.approval_mode().await,ApprovalMode::Plan);armed.store(true,Ordering::Release);
        assert!(reg.set_approval_mode(id,ApprovalMode::Yolo).await.is_err());
        let snapshot=reg.get_snapshot(id).unwrap();assert_eq!(snapshot.metadata.approval_mode,ApprovalMode::Yolo);
        assert_eq!(client.approval_mode().await,ApprovalMode::Yolo);assert_eq!(snapshot.metadata.status,SessionStatus::Failed);
        assert!(!bus.health().healthy);assert!(reg.send_prompt(id,"must preserve generated draft").await.is_err());
        let persisted=store.event_snapshot(Some(id),128).unwrap();
        assert!(persisted.operations.iter().any(|operation|operation.kind=="approval_mode_change" && operation.target=="approval:Yolo" && operation.outcome_seq.is_some()));
        let record=store.get_session(id).unwrap();let saved:serde_json::Value=serde_json::from_str(&record.metadata_json).unwrap();
        assert_eq!(saved["metadata"]["approvalMode"],"plan");
    }

    #[cfg(target_os="macos")]
    #[tokio::test]
    async fn actual_success_without_durable_process_receipt_is_not_acknowledged_completed() {
        let cwd=tempfile::tempdir().unwrap();let store=Arc::new(grok_persistence::Persistence::open(cwd.path().join("events.sqlite")).unwrap());
        let bus=shared_bus();bus.install_sink(Arc::new(RejectProcessReceipt{store:store.clone()})).unwrap();
        let config=Arc::new(tokio::sync::RwLock::new(GrokConfig::default()));config.write().await.permissions.deny.clear();
        let reg=SessionRegistry::new(bus.clone(),config,Arc::new(GrokCli::new("/usr/bin/true")));
        let id=reg.spawn_agent(cwd.path().to_str().unwrap(),SpawnOptions{mode:AgentMode::Headless,prompt:Some("generated fixture".into()),plan_mode:false,approval_mode:Some(ApprovalMode::Ask),sandbox_profile:Some("unrestricted".into()),..Default::default()}).await.unwrap();
        assert!(reg.wait_headless(id).await.is_err());assert!(!bus.health().healthy);
        assert_eq!(reg.sessions.get(&id).unwrap().child.as_ref().unwrap().outcome().unwrap().end,ProcessEnd::Success);
        assert!(!reg.sessions.get(&id).unwrap().process_receipt_committed);
        assert_ne!(store.get_session(id).unwrap().status,"completed");
        assert!(reg.cancel_session(id).await.is_err());assert!(reg.is_live(id));
        assert!(reg.sessions.get(&id).unwrap().child.as_ref().unwrap().outcome().unwrap().cleanup_complete);
    }

    #[tokio::test]
    async fn scoped_producer_commits_without_ui_and_old_runtime_cannot_change_resumed_metadata() {
        let cwd=tempfile::tempdir().unwrap();
        let store=Arc::new(grok_persistence::Persistence::open(cwd.path().join("events.sqlite")).unwrap());
        let bus=shared_bus();bus.install_sink(store.clone()).unwrap();
        let reg=SessionRegistry::new(bus.clone(),Arc::new(tokio::sync::RwLock::new(GrokConfig::default())),Arc::new(GrokCli::new(PathBuf::from("/bin/true"))));
        let id=reg.spawn_mock(cwd.path().to_str().unwrap()).await.unwrap();
        let old=reg.session_event_bus(id).unwrap();let old_id=old.runtime_id();
        old.emit_checked(ControlEvent::SessionStatusChanged{session_id:id,status:SessionStatus::Cancelled,at:Utc::now()}).unwrap();
        reg.remove_session(id).await.unwrap();
        reg.spawn_agent_preallocated(id,cwd.path().to_str().unwrap(),SpawnOptions{model:Some("mock".into()),..Default::default()},ConnectOpts::default()).await.unwrap();
        let current=reg.session_event_bus(id).unwrap();assert_ne!(current.runtime_id(),old_id);
        assert!(old.emit_checked(ControlEvent::SessionStatusChanged{session_id:id,status:SessionStatus::Cancelled,at:Utc::now()}).is_err());
        assert!(old.emit_checked(ControlEvent::AgentMessage{session_id:id,text:"retired output".into(),at:Utc::now()}).is_err());
        reg.send_user_prompt(id,"generated durable prompt",Some(Uuid::new_v4())).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        assert_eq!(reg.get_snapshot(id).unwrap().metadata.status,SessionStatus::Idle);
        let transcripts=store.transcript_entries(id).unwrap();
        assert!(transcripts.iter().any(|row|row.role=="user" && row.body=="generated durable prompt"));
        assert!(!transcripts.iter().any(|row|row.body.contains("retired output")));
        assert!(bus.health().healthy && bus.health().durable);
        reg.remove_session(id).await.unwrap();
    }

    #[tokio::test]
    async fn native_headless_admission_rejects_volatile_sink_before_child_spawn() {
        let cwd=tempfile::tempdir().unwrap();let reg=test_registry();
        reg.config.write().await.permissions.deny.clear();
        let result=reg.spawn_agent(cwd.path().to_str().unwrap(),SpawnOptions{mode:AgentMode::Headless,prompt:Some("generated offline fixture".into()),plan_mode:false,approval_mode:Some(ApprovalMode::Ask),sandbox_profile:Some("unrestricted".into()),..Default::default()}).await;
        assert!(matches!(result,Err(CoreError::Events(grok_events::EventError::NotDurable))));
        assert_eq!(reg.session_count(),0);
        assert!(reg.admitted.lock().unwrap().is_empty());
        let result=reg.spawn_agent(cwd.path().to_str().unwrap(),SpawnOptions{model:Some("mock".into()),mode:AgentMode::Headless,prompt:Some("generated offline fixture".into()),plan_mode:false,approval_mode:Some(ApprovalMode::Ask),sandbox_profile:Some("unrestricted".into()),..Default::default()}).await;
        assert!(matches!(result,Err(CoreError::Events(grok_events::EventError::NotDurable))));
        assert_eq!(reg.session_count(),0);
    }

    #[tokio::test]
    async fn status_mirror_recovers_committed_idle_after_broadcast_lag() {
        let bus=Arc::new(EventBus::with_capacity(4));
        let reg=SessionRegistry::new(bus,Arc::new(tokio::sync::RwLock::new(GrokConfig::default())),Arc::new(GrokCli::new("/bin/true")));
        let cwd=tempfile::tempdir().unwrap();let id=reg.spawn_mock(cwd.path().to_str().unwrap()).await.unwrap();
        reg.sessions.get_mut(&id).unwrap().metadata.status=SessionStatus::Running;
        let scope=reg.session_event_bus(id).unwrap();
        scope.emit_checked(ControlEvent::SessionStatusChanged{session_id:id,status:SessionStatus::Idle,at:Utc::now()}).unwrap();
        // Fill the small queue without yielding; the status event is evicted.
        for index in 0..64 {scope.emit_checked(ControlEvent::Raw{session_id:Some(id),payload:serde_json::json!({"fixture":index})}).unwrap();}
        tokio::time::timeout(std::time::Duration::from_secs(1),async {
            while reg.get_snapshot(id).unwrap().metadata.status!=SessionStatus::Idle {tokio::task::yield_now().await;}
        }).await.unwrap();
        reg.remove_session(id).await.unwrap();
    }

    #[cfg(target_os="macos")]
    #[tokio::test]
    async fn healthy_sink_never_commits_cancelled_until_all_owned_cleanup_is_observed() {
        use grok_cli_wrapper::process::{spawn_group,DrainTicket};
        let cwd=tempfile::tempdir().unwrap();let store=Arc::new(grok_persistence::Persistence::open(cwd.path().join("events.sqlite")).unwrap());
        let bus=shared_bus();bus.install_sink(store.clone()).unwrap();let mut committed=bus.subscribe_committed();
        let reg=SessionRegistry::new(bus.clone(),Arc::new(tokio::sync::RwLock::new(GrokConfig::default())),Arc::new(GrokCli::new(PathBuf::from("/bin/true"))));
        let id=reg.spawn_mock(cwd.path().to_str().unwrap()).await.unwrap();
        let mut command=tokio::process::Command::new("/bin/sleep");command.arg("30");
        let(child,proof)=spawn_group(&mut command).unwrap();let(reporter,ticket)=DrainTicket::pair();
        let process=ProcessHandle::adopt_attested(child,proof,ProcessConfig{timeout:None,cleanup_timeout:std::time::Duration::from_millis(100),..Default::default()},vec![ticket]).unwrap();
        reg.sessions.get_mut(&id).unwrap().child=Some(process.clone());
        while committed.try_recv().is_ok(){}
        assert!(reg.cancel_session(id).await.is_err());
        assert!(reg.is_live(id));assert!(!process.outcome().unwrap().cleanup_complete);
        assert_ne!(store.get_session(id).unwrap().status,"cancelled");
        while let Ok(event)=committed.try_recv(){assert!(!matches!(event.event,ControlEvent::SessionCancelled{..}|ControlEvent::SessionStatusChanged{status:SessionStatus::Cancelled,..}));}
        reporter.complete(Ok(()));
        reg.cancel_session(id).await.unwrap();
        assert!(process.outcome().unwrap().cleanup_complete);
        assert_eq!(store.get_session(id).unwrap().status,"cancelled");
        reg.remove_session(id).await.unwrap();
    }

    #[tokio::test]
    async fn checked_label_compare_and_swap_preserves_manual_label_and_retired_runner_identity() {
        let reg=test_registry();let cwd=tempfile::tempdir().unwrap();let id=reg.spawn_mock(cwd.path().to_str().unwrap()).await.unwrap();let old=reg.session_event_bus(id).unwrap().runtime_id().unwrap();
        reg.set_label(id,"manual title").unwrap();
        assert!(reg.set_label_if_current(id,old,Some("mock"),"stale generated title").is_err());
        assert_eq!(reg.get_snapshot(id).unwrap().metadata.label.as_deref(),Some("manual title"));
        reg.set_label_if_current(id,old,Some("manual title"),"reviewed title").unwrap();
        reg.remove_session(id).await.unwrap();reg.spawn_agent_preallocated(id,cwd.path().to_str().unwrap(),SpawnOptions{model:Some("mock".into()),..Default::default()},ConnectOpts::default()).await.unwrap();
        assert!(reg.set_label_for_runtime(id,old,"old runner title").is_err());assert_ne!(reg.get_snapshot(id).unwrap().metadata.label.as_deref(),Some("old runner title"));reg.remove_session(id).await.unwrap();
    }
    struct RejectAdmissionSink {store:Arc<grok_persistence::Persistence>}
    impl grok_events::EventSink for RejectAdmissionSink {
        fn identity(&self)->grok_events::StoreIdentity {grok_events::EventSink::identity(self.store.as_ref())}
        fn commit(&self,_origin:&grok_events::EventOrigin,_event:&ControlEvent)->grok_events::Result<grok_events::CommittedEvent>{Err(grok_events::EventError::Sink("generated pre-spawn commit failure".into()))}
    }
    #[cfg(target_os="macos")]
    #[tokio::test]
    async fn durable_admission_failure_never_launches_generated_marker_child() {
        use std::os::unix::fs::PermissionsExt;
        let cwd=tempfile::tempdir().unwrap();let program=cwd.path().join("worker");std::fs::write(&program,"#!/bin/sh\nprintf effect > \"$0.marker\"\n").unwrap();std::fs::set_permissions(&program,std::fs::Permissions::from_mode(0o700)).unwrap();
        let bus=shared_bus();bus.install_sink(Arc::new(RejectAdmissionSink{store:Arc::new(grok_persistence::Persistence::open(cwd.path().join("events.sqlite")).unwrap())})).unwrap();let config=Arc::new(tokio::sync::RwLock::new(GrokConfig::default()));config.write().await.permissions.deny.clear();
        let reg=SessionRegistry::new(bus.clone(),config,Arc::new(GrokCli::new(&program)));
        assert!(reg.spawn_agent(cwd.path().to_str().unwrap(),SpawnOptions{mode:AgentMode::Headless,prompt:Some("generated fixture".into()),plan_mode:false,approval_mode:Some(ApprovalMode::Ask),sandbox_profile:Some("unrestricted".into()),..Default::default()}).await.is_err());
        assert!(!program.with_file_name("worker.marker").exists());assert_eq!(reg.session_count(),0);assert!(reg.admitted.lock().unwrap().is_empty());assert!(!bus.health().healthy);
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
        let bus=shared_bus();bus.install_sink(Arc::new(grok_persistence::Persistence::open(cwd.path().join("policy-fixture.sqlite")).unwrap())).unwrap();
        let reg = SessionRegistry::new(bus, config.clone(), Arc::new(GrokCli::new(&program)));
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
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn generated_headless_completion_is_observed_and_owner_retained_until_remove() {
        use std::os::unix::fs::PermissionsExt;
        let cwd = tempfile::tempdir().unwrap();
        let script = cwd.path().join("generated-worker");
        std::fs::write(&script,"#!/bin/sh\nsleep 0.08\nprintf 'final λ\n'\nprintf 'diagnostic\n' >&2\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let bus = shared_bus();
        let store=Arc::new(grok_persistence::Persistence::open(cwd.path().join("fixture-events.sqlite")).unwrap());
        bus.install_sink(store.clone()).unwrap();
        let mut events = bus.subscribe();
        let config = Arc::new(tokio::sync::RwLock::new(GrokConfig::default()));
        config.write().await.permissions.deny.clear();
        let reg = SessionRegistry::new(bus,config,Arc::new(GrokCli::new(script)));
        let options = SpawnOptions { mode:AgentMode::Headless,prompt:Some("generated offline fixture".into()),plan_mode:false,approval_mode:Some(ApprovalMode::Ask),sandbox_profile:Some("unrestricted".into()),..Default::default() };
        let id = reg.spawn_agent(cwd.path().to_str().unwrap(),options.clone()).await.unwrap();
        assert_eq!(reg.get_snapshot(id).unwrap().metadata.status,SessionStatus::Running);
        loop {if matches!(events.recv().await.unwrap(),grok_events::ControlEvent::SessionStatusChanged {session_id,status:SessionStatus::Running,..} if session_id==id){break;}}
        let outcome = reg.wait_headless(id).await.unwrap();
        assert_eq!(outcome.end,ProcessEnd::Success);
        assert!(outcome.cleanup_complete && outcome.pipes_complete);
        assert!(outcome.output.stdout.contains("final λ") && outcome.output.stderr.contains("diagnostic"));
        let saved=store.get_session(id).unwrap();let snapshot:serde_json::Value=serde_json::from_str(&saved.metadata_json).unwrap();
        assert_eq!(snapshot["metadata"]["processOutcome"],serde_json::to_value(&outcome).unwrap());
        assert_eq!(saved.status,"completed");
        assert!(reg.spawn_agent(cwd.path().to_str().unwrap(),options.clone()).await.is_err());
        reg.remove_session(id).await.unwrap();
        let id2=reg.spawn_agent(cwd.path().to_str().unwrap(),options).await.unwrap();
        reg.cancel_session(id2).await.unwrap();
        let outcome=reg.wait_headless(id2).await.unwrap();
        assert_eq!(outcome.end,ProcessEnd::Cancelled);
        assert!(outcome.cleanup_complete);
        let saved=store.get_session(id2).unwrap();let snapshot:serde_json::Value=serde_json::from_str(&saved.metadata_json).unwrap();
        assert_eq!(snapshot["metadata"]["processOutcome"],serde_json::to_value(&outcome).unwrap());
        assert_eq!(saved.status,"cancelled");
        assert!(reg.is_live(id2));
        reg.remove_session(id2).await.unwrap();
    }

    #[tokio::test]
    async fn rejected_prompt_does_not_publish_false_running() {
        let reg = test_registry(); let cwd = tempfile::tempdir().unwrap();
        let id = reg.spawn_mock(cwd.path().to_str().unwrap()).await.unwrap();
        assert!(reg.send_prompt(id," ").await.is_err());
        assert_eq!(reg.get_snapshot(id).unwrap().metadata.status,SessionStatus::Idle);
        reg.remove_session(id).await.unwrap();
    }

    #[tokio::test]
    async fn exit_admission_fence_prevents_spawn_and_resume_before_cleanup() {
        let reg=test_registry();let cwd=tempfile::tempdir().unwrap();
        let id=reg.spawn_mock(cwd.path().to_str().unwrap()).await.unwrap();
        reg.fence_admission().unwrap();
        assert!(reg.spawn_mock(cwd.path().to_str().unwrap()).await.is_err());
        reg.shutdown_all().await.unwrap();
        assert!(!reg.is_live(id));
        assert!(reg.spawn_mock(cwd.path().to_str().unwrap()).await.is_err());
    }

    #[tokio::test]
    async fn exit_cleanup_waits_for_admitted_owner_not_yet_published() {
        let reg=test_registry();let id=Uuid::new_v4();
        reg.admitted.lock().unwrap().insert(id);
        reg.fence_admission().unwrap();
        let cleanup=reg.clone();let stop=tokio::spawn(async move{cleanup.shutdown_all().await});
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        assert!(!stop.is_finished());
        reg.admitted.lock().unwrap().remove(&id);
        stop.await.unwrap().unwrap();
    }

}
