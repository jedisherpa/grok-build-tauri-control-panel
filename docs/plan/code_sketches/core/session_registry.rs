// Deep Code Sketch: grok_control_core/src/session_registry.rs
// Full production-ready sketch for SessionRegistry and AgentHandle
// Uses tokio for async, Arc for sharing, RwLock for concurrency
// Integrates with ACP and CLI modes

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
use tokio::process::Child;
use uuid::Uuid; // Add uuid = "1.10" to Cargo.toml
use serde::{Serialize, Deserialize};
use anyhow::Result; // For error handling

use crate::acp::AcpClient; // From grok_acp crate
use crate::cli_wrapper::GrokCli; // From grok_cli_wrapper
use crate::events::{EventBus, ToolCallEvent, PlanUpdateEvent, SessionStatus};
use crate::config::GrokConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentMode {
    Acp,      // Long-lived via grok agent stdio
    Headless, // Short-lived -p for batch
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub id: SessionId,
    pub cwd: String,
    pub worktree: Option<String>, // Path to ~/.grok/worktrees/...
    pub model: String,
    pub mode: AgentMode,
    pub status: SessionStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_activity: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug)]
pub struct AgentHandle {
    pub metadata: SessionMetadata,
    pub child: Option<Child>, // For process management
    pub acp_client: Option<Arc<AcpClient>>, // For ACP mode
    pub event_tx: broadcast::Sender<serde_json::Value>, // For streaming events
    // Add more: permission_overrides, sandbox_profile, etc.
}

pub struct SessionRegistry {
    sessions: Arc<RwLock<HashMap<SessionId, AgentHandle>>>,
    event_bus: Arc<EventBus>,
    config: Arc<GrokConfig>,
    grok_cli: Arc<GrokCli>,
}

impl SessionRegistry {
    pub fn new(event_bus: Arc<EventBus>, config: Arc<GrokConfig>, grok_cli: Arc<GrokCli>) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
            config,
            grok_cli,
        }
    }

    /// Spawn a new agent session - supports both ACP (recommended) and Headless
    pub async fn spawn_agent(
        &self,
        cwd: &str,
        opts: SpawnOptions, // Struct with model, worktree, mode, rules, etc.
    ) -> Result<SessionId> {
        let id = SessionId::new();
        let metadata = SessionMetadata {
            id: id.clone(),
            cwd: cwd.to_string(),
            worktree: opts.worktree.clone(),
            model: opts.model.clone().unwrap_or_else(|| self.config.default_model.clone()),
            mode: opts.mode.clone(),
            status: SessionStatus::Starting,
            created_at: chrono::Utc::now(),
            last_activity: chrono::Utc::now(),
        };

        let handle = match opts.mode {
            AgentMode::Acp => {
                // Preferred: Long-lived ACP
                let acp_client = Arc::new(AcpClient::new(&self.grok_cli.grok_path, cwd, &opts).await?);
                // Drive initialize, auth, session/new in background task
                let (event_tx, _) = broadcast::channel(100);
                tokio::spawn({
                    let client = acp_client.clone();
                    let tx = event_tx.clone();
                    async move {
                        if let Err(e) = client.run_event_loop(tx).await {
                            eprintln!("ACP event loop error: {}", e);
                        }
                    }
                });
                AgentHandle {
                    metadata,
                    child: None,
                    acp_client: Some(acp_client),
                    event_tx,
                }
            }
            AgentMode::Headless => {
                // For batch/one-shot
                let child = self.grok_cli.spawn_headless(cwd, &opts.prompt.unwrap_or_default(), &opts).await?;
                let (event_tx, _) = broadcast::channel(100);
                AgentHandle {
                    metadata,
                    child: Some(child),
                    acp_client: None,
                    event_tx,
                }
            }
        };

        let mut sessions = self.sessions.write().await;
        sessions.insert(id.clone(), handle);
        
        // Emit event
        self.event_bus.emit_session_created(&id).await;
        
        Ok(id)
    }

    pub async fn get_session(&self, id: &SessionId) -> Option<AgentHandle> {
        let sessions = self.sessions.read().await;
        sessions.get(id).cloned() // Note: For production, use Arc or ref
    }

    pub async fn list_sessions(&self) -> Vec<SessionMetadata> {
        let sessions = self.sessions.read().await;
        sessions.values().map(|h| h.metadata.clone()).collect()
    }

    pub async fn cancel_session(&self, id: &SessionId) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        if let Some(handle) = sessions.get_mut(id) {
            if let Some(ref mut child) = handle.child {
                child.kill().await?;
            }
            if let Some(ref client) = handle.acp_client {
                client.cancel().await?;
            }
            handle.metadata.status = SessionStatus::Cancelled;
            self.event_bus.emit_session_cancelled(id).await;
        }
        Ok(())
    }

    // Additional methods: fork_session, resume, export_transcript, apply_worktree, set_permissions, etc.
    // Full impl would include worktree creation via grok_cli.worktree_create, etc.
    // Persistence: Serialize to SQLite on shutdown, restore on start.
}

// SpawnOptions struct (deep sketch)
#[derive(Debug, Clone)]
pub struct SpawnOptions {
    pub model: Option<String>,
    pub worktree: Option<String>,
    pub mode: AgentMode,
    pub prompt: Option<String>,
    pub rules: Option<String>,
    pub always_approve: bool,
    pub sandbox_profile: Option<String>,
    // ... many more from report flags
}

impl Default for SpawnOptions {
    fn default() -> Self {
        Self {
            model: None,
            worktree: None,
            mode: AgentMode::Acp,
            prompt: None,
            rules: None,
            always_approve: false,
            sandbox_profile: Some("workspace".to_string()),
        }
    }
}

// Example usage in Tauri command:
#[tauri::command]
pub async fn start_new_session(
    registry: tauri::State<'_, Arc<SessionRegistry>>,
    cwd: String,
    opts: SpawnOptions,
) -> Result<String, String> {
    let id = registry.spawn_agent(&cwd, opts).await.map_err(|e| e.to_string())?;
    Ok(id.0.to_string())
}

