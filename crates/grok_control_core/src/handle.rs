use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::process::Child;
use tokio::sync::Mutex;
use uuid::Uuid;

use grok_acp::AcpClient;
use grok_events::SessionStatus;

use crate::options::AgentMode;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub id: Uuid,
    pub acp_session_id: Option<String>,
    pub cwd: String,
    pub worktree: Option<String>,
    pub model: String,
    pub mode: AgentMode,
    pub status: SessionStatus,
    pub plan_mode: bool,
    pub always_approve: bool,
    pub sandbox_profile: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub label: Option<String>,
}

/// Serializable snapshot of a handle (no process handles).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHandleSnapshot {
    pub metadata: SessionMetadata,
}

pub struct AgentHandle {
    pub metadata: SessionMetadata,
    pub child: Option<Mutex<Child>>,
    pub acp_client: Option<Arc<AcpClient>>,
}

impl AgentHandle {
    pub fn snapshot(&self) -> AgentHandleSnapshot {
        AgentHandleSnapshot {
            metadata: self.metadata.clone(),
        }
    }

    pub fn touch(&mut self) {
        self.metadata.last_activity = Utc::now();
    }
}
