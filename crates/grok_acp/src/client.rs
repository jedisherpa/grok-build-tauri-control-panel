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
    AuthenticateParams, ClientCapabilities, ClientInfo, FsCapabilities, InitializeParams,
    JsonRpcNotification, PromptContent, SessionPromptParams,
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
    pub request_timeout: Duration,
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
        let transport = NdjsonTransport::new(stdin, stdout, notif_tx);

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
        let timeout = self.config.request_timeout;
        tokio::time::timeout(timeout, transport.request(method, params))
            .await
            .map_err(|_| AcpError::Timeout(method.to_string()))?
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
            bus.emit(ControlEvent::AgentMessage {
                session_id: self.control_session_id,
                text: format!("[local] received prompt ({} chars)", prompt.len()),
                at: Utc::now(),
            });
        }

        // Mock clients: no transport — accept and return.
        if self.transport.read().await.is_none() {
            if let Some(bus) = &self.event_bus {
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

        self.request_timeout("session/prompt", Some(serde_json::to_value(params)?))
            .await?;
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
        let update_type = params
            .get("sessionUpdate")
            .or_else(|| params.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match update_type {
            "tool_call" | "toolCall" => {
                bus.emit_tool_call(
                    sid,
                    ToolCallEvent {
                        id: params
                            .get("toolCallId")
                            .or_else(|| params.get("id"))
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| Uuid::new_v4().to_string()),
                        tool: params
                            .get("title")
                            .or_else(|| params.get("toolName"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("tool")
                            .to_string(),
                        args_summary: params
                            .get("rawInput")
                            .or_else(|| params.get("arguments"))
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                        status: ToolCallStatus::Running,
                        result_summary: None,
                        at: Utc::now(),
                    },
                );
            }
            "agent_message_chunk" | "message" | "agent_message" => {
                if let Some(text) = params
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(|v| v.as_str())
                    .or_else(|| params.get("text").and_then(|v| v.as_str()))
                {
                    bus.emit(ControlEvent::AgentMessage {
                        session_id: sid,
                        text: text.to_string(),
                        at: Utc::now(),
                    });
                }
            }
            "plan" => {
                bus.emit_plan_update(
                    sid,
                    PlanUpdateEvent {
                        plan_id: None,
                        title: None,
                        steps: Vec::new(),
                        status: "updated".into(),
                        at: Utc::now(),
                    },
                );
            }
            "available_commands_update" => {}
            other => {
                debug!(other, "unmapped session update");
                bus.emit(ControlEvent::Raw {
                    session_id: Some(sid),
                    payload: params.clone(),
                });
            }
        }
    }

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
}
