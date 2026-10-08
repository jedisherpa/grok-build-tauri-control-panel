//! Broadcast event bus for the control panel backend.
//!
//! Fans out session lifecycle, tool calls, plan updates, and system events
//! to Tauri UI subscribers and internal services.

pub mod diagnostics;

use std::sync::Arc;
mod publication;
pub use publication::*;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum EventError {
    #[error("broadcast lag: {0}")]
    Lagged(u64),
    #[error("channel closed")]
    Closed,
    #[error("event writer unhealthy: {0}")]
    Unhealthy(String),
    #[error("event sink failed: {0}")]
    Sink(String),
    #[error("producer runtime retired or superseded")]
    StaleRuntime,
    #[error("session was deleted")]
    Tombstoned,
    #[error("invalid event origin: {0}")]
    InvalidOrigin(String),
    #[error("durable event sink required")]
    NotDurable,
    #[error("event input exceeds budget: {0}")]
    Bounds(String),
}
pub type SinkError = EventError;

pub type Result<T> = std::result::Result<T, EventError>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Starting,
    Idle,
    Running,
    WaitingApproval,
    Cancelling,
    Cancelled,
    Completed,
    Failed,
    Recovering,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlEvent {
    StoreValueUpdated {
        key: String,
        value: String,
        at: DateTime<Utc>,
    },
    StoreRecoveryObserved {
        lifetime_id: Uuid,
        at: DateTime<Utc>,
    },
    ImportedConversation {
        session_id: Uuid,
        metadata_json: String,
        entries_json: String,
        at: DateTime<Utc>,
    },
    HostOperationIntent {
        operation_id: Uuid,
        session_id: Option<Uuid>,
        kind: String,
        target: String,
        at: DateTime<Utc>,
    },
    HostOperationOutcome {
        operation_id: Uuid,
        session_id: Option<Uuid>,
        kind: String,
        target: String,
        result: String,
        at: DateTime<Utc>,
    },
    SessionMetadataUpdated {
        session_id: Uuid,
        metadata_json: String,
        at: DateTime<Utc>,
    },
    SessionRemoved {
        session_id: Uuid,
        at: DateTime<Utc>,
    },
    RuntimeActivated {
        session_id: Uuid,
        runtime_id: Uuid,
        at: DateTime<Utc>,
    },
    RuntimeRetired {
        session_id: Uuid,
        runtime_id: Uuid,
        at: DateTime<Utc>,
    },
    UserMessage {
        session_id: Uuid,
        operation_id: Uuid,
        text: String,
        at: DateTime<Utc>,
    },
    SessionCreated {
        session_id: Uuid,
        cwd: String,
        mode: String,
        at: DateTime<Utc>,
    },
    SessionStatusChanged {
        session_id: Uuid,
        status: SessionStatus,
        at: DateTime<Utc>,
    },
    SessionCancelled {
        session_id: Uuid,
        at: DateTime<Utc>,
    },
    SessionCompleted {
        session_id: Uuid,
        at: DateTime<Utc>,
    },
    /// Emitted only for a real session/prompt response; Idle alone is not completion.
    PromptFinished {
        session_id: Uuid,
        stop_reason: String,
        at: DateTime<Utc>,
    },
    ToolCall {
        session_id: Uuid,
        event: ToolCallEvent,
    },
    PlanUpdate {
        session_id: Uuid,
        event: PlanUpdateEvent,
    },
    AgentMessage {
        session_id: Uuid,
        text: String,
        at: DateTime<Utc>,
    },
    /// Native message boundaries, separate from the flattened UI transcript.
    AgentOutput {
        session_id: Uuid,
        message_id: Option<String>,
        text: String,
        at: DateTime<Utc>,
    },
    ApprovalRequired {
        session_id: Uuid,
        request_id: String,
        tool: String,
        summary: String,
        options: Vec<PermissionOptionInfo>,
        auto_approved: bool,
        selected_option: Option<String>,
        /// True when this approval presents a plan (enables the
        /// "code with a different model" handoff in the UI).
        #[serde(default)]
        plan_approval: bool,
        at: DateTime<Utc>,
    },
    ApprovalResolved {
        session_id: Uuid,
        request_id: String,
        option_id: Option<String>,
        cancelled: bool,
        at: DateTime<Utc>,
    },
    Error {
        session_id: Option<Uuid>,
        message: String,
        at: DateTime<Utc>,
    },
    SchedulerJob {
        job_id: String,
        message: String,
        at: DateTime<Utc>,
    },
    McpChanged {
        name: String,
        enabled: bool,
        at: DateTime<Utc>,
    },
    MemoryUpdated {
        scope: String,
        at: DateTime<Utc>,
    },
    Raw {
        session_id: Option<Uuid>,
        payload: Value,
    },
}

/// One option offered by the agent in a `session/request_permission` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionOptionInfo {
    pub id: String,
    pub kind: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallEvent {
    pub id: String,
    pub tool: String,
    pub args_summary: String,
    pub status: ToolCallStatus,
    pub result_summary: Option<String>,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanUpdateEvent {
    pub plan_id: Option<String>,
    pub title: Option<String>,
    pub steps: Vec<PlanStep>,
    pub status: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: String,
    pub description: String,
    pub status: String,
}

impl EventBus {
    pub async fn emit_session_created(&self, session_id: Uuid, cwd: &str, mode: &str) {
        self.emit(ControlEvent::SessionCreated {
            session_id,
            cwd: cwd.to_string(),
            mode: mode.to_string(),
            at: Utc::now(),
        });
    }

    pub async fn emit_session_cancelled(&self, session_id: Uuid) {
        self.emit(ControlEvent::SessionCancelled {
            session_id,
            at: Utc::now(),
        });
    }

    pub async fn emit_status(&self, session_id: Uuid, status: SessionStatus) {
        self.emit(ControlEvent::SessionStatusChanged {
            session_id,
            status,
            at: Utc::now(),
        });
    }

    pub fn emit_error(&self, session_id: Option<Uuid>, message: impl Into<String>) {
        self.emit(ControlEvent::Error {
            session_id,
            // Errors are rendered verbatim in the UI: never ship ANSI codes,
            // team IDs or key fragments.
            message: diagnostics::sanitize_diagnostic(&message.into()),
            at: Utc::now(),
        });
    }

    pub fn emit_tool_call(&self, session_id: Uuid, event: ToolCallEvent) {
        self.emit(ControlEvent::ToolCall { session_id, event });
    }

    pub fn emit_plan_update(&self, session_id: Uuid, event: PlanUpdateEvent) {
        self.emit(ControlEvent::PlanUpdate { session_id, event });
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared handle used across crates.
pub type SharedEventBus = Arc<EventBus>;

pub fn shared_bus() -> SharedEventBus {
    Arc::new(EventBus::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn broadcast_reaches_subscriber() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        let id = Uuid::new_v4();
        bus.emit_session_created(id, "/tmp", "acp").await;
        let ev = rx.recv().await.unwrap();
        match ev {
            ControlEvent::SessionCreated {
                session_id, cwd, ..
            } => {
                assert_eq!(session_id, id);
                assert_eq!(cwd, "/tmp");
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
