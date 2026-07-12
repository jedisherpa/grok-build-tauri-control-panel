//! High-level ACP client: spawn, initialize, auth, session, prompt, event loop.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};
use uuid::Uuid;

use grok_events::{
    ControlEvent, EventBus, PermissionOptionInfo, PlanStep, PlanUpdateEvent, SessionStatus,
    ToolCallEvent, ToolCallStatus,
};

use crate::error::{AcpError, Result};
use crate::messages::{
    id_key, AuthenticateParams, ClientCapabilities, ClientInfo, FsCapabilities,
    IncomingAgentRequest, InitializeParams, JsonRpcNotification, PromptContent,
    SessionPromptParams,
};
use crate::terminals::TerminalRegistry;
use crate::transport::NdjsonTransport;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnOptions {
    pub model: Option<String>,
    pub rules: Option<Value>,
    pub mcp_servers: Vec<Value>,
    pub plan_mode: bool,
    pub always_approve: bool,
    pub sandbox_profile: Option<String>,
    pub extra_env: Vec<(String, String)>,
}

impl Default for SpawnOptions {
    fn default() -> Self {
        Self {
            model: None,
            rules: None,
            mcp_servers: Vec::new(),
            plan_mode: true,
            always_approve: false,
            sandbox_profile: Some("workspace".into()),
            extra_env: Vec::new(),
        }
    }
}

/// How much agent mind we recovered after connect / resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BrainMode {
    /// Agent reloaded its own session (`session/load` or `session/resume`).
    FullBrain,
    /// New ACP session; we will inject SQLite transcript as context.
    HistoryOnly,
    /// Brand-new session, no prior context.
    #[default]
    Fresh,
}

impl BrainMode {
    pub fn as_str(self) -> &'static str {
        match self {
            BrainMode::FullBrain => "full_brain",
            BrainMode::HistoryOnly => "history_only",
            BrainMode::Fresh => "fresh",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BrainMode::FullBrain => "full brain",
            BrainMode::HistoryOnly => "history-only",
            BrainMode::Fresh => "fresh",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ConnectOpts {
    /// Prior ACP session id from a previous process (try session/load).
    pub resume_acp_session_id: Option<String>,
    /// SQLite transcript summary to inject if load/resume fails.
    pub transcript_context: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AcpClientConfig {
    /// Program to exec (agent binary or npx for adapter packages).
    pub program: PathBuf,
    /// Args producing an ACP stdio server (e.g. ["agent","stdio"], ["acp"], npx pkg).
    pub args: Vec<String>,
    /// Backend-specific env applied to the child (API keys, base URLs).
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
    pub client_name: String,
    pub client_version: String,
    /// Timeout for short control RPCs (initialize, auth, session/new, cancel).
    pub request_timeout: Duration,
    /// Tighter timeout for the startup handshake (initialize/authenticate) —
    /// a healthy agent answers these in ms; minutes-long hangs mean it's dead.
    pub startup_timeout: Duration,
    /// Max wait for a full agent turn on session/prompt (long coding jobs).
    pub prompt_timeout: Duration,
    /// Ordered auth-method preference matched against agent-advertised methods.
    pub auth_preference: Vec<String>,
    /// Skip authenticate entirely when the agent advertises no auth methods
    /// (adapters riding an already-logged-in CLI).
    pub skip_auth_when_unadvertised: bool,
    /// Short label for logs/errors ("grok", "claude", "codex").
    pub backend_label: String,
}

impl AcpClientConfig {
    /// Grok defaults (compat constructor; also used by tests).
    pub fn new(grok_path: impl Into<PathBuf>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            program: grok_path.into(),
            args: vec!["agent".into(), "stdio".into()],
            env: Vec::new(),
            cwd: cwd.into(),
            client_name: "BombCode".into(),
            client_version: env!("CARGO_PKG_VERSION").into(),
            request_timeout: Duration::from_secs(120),
            startup_timeout: Duration::from_secs(30),
            // Long agent turns stream via notifications; still cap runaway jobs.
            prompt_timeout: Duration::from_secs(60 * 60 * 2), // 2 hours
            // Grok Build advertises cached_token + grok.com (not xai.api_key).
            auth_preference: vec![
                "cached_token".into(),
                "grok.com".into(),
                "xai.api_key".into(),
            ],
            skip_auth_when_unadvertised: false,
            backend_label: "grok".into(),
        }
    }
}

/// A `session/request_permission` we have not yet answered — awaiting the user.
#[derive(Debug)]
struct PendingPermission {
    /// Original wire id; permission responses are JSON-RPC responses to it.
    rpc_id: Value,
    options: Vec<PermissionOptionInfo>,
}

pub struct AcpClient {
    config: AcpClientConfig,
    child: Mutex<Option<Child>>,
    transport: RwLock<Option<Arc<NdjsonTransport>>>,
    session_id: RwLock<Option<String>>,
    agent_capabilities: RwLock<Option<Value>>,
    auth_methods: RwLock<Vec<String>>,
    event_bus: Option<Arc<EventBus>>,
    control_session_id: Uuid,
    notification_rx: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<JsonRpcNotification>>>,
    agent_request_rx: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<IncomingAgentRequest>>>,
    /// When true, auto-allow tool permission requests (yolo). Atomic so the
    /// UI toggle can flip it mid-session.
    always_approve: std::sync::atomic::AtomicBool,
    /// Permission requests parked until the user answers via respond_approval.
    pending_permissions: Mutex<HashMap<String, PendingPermission>>,
    /// Set during deliberate shutdown so process death isn't reported as failure.
    shutting_down: std::sync::atomic::AtomicBool,
    brain_mode: RwLock<BrainMode>,
    /// Injected once on first prompt when brain is history-only.
    pending_context: Mutex<Option<String>>,
    load_session_supported: RwLock<bool>,
    resume_session_supported: RwLock<bool>,
    /// Mode ids the agent advertised in the session/new//load result.
    available_modes: RwLock<Vec<String>>,
    current_mode: RwLock<Option<String>>,
    /// Host-side terminals for ACP terminal/* (required for run_terminal_command).
    terminals: TerminalRegistry,
}

impl AcpClient {
    pub async fn connect(
        config: AcpClientConfig,
        opts: &SpawnOptions,
        event_bus: Option<Arc<EventBus>>,
        control_session_id: Uuid,
    ) -> Result<Arc<Self>> {
        Self::connect_with(config, opts, event_bus, control_session_id, ConnectOpts::default())
            .await
    }

    pub async fn connect_with(
        config: AcpClientConfig,
        opts: &SpawnOptions,
        event_bus: Option<Arc<EventBus>>,
        control_session_id: Uuid,
        connect_opts: ConnectOpts,
    ) -> Result<Arc<Self>> {
        if !config.cwd.is_absolute() {
            return Err(AcpError::Spawn("cwd must be absolute".into()));
        }
        if !config.program.exists() {
            return Err(AcpError::Spawn(format!(
                "{} agent binary not found: {}",
                config.backend_label,
                config.program.display()
            )));
        }

        let mut cmd = Command::new(&config.program);
        cmd.args(&config.args)
            .current_dir(&config.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // GUI apps need an explicit PATH so grok can find tools/npx/git.
        // Prefer full inheritance; still force PATH/HOME for Finder launches.
        cmd.env("PATH", std::env::var("PATH").unwrap_or_else(|_| {
            "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin".into()
        }));
        if let Ok(home) = std::env::var("HOME") {
            cmd.env("HOME", home);
        }
        for (k, v) in &config.env {
            cmd.env(k, v);
        }
        for (k, v) in &opts.extra_env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|e| {
            AcpError::Spawn(format!(
                "failed to spawn {} ACP agent ({}): {e}",
                config.backend_label,
                config.program.display()
            ))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AcpError::Spawn("missing stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AcpError::Spawn("missing stdout".into()))?;
        let stderr = child.stderr.take();

        let (notif_tx, notif_rx) = tokio::sync::mpsc::unbounded_channel();
        let (agent_req_tx, agent_req_rx) = tokio::sync::mpsc::unbounded_channel();
        let transport = NdjsonTransport::new(stdin, stdout, notif_tx, agent_req_tx);

        // Mirror agent stderr into the control bus (center column / terminal view).
        if let Some(stderr) = stderr {
            let bus = event_bus.clone();
            let sid = control_session_id;
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let line = line.trim_end().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    if let Some(bus) = &bus {
                        bus.emit(ControlEvent::Raw {
                            session_id: Some(sid),
                            payload: json!({
                                "channel": "term",
                                "stream": "stderr",
                                "line": line,
                            }),
                        });
                    }
                }
            });
        }

        let pending_context = connect_opts
            .transcript_context
            .filter(|s| !s.trim().is_empty());

        let default_cwd = config.cwd.clone();
        let client = Arc::new(Self {
            config,
            child: Mutex::new(Some(child)),
            transport: RwLock::new(Some(transport)),
            session_id: RwLock::new(None),
            agent_capabilities: RwLock::new(None),
            auth_methods: RwLock::new(Vec::new()),
            event_bus,
            control_session_id,
            notification_rx: Mutex::new(Some(notif_rx)),
            agent_request_rx: Mutex::new(Some(agent_req_rx)),
            always_approve: std::sync::atomic::AtomicBool::new(opts.always_approve),
            pending_permissions: Mutex::new(HashMap::new()),
            shutting_down: std::sync::atomic::AtomicBool::new(false),
            brain_mode: RwLock::new(BrainMode::Fresh),
            pending_context: Mutex::new(pending_context),
            load_session_supported: RwLock::new(false),
            resume_session_supported: RwLock::new(false),
            available_modes: RwLock::new(Vec::new()),
            current_mode: RwLock::new(None),
            terminals: TerminalRegistry::new(default_cwd),
        });

        client.initialize().await?;
        client.authenticate().await?;
        client
            .open_session(opts, connect_opts.resume_acp_session_id.as_deref())
            .await?;

        // Background event loop for notifications
        let loop_client = client.clone();
        let death_client = client.clone();
        tokio::spawn(async move {
            if let Err(e) = loop_client.run_event_loop().await {
                warn!(error = %e, "ACP event loop terminated");
                death_client.report_process_death().await;
            }
        });

        // Answer agent→client requests (fs + permissions). Without this the turn hangs.
        let req_client = client.clone();
        tokio::spawn(async move {
            if let Err(e) = req_client.run_agent_request_loop().await {
                warn!(error = %e, "ACP agent-request loop terminated");
            }
        });

        Ok(client)
    }

    /// Mock-friendly constructor for tests without a real process.
    pub fn mock_for_tests(session_id: &str, event_bus: Option<Arc<EventBus>>) -> Arc<Self> {
        let config = AcpClientConfig::new("/bin/true", "/tmp");
        Arc::new(Self {
            config,
            child: Mutex::new(None),
            transport: RwLock::new(None),
            session_id: RwLock::new(Some(session_id.to_string())),
            agent_capabilities: RwLock::new(None),
            auth_methods: RwLock::new(Vec::new()),
            event_bus,
            control_session_id: Uuid::new_v4(),
            notification_rx: Mutex::new(None),
            agent_request_rx: Mutex::new(None),
            always_approve: std::sync::atomic::AtomicBool::new(false),
            pending_permissions: Mutex::new(HashMap::new()),
            shutting_down: std::sync::atomic::AtomicBool::new(false),
            brain_mode: RwLock::new(BrainMode::Fresh),
            pending_context: Mutex::new(None),
            load_session_supported: RwLock::new(false),
            resume_session_supported: RwLock::new(false),
            available_modes: RwLock::new(Vec::new()),
            current_mode: RwLock::new(None),
            terminals: TerminalRegistry::new(PathBuf::from("/tmp")),
        })
    }

    pub async fn brain_mode(&self) -> BrainMode {
        *self.brain_mode.read().await
    }

    async fn transport(&self) -> Result<Arc<NdjsonTransport>> {
        self.transport
            .read()
            .await
            .clone()
            .ok_or(AcpError::SessionNotReady)
    }

    async fn request_timeout(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let transport = self.transport().await?;
        transport
            .request_with_timeout(method, params, self.config.request_timeout)
            .await
    }

    /// Startup-handshake RPC with the tighter startup timeout.
    async fn request_startup(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let transport = self.transport().await?;
        transport
            .request_with_timeout(method, params, self.config.startup_timeout)
            .await
    }

    async fn initialize(&self) -> Result<()> {
        let params = InitializeParams::new(
            ClientInfo {
                name: self.config.client_name.clone(),
                version: self.config.client_version.clone(),
            },
            ClientCapabilities {
                fs: FsCapabilities {
                    read_text_file: true,
                    write_text_file: true,
                },
                terminal: true,
            },
        );
        let result = self
            .request_startup("initialize", Some(serde_json::to_value(params)?))
            .await?;
        let caps = result.get("agentCapabilities").cloned();
        *self.agent_capabilities.write().await = caps.clone();

        // loadSession: true  OR sessionCapabilities.load / resume
        let load = caps
            .as_ref()
            .and_then(|c| c.get("loadSession"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || caps
                .as_ref()
                .and_then(|c| c.pointer("/sessionCapabilities/load"))
                .is_some();
        let resume = caps
            .as_ref()
            .and_then(|c| c.pointer("/sessionCapabilities/resume"))
            .is_some()
            || caps
                .as_ref()
                .and_then(|c| c.get("resumeSession"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        *self.load_session_supported.write().await = load;
        *self.resume_session_supported.write().await = resume;
        info!(load_session = load, resume_session = resume, "ACP agent session capabilities");

        // Cache advertised auth methods (e.g. cached_token, grok.com).
        let methods = result
            .get("authMethods")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("id").and_then(|id| id.as_str()).map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        info!(?methods, "ACP initialize complete");
        *self.auth_methods.write().await = methods;
        Ok(())
    }

    fn pick_auth_method(&self, advertised: &[String]) -> String {
        // Prefer cached CLI login, then first advertised method.
        let preferred = &self.config.auth_preference;
        let fallback = || {
            preferred
                .first()
                .cloned()
                .unwrap_or_else(|| "cached_token".into())
        };
        if advertised.is_empty() {
            return fallback();
        }
        for p in preferred {
            if advertised.iter().any(|m| m == p) {
                return p.clone();
            }
        }
        advertised.first().cloned().unwrap_or_else(fallback)
    }

    async fn authenticate(&self) -> Result<()> {
        let advertised = self.auth_methods.read().await.clone();
        if advertised.is_empty() && self.config.skip_auth_when_unadvertised {
            info!(
                backend = %self.config.backend_label,
                "agent advertises no auth methods; skipping authenticate (CLI login assumed)"
            );
            return Ok(());
        }
        let method_id = self.pick_auth_method(&advertised);
        info!(%method_id, "ACP authenticate");

        let params = AuthenticateParams {
            method_id: method_id.clone(),
            meta: Some(json!({ "headless": true })),
        };
        match self
            .request_startup("authenticate", Some(serde_json::to_value(params)?))
            .await
        {
            Ok(_) => {
                info!(%method_id, "ACP authenticate complete");
                Ok(())
            }
            Err(AcpError::Rpc { code, message }) => {
                // Retry alternate advertised methods once.
                for alt in &advertised {
                    if alt == &method_id {
                        continue;
                    }
                    let params = AuthenticateParams {
                        method_id: alt.clone(),
                        meta: Some(json!({ "headless": true })),
                    };
                    match self
                        .request_startup("authenticate", Some(serde_json::to_value(params)?))
                        .await
                    {
                        Ok(_) => {
                            info!(method_id = %alt, "ACP authenticate complete (fallback)");
                            return Ok(());
                        }
                        Err(AcpError::Rpc { code: c, message: m }) => {
                            warn!(code = c, %m, method = %alt, "auth fallback failed");
                        }
                        Err(e) => return Err(e),
                    }
                }
                warn!(code, %message, "authenticate returned RPC error; continuing");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Open a session: try load → resume → new, set brain_mode accordingly.
    async fn open_session(&self, opts: &SpawnOptions, prior_sid: Option<&str>) -> Result<()> {
        let model = opts.model.clone().filter(|m| {
            let t = m.trim();
            !t.is_empty() && !t.eq_ignore_ascii_case("default") && !t.eq_ignore_ascii_case("mock")
        });

        if let Some(prior) = prior_sid.map(str::trim).filter(|s| !s.is_empty()) {
            let load_ok = *self.load_session_supported.read().await;
            let resume_ok = *self.resume_session_supported.read().await;

            if load_ok {
                match self.session_load(prior, opts, model.as_deref()).await {
                    Ok(sid) => {
                        *self.session_id.write().await = Some(sid.clone());
                        *self.brain_mode.write().await = BrainMode::FullBrain;
                        // Full brain — don't inject transcript context.
                        *self.pending_context.lock().await = None;
                        info!(%sid, prior, "ACP session/load complete (full brain)");
                        self.apply_mode_after_session(opts).await;
                        if let Some(bus) = &self.event_bus {
                            bus.emit_status(self.control_session_id, SessionStatus::Idle)
                                .await;
                            bus.emit(ControlEvent::AgentMessage {
                                session_id: self.control_session_id,
                                text: "🧠 full brain — agent reloaded prior ACP session".into(),
                                at: Utc::now(),
                            });
                        }
                        return Ok(());
                    }
                    Err(e) => {
                        warn!(error = %e, prior, "session/load failed; trying resume/new");
                    }
                }
            }

            if resume_ok {
                match self.session_resume(prior, opts, model.as_deref()).await {
                    Ok(sid) => {
                        *self.session_id.write().await = Some(sid.clone());
                        *self.brain_mode.write().await = BrainMode::FullBrain;
                        *self.pending_context.lock().await = None;
                        info!(%sid, prior, "ACP session/resume complete (full brain)");
                        self.apply_mode_after_session(opts).await;
                        if let Some(bus) = &self.event_bus {
                            bus.emit_status(self.control_session_id, SessionStatus::Idle)
                                .await;
                            bus.emit(ControlEvent::AgentMessage {
                                session_id: self.control_session_id,
                                text: "🧠 full brain — agent resumed prior ACP session".into(),
                                at: Utc::now(),
                            });
                        }
                        return Ok(());
                    }
                    Err(e) => {
                        warn!(error = %e, prior, "session/resume failed; falling back to session/new");
                    }
                }
            } else if !load_ok {
                info!(prior, "agent does not advertise loadSession/resume — history-only resume");
            }
        }

        // Fresh process session — history-only if we have transcript context pending.
        self.session_new(opts).await?;
        let has_ctx = self
            .pending_context
            .lock()
            .await
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let mode = if has_ctx || prior_sid.is_some() {
            BrainMode::HistoryOnly
        } else {
            BrainMode::Fresh
        };
        *self.brain_mode.write().await = mode;
        if mode == BrainMode::HistoryOnly {
            if let Some(bus) = &self.event_bus {
                bus.emit(ControlEvent::AgentMessage {
                    session_id: self.control_session_id,
                    text: "📜 history-only — agent is new; prior chat will be injected as context"
                        .into(),
                    at: Utc::now(),
                });
            }
        }
        Ok(())
    }

    async fn session_load(
        &self,
        session_id: &str,
        opts: &SpawnOptions,
        model: Option<&str>,
    ) -> Result<String> {
        let mut params = json!({
            "sessionId": session_id,
            "cwd": self.config.cwd.display().to_string(),
            "mcpServers": opts.mcp_servers,
        });
        if let Some(m) = model {
            params["model"] = json!(m);
        }
        let result = match self
            .request_timeout("session/load", Some(params.clone()))
            .await
        {
            Ok(r) => r,
            Err(AcpError::Rpc { code, message }) if !opts.mcp_servers.is_empty() => {
                warn!(code, %message, "session/load with MCP failed; retrying bare");
                self.emit_mcp_dropped(&opts.mcp_servers, &message);
                let mut bare = json!({
                    "sessionId": session_id,
                    "cwd": self.config.cwd.display().to_string(),
                    "mcpServers": [],
                });
                if let Some(m) = model {
                    bare["model"] = json!(m);
                }
                self.request_timeout("session/load", Some(bare)).await?
            }
            Err(e) => return Err(e),
        };
        self.capture_modes(&result).await;
        Ok(result
            .get("sessionId")
            .or_else(|| result.get("session_id"))
            .and_then(|v| v.as_str())
            .unwrap_or(session_id)
            .to_string())
    }

    async fn session_resume(
        &self,
        session_id: &str,
        opts: &SpawnOptions,
        model: Option<&str>,
    ) -> Result<String> {
        let mut params = json!({
            "sessionId": session_id,
            "cwd": self.config.cwd.display().to_string(),
            "mcpServers": opts.mcp_servers,
        });
        if let Some(m) = model {
            params["model"] = json!(m);
        }
        let result = self
            .request_timeout("session/resume", Some(params))
            .await?;
        self.capture_modes(&result).await;
        Ok(result
            .get("sessionId")
            .or_else(|| result.get("session_id"))
            .and_then(|v| v.as_str())
            .unwrap_or(session_id)
            .to_string())
    }

    async fn session_new(&self, opts: &SpawnOptions) -> Result<()> {
        // Start with minimal valid params; Grok rejects unknown fields/values.
        let model = opts.model.clone().filter(|m| {
            let t = m.trim();
            !t.is_empty() && !t.eq_ignore_ascii_case("default") && !t.eq_ignore_ascii_case("mock")
        });

        // First attempt: cwd + mcpServers only (most compatible).
        let mut params = json!({
            "cwd": self.config.cwd.display().to_string(),
            "mcpServers": opts.mcp_servers,
        });
        if let Some(ref m) = model {
            params["model"] = json!(m);
        }

        let result = match self
            .request_timeout("session/new", Some(params.clone()))
            .await
        {
            Ok(r) => r,
            Err(AcpError::Rpc { code, message }) if !opts.mcp_servers.is_empty() => {
                // Retry without MCP if attach payload was invalid.
                warn!(code, %message, "session/new with MCP failed; retrying without MCP");
                self.emit_mcp_dropped(&opts.mcp_servers, &message);
                let mut bare = json!({
                    "cwd": self.config.cwd.display().to_string(),
                    "mcpServers": [],
                });
                if let Some(ref m) = model {
                    bare["model"] = json!(m);
                }
                self.request_timeout("session/new", Some(bare)).await?
            }
            Err(e) => return Err(e),
        };

        let sid = result
            .get("sessionId")
            .or_else(|| result.get("session_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        self.capture_modes(&result).await;
        *self.session_id.write().await = Some(sid.clone());
        info!(%sid, "ACP session/new complete");

        self.apply_mode_after_session(opts).await;

        if let Some(bus) = &self.event_bus {
            bus.emit_status(self.control_session_id, SessionStatus::Idle)
                .await;
        }
        Ok(())
    }

    /// Record `modes.availableModes` / `modes.currentModeId` from a
    /// session/new//load/resume result.
    async fn capture_modes(&self, result: &Value) {
        let modes = result.get("modes");
        let available: Vec<String> = modes
            .and_then(|m| m.get("availableModes"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| {
                        m.get("id")
                            .or_else(|| m.get("modeId"))
                            .and_then(|v| v.as_str())
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let current = modes
            .and_then(|m| m.get("currentModeId").or_else(|| m.get("currentMode")))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if !available.is_empty() {
            info!(?available, ?current, "ACP agent session modes");
            *self.available_modes.write().await = available;
        }
        if current.is_some() {
            *self.current_mode.write().await = current;
        }
    }

    /// Find the advertised mode id matching an intent ("plan", "yolo", "default").
    async fn resolve_mode_id(&self, wanted: &str) -> Option<String> {
        let advertised = self.available_modes.read().await.clone();
        if advertised.is_empty() {
            return Some(wanted.to_string());
        }
        if let Some(exact) = advertised.iter().find(|m| m.eq_ignore_ascii_case(wanted)) {
            return Some(exact.clone());
        }
        // Intent aliases: different agents name the same modes differently.
        let fallback = [wanted];
        let candidates: &[&str] = match wanted {
            "plan" => &["plan", "planning"],
            "always_approve" | "yolo" => &[
                "always_approve",
                "alwaysallow",
                "always_allow",
                "bypasspermissions",
                "yolo",
                "acceptedits",
            ],
            "default" => &["default", "normal", "ask", "code"],
            _ => &fallback,
        };
        for c in candidates {
            if let Some(hit) = advertised
                .iter()
                .find(|m| m.to_lowercase().replace(['-', '_'], "") == c.replace(['-', '_'], ""))
            {
                return Some(hit.clone());
            }
        }
        None
    }

    async fn apply_mode_after_session(&self, opts: &SpawnOptions) {
        let wanted = if opts.always_approve {
            "always_approve"
        } else if opts.plan_mode {
            "plan"
        } else {
            return;
        };
        match self.set_mode(wanted).await {
            Ok(()) => {
                if let Some(bus) = &self.event_bus {
                    let applied = self.current_mode.read().await.clone();
                    Self::emit_term(
                        bus,
                        self.control_session_id,
                        format!("mode → {}", applied.as_deref().unwrap_or(wanted)),
                    );
                }
            }
            Err(e) => {
                warn!(error = %e, %wanted, "failed to set session mode");
                if let Some(bus) = &self.event_bus {
                    bus.emit_error(
                        Some(self.control_session_id),
                        format!("could not enable {wanted} mode: {e}"),
                    );
                }
            }
        }
    }

    pub async fn session_id(&self) -> Option<String> {
        self.session_id.read().await.clone()
    }

    pub async fn send_prompt(&self, prompt: &str) -> Result<()> {
        let sid = self
            .session_id
            .read()
            .await
            .clone()
            .ok_or(AcpError::SessionNotReady)?;

        if prompt.trim().is_empty() {
            return Err(AcpError::Protocol("empty prompt".into()));
        }

        // History-only: prepend transcript pack once.
        let mut text = prompt.to_string();
        if let Some(ctx) = self.pending_context.lock().await.take() {
            text = format!(
                "[Bomb Code session recovery — history-only mode]\n\
                 The previous ACP process died. Below is the durable transcript from this thread.\n\
                 Continue coherently; do not re-ask for info already covered.\n\n\
                 --- prior transcript ---\n{ctx}\n--- end prior transcript ---\n\n\
                 User message:\n{prompt}"
            );
            info!(
                ctx_chars = ctx.len(),
                "injected transcript context (history-only brain)"
            );
            if let Some(bus) = &self.event_bus {
                bus.emit(ControlEvent::AgentMessage {
                    session_id: self.control_session_id,
                    text: format!(
                        "📜 injected {} chars of prior transcript into this prompt",
                        ctx.len()
                    ),
                    at: Utc::now(),
                });
            }
        }

        if let Some(bus) = &self.event_bus {
            bus.emit_status(self.control_session_id, SessionStatus::Running)
                .await;
        }

        // Mock clients: no transport — accept and return.
        if self.transport.read().await.is_none() {
            if let Some(bus) = &self.event_bus {
                bus.emit(ControlEvent::AgentMessage {
                    session_id: self.control_session_id,
                    text: format!(
                        "[mock] Got it — working on your prompt ({} chars).",
                        prompt.len()
                    ),
                    at: Utc::now(),
                });
                bus.emit_status(self.control_session_id, SessionStatus::Idle)
                    .await;
            }
            return Ok(());
        }

        let params = SessionPromptParams {
            session_id: sid,
            prompt: vec![PromptContent {
                kind: "text".into(),
                text,
            }],
        };
        let params_val = serde_json::to_value(params)?;
        let transport = self.transport().await?;

        if let Some(bus) = &self.event_bus {
            bus.emit(ControlEvent::Raw {
                session_id: Some(self.control_session_id),
                payload: json!({
                    "channel": "term",
                    "stream": "acp",
                    "line": format!("→ session/prompt ({} chars) — waiting for stream…", prompt.len()),
                }),
            });
        }

        // Fire the RPC immediately; do not block the UI on the full agent turn.
        // Grok streams work via notifications while session/prompt stays open.
        let rx = transport
            .send_request("session/prompt", Some(params_val))
            .await?;

        let bus = self.event_bus.clone();
        let control_id = self.control_session_id;
        let prompt_timeout = self.config.prompt_timeout;

        tokio::spawn(async move {
            match tokio::time::timeout(prompt_timeout, rx).await {
                Ok(Ok(resp)) => match NdjsonTransport::unwrap_response(resp) {
                    Ok(result) => {
                        info!("session/prompt completed");
                        let stop = result
                            .get("stopReason")
                            .or_else(|| result.get("stop_reason"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("end_turn");
                        if let Some(bus) = &bus {
                            bus.emit(ControlEvent::Raw {
                                session_id: Some(control_id),
                                payload: json!({
                                    "channel": "term",
                                    "stream": "acp",
                                    "line": format!("← session/prompt complete · stopReason={stop}"),
                                }),
                            });
                            bus.emit_status(control_id, SessionStatus::Idle).await;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "session/prompt RPC error");
                        if let Some(bus) = bus {
                            bus.emit_error(Some(control_id), format!("acp error: {e}"));
                            bus.emit_status(control_id, SessionStatus::Failed).await;
                        }
                    }
                },
                Ok(Err(_)) => {
                    warn!("session/prompt response channel closed");
                    if let Some(bus) = bus {
                        bus.emit_error(
                            Some(control_id),
                            "acp error: prompt response channel closed",
                        );
                        bus.emit_status(control_id, SessionStatus::Failed).await;
                    }
                }
                Err(_) => {
                    warn!(
                        timeout_secs = prompt_timeout.as_secs(),
                        "session/prompt still open after timeout; continuing via stream"
                    );
                    if let Some(bus) = &bus {
                        bus.emit(ControlEvent::Raw {
                            session_id: Some(control_id),
                            payload: json!({
                                "channel": "term",
                                "stream": "acp",
                                "line": format!(
                                    "… session/prompt still open after {} min (stream may continue)",
                                    prompt_timeout.as_secs() / 60
                                ),
                            }),
                        });
                        // Don't leave the thread pinned on "running" forever.
                        bus.emit_status(control_id, SessionStatus::Idle).await;
                    }
                }
            }
        });

        // Return as soon as the request is on the wire.
        Ok(())
    }

    pub async fn cancel(&self) -> Result<()> {
        // Cancelled turns must resolve pending permission requests (ACP spec).
        self.drain_pending_permissions().await;
        // Mock / offline clients have no transport — treat cancel as local status update.
        let has_transport = self.transport.read().await.is_some();
        if has_transport {
            if let Some(sid) = self.session_id.read().await.clone() {
                let params = json!({ "sessionId": sid });
                match self
                    .request_timeout("session/cancel", Some(params))
                    .await
                {
                    Ok(_) | Err(AcpError::Rpc { .. }) => {}
                    Err(e) => return Err(e),
                }
            }
        }
        if let Some(bus) = &self.event_bus {
            bus.emit_status(self.control_session_id, SessionStatus::Cancelled)
                .await;
        }
        Ok(())
    }

    /// Flip client-side yolo gating mid-session (UI toggle).
    pub fn set_always_approve(&self, enabled: bool) {
        self.always_approve
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    pub async fn set_mode(&self, mode: &str) -> Result<()> {
        if self.transport.read().await.is_none() {
            debug!(%mode, "set_mode (mock/local)");
            *self.current_mode.write().await = Some(mode.to_string());
            return Ok(());
        }
        let sid = self
            .session_id
            .read()
            .await
            .clone()
            .ok_or(AcpError::SessionNotReady)?;
        let mode_id = self.resolve_mode_id(mode).await.ok_or_else(|| {
            AcpError::Protocol(format!(
                "agent does not advertise a '{mode}' mode (available: {})",
                self.available_modes
                    .try_read()
                    .map(|m| m.join(", "))
                    .unwrap_or_default()
            ))
        })?;
        // ACP spec: session/set_mode takes { sessionId, modeId }.
        let params = json!({
            "sessionId": sid,
            "modeId": mode_id,
        });
        let result = match self
            .request_timeout("session/set_mode", Some(params.clone()))
            .await
        {
            Ok(r) => Ok(r),
            // Older agents: camelCase method name.
            Err(AcpError::Rpc { .. }) => {
                self.request_timeout("session/setMode", Some(params)).await
            }
            Err(e) => Err(e),
        }?;
        let _ = result;
        *self.current_mode.write().await = Some(mode_id);
        Ok(())
    }

    /// Answer a parked `session/request_permission`. `option_id: None` = cancel.
    ///
    /// The `HashMap::remove` is the duplicate-response guard: a second call for
    /// the same request errors without touching the wire.
    pub async fn respond_approval(&self, request_id: &str, option_id: Option<&str>) -> Result<()> {
        let pending = self
            .pending_permissions
            .lock()
            .await
            .remove(request_id)
            .ok_or_else(|| {
                AcpError::Protocol(format!("no pending permission request: {request_id}"))
            })?;

        let outcome = match option_id {
            Some(oid) => {
                if !pending.options.is_empty() && !pending.options.iter().any(|o| o.id == oid) {
                    // Put it back so a corrected retry can still answer.
                    let valid: Vec<&str> = pending.options.iter().map(|o| o.id.as_str()).collect();
                    let msg = format!(
                        "unknown option '{oid}' for permission request {request_id} (valid: {})",
                        valid.join(", ")
                    );
                    self.pending_permissions
                        .lock()
                        .await
                        .insert(request_id.to_string(), pending);
                    return Err(AcpError::Protocol(msg));
                }
                json!({ "outcome": { "outcome": "selected", "optionId": oid } })
            }
            None => json!({ "outcome": { "outcome": "cancelled" } }),
        };

        if let Some(transport) = self.transport.read().await.clone() {
            transport.send_response(pending.rpc_id, outcome).await?;
        } else {
            debug!(%request_id, ?option_id, "respond_approval (mock/local)");
        }

        if let Some(bus) = &self.event_bus {
            bus.emit(ControlEvent::ApprovalResolved {
                session_id: self.control_session_id,
                request_id: request_id.to_string(),
                option_id: option_id.map(str::to_string),
                cancelled: option_id.is_none(),
                at: Utc::now(),
            });
            if self.pending_permissions.lock().await.is_empty() {
                // Permission requests only arrive mid-turn; the prompt-completion
                // path still emits Idle at end of turn.
                bus.emit_status(self.control_session_id, SessionStatus::Running)
                    .await;
            }
        }
        Ok(())
    }

    /// The agent process died out from under us — surface it instead of
    /// leaving the thread stuck on "running".
    async fn report_process_death(&self) {
        if self.shutting_down.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        self.drain_pending_permissions().await;
        if let Some(bus) = &self.event_bus {
            bus.emit_error(
                Some(self.control_session_id),
                format!("{} agent process exited unexpectedly", self.config.backend_label),
            );
            bus.emit_status(self.control_session_id, SessionStatus::Failed)
                .await;
        }
    }

    /// Answer every parked permission request as cancelled (turn cancel / shutdown).
    async fn drain_pending_permissions(&self) {
        let drained: Vec<(String, PendingPermission)> = {
            let mut map = self.pending_permissions.lock().await;
            map.drain().collect()
        };
        if drained.is_empty() {
            return;
        }
        let transport = self.transport.read().await.clone();
        for (request_id, pending) in drained {
            if let Some(t) = &transport {
                // Best-effort: the process may already be gone.
                let _ = t
                    .send_response(
                        pending.rpc_id,
                        json!({ "outcome": { "outcome": "cancelled" } }),
                    )
                    .await;
            }
            if let Some(bus) = &self.event_bus {
                bus.emit(ControlEvent::ApprovalResolved {
                    session_id: self.control_session_id,
                    request_id,
                    option_id: None,
                    cancelled: true,
                    at: Utc::now(),
                });
            }
        }
    }

    async fn run_event_loop(self: Arc<Self>) -> Result<()> {
        let mut rx = self
            .notification_rx
            .lock()
            .await
            .take()
            .ok_or(AcpError::SessionNotReady)?;

        while let Some(notif) = rx.recv().await {
            self.handle_notification(notif).await;
        }
        Err(AcpError::ProcessExited)
    }

    /// Handle agent-initiated JSON-RPC requests. Critical for unblocking turns.
    ///
    /// Each request runs in its own task so `terminal/wait_for_exit` (long) never
    /// blocks `terminal/output`, fs/*, or permission responses.
    async fn run_agent_request_loop(self: Arc<Self>) -> Result<()> {
        let mut rx = self
            .agent_request_rx
            .lock()
            .await
            .take()
            .ok_or(AcpError::SessionNotReady)?;

        while let Some(req) = rx.recv().await {
            let this = self.clone();
            tokio::spawn(async move {
                if let Err(e) = this.handle_agent_request(req).await {
                    warn!(error = %e, "failed handling agent request");
                }
            });
        }
        Err(AcpError::ProcessExited)
    }

    async fn handle_agent_request(&self, req: IncomingAgentRequest) -> Result<()> {
        let transport = self.transport().await?;
        let method = req.method.as_str();
        info!(%method, "ACP agent→client request");

        match method {
            "fs/read_text_file" | "fs/readTextFile" => {
                let path = req
                    .params
                    .as_ref()
                    .and_then(|p| {
                        p.get("path")
                            .or_else(|| p.get("file_path"))
                            .or_else(|| p.get("filePath"))
                    })
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                self.emit_host_tool("fs/read", path, ToolCallStatus::Running);
                match self.fs_read_text(&req.params).await {
                    Ok(content) => {
                        self.emit_host_tool("fs/read", path, ToolCallStatus::Completed);
                        transport
                            .send_response(req.id, json!({ "content": content }))
                            .await?;
                    }
                    Err(e) => {
                        self.emit_host_tool("fs/read", &e.to_string(), ToolCallStatus::Failed);
                        transport
                            .send_error_response(req.id, -32000, e.to_string())
                            .await?;
                    }
                }
            }
            "fs/write_text_file" | "fs/writeTextFile" => {
                let path = req
                    .params
                    .as_ref()
                    .and_then(|p| {
                        p.get("path")
                            .or_else(|| p.get("file_path"))
                            .or_else(|| p.get("filePath"))
                    })
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                self.emit_host_tool("fs/write", path, ToolCallStatus::Running);
                match self.fs_write_text(&req.params).await {
                    Ok(()) => {
                        self.emit_host_tool("fs/write", path, ToolCallStatus::Completed);
                        transport.send_response(req.id, json!({})).await?;
                    }
                    Err(e) => {
                        self.emit_host_tool("fs/write", &e.to_string(), ToolCallStatus::Failed);
                        transport
                            .send_error_response(req.id, -32000, e.to_string())
                            .await?;
                    }
                }
            }
            "session/request_permission" | "session/requestPermission" => {
                let options = parse_permission_options(&req.params);
                let tool = req
                    .params
                    .as_ref()
                    .and_then(|p| p.get("toolCall"))
                    .and_then(|t| t.get("title").or_else(|| t.get("toolName")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool")
                    .to_string();
                let summary = permission_summary(&req.params, &tool);
                let request_id = id_key(&req.id);

                if self.always_approve.load(std::sync::atomic::Ordering::Relaxed) {
                    match pick_auto_approve_option(&options) {
                        Some(picked) => {
                            transport
                                .send_response(
                                    req.id,
                                    json!({
                                        "outcome": { "outcome": "selected", "optionId": picked }
                                    }),
                                )
                                .await?;
                            if let Some(bus) = &self.event_bus {
                                bus.emit(ControlEvent::ApprovalRequired {
                                    session_id: self.control_session_id,
                                    request_id,
                                    tool,
                                    summary,
                                    options,
                                    auto_approved: true,
                                    selected_option: Some(picked),
                                    at: Utc::now(),
                                });
                            }
                        }
                        None => {
                            // Only always-allow / reject options offered — never
                            // silently flip the agent into permanent yolo.
                            transport
                                .send_response(
                                    req.id,
                                    json!({ "outcome": { "outcome": "cancelled" } }),
                                )
                                .await?;
                            if let Some(bus) = &self.event_bus {
                                bus.emit_error(
                                    Some(self.control_session_id),
                                    format!(
                                        "permission request for {tool} offered no one-shot allow option; cancelled — respond manually with yolo off"
                                    ),
                                );
                            }
                        }
                    }
                } else {
                    self.pending_permissions.lock().await.insert(
                        request_id.clone(),
                        PendingPermission {
                            rpc_id: req.id,
                            options: options.clone(),
                        },
                    );
                    if let Some(bus) = &self.event_bus {
                        bus.emit(ControlEvent::ApprovalRequired {
                            session_id: self.control_session_id,
                            request_id,
                            tool,
                            summary,
                            options,
                            auto_approved: false,
                            selected_option: None,
                            at: Utc::now(),
                        });
                        bus.emit_status(self.control_session_id, SessionStatus::WaitingApproval)
                            .await;
                    }
                    // Deliberately no response here: the request stays open on the
                    // wire until respond_approval / cancel / shutdown answers it.
                }
            }
            // ACP terminal host — required for Grok run_terminal_command.
            m if m.starts_with("terminal/") => {
                let result = self.terminals.handle(m, &req.params).await;
                let line = TerminalRegistry::summary_line(m, &req.params, &result);
                if let Some(bus) = &self.event_bus {
                    Self::emit_term(bus, self.control_session_id, line);
                    // Surface command output after wait so the center column mirrors the shell.
                    if matches!(m, "terminal/wait_for_exit" | "terminal/waitForExit") {
                        if let Ok(ref wait) = result {
                            if let Some(tid) = req
                                .params
                                .as_ref()
                                .and_then(|p| p.get("terminalId"))
                                .and_then(|v| v.as_str())
                            {
                                if let Ok(out) = self
                                    .terminals
                                    .handle(
                                        "terminal/output",
                                        &Some(json!({ "terminalId": tid })),
                                    )
                                    .await
                                {
                                    let text = out
                                        .get("output")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    if !text.is_empty() {
                                        let clip = if text.len() > 4000 {
                                            format!("{}…", &text[..4000])
                                        } else {
                                            text.to_string()
                                        };
                                        let code = wait
                                            .get("exitCode")
                                            .and_then(|c| c.as_i64())
                                            .unwrap_or(-1);
                                        Self::emit_term(
                                            bus,
                                            self.control_session_id,
                                            format!("{clip}\n[exit {code}]"),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                match result {
                    Ok(val) => {
                        self.emit_host_tool(
                            m,
                            req.params
                                .as_ref()
                                .and_then(|p| p.get("command"))
                                .and_then(|v| v.as_str())
                                .unwrap_or(m),
                            if m.contains("create") {
                                ToolCallStatus::Running
                            } else if m.contains("wait") {
                                ToolCallStatus::Completed
                            } else {
                                ToolCallStatus::Completed
                            },
                        );
                        transport.send_response(req.id, val).await?;
                    }
                    Err(e) => {
                        self.emit_host_tool(m, &e.to_string(), ToolCallStatus::Failed);
                        transport
                            .send_error_response(req.id, -32000, e.to_string())
                            .await?;
                    }
                }
            }
            other => {
                warn!(method = %other, "unhandled agent request — returning empty result");
                // Prefer empty success over hang when method is unknown optional.
                transport.send_response(req.id, json!({})).await?;
            }
        }
        Ok(())
    }

    fn resolve_sandbox_path(&self, path: &str) -> Result<PathBuf> {
        let p = PathBuf::from(path);
        let abs = if p.is_absolute() {
            p
        } else {
            self.config.cwd.join(p)
        };
        let abs = abs.canonicalize().unwrap_or(abs);
        let cwd = self
            .config
            .cwd
            .canonicalize()
            .unwrap_or_else(|_| self.config.cwd.clone());
        // Allow cwd and children only.
        if abs == cwd || abs.starts_with(&cwd) {
            Ok(abs)
        } else {
            Err(AcpError::Protocol(format!(
                "path outside workspace: {}",
                abs.display()
            )))
        }
    }

    fn emit_host_tool(&self, tool: &str, summary: &str, status: ToolCallStatus) {
        if let Some(bus) = &self.event_bus {
            bus.emit_tool_call(
                self.control_session_id,
                ToolCallEvent {
                    id: Uuid::new_v4().to_string(),
                    tool: tool.to_string(),
                    args_summary: summary.to_string(),
                    status,
                    result_summary: None,
                    at: Utc::now(),
                },
            );
        }
    }

    async fn fs_read_text(&self, params: &Option<Value>) -> Result<String> {
        let p = params.as_ref().ok_or_else(|| AcpError::Protocol("missing params".into()))?;
        let path = p
            .get("path")
            .or_else(|| p.get("file_path"))
            .or_else(|| p.get("filePath"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| AcpError::Protocol("fs/read missing path".into()))?;
        let abs = self.resolve_sandbox_path(path)?;
        let mut content = tokio::fs::read_to_string(&abs)
            .await
            .map_err(|e| AcpError::Protocol(format!("read {}: {e}", abs.display())))?;

        // Optional line/limit (1-based line)
        if let Some(line) = p.get("line").and_then(|v| v.as_u64()) {
            let start = line.saturating_sub(1) as usize;
            let lines: Vec<&str> = content.lines().collect();
            let end = if let Some(limit) = p.get("limit").and_then(|v| v.as_u64()) {
                (start + limit as usize).min(lines.len())
            } else {
                lines.len()
            };
            content = lines
                .get(start..end)
                .map(|s| s.join("\n"))
                .unwrap_or_default();
        }
        // Cap huge files so we don't blow the agent context
        const MAX: usize = 400_000;
        if content.len() > MAX {
            content.truncate(MAX);
            content.push_str("\n…[truncated]");
        }
        Ok(content)
    }

    async fn fs_write_text(&self, params: &Option<Value>) -> Result<()> {
        let p = params.as_ref().ok_or_else(|| AcpError::Protocol("missing params".into()))?;
        let path = p
            .get("path")
            .or_else(|| p.get("file_path"))
            .or_else(|| p.get("filePath"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| AcpError::Protocol("fs/write missing path".into()))?;
        let content = p
            .get("content")
            .or_else(|| p.get("text"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| AcpError::Protocol("fs/write missing content".into()))?;
        let abs = self.resolve_sandbox_path(path)?;
        if let Some(parent) = abs.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AcpError::Protocol(format!("mkdir: {e}")))?;
        }
        tokio::fs::write(&abs, content)
            .await
            .map_err(|e| AcpError::Protocol(format!("write {}: {e}", abs.display())))?;
        if let Some(bus) = &self.event_bus {
            bus.emit(ControlEvent::AgentMessage {
                session_id: self.control_session_id,
                text: format!("wrote {}", abs.display()),
                at: Utc::now(),
            });
        }
        Ok(())
    }

    async fn handle_notification(&self, notif: JsonRpcNotification) {
        debug!(method = %notif.method, "ACP notification");
        let Some(bus) = &self.event_bus else {
            return;
        };
        let sid = self.control_session_id;
        let params = notif.params.unwrap_or(Value::Null);

        match notif.method.as_str() {
            "session/update" | "session/updateNotification" => {
                self.map_session_update(bus, sid, &params).await;
            }
            m if m.contains("tool") => {
                let tool = params
                    .get("tool")
                    .or_else(|| params.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let id = params
                    .get("id")
                    .or_else(|| params.get("toolCallId"))
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| Uuid::new_v4().to_string());
                bus.emit_tool_call(
                    sid,
                    ToolCallEvent {
                        id,
                        tool,
                        args_summary: params
                            .get("arguments")
                            .or_else(|| params.get("args"))
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                        status: ToolCallStatus::Running,
                        result_summary: None,
                        at: Utc::now(),
                    },
                );
            }
            m if m.contains("plan") => {
                bus.emit_plan_update(
                    sid,
                    PlanUpdateEvent {
                        plan_id: params
                            .get("planId")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        title: params
                            .get("title")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        steps: params
                            .get("steps")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .enumerate()
                                    .map(|(i, s)| PlanStep {
                                        id: s
                                            .get("id")
                                            .and_then(|v| v.as_str())
                                            .map(str::to_string)
                                            .unwrap_or_else(|| i.to_string()),
                                        description: s
                                            .get("description")
                                            .or_else(|| s.get("text"))
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                        status: s
                                            .get("status")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("pending")
                                            .to_string(),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        status: params
                            .get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("updated")
                            .to_string(),
                        at: Utc::now(),
                    },
                );
            }
            // Permission requests arrive as agent→client *requests* and are handled
            // in handle_agent_request; permission-flavored notifications are just
            // informational, so let them fall through to Raw.
            _ => {
                bus.emit(ControlEvent::Raw {
                    session_id: Some(sid),
                    payload: json!({ "method": notif.method, "params": params }),
                });
            }
        }
    }

    /// Surface a bare-retry MCP drop in the thread instead of only a log line —
    /// the user believes those tools are available otherwise.
    fn emit_mcp_dropped(&self, servers: &[Value], error: &str) {
        let Some(bus) = &self.event_bus else { return };
        let names: Vec<&str> = servers
            .iter()
            .filter_map(|s| s.get("name").and_then(|v| v.as_str()))
            .collect();
        Self::emit_term(
            bus,
            self.control_session_id,
            format!(
                "⚠ MCP servers dropped after agent error ({}): {error} — session continues without them",
                if names.is_empty() { "?".into() } else { names.join(", ") },
            ),
        );
    }

    fn emit_term(bus: &EventBus, sid: Uuid, line: impl Into<String>) {
        bus.emit(ControlEvent::Raw {
            session_id: Some(sid),
            payload: json!({
                "channel": "term",
                "stream": "acp",
                "line": line.into(),
            }),
        });
    }

    async fn map_session_update(&self, bus: &EventBus, sid: Uuid, params: &Value) {
        // ACP SessionNotification: { sessionId, update: SessionUpdate }
        // Some agents also flatten update fields onto params.
        let update = params.get("update").unwrap_or(params);

        // Grok streams totalTokens on params._meta (context window usage).
        if let Some(tokens) = params
            .get("_meta")
            .and_then(|m| m.get("totalTokens"))
            .and_then(|v| v.as_u64())
            .or_else(|| {
                update
                    .get("_meta")
                    .and_then(|m| m.get("totalTokens"))
                    .and_then(|v| v.as_u64())
            })
        {
            bus.emit(ControlEvent::Raw {
                session_id: Some(sid),
                payload: json!({
                    "channel": "usage",
                    "totalTokens": tokens,
                }),
            });
        }

        let update_type = update
            .get("sessionUpdate")
            .or_else(|| update.get("type"))
            .or_else(|| update.get("kind"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let update_type_norm = update_type
            .replace('-', "_")
            .chars()
            .flat_map(|c| {
                // agentMessageChunk → agent_message_chunk-ish lowercase
                if c.is_uppercase() {
                    vec!['_', c.to_ascii_lowercase()]
                } else {
                    vec![c]
                }
            })
            .collect::<String>()
            .trim_start_matches('_')
            .to_string();

        // Always leave a breadcrumb for non-text updates so the center looks like a TTY.
        match update_type_norm.as_str() {
            "agent_message_chunk"
            | "agent_message"
            | "message"
            | "agentmessagechunk"
            | "agentmessage"
            | "agent_thought_chunk"
            | "agent_thought"
            | "agentthoughtchunk"
            | "thought"
            | "user_message_chunk"
            | "usermessagechunk" => {}
            other if !other.is_empty() => {
                let snippet = extract_agent_text(update)
                    .map(|t| {
                        let t = t.replace('\n', " ");
                        if t.len() > 120 {
                            format!("{}…", &t[..120])
                        } else {
                            t
                        }
                    })
                    .or_else(|| {
                        update
                            .get("title")
                            .or_else(|| update.get("status"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_default();
                if snippet.is_empty() {
                    Self::emit_term(bus, sid, format!("· session/update {other}"));
                } else {
                    Self::emit_term(bus, sid, format!("· {other}: {snippet}"));
                }
            }
            _ => {
                Self::emit_term(bus, sid, "· session/update (untyped)");
            }
        }

        match update_type_norm.as_str() {
            "tool_call" | "toolcall" => {
                bus.emit_tool_call(
                    sid,
                    ToolCallEvent {
                        id: update
                            .get("toolCallId")
                            .or_else(|| update.get("id"))
                            .map(|v| match v {
                                Value::String(s) => s.clone(),
                                other => other.to_string(),
                            })
                            .unwrap_or_else(|| Uuid::new_v4().to_string()),
                        tool: update
                            .get("title")
                            .or_else(|| update.get("toolName"))
                            .or_else(|| update.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("tool")
                            .to_string(),
                        args_summary: update
                            .get("rawInput")
                            .or_else(|| update.get("arguments"))
                            .or_else(|| update.get("input"))
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                        status: ToolCallStatus::Running,
                        result_summary: None,
                        at: Utc::now(),
                    },
                );
            }
            "tool_call_update" | "toolcallupdate" => {
                let status = update
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("running");
                let tool_status = match status {
                    "completed" | "done" | "success" => ToolCallStatus::Completed,
                    "failed" | "error" => ToolCallStatus::Failed,
                    "denied" | "rejected" => ToolCallStatus::Denied,
                    "pending" => ToolCallStatus::Pending,
                    _ => ToolCallStatus::Running,
                };
                bus.emit_tool_call(
                    sid,
                    ToolCallEvent {
                        id: update
                            .get("toolCallId")
                            .or_else(|| update.get("id"))
                            .map(|v| match v {
                                Value::String(s) => s.clone(),
                                other => other.to_string(),
                            })
                            .unwrap_or_else(|| Uuid::new_v4().to_string()),
                        tool: update
                            .get("title")
                            .or_else(|| update.get("toolName"))
                            .or_else(|| update.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("tool")
                            .to_string(),
                        args_summary: update
                            .get("rawInput")
                            .or_else(|| update.get("arguments"))
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                        status: tool_status,
                        result_summary: extract_text_content(update.get("content"))
                            .or_else(|| {
                                update
                                    .get("rawOutput")
                                    .map(|v| v.to_string())
                            }),
                        at: Utc::now(),
                    },
                );
            }
            "agent_message_chunk"
            | "agent_message"
            | "message"
            | "agentmessagechunk"
            | "agentmessage" => {
                if let Some(text) = extract_agent_text(update) {
                    if !text.is_empty() {
                        bus.emit(ControlEvent::AgentMessage {
                            session_id: sid,
                            text,
                            at: Utc::now(),
                        });
                    }
                } else {
                    debug!(?update, "agent message chunk with no extractable text");
                }
            }
            "agent_thought_chunk" | "agent_thought" | "agentthoughtchunk" | "thought" => {
                if let Some(text) = extract_agent_text(update) {
                    if !text.is_empty() {
                        // Surface thoughts in the transcript so the user sees reasoning stream.
                        bus.emit(ControlEvent::AgentMessage {
                            session_id: sid,
                            text: format!("💭 {text}"),
                            at: Utc::now(),
                        });
                    }
                }
            }
            "user_message_chunk" | "usermessagechunk" => {
                // Echo of user prompt while streaming — optional; skip to avoid dup.
            }
            "plan" => {
                let steps = update
                    .get("entries")
                    .or_else(|| update.get("steps"))
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .enumerate()
                            .map(|(i, s)| PlanStep {
                                id: s
                                    .get("id")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string)
                                    .unwrap_or_else(|| i.to_string()),
                                description: s
                                    .get("content")
                                    .or_else(|| s.get("description"))
                                    .or_else(|| s.get("text"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                status: s
                                    .get("status")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("pending")
                                    .to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                bus.emit_plan_update(
                    sid,
                    PlanUpdateEvent {
                        plan_id: None,
                        title: update
                            .get("title")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        steps,
                        status: "updated".into(),
                        at: Utc::now(),
                    },
                );
            }
            "available_commands_update" | "availablecommandsupdate" => {}
            "current_mode_update" | "currentmodeupdate" => {
                if let Some(mode) = update
                    .get("currentModeId")
                    .or_else(|| update.get("modeId"))
                    .or_else(|| update.get("mode"))
                    .and_then(|v| v.as_str())
                {
                    *self.current_mode.write().await = Some(mode.to_string());
                    Self::emit_term(bus, sid, format!("mode → {mode}"));
                }
            }
            "usage_update" | "usageupdate" => {
                let used = update.get("used").and_then(|v| v.as_u64());
                let size = update.get("size").and_then(|v| v.as_u64());
                Self::emit_term(
                    bus,
                    sid,
                    format!(
                        "· usage tokens {}/{}",
                        used.map(|u| u.to_string()).unwrap_or_else(|| "?".into()),
                        size.map(|s| s.to_string()).unwrap_or_else(|| "?".into())
                    ),
                );
            }
            other => {
                // Last-resort: if content looks like text, still surface it.
                if let Some(text) = extract_agent_text(update) {
                    if !text.is_empty() {
                        info!(other, "treating unmapped update as agent text");
                        bus.emit(ControlEvent::AgentMessage {
                            session_id: sid,
                            text,
                            at: Utc::now(),
                        });
                        return;
                    }
                }
                debug!(other, "unmapped session update");
                let compact = serde_json::to_string(update).unwrap_or_default();
                let compact = if compact.len() > 280 {
                    format!("{}…", &compact[..280])
                } else {
                    compact
                };
                Self::emit_term(
                    bus,
                    sid,
                    format!("· acp/{other}: {compact}"),
                );
            }
        }
    }
}

/// Parse the options array of a `session/request_permission` request.
fn parse_permission_options(params: &Option<Value>) -> Vec<PermissionOptionInfo> {
    params
        .as_ref()
        .and_then(|p| p.get("options"))
        .and_then(|o| o.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    let id = o
                        .get("optionId")
                        .or_else(|| o.get("id"))
                        .and_then(|v| v.as_str())?
                        .to_string();
                    let kind = o
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let label = o
                        .get("name")
                        .or_else(|| o.get("label"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| if kind.is_empty() { id.clone() } else { kind.clone() });
                    Some(PermissionOptionInfo { id, kind, label })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Pick the option yolo mode auto-selects: a one-shot allow, never `allow_always`
/// (selecting it would flip the agent itself into permanent always-allow mode).
fn pick_auto_approve_option(options: &[PermissionOptionInfo]) -> Option<String> {
    let is_always = |o: &PermissionOptionInfo| {
        o.kind.eq_ignore_ascii_case("allow_always")
            || o.kind.eq_ignore_ascii_case("allowalways")
            || o.label.to_lowercase().contains("always")
    };
    let is_reject = |o: &PermissionOptionInfo| {
        let k = o.kind.to_lowercase();
        k.contains("reject") || k.contains("deny") || k.contains("cancel")
    };

    options
        .iter()
        .find(|o| o.kind.eq_ignore_ascii_case("allow_once") || o.kind.eq_ignore_ascii_case("allowonce"))
        .or_else(|| {
            options
                .iter()
                .find(|o| o.kind.to_lowercase().starts_with("allow") && !is_always(o))
        })
        .or_else(|| {
            options.iter().find(|o| {
                let l = o.label.to_lowercase();
                (l.contains("allow") || l.contains("approve") || l.contains("yes"))
                    && !is_always(o)
                    && !is_reject(o)
            })
        })
        .or_else(|| options.iter().find(|o| !is_always(o) && !is_reject(o)))
        .map(|o| o.id.clone())
}

/// Human-readable one-liner for the approval card body.
fn permission_summary(params: &Option<Value>, tool: &str) -> String {
    let detail = params
        .as_ref()
        .and_then(|p| p.get("toolCall"))
        .and_then(|t| {
            t.get("command")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| t.get("rawInput").map(|v| v.to_string()))
        })
        .unwrap_or_default();
    let mut s = if detail.is_empty() {
        format!("{tool} requests permission")
    } else {
        format!("{tool}: {detail}")
    };
    if s.chars().count() > 200 {
        s = s.chars().take(200).collect::<String>() + "…";
    }
    s
}

/// Pull plain text from ACP ContentBlock shapes (and common variants).
fn extract_text_content(content: Option<&Value>) -> Option<String> {
    let content = content?;
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    if let Some(t) = content.get("text").and_then(|v| v.as_str()) {
        return Some(t.to_string());
    }
    if let Some(t) = content.get("thought").and_then(|v| v.as_str()) {
        return Some(t.to_string());
    }
    if let Some(arr) = content.as_array() {
        let mut out = String::new();
        for item in arr {
            if let Some(t) = extract_text_content(Some(item)) {
                out.push_str(&t);
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    // Nested: { role, content: [...] } or { content: "..." }
    if let Some(inner) = content.get("content") {
        if let Some(t) = extract_text_content(Some(inner)) {
            return Some(t);
        }
    }
    None
}

/// Extract streamed agent text from a session update object.
fn extract_agent_text(update: &Value) -> Option<String> {
    if let Some(t) = extract_text_content(update.get("content")) {
        return Some(t);
    }
    if let Some(t) = update.get("text").and_then(|v| v.as_str()) {
        return Some(t.to_string());
    }
    if let Some(t) = update.get("message").and_then(|v| v.as_str()) {
        return Some(t.to_string());
    }
    if let Some(t) = update.get("delta").and_then(|v| v.as_str()) {
        return Some(t.to_string());
    }
    // content may be top-level array of blocks
    if update.get("type").and_then(|v| v.as_str()) == Some("text") {
        if let Some(t) = update.get("text").and_then(|v| v.as_str()) {
            return Some(t.to_string());
        }
    }
    None
}

impl AcpClient {
    pub async fn shutdown(&self) -> Result<()> {
        self.shutting_down
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.drain_pending_permissions().await;
        let _ = self.cancel().await;
        let mut child_guard = self.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            let _ = child.kill().await;
        }
        *self.transport.write().await = None;
        Ok(())
    }

    pub fn cwd(&self) -> &Path {
        &self.config.cwd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_client_has_session() {
        let c = AcpClient::mock_for_tests("sess-1", None);
        assert_eq!(c.session_id().await.as_deref(), Some("sess-1"));
    }

    #[tokio::test]
    async fn mock_send_prompt_returns_without_blocking() {
        let c = AcpClient::mock_for_tests("sess-1", None);
        // Must not hang waiting for a full agent turn.
        tokio::time::timeout(Duration::from_secs(1), c.send_prompt("long job prompt"))
            .await
            .expect("send_prompt should return immediately")
            .expect("mock prompt ok");
    }

    #[tokio::test]
    async fn empty_prompt_rejected() {
        let c = AcpClient::mock_for_tests("sess-1", None);
        let err = c.send_prompt("   ").await.unwrap_err();
        assert!(matches!(err, AcpError::Protocol(_)));
    }

    #[test]
    fn auto_approve_prefers_allow_once_never_always() {
        let opts = parse_permission_options(&Some(json!({
            "options": [
                { "optionId": "always", "kind": "allow_always", "name": "Always allow" },
                { "optionId": "once", "kind": "allow_once", "name": "Allow once" },
                { "optionId": "no", "kind": "reject_once", "name": "Deny" }
            ]
        })));
        assert_eq!(pick_auto_approve_option(&opts).as_deref(), Some("once"));

        let only_always = parse_permission_options(&Some(json!({
            "options": [
                { "optionId": "always", "kind": "allow_always", "name": "Always allow" },
                { "optionId": "no", "kind": "reject_once", "name": "Deny" }
            ]
        })));
        assert_eq!(pick_auto_approve_option(&only_always), None);
    }

    #[test]
    fn parses_option_field_variants() {
        let opts = parse_permission_options(&Some(json!({
            "options": [
                { "id": "a", "kind": "allow_once" },
                { "optionId": "b", "label": "Deny it" }
            ]
        })));
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].id, "a");
        assert_eq!(opts[0].label, "allow_once");
        assert_eq!(opts[1].id, "b");
        assert_eq!(opts[1].label, "Deny it");
    }

    #[tokio::test]
    async fn respond_approval_unknown_request_errors() {
        let c = AcpClient::mock_for_tests("sess-1", None);
        let err = c.respond_approval("nope", Some("allow")).await.unwrap_err();
        assert!(matches!(err, AcpError::Protocol(_)));
    }

    #[tokio::test]
    async fn respond_approval_is_single_shot() {
        let c = AcpClient::mock_for_tests("sess-1", None);
        c.pending_permissions.lock().await.insert(
            "42".into(),
            PendingPermission {
                rpc_id: json!(42),
                options: vec![PermissionOptionInfo {
                    id: "allow".into(),
                    kind: "allow_once".into(),
                    label: "Allow once".into(),
                }],
            },
        );
        c.respond_approval("42", Some("allow")).await.unwrap();
        let err = c.respond_approval("42", Some("allow")).await.unwrap_err();
        assert!(matches!(err, AcpError::Protocol(_)));
    }

    #[tokio::test]
    async fn respond_approval_rejects_unknown_option_and_keeps_pending() {
        let c = AcpClient::mock_for_tests("sess-1", None);
        c.pending_permissions.lock().await.insert(
            "7".into(),
            PendingPermission {
                rpc_id: json!(7),
                options: vec![PermissionOptionInfo {
                    id: "allow".into(),
                    kind: "allow_once".into(),
                    label: "Allow once".into(),
                }],
            },
        );
        assert!(c.respond_approval("7", Some("bogus")).await.is_err());
        // Still answerable with a valid option after the bad attempt.
        c.respond_approval("7", Some("allow")).await.unwrap();
    }

    #[test]
    fn extracts_nested_acp_message_chunk() {
        let params = json!({
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "Hello from agent" }
            }
        });
        let update = params.get("update").unwrap();
        assert_eq!(
            extract_agent_text(update).as_deref(),
            Some("Hello from agent")
        );
    }

    #[test]
    fn extracts_content_block_array() {
        let update = json!({
            "sessionUpdate": "agent_message_chunk",
            "content": [
                { "type": "text", "text": "Hi " },
                { "type": "text", "text": "there" }
            ]
        });
        assert_eq!(extract_agent_text(&update).as_deref(), Some("Hi there"));
    }

    #[test]
    fn extracts_thought_chunk() {
        let update = json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": { "type": "text", "text": "I should check tests" }
        });
        assert_eq!(
            extract_agent_text(&update).as_deref(),
            Some("I should check tests")
        );
    }
}
