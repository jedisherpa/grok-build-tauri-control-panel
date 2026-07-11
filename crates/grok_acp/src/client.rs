//! High-level ACP client: spawn, initialize, auth, session, prompt, event loop.

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
    ControlEvent, EventBus, PlanStep, PlanUpdateEvent, SessionStatus, ToolCallEvent, ToolCallStatus,
};

use crate::error::{AcpError, Result};
use crate::messages::{
    AuthenticateParams, ClientCapabilities, ClientInfo, FsCapabilities, IncomingAgentRequest,
    InitializeParams, JsonRpcNotification, PromptContent, SessionPromptParams,
};
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

#[derive(Debug, Clone)]
pub struct AcpClientConfig {
    pub grok_path: PathBuf,
    pub cwd: PathBuf,
    pub client_name: String,
    pub client_version: String,
    /// Timeout for short control RPCs (initialize, auth, session/new, cancel).
    pub request_timeout: Duration,
    /// Max wait for a full agent turn on session/prompt (long coding jobs).
    pub prompt_timeout: Duration,
    /// Preferred auth method; may be overridden by agent-advertised methods.
    pub auth_method_id: String,
}

impl AcpClientConfig {
    pub fn new(grok_path: impl Into<PathBuf>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            grok_path: grok_path.into(),
            cwd: cwd.into(),
            client_name: "BombCode".into(),
            client_version: env!("CARGO_PKG_VERSION").into(),
            request_timeout: Duration::from_secs(120),
            // Long agent turns stream via notifications; still cap runaway jobs.
            prompt_timeout: Duration::from_secs(60 * 60 * 2), // 2 hours
            // Grok Build advertises cached_token + grok.com (not xai.api_key).
            auth_method_id: "cached_token".into(),
        }
    }
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
    /// When true, auto-allow tool permission requests (yolo).
    always_approve: bool,
}

impl AcpClient {
    pub async fn connect(
        config: AcpClientConfig,
        opts: &SpawnOptions,
        event_bus: Option<Arc<EventBus>>,
        control_session_id: Uuid,
    ) -> Result<Arc<Self>> {
        if !config.cwd.is_absolute() {
            return Err(AcpError::Spawn("cwd must be absolute".into()));
        }
        if !config.grok_path.exists() {
            return Err(AcpError::Spawn(format!(
                "grok binary not found: {}",
                config.grok_path.display()
            )));
        }

        let mut cmd = Command::new(&config.grok_path);
        cmd.args(["agent", "stdio"])
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
        if let Ok(key) = std::env::var("XAI_API_KEY") {
            if !key.is_empty() {
                cmd.env("XAI_API_KEY", key);
            }
        }
        for (k, v) in &opts.extra_env {
            cmd.env(k, v);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| AcpError::Spawn(format!("failed to spawn grok agent stdio: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AcpError::Spawn("missing stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AcpError::Spawn("missing stdout".into()))?;

        let (notif_tx, notif_rx) = tokio::sync::mpsc::unbounded_channel();
        let (agent_req_tx, agent_req_rx) = tokio::sync::mpsc::unbounded_channel();
        let transport = NdjsonTransport::new(stdin, stdout, notif_tx, agent_req_tx);

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
            always_approve: opts.always_approve,
        });

        client.initialize().await?;
        client.authenticate().await?;
        client.session_new(opts).await?;

        // Background event loop for notifications
        let loop_client = client.clone();
        tokio::spawn(async move {
            if let Err(e) = loop_client.run_event_loop().await {
                warn!(error = %e, "ACP event loop terminated");
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
            always_approve: false,
        })
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
            .request_timeout("initialize", Some(serde_json::to_value(params)?))
            .await?;
        *self.agent_capabilities.write().await = result.get("agentCapabilities").cloned();

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
        const PREFERRED: &[&str] = &["cached_token", "grok.com", "xai.api_key"];
        if advertised.is_empty() {
            return self.config.auth_method_id.clone();
        }
        for p in PREFERRED {
            if advertised.iter().any(|m| m == *p) {
                return (*p).to_string();
            }
        }
        advertised
            .first()
            .cloned()
            .unwrap_or_else(|| self.config.auth_method_id.clone())
    }

    async fn authenticate(&self) -> Result<()> {
        let advertised = self.auth_methods.read().await.clone();
        let method_id = self.pick_auth_method(&advertised);
        info!(%method_id, "ACP authenticate");

        let params = AuthenticateParams {
            method_id: method_id.clone(),
            meta: Some(json!({ "headless": true })),
        };
        match self
            .request_timeout("authenticate", Some(serde_json::to_value(params)?))
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
                        .request_timeout("authenticate", Some(serde_json::to_value(params)?))
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

        *self.session_id.write().await = Some(sid.clone());
        info!(%sid, "ACP session/new complete");

        // Best-effort plan / always-approve mode after session exists.
        if opts.always_approve {
            let _ = self.set_mode("always_approve").await;
        } else if opts.plan_mode {
            let _ = self.set_mode("plan").await;
        }

        if let Some(bus) = &self.event_bus {
            bus.emit_status(self.control_session_id, SessionStatus::Idle)
                .await;
        }
        Ok(())
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
                text: prompt.to_string(),
            }],
        };
        let params_val = serde_json::to_value(params)?;
        let transport = self.transport().await?;

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
                    Ok(_) => {
                        info!("session/prompt completed");
                        if let Some(bus) = bus {
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
                    // Soft timeout: agent may still be running; keep session Running.
                    warn!(
                        timeout_secs = prompt_timeout.as_secs(),
                        "session/prompt still open after timeout; continuing via stream"
                    );
                    // Soft timeout only — do not invent agent speech.
                    debug!(
                        "prompt still open after {} min",
                        prompt_timeout.as_secs() / 60
                    );
                }
            }
        });

        // Return as soon as the request is on the wire.
        Ok(())
    }

    pub async fn cancel(&self) -> Result<()> {
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

    pub async fn set_mode(&self, mode: &str) -> Result<()> {
        if self.transport.read().await.is_none() {
            debug!(%mode, "set_mode (mock/local)");
            return Ok(());
        }
        let sid = self
            .session_id
            .read()
            .await
            .clone()
            .ok_or(AcpError::SessionNotReady)?;
        let params = json!({
            "sessionId": sid,
            "mode": mode,
        });
        // Best-effort — method name may vary by agent version
        match self
            .request_timeout("session/set_mode", Some(params.clone()))
            .await
        {
            Ok(_) => Ok(()),
            Err(AcpError::Rpc { .. }) => {
                let _ = self
                    .request_timeout("session/setMode", Some(params))
                    .await;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub async fn respond_approval(&self, request_id: &str, approved: bool) -> Result<()> {
        if self.transport.read().await.is_none() {
            debug!(%request_id, approved, "respond_approval (mock/local)");
            return Ok(());
        }
        let params = json!({
            "requestId": request_id,
            "approved": approved,
        });
        match self
            .request_timeout("session/approve", Some(params.clone()))
            .await
        {
            Ok(_) => Ok(()),
            Err(AcpError::Rpc { .. }) => {
                let _ = self
                    .request_timeout("client/permission/respond", Some(params))
                    .await;
                Ok(())
            }
            Err(e) => Err(e),
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
    async fn run_agent_request_loop(self: Arc<Self>) -> Result<()> {
        let mut rx = self
            .agent_request_rx
            .lock()
            .await
            .take()
            .ok_or(AcpError::SessionNotReady)?;

        while let Some(req) = rx.recv().await {
            if let Err(e) = self.handle_agent_request(req).await {
                warn!(error = %e, "failed handling agent request");
            }
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
                    .and_then(|p| p.get("path"))
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
                    .and_then(|p| p.get("path"))
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
                let outcome = self.permission_outcome(&req.params).await;
                if let Some(bus) = &self.event_bus {
                    let tool = req
                        .params
                        .as_ref()
                        .and_then(|p| p.get("toolCall"))
                        .and_then(|t| t.get("title").or_else(|| t.get("toolName")))
                        .and_then(|v| v.as_str())
                        .unwrap_or("tool");
                    bus.emit(ControlEvent::ApprovalRequired {
                        session_id: self.control_session_id,
                        request_id: req.id.to_string(),
                        tool: tool.to_string(),
                        summary: if self.always_approve {
                            "auto-approved (yolo)".into()
                        } else {
                            "auto-allowed for session progress (plan mode soft-approve)".into()
                        },
                        at: Utc::now(),
                    });
                }
                transport.send_response(req.id, outcome).await?;
            }
            // Terminal stubs — not fully implemented; return method-not-found cleanly
            m if m.starts_with("terminal/") => {
                transport
                    .send_error_response(
                        req.id,
                        -32601,
                        format!("terminal capability not fully implemented: {m}"),
                    )
                    .await?;
            }
            other => {
                warn!(method = %other, "unhandled agent request — returning empty result");
                // Prefer empty success over hang when method is unknown optional.
                transport.send_response(req.id, json!({})).await?;
            }
        }
        Ok(())
    }

    async fn permission_outcome(&self, params: &Option<Value>) -> Value {
        // Pick first allow-ish option if present; else selected generic allow.
        let options = params
            .as_ref()
            .and_then(|p| p.get("options"))
            .and_then(|o| o.as_array())
            .cloned()
            .unwrap_or_default();

        let option_id = options
            .iter()
            .find_map(|o| {
                let id = o.get("optionId").or_else(|| o.get("id"))?.as_str()?;
                let kind = o
                    .get("kind")
                    .or_else(|| o.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_lowercase();
                if kind.contains("allow") || kind.contains("approve") || kind.contains("yes") {
                    Some(id.to_string())
                } else {
                    None
                }
            })
            .or_else(|| {
                options
                    .first()
                    .and_then(|o| o.get("optionId").or_else(|| o.get("id")))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "allow".into());

        json!({
            "outcome": {
                "outcome": "selected",
                "optionId": option_id
            }
        })
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
            .and_then(|v| v.as_str())
            .ok_or_else(|| AcpError::Protocol("fs/write missing path".into()))?;
        let content = p
            .get("content")
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
            m if m.contains("permission") || m.contains("approval") => {
                bus.emit(ControlEvent::ApprovalRequired {
                    session_id: sid,
                    request_id: params
                        .get("requestId")
                        .or_else(|| params.get("id"))
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| Uuid::new_v4().to_string()),
                    tool: params
                        .get("tool")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    summary: params
                        .get("summary")
                        .or_else(|| params.get("description"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("approval required")
                        .to_string(),
                    at: Utc::now(),
                });
                bus.emit_status(sid, SessionStatus::WaitingApproval).await;
            }
            _ => {
                bus.emit(ControlEvent::Raw {
                    session_id: Some(sid),
                    payload: json!({ "method": notif.method, "params": params }),
                });
            }
        }
    }

    async fn map_session_update(&self, bus: &EventBus, sid: Uuid, params: &Value) {
        // ACP SessionNotification: { sessionId, update: SessionUpdate }
        // Some agents also flatten update fields onto params.
        let update = params.get("update").unwrap_or(params);

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
            "available_commands_update" | "availablecommandsupdate" | "current_mode_update"
            | "currentmodeupdate" => {}
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
                bus.emit(ControlEvent::Raw {
                    session_id: Some(sid),
                    payload: params.clone(),
                });
            }
        }
    }
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
