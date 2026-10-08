use super::{ControlEvent, EventError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreIdentity {
    pub store_id: Uuid,
    pub generation: Uuid,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventOrigin {
    pub session_id: Option<Uuid>,
    pub runtime_id: Option<Uuid>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectionDelta {
    pub transcripts: Vec<TranscriptDelta>,
    pub statuses: Vec<StatusDelta>,
    pub deleted_sessions: Vec<Uuid>,
    #[serde(default)]
    pub baseline_changed_sessions: Vec<Uuid>,
    #[serde(default)]
    pub baseline_changed_all: bool,
    #[serde(default)]
    pub operations: Vec<OperationRecord>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationRecord {
    #[serde(default)]
    pub submission: bool,
    pub operation_id: Uuid,
    pub session_id: Option<Uuid>,
    pub kind: String,
    pub target: String,
    pub intent_seq: u64,
    pub outcome_seq: Option<u64>,
    pub result: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptDelta {
    pub session_id: Uuid,
    pub seq: u64,
    pub role: String,
    pub body: String,
    pub at: String,
    pub append: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusDelta {
    pub session_id: Uuid,
    pub status: String,
    pub runtime_id: Option<Uuid>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommittedEvent {
    pub store_id: Uuid,
    pub generation: Uuid,
    pub seq: u64,
    pub origin: EventOrigin,
    pub event: ControlEvent,
    #[serde(default)]
    pub projection: ProjectionDelta,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventHealth {
    pub durable: bool,
    pub healthy: bool,
    pub error: Option<String>,
    pub store_id: Uuid,
    pub generation: Uuid,
    pub last_seq: u64,
}

/// Synchronous transaction boundary. Implementations must not emit recursively
/// or acquire a registry/service lock. Success means journal and projections
/// committed atomically, not merely that the work was queued.
pub trait EventSink: Send + Sync {
    fn commit(&self, origin: &EventOrigin, event: &ControlEvent) -> Result<CommittedEvent>;
    fn identity(&self) -> StoreIdentity;
    fn durable(&self) -> bool {
        true
    }
    fn watermark(&self) -> Result<u64> {
        Ok(0)
    }
}
pub struct MemorySink {
    identity: StoreIdentity,
    seq: Mutex<u64>,
}
impl MemorySink {
    pub fn new() -> Self {
        Self {
            identity: StoreIdentity {
                store_id: Uuid::new_v4(),
                generation: Uuid::new_v4(),
            },
            seq: Mutex::new(0),
        }
    }
}
impl Default for MemorySink {
    fn default() -> Self {
        Self::new()
    }
}
impl EventSink for MemorySink {
    fn identity(&self) -> StoreIdentity {
        self.identity
    }
    fn durable(&self) -> bool {
        false
    }
    fn commit(&self, origin: &EventOrigin, event: &ControlEvent) -> Result<CommittedEvent> {
        let mut seq = self
            .seq
            .lock()
            .map_err(|_| EventError::Sink("memory sink poisoned".into()))?;
        *seq += 1;
        Ok(CommittedEvent {
            store_id: self.identity.store_id,
            generation: self.identity.generation,
            seq: *seq,
            origin: *origin,
            event: event.clone(),
            projection: ProjectionDelta::default(),
        })
    }
}
struct State {
    sink: Arc<dyn EventSink>,
    failure: Option<String>,
    last_seq: u64,
    published: bool,
    runtimes: HashMap<Uuid, Uuid>,
    statuses: HashMap<Uuid, (Uuid, super::SessionStatus)>,
}
struct Pipeline {
    state: Mutex<State>,
    raw: broadcast::Sender<ControlEvent>,
    committed: broadcast::Sender<CommittedEvent>,
}
pub struct EventBus {
    pipeline: Arc<Pipeline>,
    origin: EventOrigin,
}
impl std::fmt::Debug for EventBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventBus")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}
impl EventBus {
    pub fn new() -> Self {
        Self::with_capacity(8192)
    }
    pub fn with_capacity(capacity: usize) -> Self {
        let (raw, _) = broadcast::channel(capacity.max(1));
        let (committed, _) = broadcast::channel(capacity.max(1));
        Self {
            pipeline: Arc::new(Pipeline {
                state: Mutex::new(State {
                    sink: Arc::new(MemorySink::new()),
                    failure: None,
                    last_seq: 0,
                    published: false,
                    runtimes: HashMap::new(),
                    statuses: HashMap::new(),
                }),
                raw,
                committed,
            }),
            origin: EventOrigin::default(),
        }
    }
    pub fn origin(&self) -> EventOrigin {
        self.origin
    }
    /// Host services may derive a facade only from an already captured committed origin.
    pub fn scope_for_origin(&self, origin: EventOrigin) -> Result<Arc<EventBus>> {
        if self.origin.session_id.is_some() {
            return Err(EventError::InvalidOrigin(
                "only host root derives producer scopes".into(),
            ));
        }
        let (Some(session), Some(runtime)) = (origin.session_id, origin.runtime_id) else {
            return Err(EventError::InvalidOrigin(
                "session runtime origin required".into(),
            ));
        };
        let state = self
            .pipeline
            .state
            .lock()
            .map_err(|_| EventError::Unhealthy("publication lock poisoned".into()))?;
        if state.runtimes.get(&session) != Some(&runtime) {
            return Err(EventError::StaleRuntime);
        }
        if let Some(error) = &state.failure {
            return Err(EventError::Unhealthy(error.clone()));
        }
        Ok(Arc::new(Self {
            pipeline: self.pipeline.clone(),
            origin,
        }))
    }
    /// Bounded mirror of committed status for live runtimes only; no raw producer input.
    pub fn current_status(&self, session: Uuid) -> Option<(Uuid, super::SessionStatus)> {
        let state = self.pipeline.state.lock().ok()?;
        let (runtime, status) = *state.statuses.get(&session)?;
        (state.runtimes.get(&session) == Some(&runtime)).then_some((runtime, status))
    }
    pub fn runtime_id(&self) -> Option<Uuid> {
        self.origin.runtime_id
    }
    pub fn subscribe(&self) -> broadcast::Receiver<ControlEvent> {
        self.pipeline.raw.subscribe()
    }
    pub fn subscribe_committed(&self) -> broadcast::Receiver<CommittedEvent> {
        self.pipeline.committed.subscribe()
    }
    pub fn receiver_count(&self) -> usize {
        self.pipeline.raw.receiver_count()
    }
    pub fn install_sink(&self, sink: Arc<dyn EventSink>) -> Result<()> {
        let mut state = self
            .pipeline
            .state
            .lock()
            .map_err(|_| EventError::Unhealthy("publication lock poisoned".into()))?;
        if state.published || state.failure.is_some() {
            return Err(EventError::InvalidOrigin(
                "sink must be installed before any producer".into(),
            ));
        }
        state.last_seq = sink.watermark()?;
        state.sink = sink;
        Ok(())
    }
    pub fn health(&self) -> EventHealth {
        let lock = self.pipeline.state.lock();
        let poisoned = lock.is_err();
        let state = lock.unwrap_or_else(|e| e.into_inner());
        let identity = state.sink.identity();
        EventHealth {
            durable: state.sink.durable(),
            healthy: !poisoned && state.failure.is_none(),
            error: if poisoned {
                Some("publication lock poisoned".into())
            } else {
                state.failure.clone()
            },
            store_id: identity.store_id,
            generation: identity.generation,
            last_seq: state.last_seq,
        }
    }
    /// Pins writer health/watermark while a reader opens its consistent database transaction.
    /// The closure must only read its persistence owner and must not call back into this bus.
    pub fn with_read_boundary<T>(&self, read: impl FnOnce(EventHealth) -> T) -> Result<T> {
        let state = self
            .pipeline
            .state
            .lock()
            .map_err(|_| EventError::Unhealthy("publication lock poisoned".into()))?;
        let identity = state.sink.identity();
        let health = EventHealth {
            durable: state.sink.durable(),
            healthy: state.failure.is_none(),
            error: state.failure.clone(),
            store_id: identity.store_id,
            generation: identity.generation,
            last_seq: state.last_seq,
        };
        Ok(read(health))
    }
    /// Accepted native output that cannot be durably represented is lost coverage,
    /// unlike an invalid request rejected before effect admission. Only a current
    /// host-created producer scope may fail this shared pipeline.
    pub fn mark_coverage_failed(&self, reason: impl AsRef<str>) -> Result<()> {
        let (Some(session), Some(runtime)) = (self.origin.session_id, self.origin.runtime_id)
        else {
            return Err(EventError::InvalidOrigin(
                "current scoped producer required for coverage failure".into(),
            ));
        };
        let mut state = self
            .pipeline
            .state
            .lock()
            .map_err(|_| EventError::Unhealthy("publication lock poisoned".into()))?;
        if state.runtimes.get(&session) != Some(&runtime) {
            return Err(EventError::StaleRuntime);
        }
        if state.failure.is_none() {
            state.failure = Some(super::diagnostics::sanitize_diagnostic(reason.as_ref()));
        }
        Ok(())
    }
    pub fn ensure_healthy(&self) -> Result<()> {
        let state = self
            .pipeline
            .state
            .lock()
            .map_err(|_| EventError::Unhealthy("publication lock poisoned".into()))?;
        if let Some(error) = &state.failure {
            return Err(EventError::Unhealthy(error.clone()));
        }
        if let Some(id) = self.origin.session_id {
            if self.origin.runtime_id.is_some()
                && state.runtimes.get(&id) != self.origin.runtime_id.as_ref()
            {
                return Err(EventError::StaleRuntime);
            }
        }
        Ok(())
    }
    pub fn ensure_durable(&self) -> Result<()> {
        self.ensure_healthy()?;
        if !self.health().durable {
            return Err(EventError::NotDurable);
        }
        Ok(())
    }
    fn commit_locked(
        &self,
        state: &mut State,
        origin: EventOrigin,
        event: ControlEvent,
    ) -> Result<CommittedEvent> {
        if let Some(error) = &state.failure {
            return Err(EventError::Unhealthy(error.clone()));
        }
        let result = state.sink.commit(&origin, &event);
        match result {
            Ok(envelope) => {
                state.last_seq = state.last_seq.max(envelope.seq);
                state.published = true;
                if matches!(envelope.event, ControlEvent::StoreRecoveryObserved { .. }) {
                    state.statuses.clear();
                }
                for status in &envelope.projection.statuses {
                    if let (Some(runtime), Some(value)) =
                        (status.runtime_id, parse_status(&status.status))
                    {
                        state.statuses.insert(status.session_id, (runtime, value));
                    }
                }
                if let (Some(id), Some(runtime)) =
                    (envelope.origin.session_id, envelope.origin.runtime_id)
                {
                    let value = match &envelope.event {
                        ControlEvent::RuntimeActivated { .. } => {
                            Some(super::SessionStatus::Starting)
                        }
                        ControlEvent::SessionStatusChanged { status, .. } => Some(*status),
                        ControlEvent::SessionCancelled { .. } => {
                            Some(super::SessionStatus::Cancelled)
                        }
                        ControlEvent::SessionCompleted { .. } => {
                            Some(super::SessionStatus::Completed)
                        }
                        _ => None,
                    };
                    if let Some(value) = value {
                        state.statuses.insert(id, (runtime, value));
                    }
                }
                Ok(envelope)
            }
            Err(
                error @ (EventError::StaleRuntime
                | EventError::Tombstoned
                | EventError::InvalidOrigin(_)
                | EventError::Bounds(_)),
            ) => Err(error),
            Err(error) => {
                state.failure = Some(super::diagnostics::sanitize_diagnostic(&error.to_string()));
                Err(error)
            }
        }
    }
    fn notify(&self, envelope: &CommittedEvent) {
        let _ = self.pipeline.committed.send(envelope.clone());
        let _ = self.pipeline.raw.send(envelope.event.clone());
    }
    /// Called once after physical profile ownership and durable sink installation,
    /// before any producer. Records prior owner loss without certifying child cleanup.
    pub fn begin_owned_lifetime(&self) -> Result<CommittedEvent> {
        if self.origin.session_id.is_some() {
            return Err(EventError::InvalidOrigin(
                "lifetime recovery is host-only".into(),
            ));
        }
        let mut state = self
            .pipeline
            .state
            .lock()
            .map_err(|_| EventError::Unhealthy("publication lock poisoned".into()))?;
        if state.published || !state.runtimes.is_empty() {
            return Err(EventError::InvalidOrigin(
                "lifetime recovery must precede producers".into(),
            ));
        }
        if !state.sink.durable() {
            return Err(EventError::NotDurable);
        }
        let event = ControlEvent::StoreRecoveryObserved {
            lifetime_id: Uuid::new_v4(),
            at: Utc::now(),
        };
        let envelope = self.commit_locked(&mut state, EventOrigin::default(), event)?;
        self.notify(&envelope);
        Ok(envelope)
    }
    pub fn register_runtime(&self, session_id: Uuid) -> Result<Arc<EventBus>> {
        if self.origin.runtime_id.is_some() {
            return Err(EventError::InvalidOrigin(
                "only the host registers runtime generations".into(),
            ));
        }
        let runtime_id = Uuid::new_v4();
        let origin = EventOrigin {
            session_id: Some(session_id),
            runtime_id: Some(runtime_id),
        };
        let mut state = self
            .pipeline
            .state
            .lock()
            .map_err(|_| EventError::Unhealthy("publication lock poisoned".into()))?;
        let event = ControlEvent::RuntimeActivated {
            session_id,
            runtime_id,
            at: Utc::now(),
        };
        let envelope = self.commit_locked(&mut state, origin, event)?;
        state.runtimes.insert(session_id, runtime_id);
        self.notify(&envelope);
        Ok(Arc::new(Self {
            pipeline: self.pipeline.clone(),
            origin,
        }))
    }
    pub fn retire_runtime(&self, expected_runtime_id: Uuid) -> Result<()> {
        let mut state = self
            .pipeline
            .state
            .lock()
            .map_err(|_| EventError::Unhealthy("publication lock poisoned".into()))?;
        let session_id = state
            .runtimes
            .iter()
            .find_map(|(session, runtime)| (*runtime == expected_runtime_id).then_some(*session))
            .ok_or(EventError::StaleRuntime)?;
        if self
            .origin
            .runtime_id
            .is_some_and(|id| id != expected_runtime_id)
        {
            return Err(EventError::StaleRuntime);
        }
        let origin = EventOrigin {
            session_id: Some(session_id),
            runtime_id: Some(expected_runtime_id),
        };
        let event = ControlEvent::RuntimeRetired {
            session_id,
            runtime_id: expected_runtime_id,
            at: Utc::now(),
        };
        let envelope = self.commit_locked(&mut state, origin, event)?;
        state.runtimes.remove(&session_id);
        state.statuses.remove(&session_id);
        self.notify(&envelope);
        Ok(())
    }
    pub fn emit_checked(&self, event: ControlEvent) -> Result<CommittedEvent> {
        if matches!(
            event,
            ControlEvent::RuntimeActivated { .. }
                | ControlEvent::RuntimeRetired { .. }
                | ControlEvent::StoreRecoveryObserved { .. }
        ) {
            return Err(EventError::InvalidOrigin(
                "runtime activation is host-owned".into(),
            ));
        }
        let session = event.session_id();
        if self.origin.session_id.is_some() && session != self.origin.session_id {
            return Err(EventError::InvalidOrigin(
                "event does not match producer session".into(),
            ));
        }
        let mut state = self
            .pipeline
            .state
            .lock()
            .map_err(|_| EventError::Unhealthy("publication lock poisoned".into()))?;
        if let Some(session) = session {
            match (state.runtimes.get(&session), self.origin.runtime_id) {
                (Some(active), Some(runtime)) if *active == runtime => {}
                (None, None) => {}
                (Some(_), None) if self.origin.session_id.is_none() && event.is_host_control() => {}
                _ => return Err(EventError::StaleRuntime),
            }
        }
        let origin = EventOrigin {
            session_id: session,
            runtime_id: self.origin.runtime_id,
        };
        let envelope = self.commit_locked(&mut state, origin, event)?;
        self.notify(&envelope);
        Ok(envelope)
    }
    pub fn emit(&self, event: ControlEvent) {
        if let Err(error) = self.emit_checked(event) {
            tracing::warn!(error=%error,"event rejected before notification");
        }
    }
}
fn parse_status(value: &str) -> Option<super::SessionStatus> {
    use super::SessionStatus::*;
    Some(match value {
        "starting" => Starting,
        "idle" => Idle,
        "running" => Running,
        "waitingapproval" | "waiting_approval" => WaitingApproval,
        "cancelling" => Cancelling,
        "cancelled" => Cancelled,
        "completed" => Completed,
        "failed" => Failed,
        "recovering" => Recovering,
        _ => return None,
    })
}
impl ControlEvent {
    pub fn is_host_control(&self) -> bool {
        matches!(
            self,
            Self::ImportedConversation { .. }
                | Self::SessionMetadataUpdated { .. }
                | Self::SessionRemoved { .. }
                | Self::HostOperationIntent { .. }
                | Self::HostOperationOutcome { .. }
        )
    }
    pub fn session_id(&self) -> Option<Uuid> {
        use ControlEvent::*;
        match self {
            ImportedConversation { session_id, .. }
            | RuntimeActivated { session_id, .. }
            | RuntimeRetired { session_id, .. }
            | UserMessage { session_id, .. }
            | SessionCreated { session_id, .. }
            | SessionStatusChanged { session_id, .. }
            | SessionCancelled { session_id, .. }
            | SessionCompleted { session_id, .. }
            | PromptFinished { session_id, .. }
            | ToolCall { session_id, .. }
            | PlanUpdate { session_id, .. }
            | AgentMessage { session_id, .. }
            | SessionMetadataUpdated { session_id, .. }
            | SessionRemoved { session_id, .. }
            | AgentOutput { session_id, .. }
            | ApprovalRequired { session_id, .. }
            | ApprovalResolved { session_id, .. } => Some(*session_id),
            Error { session_id, .. }
            | Raw { session_id, .. }
            | HostOperationIntent { session_id, .. }
            | HostOperationOutcome { session_id, .. } => *session_id,
            _ => None,
        }
    }
}
