use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use grok_cli_wrapper::process::{ProcessHandle, ProcessOutcome};
use uuid::Uuid;

use grok_acp::{AcpClient, BrainMode};
use grok_config::Backend;
use grok_events::SessionStatus;

use crate::options::AgentMode;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetadata {
    pub id: Uuid,
    /// Current host runtime identity, informational when serialized; never restored as authority.
    #[serde(default)]
    pub runtime_id: Option<Uuid>,
    pub acp_session_id: Option<String>,
    pub cwd: String,
    pub worktree: Option<String>,
    /// Original project folder when cwd is a thread worktree.
    #[serde(default)]
    pub project_root: Option<String>,
    pub model: String,
    /// Agent backend this session runs on; old records default to grok.
    #[serde(default)]
    pub backend: Backend,
    pub mode: AgentMode,
    pub status: SessionStatus,
    /// Approval stance for this session (plan | ask | auto | yolo).
    /// The booleans below are kept for back-compat with older records.
    #[serde(default)]
    pub approval_mode: grok_acp::ApprovalMode,
    pub plan_mode: bool,
    #[serde(default)]
    pub read_only: bool,
    pub always_approve: bool,
    pub sandbox_profile: Option<String>,
    #[serde(default)]
    pub permission_allow: Vec<String>,
    #[serde(default)]
    pub permission_deny: Vec<String>,
    #[serde(default)]
    pub rules: Vec<String>,
    #[serde(default)]
    pub trust_repo: bool,
    #[serde(default)]
    pub process_outcome: Option<ProcessOutcome>,
    /// MCP server names attached at spawn time.
    pub mcp_servers: Vec<String>,
    /// High-risk MCP approvals granted for this session (persisted so resume
    /// doesn't silently drop approved servers).
    #[serde(default)]
    pub approved_high_risk_mcp: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub label: Option<String>,
    /// full_brain | history_only | fresh — agent context recovery mode.
    #[serde(default)]
    pub brain_mode: BrainMode,
}

/// Serializable snapshot of a handle (no process handles).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHandleSnapshot {
    pub metadata: SessionMetadata,
}

pub struct AgentHandle {
    /// Host-owned producer scope; saved metadata never selects this authority.
    pub(crate) event_bus: Arc<grok_events::EventBus>,
    /// Physical observation is acknowledged only after its checked durable receipt.
    pub(crate) process_receipt_committed: bool,
    pub metadata: SessionMetadata,
    pub child: Option<ProcessHandle>,
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
