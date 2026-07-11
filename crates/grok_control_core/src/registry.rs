//! Concurrent session registry with ACP-first spawn path.

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use dashmap::DashMap;
use serde_json::json;
use tracing::{info, warn};
use uuid::Uuid;

use grok_acp::{AcpClient, AcpClientConfig, AcpSpawnOptions};
use grok_cli_wrapper::{GrokCli, HeadlessSpawnOptions};
use grok_config::GrokConfig;
use grok_events::{EventBus, SessionStatus};

use crate::error::{CoreError, Result};
use crate::handle::{AgentHandle, AgentHandleSnapshot, SessionMetadata};
use crate::options::{AgentMode, SpawnOptions};

pub struct SessionRegistry {
    sessions: DashMap<Uuid, AgentHandle>,
    event_bus: Arc<EventBus>,
    config: Arc<tokio::sync::RwLock<GrokConfig>>,
    grok_cli: Arc<GrokCli>,
}

impl SessionRegistry {
    pub fn new(
        event_bus: Arc<EventBus>,
        config: Arc<tokio::sync::RwLock<GrokConfig>>,
        grok_cli: Arc<GrokCli>,
    ) -> Arc<Self> {
        Arc::new(Self {
            sessions: DashMap::new(),
            event_bus,
            config,
            grok_cli,
        })
    }

    pub async fn spawn_agent(&self, cwd: &str, opts: SpawnOptions) -> Result<Uuid> {
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
        if self.sessions.len() >= max {
            return Err(CoreError::MaxSessions(max));
        }

        // Security: never silently force always_approve from config default if plan_mode preferred
        if cfg.always_approve_default && !opts.plan_mode {
            warn!("config always_approve_default is true; respecting explicit spawn opts");
        }
        if opts.always_approve {
            warn!("spawning with always_approve=true — elevated trust mode");
        }

        let model = opts
            .model
            .clone()
            .unwrap_or_else(|| cfg.default_model.clone());
        let binary = cfg.resolve_grok_binary()?;
        drop(cfg);

        let id = Uuid::new_v4();
        let now = Utc::now();
        let mut metadata = SessionMetadata {
            id,
            acp_session_id: None,
            cwd: cwd.to_string(),
            worktree: opts.worktree.clone(),
            model: model.clone(),
            mode: opts.mode,
            status: SessionStatus::Starting,
            plan_mode: opts.plan_mode && !opts.always_approve,
            always_approve: opts.always_approve,
            sandbox_profile: opts.sandbox_profile.clone(),
            mcp_servers: opts.mcp_server_names.clone(),
            created_at: now,
            last_activity: now,
            label: None,
        };

        let handle = match opts.mode {
            AgentMode::Acp => {
                let acp_opts = AcpSpawnOptions {
                    model: Some(model),
                    rules: if opts.rules.is_empty() {
                        None
                    } else {
                        Some(json!(opts.rules))
                    },
                    mcp_servers: opts.mcp_servers.clone(),
                    plan_mode: metadata.plan_mode,
                    always_approve: opts.always_approve,
                    sandbox_profile: opts.sandbox_profile.clone(),
                    extra_env: Vec::new(),
                };
                let client_cfg = AcpClientConfig::new(&binary, cwd_path);
                let client = AcpClient::connect(
                    client_cfg,
                    &acp_opts,
                    Some(self.event_bus.clone()),
                    id,
                )
                .await?;
                metadata.acp_session_id = client.session_id().await;
                metadata.status = SessionStatus::Idle;
                AgentHandle {
                    metadata,
                    child: None,
                    acp_client: Some(client),
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
        self.event_bus
            .emit_session_created(id, cwd, mode_str)
            .await;
        self.event_bus
            .emit_status(id, handle.metadata.status)
            .await;

        info!(%id, mode = mode_str, cwd, "session spawned");
        self.sessions.insert(id, handle);
        Ok(id)
    }

    /// Spawn a mock ACP session for tests / offline UI development.
    pub async fn spawn_mock(&self, cwd: &str) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let client = AcpClient::mock_for_tests(&format!("mock-{id}"), Some(self.event_bus.clone()));
        let handle = AgentHandle {
            metadata: SessionMetadata {
                id,
                acp_session_id: Some(format!("mock-{id}")),
                cwd: cwd.to_string(),
                worktree: None,
                model: "mock".into(),
                mode: AgentMode::Acp,
                status: SessionStatus::Idle,
                plan_mode: true,
                always_approve: false,
                sandbox_profile: Some("workspace".into()),
                mcp_servers: Vec::new(),
                created_at: now,
                last_activity: now,
                label: Some("mock".into()),
            },
            child: None,
            acp_client: Some(client),
        };
        self.event_bus.emit_session_created(id, cwd, "acp").await;
        self.sessions.insert(id, handle);
        Ok(id)
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
        let client = {
            let mut entry = self
                .sessions
                .get_mut(&id)
                .ok_or(CoreError::SessionNotFound(id))?;
            entry.touch();
            entry.metadata.status = SessionStatus::Running;
            entry
                .acp_client
                .clone()
                .ok_or(CoreError::NotAcp)?
        };
        self.event_bus
            .emit_status(id, SessionStatus::Running)
            .await;
        client.send_prompt(prompt).await?;
        Ok(())
    }

    pub async fn cancel_session(&self, id: Uuid) -> Result<()> {
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
            if let Some(mut entry) = self.sessions.get_mut(&id) {
                if let Some(child) = entry.child.take() {
                    let mut c = child.lock().await;
                    let _ = c.kill().await;
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
        let client = {
            let mut entry = self
                .sessions
                .get_mut(&id)
                .ok_or(CoreError::SessionNotFound(id))?;
            entry.metadata.plan_mode = enabled;
            if enabled {
                entry.metadata.always_approve = false;
            }
            entry.touch();
            entry.acp_client.clone()
        };
        if let Some(client) = client {
            let mode = if enabled { "plan" } else { "default" };
            client.set_mode(mode).await?;
        }
        Ok(())
    }

    pub async fn respond_approval(&self, id: Uuid, request_id: &str, approved: bool) -> Result<()> {
        let client = self
            .sessions
            .get(&id)
            .ok_or(CoreError::SessionNotFound(id))?
            .acp_client
            .clone()
            .ok_or(CoreError::NotAcp)?;
        client.respond_approval(request_id, approved).await?;
        if let Some(mut entry) = self.sessions.get_mut(&id) {
            entry.metadata.status = SessionStatus::Running;
            entry.touch();
        }
        Ok(())
    }

    pub async fn remove_session(&self, id: Uuid) -> Result<()> {
        let _ = self.cancel_session(id).await;
        self.sessions.remove(&id);
        Ok(())
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub async fn shutdown_all(&self) {
        let ids: Vec<Uuid> = self.sessions.iter().map(|e| *e.key()).collect();
        for id in ids {
            if let Err(e) = self.cancel_session(id).await {
                warn!(%id, error = %e, "error cancelling session during shutdown");
            }
            self.sessions.remove(&id);
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
        let id = reg.spawn_mock("/tmp").await.unwrap();
        assert_eq!(reg.session_count(), 1);
        let snap = reg.get_snapshot(id).unwrap();
        assert_eq!(snap.metadata.cwd, "/tmp");
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
    async fn list_sessions() {
        let reg = test_registry();
        let _a = reg.spawn_mock("/tmp").await.unwrap();
        let _b = reg.spawn_mock("/tmp").await.unwrap();
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
}
