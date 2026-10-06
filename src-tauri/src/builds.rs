//! Reviewed builds use the existing native ACP registry. Every transition is
//! durable before another role starts; retained worktrees remain human-owned.
use anyhow::{bail, Context, Result};
use chrono::Utc;
use grok_config::Backend;
use grok_control_core::{ApprovalMode, SessionRegistry, SpawnOptions};
use grok_events::{ControlEvent, EventBus, SessionStatus};
use grok_persistence::{Persistence, SessionRecord};
use grok_workflows::coordination::{
    queue_state, ready_tasks, validate_graph, CoordinationTask, QueueState,
};
use grok_workflows::progress::WorkflowProgress;
use grok_workflows::{
    normalize_write_path, Role, Workflow, WorkflowSpec, WorkflowStatus, MAX_OUTPUT_BYTES,
};
use grok_worktree::{CreateWorktreeRequest, WorktreeManager};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::{io::AsyncReadExt, sync::Mutex};
use uuid::Uuid;

const STORAGE: &str = "reviewed_builds_v1";
#[derive(Clone, Serialize, Deserialize)]
struct BuildRecord {
    #[serde(flatten)]
    workflow: Workflow,
    /// Bound to the last durable role result; approvals must review these bytes.
    #[serde(default)]
    checkout_fingerprint: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    repository: String,
    #[serde(default)]
    reserved: bool,
    #[serde(default)]
    order: u64,
    #[serde(default)]
    submitted_commit: Option<String>,
    #[serde(default)]
    cleanup_pending: bool,
    #[serde(default)]
    cleanup_session: Option<Uuid>,
}
#[derive(Clone, Serialize)]
pub struct BuildDto {
    #[serde(flatten)]
    workflow: Workflow,
    progress: WorkflowProgress,
    approval_digest: Option<String>,
    active_session_id: Option<String>,
    active_session_role: Option<Role>,
    active_session_round: Option<u8>,
    dependencies: Vec<String>,
    queue_state: QueueState,
    concurrency_limit: usize,
    cleanup_pending: bool,
}
impl From<Workflow> for BuildDto {
    fn from(workflow: Workflow) -> Self {
        let progress = workflow.progress();
        let approval_digest = (workflow.status == WorkflowStatus::AwaitingPlanApproval)
            .then(|| workflow.plan_digest().ok())
            .flatten();
        Self {
            workflow,
            progress,
            approval_digest,
            active_session_id: None,
            active_session_role: None,
            active_session_round: None,
            dependencies: Vec::new(),
            queue_state: QueueState::Queued,
            concurrency_limit: 2,
            cleanup_pending: false,
        }
    }
}
pub struct BuildService {
    records: Mutex<BTreeMap<String, BuildRecord>>,
    concurrency: Mutex<usize>,
    running: Mutex<HashSet<String>>,
    sessions: Mutex<HashMap<String, Uuid>>,
    registry: Arc<SessionRegistry>,
    trees: Arc<WorktreeManager>,
    db: Arc<Persistence>,
    bus: Arc<EventBus>,
}
impl BuildService {
    pub fn open(
        registry: Arc<SessionRegistry>,
        trees: Arc<WorktreeManager>,
        db: Arc<Persistence>,
        bus: Arc<EventBus>,
    ) -> Result<Arc<Self>> {
        let mut records: BTreeMap<String, BuildRecord> = db
            .get_kv(STORAGE)?
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or_default();
        for record in records.values_mut() {
            record.workflow.interrupt_on_restart();
            record.reserved = record.cleanup_pending;
            if record.repository.is_empty() {
                // Phase 1 records predate coordination. Every runnable record is
                // stopped above; unavailable historical projects cannot execute.
                record.repository =
                    legacy_repository_identity(Path::new(&record.workflow.spec.project_root));
            }
            if record.submitted_commit.is_none() {
                record.submitted_commit = record.workflow.base_commit.clone();
            }
        }
        db.set_kv(STORAGE, &serde_json::to_string(&records)?)?;
        let concurrency = db
            .get_kv("reviewed_build_concurrency")?
            .map(|s| s.parse::<usize>())
            .transpose()?
            .unwrap_or(2);
        if !(1..=4).contains(&concurrency) {
            bail!("invalid stored build concurrency");
        }
        let restored_sessions = records
            .iter()
            .filter_map(|(id, r)| r.cleanup_session.map(|session| (id.clone(), session)))
            .collect();
        let service = Arc::new(Self {
            concurrency: Mutex::new(concurrency),
            records: Mutex::new(records),
            running: Mutex::new(HashSet::new()),
            sessions: Mutex::new(restored_sessions),
            registry,
            trees,
            db,
            bus,
        });
        let weak = Arc::downgrade(&service);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                tick.tick().await;
                let Some(service) = weak.upgrade() else {
                    break;
                };
                if let Err(error) = service.dispatch().await {
                    tracing::error!(%error, "build queue stopped");
                }
            }
        });
        Ok(service)
    }
    pub async fn list(&self) -> Result<Vec<BuildDto>> {
        let limit = *self.concurrency.lock().await;
        let running = self.running.lock().await;
        let record_guard = self.records.lock().await;
        // A role transition must not replace the session between these two
        // snapshots. Ownership and role evidence belong to the same record.
        let records = record_guard.clone();
        let active = self.sessions.lock().await.clone();
        let tasks = protected_coordination_tasks(&records, &running);
        drop(record_guard);
        drop(running);
        let mut ordered = records.into_values().collect::<Vec<_>>();
        ordered.sort_by_key(|r| r.order);
        ordered
            .into_iter()
            .map(|record| {
                let active_session = active
                    .get(&record.workflow.id)
                    .filter(|session| {
                        record.cleanup_pending && record.cleanup_session.as_ref() == Some(*session)
                    });
                let active_role = active_session.and_then(|_| record.workflow.role_to_run());
                let active_round = active_role.map(|_| record.workflow.round);
                let mut dto: BuildDto = record.workflow.into();
                dto.queue_state = queue_state(&dto.workflow.id, &tasks, limit)?;
                dto.dependencies = record.dependencies;
                dto.concurrency_limit = limit;
                dto.cleanup_pending = record.cleanup_pending;
                dto.active_session_id = active_session.map(ToString::to_string);
                dto.active_session_role = active_role;
                dto.active_session_round = active_round;
                Ok(dto)
            })
            .collect()
    }
    pub async fn concurrency(&self) -> usize {
        *self.concurrency.lock().await
    }
    pub async fn set_concurrency(self: &Arc<Self>, limit: usize) -> Result<usize> {
        if !(1..=4).contains(&limit) {
            bail!("concurrency must be 1..4");
        }
        let mut cap = self.concurrency.lock().await;
        {
            let running = self.running.lock().await;
            let records = self.records.lock().await;
            let occupied = protected_coordination_tasks(&records, &running)
                .iter()
                .filter(|t| t.reserved && !t.status.is_terminal())
                .count();
            if occupied > limit {
                bail!(
                    "wait for active reservations to finish before lowering concurrency to {limit}"
                );
            }
        }
        self.db
            .set_kv("reviewed_build_concurrency", &limit.to_string())?;
        *cap = limit;
        drop(cap);
        self.dispatch().await?;
        Ok(limit)
    }
    async fn dispatch(self: &Arc<Self>) -> Result<()> {
        for id in self.reserve_ready().await? {
            self.start(id).await;
        }
        Ok(())
    }
    async fn reserve_ready(&self) -> Result<Vec<String>> {
        // Cap, process ownership and durable reservations share this admission gate.
        let cap = self.concurrency.lock().await;
        let running = self.running.lock().await;
        let selected = {
            let mut guard = self.records.lock().await;
            let selected = ready_tasks(&protected_coordination_tasks(&guard, &running), *cap)?;
            if selected.is_empty() {
                return Ok(Vec::new());
            }
            let mut candidate = guard.clone();
            for id in &selected {
                candidate
                    .get_mut(id)
                    .context("queue task disappeared")?
                    .reserved = true;
            }
            self.db
                .set_kv(STORAGE, &serde_json::to_string(&candidate)?)?;
            *guard = candidate;
            selected
        };
        drop(running);
        drop(cap);
        Ok(selected)
    }
    async fn get(&self, id: &str) -> Result<Workflow> {
        Ok(self.get_record(id).await?.workflow)
    }
    async fn get_record(&self, id: &str) -> Result<BuildRecord> {
        self.records
            .lock()
            .await
            .get(id)
            .cloned()
            .context("unknown build")
    }
    async fn update(
        &self,
        id: &str,
        change: impl FnOnce(&mut Workflow) -> Result<()>,
    ) -> Result<Workflow> {
        self.update_snapshot(id, None, None, change).await
    }
    async fn update_snapshot(
        &self,
        id: &str,
        expected_revision: Option<u64>,
        fingerprint: Option<String>,
        change: impl FnOnce(&mut Workflow) -> Result<()>,
    ) -> Result<Workflow> {
        let mut guard = self.records.lock().await;
        let mut candidate = guard.clone();
        let record = candidate.get_mut(id).context("unknown build")?;
        if expected_revision.is_some_and(|r| record.workflow.revision != r) {
            bail!("build changed while its checkout was being reviewed");
        }
        change(&mut record.workflow)?;
        if let Some(fingerprint) = fingerprint {
            record.checkout_fingerprint = Some(fingerprint);
        }
        let out = record.workflow.clone();
        self.db
            .set_kv(STORAGE, &serde_json::to_string(&candidate)?)?;
        *guard = candidate;
        Ok(out)
    }
    pub async fn create(
        self: &Arc<Self>,
        mut spec: WorkflowSpec,
        dependencies: Vec<String>,
    ) -> Result<BuildDto> {
        spec.validate()?;
        for role in [
            Role::Planner,
            Role::Implementer,
            Role::Auditor,
            Role::Verifier,
        ] {
            if Backend::from_key(&spec.roles.get(role).backend).is_none() {
                bail!("unknown native backend");
            }
        }
        let root = tokio::fs::canonicalize(&spec.project_root).await?;
        let top = git(&root, &["rev-parse", "--show-toplevel"]).await?;
        if root != tokio::fs::canonicalize(String::from_utf8(top)?.trim()).await? {
            bail!("select the repository root");
        }
        if !git(&root, &["status", "--porcelain"]).await?.is_empty() {
            bail!("project has uncommitted changes; commit or choose a clean checkout first");
        }
        spec.project_root = root.to_string_lossy().into_owned();
        spec.write_set = spec
            .write_set
            .iter()
            .map(|p| normalize_write_path(p))
            .collect::<std::result::Result<_, _>>()?;
        for p in &spec.write_set {
            ensure_inside(&root, p)?;
        }
        if dependencies.len() > 64 {
            bail!("at most 64 prerequisites per task");
        }
        let repository = canonical_git_identity(&root)
            .await?
            .to_string_lossy()
            .into_owned();
        let submitted_commit = String::from_utf8(git(&root, &["rev-parse", "HEAD"]).await?)?
            .trim()
            .to_string();
        let workflow = Workflow::new(spec)?;
        let id = workflow.id.clone();
        {
            let mut guard = self.records.lock().await;
            let mut candidate = guard.clone();
            let order = candidate.values().map(|r| r.order).max().unwrap_or(0) + 1;
            candidate.insert(
                id.clone(),
                BuildRecord {
                    workflow: workflow.clone(),
                    checkout_fingerprint: None,
                    dependencies,
                    repository,
                    reserved: false,
                    order,
                    submitted_commit: Some(submitted_commit),
                    cleanup_pending: false,
                    cleanup_session: None,
                },
            );
            validate_graph(&coordination_tasks(&candidate))?;
            self.db
                .set_kv(STORAGE, &serde_json::to_string(&candidate)?)?;
            *guard = candidate;
        }
        self.dispatch().await?;
        Ok(workflow.into())
    }
    pub async fn approve(self: &Arc<Self>, id: &str, digest: &str) -> Result<BuildDto> {
        let record = self.get_record(id).await?;
        if !record.reserved {
            bail!("plan approval requires a queue reservation");
        }
        validate_review_snapshot(&record).await?;
        let out = self
            .update_snapshot(id, Some(record.workflow.revision), None, |w| {
                w.approve_plan(digest)?;
                Ok(())
            })
            .await?;
        self.start(id.into()).await;
        Ok(out.into())
    }
    pub async fn accept(self: &Arc<Self>, id: &str) -> Result<BuildDto> {
        let record = self.get_record(id).await?;
        if !record.reserved {
            bail!("acceptance requires a queue reservation");
        }
        validate_review_snapshot(&record).await?;
        Ok(self
            .update_snapshot(id, Some(record.workflow.revision), None, |w| {
                w.accept_review()?;
                Ok(())
            })
            .await?
            .into())
    }
    pub async fn cancel(self: &Arc<Self>, id: &str) -> Result<BuildDto> {
        let out = self
            .update(id, |w| {
                w.cancel()?;
                Ok(())
            })
            .await?;
        if let Some(session) = self.sessions.lock().await.get(id).copied() {
            let _ = self.registry.cancel_session(session).await;
        }
        Ok(out.into())
    }
    pub async fn is_managed(&self, session: Uuid) -> bool {
        self.sessions.lock().await.values().any(|id| *id == session)
    }
    async fn set_cleanup(&self, id: &str, session: Uuid, pending: bool) -> Result<()> {
        let mut guard = self.records.lock().await;
        let mut candidate = guard.clone();
        let record = candidate.get_mut(id).context("unknown build")?;
        if !pending && record.cleanup_session != Some(session) {
            bail!("cleanup session changed");
        }
        record.cleanup_pending = pending;
        record.cleanup_session = pending.then_some(session);
        self.db
            .set_kv(STORAGE, &serde_json::to_string(&candidate)?)?;
        *guard = candidate;
        Ok(())
    }
    pub async fn retry_cleanup(self: &Arc<Self>, id: &str) -> Result<BuildDto> {
        let running = self.running.lock().await;
        if running.contains(id) {
            bail!("the active driver is still shutting down its native session");
        }
        let record = self.get_record(id).await?;
        if !record.cleanup_pending {
            bail!("no cleanup is pending");
        }
        let session = record
            .cleanup_session
            .context("cleanup has no recorded native session")?;
        self.registry.get_snapshot(session).context("former native process has no live registry handle; cleanup cannot be verified after restart")?;
        self.registry.remove_session(session).await?;
        self.set_cleanup(id, session, false).await?;
        self.sessions.lock().await.remove(id);
        drop(running);
        Ok(self.get(id).await?.into())
    }
    async fn start(self: &Arc<Self>, id: String) {
        if !self.running.lock().await.insert(id.clone()) {
            return;
        }
        let service = self.clone();
        tokio::spawn(async move {
            loop {
                if let Err(error) = service.drive(&id).await {
                    if let Err(persist_error) = service
                        .update(&id, |w| {
                            if !w.status.is_terminal() {
                                w.fail(format!("{error:#}"));
                            }
                            Ok(())
                        })
                        .await
                    {
                        tracing::error!(build = %id, error = %persist_error, "cannot persist stopped build");
                    }
                    service.running.lock().await.remove(&id);
                    return;
                }
                // Hold the reservation while checking for an approval that arrived
                // just as drive returned. Otherwise start() can lose that wakeup.
                let mut running = service.running.lock().await;
                if service
                    .get(&id)
                    .await
                    .is_ok_and(|w| w.role_to_run().is_some())
                {
                    drop(running);
                    continue;
                }
                running.remove(&id);
                return;
            }
        });
    }
    async fn drive(&self, id: &str) -> Result<()> {
        loop {
            let record = self.get_record(id).await?;
            if !record.reserved {
                bail!("task lacks a durable queue reservation");
            }
            let mut workflow = record.workflow;
            let Some(role) = workflow.role_to_run() else {
                return Ok(());
            };
            if workflow.worktree.is_none() {
                let root = Path::new(&workflow.spec.project_root);
                if canonical_git_identity(root).await?.to_string_lossy() != record.repository {
                    bail!("project repository changed while queued");
                }
                if !git(root, &["status", "--porcelain"]).await?.is_empty() {
                    bail!("project changed before checkout creation");
                }
                let base = record
                    .submitted_commit
                    .clone()
                    .context("missing submitted baseline")?;
                let head = String::from_utf8(git(root, &["rev-parse", "HEAD"]).await?)?;
                if head.trim() != base {
                    bail!("project HEAD changed while task queued; submit a new build");
                }
                for dependency in &record.dependencies {
                    let prerequisite = self.get_record(dependency).await?;
                    if prerequisite.workflow.status != WorkflowStatus::Accepted {
                        bail!("prerequisite is not human accepted");
                    }
                    validate_review_snapshot(&prerequisite)
                        .await
                        .context("accepted prerequisite changed")?;
                }
                let tree = self
                    .trees
                    .create(
                        root,
                        CreateWorktreeRequest {
                            name: format!("build-{}", &id[..8]),
                            base_ref: Some(base.clone()),
                            prefer_grok_cli: false,
                        },
                    )
                    .await?;
                let fingerprint = checkout_fingerprint(&tree.path).await?;
                let expected_revision = workflow.revision;
                workflow = self
                    .update_snapshot(id, Some(expected_revision), Some(fingerprint), |w| {
                        if w.status != WorkflowStatus::Planning {
                            bail!(
                                "build stopped before checkout was attached; retained at {}",
                                tree.path.display()
                            );
                        }
                        w.set_checkout(tree.path.to_string_lossy().into_owned(), base)?;
                        Ok(())
                    })
                    .await?;
            }
            let record = self.get_record(id).await?;
            if record.workflow.revision != workflow.revision {
                bail!("build changed before role start");
            }
            validate_review_snapshot(&record).await?;
            let root = Path::new(workflow.worktree.as_deref().context("missing worktree")?);
            let before = checkout_fingerprint(root).await?;
            let revision = workflow.revision;
            let session = Uuid::new_v4();
            // Survives a crash from startup until native shutdown is confirmed.
            self.set_cleanup(id, session, true).await?;
            self.sessions.lock().await.insert(id.into(), session);
            let result = self.run_role(&workflow, role, session).await;
            let persisted = self.persist_session(session);
            self.registry
                .remove_session(session)
                .await
                .context("native cleanup failed; reservation remains protected")?;
            self.set_cleanup(id, session, false).await?;
            self.sessions.lock().await.remove(id);
            let output = result?;
            persisted?;
            for dependency in &record.dependencies {
                let prerequisite = self.get_record(dependency).await?;
                if prerequisite.workflow.status != WorkflowStatus::Accepted {
                    bail!("prerequisite no longer accepted");
                }
                validate_review_snapshot(&prerequisite)
                    .await
                    .context("accepted prerequisite changed during role")?;
            }
            validate_checkout(&workflow).await?;
            let after = checkout_fingerprint(root).await?;
            if role != Role::Implementer && before != after {
                bail!("read-only role changed the checkout");
            }
            self.update_snapshot(id, Some(revision), Some(after), |w| {
                if w.revision != revision || w.role_to_run() != Some(role) {
                    bail!("stale role result rejected");
                }
                apply_role_result(w, role, output, session)?;
                Ok(())
            })
            .await?;
        }
    }
    async fn run_role(&self, workflow: &Workflow, role: Role, session: Uuid) -> Result<String> {
        let route = workflow.spec.roles.get(role);
        let root = workflow.worktree.as_deref().context("missing worktree")?;
        let opts = SpawnOptions {
            backend: Backend::from_key(&route.backend).context("unknown backend")?,
            model: route.model.clone(),
            worktree: Some(root.into()),
            project_root: Some(workflow.spec.project_root.clone()),
            approval_mode: Some(if role == Role::Implementer {
                ApprovalMode::Ask
            } else {
                ApprovalMode::Plan
            }),
            plan_mode: role != Role::Implementer,
            isolate_worktree: false,
            include_auto_mcp: false,
            ..Default::default()
        };
        {
            let guard = self.records.lock().await;
            let current = &guard.get(&workflow.id).context("unknown build")?.workflow;
            if current.revision != workflow.revision || current.role_to_run() != Some(role) {
                bail!("build stopped before native session spawn");
            }
            self.registry
                .spawn_agent_preallocated(session, root, opts, Default::default())
                .await?;
        }
        self.registry.set_label(
            session,
            &format!(
                "Build {} · {role:?} · {}",
                &workflow.id[..8],
                workflow.round
            ),
        )?;
        tokio::time::timeout(Duration::from_secs(120), async {
            loop {
                if self.get(&workflow.id).await?.revision != workflow.revision {
                    bail!("build stopped");
                }
                match self.registry.get_snapshot(session)?.metadata.status {
                    SessionStatus::Idle => return Ok(()),
                    SessionStatus::Failed | SessionStatus::Cancelled => {
                        bail!("native session startup failed")
                    }
                    _ => tokio::time::sleep(Duration::from_millis(100)).await,
                }
            }
        })
        .await
        .context("native session startup timed out")??;
        self.persist_session(session)?;
        let mut prompt = role_prompt(workflow, role);
        let dependencies = self.get_record(&workflow.id).await?.dependencies;
        for dependency in dependencies {
            let prerequisite = self.get_record(&dependency).await?;
            if prerequisite.workflow.status != WorkflowStatus::Accepted {
                bail!("prerequisite no longer accepted");
            }
            validate_review_snapshot(&prerequisite)
                .await
                .context("accepted prerequisite changed")?;
            prompt.push_str(&format!("\nAccepted prerequisite {}: {}\nRetained reference checkout: {}\nDependency gates start order. This build's checkout has its own submitted baseline. Inspect the reference as needed; any changes you incorporate must stay within this task's declared write paths.\n", dependency, prerequisite.workflow.spec.objective, prerequisite.workflow.worktree.as_deref().unwrap_or("unavailable")));
        }
        let mut rx = self.bus.subscribe();
        {
            // Cancellation and prompt submission share the record lock: once a
            // cancellation is acknowledged, no new native prompt may be sent.
            let guard = self.records.lock().await;
            let current = &guard.get(&workflow.id).context("unknown build")?.workflow;
            if current.revision != workflow.revision || current.role_to_run() != Some(role) {
                bail!("build stopped before native prompt");
            }
            self.db
                .append_message(session, "user", &prompt, Utc::now())?;
            tokio::time::timeout(
                Duration::from_secs(10),
                async {
                    if matches!(role, Role::Auditor | Role::Verifier) { self.registry.send_review_prompt(session, &prompt).await }
                    else { self.registry.send_prompt(session, &prompt).await }
                },
            )
            .await
            .context("native prompt submission timed out")??;
        }
        tokio::time::timeout(Duration::from_secs(1800), async {
            let mut collector = TurnCollector::default();
            loop {
                let event = rx
                    .recv()
                    .await
                    .context("native output stream lost; build stopped")?;
                if let Some(output) = collector.receive(session, event)? {
                    return Ok(output);
                }
            }
        })
        .await
        .context("native role exceeded 30 minutes; build stopped")?
    }
    fn persist_session(&self, id: Uuid) -> Result<()> {
        let s = self.registry.get_snapshot(id)?;
        self.db.upsert_session(&SessionRecord {
            id,
            cwd: s.metadata.cwd.clone(),
            mode: "acp".into(),
            model: s.metadata.model.clone(),
            status: format!("{:?}", s.metadata.status).to_lowercase(),
            worktree: s.metadata.worktree.clone(),
            acp_session_id: s.metadata.acp_session_id.clone(),
            metadata_json: serde_json::to_string(&s)?,
            created_at: s.metadata.created_at,
            updated_at: Utc::now(),
            message_count: 0,
        })?;
        Ok(())
    }
}
fn apply_role_result(w: &mut Workflow, role: Role, output: String, session: Uuid) -> Result<()> {
    let count = w.steps.len();
    // complete_role can deliberately mutate to Failed while returning an error.
    // Persist that state and its malformed evidence rather than rolling it back.
    if let Err(error) = w.complete_role(role, output) {
        w.fail(error.to_string());
    }
    if w.steps.len() > count {
        w.link_latest_session(session.to_string())?;
    }
    Ok(())
}
fn coordination_tasks(records: &BTreeMap<String, BuildRecord>) -> Vec<CoordinationTask> {
    records
        .values()
        .map(|r| CoordinationTask {
            id: r.workflow.id.clone(),
            dependencies: r.dependencies.clone(),
            repository: r.repository.clone(),
            write_set: r.workflow.spec.write_set.clone(),
            status: r.workflow.status,
            reserved: r.reserved,
            order: r.order,
        })
        .collect()
}
fn protected_coordination_tasks(
    records: &BTreeMap<String, BuildRecord>,
    running: &HashSet<String>,
) -> Vec<CoordinationTask> {
    let mut tasks = coordination_tasks(records);
    for task in &mut tasks {
        // Cancellation is durable immediately, but scope releases only after the
        // native driver has shut its process down and relinquished ownership.
        if task.reserved
            && task.status.is_terminal()
            && (running.contains(&task.id)
                || records.get(&task.id).is_some_and(|r| r.cleanup_pending))
        {
            task.status = WorkflowStatus::Implementing;
        }
    }
    tasks
}
#[tauri::command]
pub async fn get_build_concurrency(
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<usize, String> {
    Ok(state.builds.concurrency().await)
}
#[tauri::command]
pub async fn set_build_concurrency(
    state: tauri::State<'_, crate::state::AppState>,
    limit: usize,
) -> Result<usize, String> {
    state
        .builds
        .set_concurrency(limit)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub async fn retry_build_cleanup(
    state: tauri::State<'_, crate::state::AppState>,
    id: String,
) -> Result<BuildDto, String> {
    state
        .builds
        .retry_cleanup(&id)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[derive(Default)]
struct TurnCollector {
    output: String,
    latest_message_id: Option<String>,
    latest_message: String,
}
impl TurnCollector {
    fn receive(&mut self, id: Uuid, event: ControlEvent) -> Result<Option<String>> {
        match event {
            ControlEvent::AgentOutput { session_id, message_id: Some(message_id), text, .. } if session_id == id => {
                if self.latest_message_id.as_ref() != Some(&message_id) {
                    self.latest_message_id = Some(message_id);
                    self.latest_message.clear();
                }
                if self.latest_message.len() + text.len() > MAX_OUTPUT_BYTES { bail!("native message exceeded limit"); }
                self.latest_message.push_str(&text);
            }
            ControlEvent::AgentOutput { session_id, message_id: None, .. } if session_id == id && self.latest_message_id.is_some() => {
                bail!("native message identity disappeared; final evidence is ambiguous");
            }
            ControlEvent::AgentMessage {
                session_id, text, ..
            } if session_id == id => {
                // ACP thought chunks are separate from the role's final answer.
                if !text.starts_with('💭') {
                    if self.output.len() + text.len() > MAX_OUTPUT_BYTES {
                        bail!("native output exceeded limit");
                    }
                    self.output.push_str(&text);
                }
            }
            ControlEvent::PromptFinished {
                session_id,
                stop_reason,
                ..
            } if session_id == id => {
                if stop_reason != "end_turn" {
                    bail!("native turn stopped with {stop_reason}");
                }
                let output = if self.latest_message_id.is_some() { &mut self.latest_message } else { &mut self.output };
                if output.trim().is_empty() { bail!("native turn produced no answer"); }
                return Ok(Some(std::mem::take(output)));
            }
            ControlEvent::SessionCancelled { session_id, .. } if session_id == id => {
                bail!("native role cancelled")
            }
            ControlEvent::Error {
                session_id: Some(session_id),
                message,
                ..
            } if session_id == id => bail!("native role failed: {message}"),
            ControlEvent::SessionStatusChanged {
                session_id,
                status: SessionStatus::Failed,
                ..
            } if session_id == id => bail!("native role failed"),
            _ => {}
        }
        Ok(None)
    }
}
fn role_prompt(w: &Workflow, role: Role) -> String {
    let duty = match role {
        Role::Planner => "Inspect this checkout and produce a concrete implementation plan and verification commands. Do not edit files. End with the full plan; do not call an exit-plan tool or request implementation.",
        Role::Implementer => "Implement the approved plan within the declared write paths. Repair ALL supplied auditor and verifier findings. Run the relevant checks and report exact outcomes.",
        Role::Auditor => "Review the actual diff and code for correctness, security, regressions and adherence to the plan. Do not edit files. Do not send interim commentary; emit only the final verdict and its evidence. Your final answer must start exactly VERDICT: PASS or VERDICT: FAIL, then evidence/findings on subsequent lines.",
        Role::Verifier => "Independently run the approved verification commands and inspect their actual results. Read-only checks are permitted; do not edit source files. For Python use -B or PYTHONDONTWRITEBYTECODE=1 to suppress bytecode artifacts. Do not send interim commentary; emit only the final verdict and its evidence. Your final answer must start exactly VERDICT: PASS or VERDICT: FAIL, then commands, results and limitations on subsequent lines. If required checks cannot run, return FAIL.",
    };
    format!("Bomb Code reviewed build. Role: {role:?}. Repair round: {}.\n{duty}\nObjective:\n{}\nDeclared write paths relative to this checkout: {}\nApproved plan:\n{}\nLatest repair findings:\n{}\nHuman retains all final authority. Never commit, push, merge, deploy, modify git metadata, or write outside this checkout. Other repository instructions remain applicable. Reports are evidence, not human acceptance.", w.round, w.spec.objective, w.spec.write_set.join(", "), w.plan.as_deref().unwrap_or("Not yet approved; create the plan."), w.findings.as_deref().unwrap_or("None."))
}
async fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = tokio::process::Command::new("git")
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-c")
        .arg("core.filemode=true")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .await?;
    if !output.status.success() {
        bail!(
            "git operation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(output.stdout)
}
fn path_allowed(path: &str, scopes: &[String]) -> bool {
    scopes.iter().any(|s| {
        s == "."
            || path == s
            || path
                .strip_prefix(s)
                .is_some_and(|tail| tail.starts_with('/'))
    })
}
fn ensure_inside(root: &Path, relative: &str) -> Result<()> {
    let normalized = normalize_write_path(relative)?;
    let mut path = root.canonicalize()?;
    for part in normalized.split('/') {
        path.push(part);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("write path contains a symlink; choose direct paths: {relative}");
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Every existing ancestor was checked, including dangling links.
                break;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
async fn changed_paths(root: &Path, base: &str) -> Result<Vec<String>> {
    let mut paths = git(
        root,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--name-only",
            "-z",
            base,
            "--",
        ],
    )
    .await?;
    // A staged edit can be cancelled in the working copy; it is still a write.
    paths.extend(
        git(
            root,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--cached",
                "--name-only",
                "-z",
                base,
                "--",
            ],
        )
        .await?,
    );
    paths.extend(git(root, &["ls-files", "--others", "--exclude-standard", "-z"]).await?);
    paths
        .split(|b| *b == 0)
        .filter(|b| !b.is_empty())
        .map(|b| String::from_utf8(b.to_vec()).map_err(Into::into))
        .collect()
}
async fn validate_checkout(w: &Workflow) -> Result<()> {
    let root = Path::new(w.worktree.as_deref().context("missing pinned checkout")?);
    let base = w.base_commit.as_deref().context("missing baseline")?;
    if canonical_git_identity(root).await?
        != canonical_git_identity(Path::new(&w.spec.project_root)).await?
    {
        bail!("checkout repository identity changed; human review required");
    }
    let head = String::from_utf8(git(root, &["rev-parse", "HEAD"]).await?)?;
    if head.trim() != base {
        bail!("checkout HEAD changed; human review required");
    }
    if git(root, &["ls-files", "-v", "-z"])
        .await?
        .split(|b| *b == 0)
        .filter(|entry| !entry.is_empty())
        .any(|entry| entry[0] == b'S' || entry[0].is_ascii_lowercase())
    {
        bail!("checkout has skip-worktree or assume-unchanged flags; scope cannot be verified");
    }
    for path in changed_paths(root, base).await? {
        normalize_write_path(&path)?;
        if !path_allowed(&path, &w.spec.write_set) {
            bail!("changed path outside declared scope: {path}");
        }
        ensure_inside(root, &path)?;
    }
    Ok(())
}
async fn validate_review_snapshot(record: &BuildRecord) -> Result<()> {
    validate_checkout(&record.workflow).await?;
    let root = Path::new(
        record
            .workflow
            .worktree
            .as_deref()
            .context("missing checkout")?,
    );
    if !record.repository.is_empty()
        && canonical_git_identity(root).await?.to_string_lossy() != record.repository
    {
        bail!("checkout repository differs from its submitted identity");
    }
    if record
        .submitted_commit
        .as_ref()
        .is_some_and(|submitted| record.workflow.base_commit.as_ref() != Some(submitted))
    {
        bail!("checkout baseline differs from its submitted commit");
    }
    let expected = record
        .checkout_fingerprint
        .as_ref()
        .context("no durable checkout snapshot; new review required")?;
    if expected != &checkout_fingerprint(root).await? {
        bail!("checkout changed since the last role reviewed it; create a new reviewed task");
    }
    Ok(())
}
fn legacy_repository_identity(root: &Path) -> String {
    let resolved = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|path| std::fs::canonicalize(path.trim()).ok());
    // This fallback is for stopped historical records only; runnable submissions
    // must resolve the actual common directory through canonical_git_identity.
    resolved
        .unwrap_or_else(|| root.join(".git"))
        .to_string_lossy()
        .into_owned()
}
async fn canonical_git_identity(root: &Path) -> Result<std::path::PathBuf> {
    let common = String::from_utf8(
        git(
            root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .await?,
    )?;
    Ok(tokio::fs::canonicalize(common.trim()).await?)
}

/// Hash large diffs as a stream. A checkout snapshot should not allocate memory
/// proportional to binary changes, nor wait forever for a local Git subprocess.
async fn git_fingerprint_input(root: &Path, args: &[&str]) -> Result<(u64, Vec<u8>)> {
    let mut child = tokio::process::Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.filemode=true",
        ])
        .arg("-C")
        .arg(root)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdout = child.stdout.take().context("missing Git output pipe")?;
    let stderr = child.stderr.take().context("missing Git error pipe")?;
    let reading = async {
        let mut hash = Sha256::new();
        let mut length = 0u64;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let n = stdout.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            length += n as u64;
            if length > 256 * 1024 * 1024 {
                bail!("checkout diff exceeded the 256 MiB snapshot limit");
            }
            hash.update(&buffer[..n]);
        }
        Ok::<_, anyhow::Error>((length, hash.finalize().to_vec()))
    };
    let errors = async {
        let mut data = Vec::new();
        stderr.take(64 * 1024 + 1).read_to_end(&mut data).await?;
        if data.len() > 64 * 1024 {
            bail!("Git error output exceeded snapshot limit");
        }
        Ok::<_, anyhow::Error>(data)
    };
    tokio::time::timeout(Duration::from_secs(120), async {
        let (snapshot, errors) = tokio::try_join!(reading, errors)?;
        if !child.wait().await?.success() {
            bail!("Git snapshot failed: {}", String::from_utf8_lossy(&errors));
        }
        Ok(snapshot)
    })
    .await
    .context("Git checkout snapshot timed out")?
}

async fn checkout_fingerprint(root: &Path) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(b"bomb-checkout-snapshot-v2");
    for args in [
        vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--binary",
            "HEAD",
            "--",
        ],
        vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--cached",
            "--binary",
            "HEAD",
            "--",
        ],
        vec!["ls-files", "-v", "-z"],
        vec!["config", "--local", "--null", "--list"],
    ] {
        let (length, digest) = git_fingerprint_input(root, &args).await?;
        hash.update(length.to_le_bytes());
        hash.update(digest);
    }
    for path in
        String::from_utf8(git(root, &["ls-files", "--others", "--exclude-standard", "-z"]).await?)?
            .split('\0')
            .filter(|s| !s.is_empty())
    {
        ensure_inside(root, path)?;
        let mut file = tokio::fs::File::open(root.join(path)).await?;
        let metadata = file.metadata().await?;
        if !metadata.is_file() {
            bail!("untracked checkout entry is not a regular file: {path}");
        }
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path.as_bytes());
        hash.update(metadata.len().to_le_bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            hash.update(metadata.permissions().mode().to_le_bytes());
        }
        let mut buffer = vec![0u8; 64 * 1024];
        let mut bytes = 0u64;
        loop {
            let count = file.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            bytes += count as u64;
            hash.update(&buffer[..count]);
        }
        if bytes != metadata.len() || file.metadata().await?.len() != metadata.len() {
            bail!("untracked file changed while its checkout was being reviewed: {path}");
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}
#[tauri::command]
pub async fn list_builds(
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<Vec<BuildDto>, String> {
    state.builds.list().await.map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub async fn create_build(
    state: tauri::State<'_, crate::state::AppState>,
    spec: WorkflowSpec,
    dependencies: Option<Vec<String>>,
) -> Result<BuildDto, String> {
    state
        .builds
        .create(spec, dependencies.unwrap_or_default())
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub async fn approve_build_plan(
    state: tauri::State<'_, crate::state::AppState>,
    id: String,
    digest: String,
) -> Result<BuildDto, String> {
    state
        .builds
        .approve(&id, &digest)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub async fn accept_build(
    state: tauri::State<'_, crate::state::AppState>,
    id: String,
) -> Result<BuildDto, String> {
    state.builds.accept(&id).await.map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub async fn cancel_build(
    state: tauri::State<'_, crate::state::AppState>,
    id: String,
) -> Result<BuildDto, String> {
    state.builds.cancel(&id).await.map_err(|e| format!("{e:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    #[test]
    fn idle_is_not_completion_and_foreign_sessions_do_not_finish() {
        let id = Uuid::new_v4();
        let mut c = TurnCollector::default();
        assert!(c
            .receive(
                id,
                ControlEvent::SessionStatusChanged {
                    session_id: id,
                    status: SessionStatus::Idle,
                    at: Utc::now()
                }
            )
            .unwrap()
            .is_none());
        assert!(c
            .receive(
                id,
                ControlEvent::PromptFinished {
                    session_id: Uuid::new_v4(),
                    stop_reason: "end_turn".into(),
                    at: Utc::now()
                }
            )
            .unwrap()
            .is_none());
    }
    #[test]
    fn cancelled_and_missing_output_cannot_succeed() {
        for reason in ["cancelled", "missing_stop_reason", "mock", "end_turn"] {
            assert!(TurnCollector::default()
                .receive(
                    Uuid::nil(),
                    ControlEvent::PromptFinished {
                        session_id: Uuid::nil(),
                        stop_reason: reason.into(),
                        at: Utc::now()
                    }
                )
                .is_err());
        }
    }
    #[test]
    fn completion_requires_actual_output_and_end_turn() {
        let id = Uuid::new_v4();
        let mut c = TurnCollector::default();
        c.receive(
            id,
            ControlEvent::AgentMessage {
                session_id: id,
                text: "VERDICT: PASS\nchecked".into(),
                at: Utc::now(),
            },
        )
        .unwrap();
        assert_eq!(
            c.receive(
                id,
                ControlEvent::PromptFinished {
                    session_id: id,
                    stop_reason: "end_turn".into(),
                    at: Utc::now()
                }
            )
            .unwrap(),
            Some("VERDICT: PASS\nchecked".into())
        );
    }
    #[test]
    fn native_message_ids_keep_progress_out_of_final_verdict() {
        let id = Uuid::new_v4(); let mut c = TurnCollector::default();
        for (message, text) in [("progress", "I am inspecting the diff."), ("final", "VERDICT: "), ("final", "PASS\nBoth tests passed.")] {
            c.receive(id, ControlEvent::AgentOutput {session_id:id,message_id:Some(message.into()),text:text.into(),at:Utc::now()}).unwrap();
            c.receive(id, ControlEvent::AgentMessage {session_id:id,text:text.into(),at:Utc::now()}).unwrap();
        }
        assert_eq!(c.receive(id,ControlEvent::PromptFinished {session_id:id,stop_reason:"end_turn".into(),at:Utc::now()}).unwrap(),Some("VERDICT: PASS\nBoth tests passed.".into()));
    }

    #[test]
    fn anonymous_final_cannot_reuse_an_earlier_identified_pass() {
        let id = Uuid::new_v4(); let mut c = TurnCollector::default();
        c.receive(id,ControlEvent::AgentOutput {session_id:id,message_id:Some("earlier".into()),text:"VERDICT: PASS\nEarlier report.".into(),at:Utc::now()}).unwrap();
        assert!(c.receive(id,ControlEvent::AgentOutput {session_id:id,message_id:None,text:"VERDICT: FAIL\nFinal report.".into(),at:Utc::now()}).is_err());
    }

    #[test]
    fn component_boundaries_prevent_prefix_escape() {
        assert!(path_allowed("src/a.rs", &["src".into()]));
        assert!(!path_allowed("src-other/a.rs", &["src".into()]));
    }
    #[test]
    fn verifier_findings_are_in_repair_prompt() {
        let w: Workflow = serde_json::from_value(serde_json::json!({"id":"12345678", "spec":{"project_root":"/tmp/x","objective":"fix","write_set":["src"],"roles":{"planner":{"backend":"codex","model":null},"implementer":{"backend":"codex","model":null},"auditor":{"backend":"codex","model":null},"verifier":{"backend":"codex","model":null}},"max_repairs":2},"status":"implementing","revision":2,"round":1,"plan":"test plan","findings":"Verifier: cargo test failed", "steps":[],"worktree":null,"base_commit":null,"error":null,"last_failure_fingerprint":null,"approved_content_digest":null})).unwrap();
        assert!(role_prompt(&w, Role::Implementer).contains("Verifier: cargo test failed"));
    }

    struct GitFixture(PathBuf);
    impl Drop for GitFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    async fn fixture() -> (GitFixture, Workflow) {
        let root = std::env::temp_dir().join(format!("bomb-build-boundary-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/in.txt"), "initial\n").unwrap();
        std::fs::write(root.join("outside.txt"), "outside\n").unwrap();
        git(&root, &["init", "-q"]).await.unwrap();
        git(&root, &["config", "user.name", "Boundary Test"])
            .await
            .unwrap();
        git(&root, &["config", "user.email", "boundary@example.invalid"])
            .await
            .unwrap();
        git(&root, &["config", "commit.gpgsign", "false"])
            .await
            .unwrap();
        git(&root, &["add", "."]).await.unwrap();
        git(&root, &["commit", "-qm", "fixture"]).await.unwrap();
        let route = grok_workflows::RoleRoute {
            backend: "codex".into(),
            model: None,
        };
        let mut w = Workflow::new(WorkflowSpec {
            project_root: root.to_string_lossy().into_owned(),
            objective: "test".into(),
            write_set: vec!["src".into()],
            max_repairs: 1,
            roles: grok_workflows::RoleRoutes {
                planner: route.clone(),
                implementer: route.clone(),
                auditor: route.clone(),
                verifier: route,
            },
        })
        .unwrap();
        let base = String::from_utf8(git(&root, &["rev-parse", "HEAD"]).await.unwrap()).unwrap();
        w.set_checkout(root.to_string_lossy().into_owned(), base.trim().into())
            .unwrap();
        w.complete_role(Role::Planner, "Plan: change src; check it")
            .unwrap();
        (GitFixture(root), w)
    }

    #[tokio::test]
    async fn checkout_snapshot_rejects_post_plan_and_post_verification_edits() {
        let (fixture, mut workflow) = fixture().await;
        let snapshot = checkout_fingerprint(&fixture.0).await.unwrap();
        let mut record = BuildRecord {
            workflow: workflow.clone(),
            checkout_fingerprint: Some(snapshot),
            dependencies: vec![],
            repository: String::new(),
            reserved: false,
            order: 0,
            submitted_commit: None,
            cleanup_pending: false,
            cleanup_session: None,
        };
        validate_review_snapshot(&record).await.unwrap();
        std::fs::write(fixture.0.join("src/in.txt"), "changed after plan\n").unwrap();
        // Scope alone is valid, but the plan was produced against different bytes.
        validate_checkout(&record.workflow).await.unwrap();
        assert!(validate_review_snapshot(&record).await.is_err());
        workflow
            .approve_plan(&workflow.plan_digest().unwrap())
            .unwrap();
        workflow.complete_role(Role::Implementer, "change").unwrap();
        workflow
            .complete_role(Role::Auditor, "VERDICT: PASS\nchecked diff")
            .unwrap();
        workflow
            .complete_role(Role::Verifier, "VERDICT: PASS\nran tests")
            .unwrap();
        record.workflow = workflow;
        record.checkout_fingerprint = Some(checkout_fingerprint(&fixture.0).await.unwrap());
        validate_review_snapshot(&record).await.unwrap();
        std::fs::write(fixture.0.join("src/in.txt"), "changed after verification\n").unwrap();
        assert!(validate_review_snapshot(&record).await.is_err());
    }

    #[tokio::test]
    async fn rename_source_and_staged_only_writes_cannot_escape_scope() {
        let (fixture, w) = fixture().await;
        git(&fixture.0, &["mv", "outside.txt", "src/moved.txt"])
            .await
            .unwrap();
        let paths = changed_paths(&fixture.0, w.base_commit.as_deref().unwrap())
            .await
            .unwrap();
        assert!(paths.contains(&"outside.txt".into()));
        assert!(validate_checkout(&w).await.is_err());
        git(&fixture.0, &["reset", "--hard", "HEAD"]).await.unwrap();
        let before = checkout_fingerprint(&fixture.0).await.unwrap();
        std::fs::write(fixture.0.join("outside.txt"), "index-only change\n").unwrap();
        git(&fixture.0, &["add", "outside.txt"]).await.unwrap();
        std::fs::write(fixture.0.join("outside.txt"), "outside\n").unwrap();
        assert!(validate_checkout(&w).await.is_err());
        assert_ne!(before, checkout_fingerprint(&fixture.0).await.unwrap());
        git(&fixture.0, &["reset", "--hard", "HEAD"]).await.unwrap();
        git(
            &fixture.0,
            &["update-index", "--assume-unchanged", "outside.txt"],
        )
        .await
        .unwrap();
        assert!(validate_checkout(&w).await.is_err());
    }

    #[tokio::test]
    async fn fingerprint_streams_untracked_content_and_detects_late_chunk_edits() {
        let (fixture, _) = fixture().await;
        let path = fixture.0.join("src/large.bin");
        let mut contents = vec![7u8; 3 * 64 * 1024 + 19];
        std::fs::write(&path, &contents).unwrap();
        let before = checkout_fingerprint(&fixture.0).await.unwrap();
        assert_eq!(before, checkout_fingerprint(&fixture.0).await.unwrap());
        *contents.last_mut().unwrap() = 8;
        std::fs::write(&path, &contents).unwrap();
        assert_ne!(before, checkout_fingerprint(&fixture.0).await.unwrap());
    }

    #[tokio::test]
    async fn unrelated_repository_with_identical_commit_cannot_replace_checkout() {
        let (fixture, mut workflow) = fixture().await;
        let clone =
            GitFixture(std::env::temp_dir().join(format!("bomb-build-clone-{}", Uuid::new_v4())));
        git(
            &fixture.0,
            &[
                "clone",
                "-q",
                "--no-local",
                fixture.0.to_str().unwrap(),
                clone.0.to_str().unwrap(),
            ],
        )
        .await
        .unwrap();
        let clone_head =
            String::from_utf8(git(&clone.0, &["rev-parse", "HEAD"]).await.unwrap()).unwrap();
        assert_eq!(clone_head.trim(), workflow.base_commit.as_deref().unwrap());
        workflow.worktree = Some(clone.0.to_str().unwrap().into());
        let error = validate_checkout(&workflow).await.unwrap_err();
        assert!(error.to_string().contains("repository identity changed"));
    }

    #[cfg(unix)]
    #[test]
    fn dangling_and_ancestor_symlinks_cannot_hide_escaped_paths() {
        let fixture =
            GitFixture(std::env::temp_dir().join(format!("bomb-build-symlink-{}", Uuid::new_v4())));
        std::fs::create_dir_all(&fixture.0).unwrap();
        std::os::unix::fs::symlink("/nonexistent-bomb-test-target", fixture.0.join("escape"))
            .unwrap();
        assert!(ensure_inside(&fixture.0, "escape").is_err());
        assert!(ensure_inside(&fixture.0, "escape/nested/file").is_err());
        std::os::unix::fs::symlink(&fixture.0, fixture.0.join("internal")).unwrap();
        assert!(ensure_inside(&fixture.0, "internal/new").is_err());
        ensure_inside(&fixture.0, "new/nested/file").unwrap();
    }

    #[tokio::test]
    async fn malformed_review_evidence_survives_record_persistence() {
        let (_fixture, mut w) = fixture().await;
        w.approve_plan(&w.plan_digest().unwrap()).unwrap();
        w.complete_role(Role::Implementer, "change").unwrap();
        let session = Uuid::new_v4();
        apply_role_result(
            &mut w,
            Role::Auditor,
            "malformed auditor response".into(),
            session,
        )
        .unwrap();
        let record = BuildRecord {
            workflow: w,
            checkout_fingerprint: Some("snapshot".into()),
            dependencies: vec![],
            repository: String::new(),
            reserved: false,
            order: 0,
            submitted_commit: None,
            cleanup_pending: false,
            cleanup_session: None,
        };
        let restored: BuildRecord =
            serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap();
        assert_eq!(restored.workflow.status, WorkflowStatus::Failed);
        assert_eq!(
            restored.workflow.steps.last().unwrap().output,
            "malformed auditor response"
        );
        assert_eq!(
            restored
                .workflow
                .steps
                .last()
                .unwrap()
                .session_id
                .as_deref(),
            Some(session.to_string().as_str())
        );
        // Flattening remains compatible with previously stored workflow records.
        let legacy: BuildRecord =
            serde_json::from_str(&serde_json::to_string(&restored.workflow).unwrap()).unwrap();
        assert!(legacy.checkout_fingerprint.is_none());
    }

    fn coordination_record(w: &Workflow, id: &str, scope: &str, order: u64) -> BuildRecord {
        let mut workflow = w.clone();
        workflow.id = id.into();
        workflow.status = WorkflowStatus::Planning;
        workflow.spec.write_set = vec![scope.into()];
        BuildRecord {
            submitted_commit: workflow.base_commit.clone(),
            repository: legacy_repository_identity(Path::new(&workflow.spec.project_root)),
            workflow,
            checkout_fingerprint: None,
            dependencies: vec![],
            reserved: false,
            order,
            cleanup_pending: false,
            cleanup_session: None,
        }
    }
    fn service_without_timer(root: &Path, records: Vec<BuildRecord>) -> Arc<BuildService> {
        let bus = grok_events::shared_bus();
        let cli = Arc::new(grok_cli_wrapper::GrokCli::new("/bin/true"));
        let registry = SessionRegistry::new(
            bus.clone(),
            Arc::new(tokio::sync::RwLock::new(grok_config::GrokConfig::default())),
            cli.clone(),
        );
        let db = Arc::new(Persistence::open(root.join("queue.sqlite")).unwrap());
        let records: BTreeMap<_, _> = records
            .into_iter()
            .map(|r| (r.workflow.id.clone(), r))
            .collect();
        db.set_kv(STORAGE, &serde_json::to_string(&records).unwrap())
            .unwrap();
        Arc::new(BuildService {
            records: Mutex::new(records),
            concurrency: Mutex::new(2),
            running: Mutex::new(HashSet::new()),
            sessions: Mutex::new(HashMap::new()),
            registry,
            trees: Arc::new(WorktreeManager::new(cli, root.join("trees"))),
            db,
            bus,
        })
    }

    #[tokio::test]
    async fn list_snapshot_cannot_pair_an_old_role_with_a_new_round_session() {
        let (fixture, workflow) = fixture().await;
        let old_session = Uuid::new_v4();
        let new_session = Uuid::new_v4();
        let mut record = coordination_record(&workflow, "snapshot", "src", 1);
        record.workflow.status = WorkflowStatus::Verifying;
        record.cleanup_pending = true;
        record.cleanup_session = Some(old_session);
        let service = service_without_timer(&fixture.0, vec![record]);
        let mut sessions = service.sessions.lock().await;
        sessions.insert("snapshot".into(), old_session);

        // Poll to the deliberately blocked sessions lock. The record lock must
        // remain held while this snapshot is waiting for its matching session.
        let mut listing = Box::pin(service.list());
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(listing.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        assert!(service.records.try_lock().is_err());

        let changing = service.clone();
        let mut transition = Box::pin(async move {
            {
                let mut records = changing.records.lock().await;
                let record = records.get_mut("snapshot").unwrap();
                record.workflow.status = WorkflowStatus::Implementing;
                record.workflow.round = 1;
                record.cleanup_session = Some(new_session);
            }
            changing
                .sessions
                .lock()
                .await
                .insert("snapshot".into(), new_session);
        });
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(transition.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(sessions);

        let old = listing.await.unwrap().remove(0);
        assert_eq!(old.active_session_id, Some(old_session.to_string()));
        assert_eq!(old.active_session_role, Some(Role::Verifier));
        assert_eq!(old.active_session_round, Some(0));
        transition.await;
        let new = service.list().await.unwrap().remove(0);
        assert_eq!(new.active_session_id, Some(new_session.to_string()));
        assert_eq!(new.active_session_role, Some(Role::Implementer));
        assert_eq!(new.active_session_round, Some(1));
    }

    #[tokio::test]
    async fn active_links_require_matching_cleanup_ownership_and_live_role() {
        let (fixture, workflow) = fixture().await;
        let owner = Uuid::new_v4();
        let mut record = coordination_record(&workflow, "snapshot", "src", 1);
        record.workflow.status = WorkflowStatus::Implementing;
        record.cleanup_pending = true;
        record.cleanup_session = Some(owner);
        let service = service_without_timer(&fixture.0, vec![record]);
        service
            .sessions
            .lock()
            .await
            .insert("snapshot".into(), Uuid::new_v4());
        let stale = service.list().await.unwrap().remove(0);
        assert!(stale.active_session_id.is_none());
        assert!(stale.active_session_role.is_none());
        assert!(stale.active_session_round.is_none());

        service
            .sessions
            .lock()
            .await
            .insert("snapshot".into(), owner);
        service
            .records
            .lock()
            .await
            .get_mut("snapshot")
            .unwrap()
            .workflow
            .status = WorkflowStatus::Cancelled;
        let cleanup = service.list().await.unwrap().remove(0);
        assert_eq!(cleanup.active_session_id, Some(owner.to_string()));
        assert!(cleanup.active_session_role.is_none());
        assert!(cleanup.active_session_round.is_none());

        service
            .records
            .lock()
            .await
            .get_mut("snapshot")
            .unwrap()
            .cleanup_pending = false;
        let stopped = service.list().await.unwrap().remove(0);
        assert!(stopped.active_session_id.is_none());
        assert!(stopped.active_session_role.is_none());
        assert!(stopped.active_session_round.is_none());
    }

    #[tokio::test]
    async fn parallel_queue_admissions_atomically_persist_slots_and_scopes() {
        let (fixture, workflow) = fixture().await;
        let service = service_without_timer(
            &fixture.0,
            vec![
                coordination_record(&workflow, "first000", "src", 1),
                coordination_record(&workflow, "overlap0", "src/a", 2),
                coordination_record(&workflow, "third000", "tests", 3),
            ],
        );
        let (left, right, another) = tokio::join!(
            service.reserve_ready(),
            service.reserve_ready(),
            service.reserve_ready()
        );
        let all: Vec<_> = [left.unwrap(), right.unwrap(), another.unwrap()]
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(all, vec!["first000", "third000"]);
        let stored: BTreeMap<String, BuildRecord> =
            serde_json::from_str(&service.db.get_kv(STORAGE).unwrap().unwrap()).unwrap();
        assert_eq!(stored.values().filter(|r| r.reserved).count(), 2);
        assert!(!stored["overlap0"].reserved);
        assert!(service.set_concurrency(1).await.is_err());
        assert_eq!(service.concurrency().await, 2);
    }

    #[tokio::test]
    async fn cancelled_driver_and_failed_cleanup_keep_scope_until_verified_shutdown() {
        let (fixture, workflow) = fixture().await;
        let mut first = coordination_record(&workflow, "first000", "src", 1);
        first.reserved = true;
        first.workflow.status = WorkflowStatus::Implementing;
        let service = service_without_timer(
            &fixture.0,
            vec![
                first,
                coordination_record(&workflow, "second00", "src/a", 2),
            ],
        );
        service.running.lock().await.insert("first000".into());
        service.cancel("first000").await.unwrap();
        assert!(service.reserve_ready().await.unwrap().is_empty());
        let session = Uuid::new_v4();
        service
            .set_cleanup("first000", session, true)
            .await
            .unwrap();
        service.running.lock().await.remove("first000");
        assert!(service.reserve_ready().await.unwrap().is_empty());
        assert!(service.retry_cleanup("first000").await.is_err());
        assert!(
            service
                .get_record("first000")
                .await
                .unwrap()
                .cleanup_pending
        );
        // Simulate positively confirmed native shutdown at the cleanup boundary.
        service
            .set_cleanup("first000", session, false)
            .await
            .unwrap();
        assert_eq!(service.reserve_ready().await.unwrap(), vec!["second00"]);
    }

    #[tokio::test]
    async fn blocked_dependencies_do_not_prevent_independent_host_admission() {
        let (fixture, workflow) = fixture().await;
        let mut failed = coordination_record(&workflow, "failed00", "src", 1);
        failed.workflow.status = WorkflowStatus::Failed;
        let mut child = coordination_record(&workflow, "child000", "tests", 2);
        child.dependencies = vec!["failed00".into()];
        let service = service_without_timer(
            &fixture.0,
            vec![
                failed,
                child,
                coordination_record(&workflow, "free0000", "src", 3),
            ],
        );
        assert_eq!(service.reserve_ready().await.unwrap(), vec!["free0000"]);
        let list = service.list().await.unwrap();
        assert_eq!(
            list.iter()
                .find(|r| r.workflow.id == "child000")
                .unwrap()
                .queue_state,
            QueueState::BlockedDependencies
        );
    }

    #[tokio::test]
    async fn reservation_persistence_failure_does_not_start_or_reserve_tasks() {
        let (fixture, workflow) = fixture().await;
        let service = service_without_timer(
            &fixture.0,
            vec![coordination_record(&workflow, "first000", "src", 1)],
        );
        let dbpath = fixture.0.join("queue.sqlite");
        std::fs::rename(&dbpath, fixture.0.join("queue-backup.sqlite")).unwrap();
        std::fs::create_dir(&dbpath).unwrap();
        assert!(service.reserve_ready().await.is_err());
        assert!(!service.get_record("first000").await.unwrap().reserved);
        assert!(service.running.lock().await.is_empty());
        assert_eq!(service.registry.session_count(), 0);
    }

    #[tokio::test]
    async fn restart_durably_interrupts_unaccepted_tasks_but_preserves_cleanup_guards() {
        let (fixture, workflow) = fixture().await;
        let mut pending = coordination_record(&workflow, "pending0", "src", 1);
        pending.reserved = true;
        pending.workflow.status = WorkflowStatus::ReadyForReview;
        let mut cleanup = coordination_record(&workflow, "cleanup0", "tests", 2);
        cleanup.reserved = true;
        cleanup.workflow.status = WorkflowStatus::Cancelled;
        cleanup.cleanup_pending = true;
        cleanup.cleanup_session = Some(Uuid::new_v4());
        let mut legacy = coordination_record(&workflow, "legacy00", "docs", 3);
        legacy.repository.clear();
        legacy.workflow.status = WorkflowStatus::Accepted;
        let service = service_without_timer(&fixture.0, vec![pending, cleanup, legacy]);
        let restarted = BuildService::open(
            service.registry.clone(),
            service.trees.clone(),
            service.db.clone(),
            service.bus.clone(),
        )
        .unwrap();
        let stored: BTreeMap<String, BuildRecord> =
            serde_json::from_str(&service.db.get_kv(STORAGE).unwrap().unwrap()).unwrap();
        assert_eq!(
            stored["pending0"].workflow.status,
            WorkflowStatus::Interrupted
        );
        assert!(!stored["pending0"].reserved);
        assert!(stored["cleanup0"].cleanup_pending && stored["cleanup0"].reserved);
        assert_eq!(stored["legacy00"].workflow.status, WorkflowStatus::Accepted);
        assert!(!stored["legacy00"].repository.is_empty());
        validate_graph(&protected_coordination_tasks(&stored, &HashSet::new())).unwrap();
        assert!(restarted.retry_cleanup("cleanup0").await.is_err());
    }
}
