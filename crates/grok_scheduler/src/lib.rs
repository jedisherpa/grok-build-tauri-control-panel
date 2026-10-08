//! Existing routines with observed outcomes and serialized durable admission.
//! A run ID records intent, not exactly-once external effects. Interrupted or
//! uncertain runs never replay automatically.
use chrono::{DateTime, Utc};
use grok_cli_wrapper::process::{ProcessEnd, ProcessOutcome};
use grok_events::{ControlEvent, EventBus};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use thiserror::Error;
use tokio::sync::{watch, Mutex, Notify, RwLock};
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("job not found: {0}")]
    NotFound(String),
    #[error("invalid schedule: {0}")]
    InvalidSchedule(String),
    #[error("job already running: {0}")]
    AlreadyRunning(String),
    #[error("scheduler persistence unavailable: {0}")]
    Persistence(String),
    #[error("run requires deliberate recovery: {0}")]
    Recovery(String),
}
pub type Result<T> = std::result::Result<T, SchedulerError>;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleKind {
    Interval { secs: u64 },
    Cron { expr: String },
    Once { delay_secs: u64 },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Scheduled,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
    Cancelling,
    Interrupted,
    Uncertain,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobOutcome {
    pub process: Option<ProcessOutcome>,
    pub error: Option<String>,
    pub cleanup_complete: bool,
}
impl JobOutcome {
    pub fn from_process(process: ProcessOutcome) -> Self {
        Self {
            cleanup_complete: process.cleanup_complete,
            error: process.error.clone(),
            process: Some(process),
        }
    }
    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            process: None,
            error: Some(error.into()),
            cleanup_complete: true,
        }
    }
    fn success(&self) -> bool {
        self.cleanup_complete
            && self.error.is_none()
            && self
                .process
                .as_ref()
                .is_some_and(|process| process.end == ProcessEnd::Success)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRun {
    pub run_id: String,
    pub session_id: Uuid,
    pub admitted_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub outcome: Option<JobOutcome>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledJob {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub cwd: Option<String>,
    pub schedule: ScheduleKind,
    pub status: JobStatus,
    pub created_at: DateTime<Utc>,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
    /// Number of durably admitted attempts, not successful completions.
    pub run_count: u64,
    pub max_runs: Option<u64>,
    #[serde(default)]
    pub active_run: Option<JobRun>,
    #[serde(default)]
    pub runs: Vec<JobRun>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub control_revision: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerSnapshot {
    pub version: u32,
    pub revision: u64,
    pub jobs: Vec<ScheduledJob>,
}
#[derive(Clone, Debug)]
pub struct RunCancellation(watch::Sender<bool>);
impl RunCancellation {
    pub fn requested(&self) -> bool {
        *self.0.borrow()
    }
    pub async fn cancelled(&self) {
        let mut rx = self.0.subscribe();
        while !*rx.borrow_and_update() {
            if rx.changed().await.is_err() {
                return;
            }
        }
    }
    fn request(&self) {
        self.0.send_replace(true);
    }
}
#[derive(Clone)]
pub struct JobRunContext {
    pub job: ScheduledJob,
    pub run_id: String,
    pub session_id: Uuid,
    pub cancel: RunCancellation,
}
type JobFuture = std::pin::Pin<
    Box<dyn std::future::Future<Output = std::result::Result<JobOutcome, String>> + Send>,
>;
type JobFn = dyn Fn(JobRunContext) -> JobFuture + Send + Sync;
#[derive(Clone)]
pub struct JobHandler {
    inner: Arc<JobFn>,
}
impl JobHandler {
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: Fn(JobRunContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = std::result::Result<JobOutcome, String>> + Send + 'static,
    {
        Self {
            inner: Arc::new(move |run| Box::pin(f(run))),
        }
    }
    pub async fn call(&self, run: JobRunContext) -> std::result::Result<JobOutcome, String> {
        (self.inner)(run).await
    }
}
type CleanupFn = dyn Fn(Uuid) -> JobFuture + Send + Sync;
#[derive(Clone)]
pub struct JobCleanupHandler {
    inner: Arc<CleanupFn>,
}
impl JobCleanupHandler {
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: Fn(Uuid) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = std::result::Result<JobOutcome, String>> + Send + 'static,
    {
        Self {
            inner: Arc::new(move |id| Box::pin(f(id))),
        }
    }
    async fn call(&self, id: Uuid) -> std::result::Result<JobOutcome, String> {
        (self.inner)(id).await
    }
}
struct ActiveRun {
    run_id: String,
    cancel: RunCancellation,
    done: watch::Sender<Option<JobOutcome>>,
}
type ChangeHook = dyn Fn(SchedulerSnapshot) -> std::result::Result<(), String> + Send + Sync;
pub struct Scheduler {
    jobs: RwLock<HashMap<String, ScheduledJob>>,
    event_bus: Arc<EventBus>,
    handler: Mutex<Option<JobHandler>>,
    cleanup_handler: Mutex<Option<JobCleanupHandler>>,
    on_change: Mutex<Option<Arc<ChangeHook>>>,
    transition: Mutex<()>,
    revision: Mutex<u64>,
    active: Mutex<HashMap<String, ActiveRun>>,
    wake: Arc<Notify>,
    shutting_down: AtomicBool,
}
impl Scheduler {
    pub fn new(event_bus: Arc<EventBus>) -> Arc<Self> {
        Arc::new(Self {
            jobs: RwLock::new(HashMap::new()),
            event_bus,
            handler: Mutex::new(None),
            cleanup_handler: Mutex::new(None),
            on_change: Mutex::new(None),
            transition: Mutex::new(()),
            revision: Mutex::new(0),
            active: Mutex::new(HashMap::new()),
            wake: Arc::new(Notify::new()),
            shutting_down: AtomicBool::new(false),
        })
    }
    /// Fence all future admission before protective Stop; a deadline does not
    /// discard the existing process or run owners.
    pub fn fence_admission(&self) {
        self.shutting_down.store(true, Ordering::Release);
        self.wake.notify_waiters();
    }
    pub async fn shutdown(self: &Arc<Self>) -> Result<()> {
        self.fence_admission();
        let ids = {
            let _gate = self.transition.lock().await;
            self.shutting_down.store(true, Ordering::Release);
            self.wake.notify_waiters();
            let mut ids = self.active.lock().await.keys().cloned().collect::<Vec<_>>();
            for job in self.jobs.read().await.values() {
                if (job
                    .active_run
                    .as_ref()
                    .is_some_and(|run| run.outcome.is_some())
                    || job
                        .runs
                        .last()
                        .and_then(|run| run.outcome.as_ref())
                        .is_some_and(|outcome| !outcome.cleanup_complete))
                    && !ids.contains(&job.id)
                {
                    ids.push(job.id.clone());
                }
            }
            ids
        };
        self.stop_runs(ids).await
    }
    /// User Stop-all pauses current future routines; explicit Resume and new
    /// routines remain available after cleanup, unlike the exit lifetime fence.
    pub async fn stop_all(self: &Arc<Self>) -> Result<()> {
        let (ids, persistence_error) = {
            let _gate = self.transition.lock().await;
            let mut candidate = self.jobs.read().await.clone();
            for job in candidate
                .values_mut()
                .filter(|job| job.status == JobStatus::Scheduled)
            {
                job.status = JobStatus::Paused;
                job.control_revision += 1;
            }
            let error = self.commit(candidate).await.err();
            if let Some(error) = &error {
                for job in self
                    .jobs
                    .write()
                    .await
                    .values_mut()
                    .filter(|job| job.status == JobStatus::Scheduled)
                {
                    job.status = JobStatus::Uncertain;
                    job.next_run = None;
                    job.error = Some(error.to_string());
                }
                self.wake.notify_waiters();
            }
            (
                self.active.lock().await.keys().cloned().collect::<Vec<_>>(),
                error,
            )
        };
        let cleanup = self.stop_runs(ids).await;
        match (persistence_error, cleanup) {
            (None, result) => result,
            (Some(error), Ok(())) => Err(error),
            (Some(error), Err(cleanup)) => {
                Err(SchedulerError::Recovery(format!("{error}; {cleanup}")))
            }
        }
    }
    pub async fn retained_session_ids(&self) -> std::collections::HashSet<Uuid> {
        self.jobs
            .read()
            .await
            .values()
            .filter_map(|job| {
                job.active_run
                    .as_ref()
                    .or_else(|| {
                        job.runs.last().filter(|run| {
                            run.outcome
                                .as_ref()
                                .is_some_and(|outcome| !outcome.cleanup_complete)
                        })
                    })
                    .map(|run| run.session_id)
            })
            .collect()
    }
    async fn stop_runs(self: &Arc<Self>, ids: Vec<String>) -> Result<()> {
        let mut tasks = Vec::new();
        for id in ids {
            let this = self.clone();
            tasks.push(tokio::spawn(async move { this.cancel(&id).await }));
        }
        let stopped = tokio::time::timeout(Duration::from_secs(31), async {
            let mut errors = Vec::new();
            for task in tasks {
                match task.await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => errors.push(error.to_string()),
                    Err(error) => errors.push(format!("Stop observer failed: {error}")),
                }
            }
            errors
        })
        .await
        .map_err(|_| {
            SchedulerError::Recovery(
                "shutdown deadline exceeded; run owners remain retained".into(),
            )
        })?;
        if stopped.is_empty() {
            Ok(())
        } else {
            Err(SchedulerError::Recovery(stopped.join("; ")))
        }
    }
    pub async fn set_handler(&self, handler: JobHandler) {
        *self.handler.lock().await = Some(handler);
    }
    pub async fn set_cleanup_handler(&self, handler: JobCleanupHandler) {
        *self.cleanup_handler.lock().await = Some(handler);
    }
    pub async fn set_change_hook<F>(&self, hook: F)
    where
        F: Fn(SchedulerSnapshot) -> std::result::Result<(), String> + Send + Sync + 'static,
    {
        *self.on_change.lock().await = Some(Arc::new(hook));
    }
    /// Caller holds transition. Save the exact candidate before publishing it.
    async fn commit(&self, candidate: HashMap<String, ScheduledJob>) -> Result<()> {
        let hook =
            self.on_change.lock().await.clone().ok_or_else(|| {
                SchedulerError::Persistence("no persistence hook configured".into())
            })?;
        let mut revision = self.revision.lock().await;
        let mut jobs: Vec<_> = candidate.values().cloned().collect();
        jobs.sort_by(|a, b| a.id.cmp(&b.id));
        hook(SchedulerSnapshot {
            version: 2,
            revision: *revision + 1,
            jobs,
        })
        .map_err(SchedulerError::Persistence)?;
        *self.jobs.write().await = candidate;
        *revision += 1;
        self.wake.notify_waiters();
        Ok(())
    }
    pub async fn list(&self) -> Vec<ScheduledJob> {
        let mut jobs: Vec<_> = self.jobs.read().await.values().cloned().collect();
        jobs.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        jobs
    }
    pub async fn restore_jobs(self: &Arc<Self>, specs: Vec<ScheduledJob>) -> Result<()> {
        let _gate = self.transition.lock().await;
        let mut candidate = HashMap::new();
        for mut job in specs {
            validate_schedule(&job.schedule)?;
            if candidate.contains_key(&job.id) {
                return Err(SchedulerError::Recovery("duplicate saved job ID".into()));
            }
            if job.active_run.is_some()
                || matches!(job.status, JobStatus::Running | JobStatus::Cancelling)
            {
                job.status = JobStatus::Interrupted;
                job.next_run = None;
                job.error=Some("A prior admitted run has no durable terminal outcome. Inspect its session and effects before explicitly resuming; nothing was replayed.".into());
            }
            candidate.insert(job.id.clone(), job);
        }
        self.commit(candidate).await?;
        let ids: Vec<_> = self.jobs.read().await.keys().cloned().collect();
        drop(_gate);
        for id in ids {
            self.arm(id);
        }
        Ok(())
    }
    pub async fn restore_snapshot(self: &Arc<Self>, snapshot: SchedulerSnapshot) -> Result<()> {
        if snapshot.version != 2 {
            return Err(SchedulerError::Recovery(
                "unsupported scheduler snapshot version".into(),
            ));
        }
        *self.revision.lock().await = snapshot.revision;
        self.restore_jobs(snapshot.jobs).await
    }
    pub async fn add(
        self: &Arc<Self>,
        name: String,
        prompt: String,
        schedule: ScheduleKind,
        cwd: Option<String>,
        max_runs: Option<u64>,
    ) -> Result<ScheduledJob> {
        self.add_with_enabled(name, prompt, schedule, cwd, max_runs, true)
            .await
    }
    pub async fn add_paused(
        self: &Arc<Self>,
        name: String,
        prompt: String,
        schedule: ScheduleKind,
        cwd: Option<String>,
        max_runs: Option<u64>,
    ) -> Result<ScheduledJob> {
        self.add_with_enabled(name, prompt, schedule, cwd, max_runs, false)
            .await
    }
    async fn add_with_enabled(
        self: &Arc<Self>,
        name: String,
        prompt: String,
        schedule: ScheduleKind,
        cwd: Option<String>,
        max_runs: Option<u64>,
        enabled: bool,
    ) -> Result<ScheduledJob> {
        if prompt.trim().is_empty() {
            return Err(SchedulerError::InvalidSchedule("empty prompt".into()));
        }
        validate_schedule(&schedule)?;
        if max_runs == Some(0) {
            return Err(SchedulerError::InvalidSchedule(
                "max_runs must be greater than zero".into(),
            ));
        }
        let now = Utc::now();
        let spec = ScheduledJob {
            id: Uuid::new_v4().to_string(),
            name,
            prompt,
            cwd,
            schedule: schedule.clone(),
            status: if enabled {
                JobStatus::Scheduled
            } else {
                JobStatus::Paused
            },
            created_at: now,
            last_run: None,
            next_run: compute_next(&schedule, now),
            run_count: 0,
            max_runs,
            active_run: None,
            runs: vec![],
            error: None,
            control_revision: 0,
        };
        let _gate = self.transition.lock().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(SchedulerError::Recovery(
                "scheduler is stopping; admission refused".into(),
            ));
        }
        let mut candidate = self.jobs.read().await.clone();
        candidate.insert(spec.id.clone(), spec.clone());
        self.commit(candidate).await?;
        drop(_gate);
        self.arm(spec.id.clone());
        Ok(spec)
    }
    fn arm(self: &Arc<Self>, id: String) {
        let weak = Arc::downgrade(self);
        let wake = self.wake.clone();
        tokio::spawn(async move {
            loop {
                let notified = wake.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                let Some(this) = weak.upgrade() else {
                    return;
                };
                if this.shutting_down.load(Ordering::Acquire) {
                    return;
                }
                let Some(job) = this.jobs.read().await.get(&id).cloned() else {
                    return;
                };
                if job.status != JobStatus::Scheduled {
                    drop(this);
                    notified.await;
                    continue;
                }
                let when = job.next_run.unwrap_or_else(Utc::now);
                let delay = when
                    .signed_duration_since(Utc::now())
                    .to_std()
                    .unwrap_or_default();
                drop(this);
                tokio::select! {_=tokio::time::sleep(delay)=>{},_=&mut notified=>continue}
                let Some(this) = weak.upgrade() else {
                    return;
                };
                this.fire(&id).await;
            }
        });
    }
    async fn fire(&self, id: &str) {
        let context = {
            let _gate = self.transition.lock().await;
            if self.shutting_down.load(Ordering::Acquire) {
                return;
            }
            let mut candidate = self.jobs.read().await.clone();
            let Some(job) = candidate.get_mut(id) else {
                return;
            };
            if job.status != JobStatus::Scheduled || job.active_run.is_some() {
                return;
            }
            if job.next_run.is_some_and(|next| next > Utc::now()) {
                return;
            }
            let run = JobRun {
                run_id: Uuid::new_v4().to_string(),
                session_id: Uuid::new_v4(),
                admitted_at: Utc::now(),
                finished_at: None,
                outcome: None,
            };
            job.status = JobStatus::Running;
            job.run_count += 1;
            job.last_run = Some(run.admitted_at);
            job.next_run = None;
            job.active_run = Some(run.clone());
            job.error = None;
            let job = job.clone();
            let (tx, _) = watch::channel(false);
            let cancel = RunCancellation(tx);
            let (done, _) = watch::channel(None);
            if let Err(error) = self.commit(candidate).await {
                // No effect dispatched. Leave a visible failed admission rather
                // than a zero-delay retry loop against unavailable persistence.
                if let Some(job) = self.jobs.write().await.get_mut(id) {
                    job.status = JobStatus::Uncertain;
                    job.error = Some(error.to_string());
                    job.next_run = None;
                }
                self.event_bus.emit_error(None, error.to_string());
                return;
            }
            self.active.lock().await.insert(
                id.into(),
                ActiveRun {
                    run_id: run.run_id.clone(),
                    cancel: cancel.clone(),
                    done,
                },
            );
            JobRunContext {
                job,
                run_id: run.run_id,
                session_id: run.session_id,
                cancel,
            }
        };
        self.event_bus.emit(ControlEvent::SchedulerJob {
            job_id: id.into(),
            message: format!("admitted run {}", context.run_id),
            at: Utc::now(),
        });
        let handler = self.handler.lock().await.clone();
        let outcome = if let Some(handler) = handler {
            match tokio::spawn(async move { handler.call(context.clone()).await }).await {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(error)) => JobOutcome {
                    process: None,
                    error: Some(format!("run handler failed after admission: {error}")),
                    cleanup_complete: false,
                },
                Err(error) => JobOutcome {
                    process: None,
                    error: Some(format!("run handler interrupted: {error}")),
                    cleanup_complete: false,
                },
            }
        } else {
            JobOutcome::failed("no run handler configured")
        };
        self.finish(id, outcome).await;
    }
    async fn finish(&self, id: &str, outcome: JobOutcome) {
        let _gate = self.transition.lock().await;
        let mut candidate = self.jobs.read().await.clone();
        let mut active = self.active.lock().await;
        let Some(runtime) = active.get(id) else {
            return;
        };
        let Some(job) = candidate.get_mut(id) else {
            return;
        };
        if job
            .active_run
            .as_ref()
            .is_none_or(|run| run.run_id != runtime.run_id)
        {
            return;
        }
        let mut run = job.active_run.take().expect("checked active run");
        run.finished_at = Some(Utc::now());
        run.outcome = Some(outcome.clone());
        job.runs.push(run);
        job.error = outcome.error.clone();
        job.status = if !outcome.cleanup_complete {
            JobStatus::Uncertain
        } else if runtime.cancel.requested() {
            JobStatus::Cancelled
        } else if !outcome.success() {
            JobStatus::Failed
        } else if job.status == JobStatus::Paused {
            JobStatus::Paused
        } else if matches!(job.schedule, ScheduleKind::Once { .. })
            || job.max_runs.is_some_and(|max| job.run_count >= max)
        {
            JobStatus::Completed
        } else {
            JobStatus::Scheduled
        };
        job.next_run = if job.status == JobStatus::Scheduled {
            compute_next(&job.schedule, Utc::now())
        } else {
            None
        };
        if let Err(error) = self.commit(candidate).await {
            // The admitted durable snapshot survives unchanged. Mark current
            // memory uncertain, retain the run/session binding, and never rearm.
            if let Some(job) = self.jobs.write().await.get_mut(id) {
                job.status = JobStatus::Uncertain;
                job.next_run = None;
                job.error = Some(format!("observed outcome could not be committed: {error}"));
                if let Some(run) = job.active_run.as_mut() {
                    run.outcome = Some(outcome.clone());
                    run.finished_at = Some(Utc::now());
                }
            }
            self.event_bus.emit_error(None, error.to_string());
        }
        if outcome.cleanup_complete {
            if let Some(runtime) = active.remove(id) {
                runtime.done.send_replace(Some(outcome));
            }
        } else if let Some(runtime) = active.get(id) {
            runtime.done.send_replace(Some(outcome));
        }
    }
    pub async fn pause(&self, id: &str) -> Result<()> {
        let _gate = self.transition.lock().await;
        let mut candidate = self.jobs.read().await.clone();
        let job = candidate
            .get_mut(id)
            .ok_or_else(|| SchedulerError::NotFound(id.into()))?;
        if matches!(
            job.status,
            JobStatus::Cancelled
                | JobStatus::Completed
                | JobStatus::Failed
                | JobStatus::Interrupted
                | JobStatus::Uncertain
                | JobStatus::Cancelling
        ) {
            return Err(SchedulerError::Recovery(
                "this stopped run cannot be paused".into(),
            ));
        }
        job.status = JobStatus::Paused;
        job.control_revision += 1;
        self.commit(candidate).await
    }
    pub async fn resume(self: &Arc<Self>, id: &str) -> Result<()> {
        let _gate = self.transition.lock().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(SchedulerError::Recovery(
                "scheduler is stopping; admission refused".into(),
            ));
        }
        let mut candidate = self.jobs.read().await.clone();
        let job = candidate
            .get_mut(id)
            .ok_or_else(|| SchedulerError::NotFound(id.into()))?;
        if job
            .runs
            .last()
            .and_then(|run| run.outcome.as_ref())
            .is_some_and(|outcome| !outcome.cleanup_complete)
            || job
                .active_run
                .as_ref()
                .and_then(|run| run.outcome.as_ref())
                .is_some_and(|outcome| !outcome.cleanup_complete)
        {
            return Err(SchedulerError::Recovery("recorded native cleanup is unconfirmed; inspect and verify that session before another run".into()));
        }
        if self.active.lock().await.contains_key(id)
            || (job.active_run.is_some()
                && matches!(
                    job.status,
                    JobStatus::Running | JobStatus::Paused | JobStatus::Cancelling
                ))
        {
            return Err(SchedulerError::AlreadyRunning(id.into()));
        }
        if !matches!(
            job.status,
            JobStatus::Paused
                | JobStatus::Interrupted
                | JobStatus::Uncertain
                | JobStatus::Failed
                | JobStatus::Cancelled
        ) {
            return Ok(());
        }
        // An explicit recovery retires the unresolved record without asserting
        // that its external effects were rolled back or completed.
        if let Some(mut run) = job.active_run.take() {
            run.finished_at = Some(Utc::now());
            job.runs.push(run);
        }
        job.status = JobStatus::Scheduled;
        job.control_revision += 1;
        job.error = None;
        job.next_run = compute_next(&job.schedule, Utc::now());
        self.commit(candidate).await
    }
    async fn retry_cleanup(&self, id: &str) -> Result<()> {
        let handler = self.cleanup_handler.lock().await.clone().ok_or_else(|| {
            SchedulerError::Recovery(
                "retained cleanup handler unavailable; native owner remains unresolved".into(),
            )
        })?;
        let (run_id, session_id, control_revision, run_count, intent_error) = {
            let _gate = self.transition.lock().await;
            let mut candidate = self.jobs.read().await.clone();
            let job = candidate
                .get_mut(id)
                .ok_or_else(|| SchedulerError::NotFound(id.into()))?;
            let run = job
                .active_run
                .as_ref()
                .or_else(|| job.runs.last())
                .ok_or_else(|| SchedulerError::Recovery("no retained run binding".into()))?;
            let binding = (run.run_id.clone(), run.session_id);
            job.status = JobStatus::Cancelling;
            job.control_revision += 1;
            let control_revision = job.control_revision;
            let run_count = job.run_count;
            job.next_run = None;
            let error = self.commit(candidate).await.err();
            (binding.0, binding.1, control_revision, run_count, error)
        };
        let outcome =
            match tokio::time::timeout(Duration::from_secs(30), handler.call(session_id)).await {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(error)) => JobOutcome {
                    process: None,
                    error: Some(error),
                    cleanup_complete: false,
                },
                Err(_) => JobOutcome {
                    process: None,
                    error: Some("cleanup retry deadline exceeded".into()),
                    cleanup_complete: false,
                },
            };
        let _gate = self.transition.lock().await;
        let mut candidate = self.jobs.read().await.clone();
        let job = candidate
            .get_mut(id)
            .ok_or_else(|| SchedulerError::NotFound(id.into()))?;
        if job.run_count != run_count
            || job.control_revision != control_revision
            || job
                .active_run
                .as_ref()
                .is_some_and(|run| run.run_id != run_id)
        {
            return Err(SchedulerError::Recovery("cleanup result belongs to an earlier control/run generation; current owner retained".into()));
        }
        let run = if job
            .active_run
            .as_ref()
            .is_some_and(|run| run.run_id == run_id)
        {
            job.active_run.as_mut()
        } else {
            job.runs.iter_mut().find(|run| run.run_id == run_id)
        }
        .ok_or_else(|| SchedulerError::Recovery("cleanup run binding changed".into()))?;
        run.outcome = Some(outcome.clone());
        run.finished_at = Some(Utc::now());
        job.status = if outcome.cleanup_complete {
            JobStatus::Cancelled
        } else {
            JobStatus::Uncertain
        };
        job.error = outcome.error.clone();
        job.next_run = None;
        if outcome.cleanup_complete {
            if let Some(run) = job.active_run.take() {
                if !job.runs.iter().any(|saved| saved.run_id == run.run_id) {
                    job.runs.push(run);
                }
            }
        }
        if let Err(error) = self.commit(candidate).await {
            if let Some(job) = self.jobs.write().await.get_mut(id) {
                job.status = JobStatus::Uncertain;
                job.error = Some(error.to_string());
                job.next_run = None;
            }
            return Err(error);
        }
        if outcome.cleanup_complete {
            let mut active = self.active.lock().await;
            if active
                .get(id)
                .is_some_and(|runtime| runtime.run_id == run_id)
            {
                if let Some(runtime) = active.remove(id) {
                    runtime.done.send_replace(Some(outcome));
                }
            }
        } else {
            return Err(SchedulerError::Recovery(
                "native cleanup retry remains unconfirmed".into(),
            ));
        }
        if let Some(error) = intent_error {
            return Err(error);
        }
        Ok(())
    }
    pub async fn cancel(&self, id: &str) -> Result<()> {
        let observed_pending = self
            .jobs
            .read()
            .await
            .get(id)
            .and_then(|job| job.active_run.as_ref())
            .is_some_and(|run| run.outcome.is_some());
        let unconfirmed = self
            .jobs
            .read()
            .await
            .get(id)
            .and_then(|job| job.active_run.as_ref().or_else(|| job.runs.last()))
            .and_then(|run| run.outcome.as_ref())
            .is_some_and(|outcome| !outcome.cleanup_complete);
        if unconfirmed || observed_pending {
            return self.retry_cleanup(id).await;
        }
        let (completion, persistence_error) = {
            let _gate = self.transition.lock().await;
            let mut candidate = self.jobs.read().await.clone();
            let job = candidate
                .get_mut(id)
                .ok_or_else(|| SchedulerError::NotFound(id.into()))?;
            let active = self.active.lock().await;
            if !active.contains_key(id)
                && (job
                    .runs
                    .last()
                    .and_then(|run| run.outcome.as_ref())
                    .is_some_and(|outcome| !outcome.cleanup_complete)
                    || job.active_run.is_some()
                    || matches!(job.status, JobStatus::Interrupted | JobStatus::Uncertain))
            {
                return Err(SchedulerError::Recovery(
                    "prior run remains unresolved; cancellation cannot certify its cleanup".into(),
                ));
            }
            job.status = if active.contains_key(id) {
                JobStatus::Cancelling
            } else {
                JobStatus::Cancelled
            };
            job.control_revision += 1;
            job.next_run = None;
            let persistence_error = self.commit(candidate).await.err();
            if let Some(error) = &persistence_error {
                // Protective Stop still reaches an admitted effect. Its saved
                // run binding survives; failed persistence remains visible.
                if active.contains_key(id) {
                    if let Some(job) = self.jobs.write().await.get_mut(id) {
                        job.status = JobStatus::Uncertain;
                        job.error = Some(error.to_string());
                        job.next_run = None;
                    }
                }
            }
            let completion = active.get(id).map(|runtime| {
                let rx = runtime.done.subscribe();
                runtime.cancel.request();
                rx
            });
            (completion, persistence_error)
        };
        if let Some(mut done) = completion {
            tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    if let Some(outcome) = done.borrow().clone() {
                        if !outcome.cleanup_complete {
                            return Err(SchedulerError::Recovery(
                                "run cleanup remains unconfirmed".into(),
                            ));
                        }
                        return Ok(());
                    }
                    done.changed().await.map_err(|_| {
                        SchedulerError::Recovery("run owner disappeared during cancellation".into())
                    })?;
                }
            })
            .await
            .map_err(|_| {
                SchedulerError::Recovery(
                    "Stop deadline exceeded; active run and cleanup ownership retained".into(),
                )
            })??;
        }
        if let Some(error) = persistence_error {
            return Err(error);
        }
        if self
            .jobs
            .read()
            .await
            .get(id)
            .is_some_and(|job| job.status == JobStatus::Uncertain)
        {
            return Err(SchedulerError::Recovery(
                "cancel outcome could not be durably confirmed".into(),
            ));
        }
        Ok(())
    }
}
fn validate_schedule(schedule: &ScheduleKind) -> Result<()> {
    match schedule {
        ScheduleKind::Interval { secs } if *secs == 0 || *secs > 31_536_000 => Err(
            SchedulerError::InvalidSchedule("interval must be 1..31536000 seconds".into()),
        ),
        ScheduleKind::Once { delay_secs } if *delay_secs > 31_536_000 => Err(
            SchedulerError::InvalidSchedule("delay exceeds one year".into()),
        ),
        ScheduleKind::Cron { expr } => cron_delay(expr).map(|_| ()),
        _ => Ok(()),
    }
}
fn compute_next(schedule: &ScheduleKind, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match schedule {
        ScheduleKind::Interval { secs } => Some(from + chrono::Duration::seconds(*secs as i64)),
        ScheduleKind::Once { delay_secs } => {
            Some(from + chrono::Duration::seconds(*delay_secs as i64))
        }
        ScheduleKind::Cron { expr } => cron_delay(expr)
            .ok()
            .and_then(|delay| chrono::Duration::from_std(delay).ok())
            .map(|delay| from + delay),
    }
}
fn cron_delay(expr: &str) -> Result<Duration> {
    use std::str::FromStr;
    let schedule = cron::Schedule::from_str(expr)
        .map_err(|error| SchedulerError::InvalidSchedule(error.to_string()))?;
    let next = schedule
        .upcoming(Utc)
        .next()
        .ok_or_else(|| SchedulerError::InvalidSchedule("no upcoming cron tick".into()))?;
    Ok(next
        .signed_duration_since(Utc::now())
        .to_std()
        .unwrap_or_default())
}
#[cfg(test)]
mod tests {
    use super::*;
    use grok_events::shared_bus;
    use std::sync::atomic::{AtomicUsize, Ordering};
    async fn fixture() -> Arc<Scheduler> {
        let scheduler = Scheduler::new(shared_bus());
        scheduler.set_change_hook(|_| Ok(())).await;
        scheduler
    }
    async fn wait_status(scheduler: &Scheduler, id: &str, status: JobStatus) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if scheduler
                    .list()
                    .await
                    .iter()
                    .any(|job| job.id == id && job.status == status)
                {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    fn successful() -> JobOutcome {
        JobOutcome::from_process(ProcessOutcome {
            end: ProcessEnd::Success,
            exit_code: Some(0),
            signal: None,
            cleanup_scope: grok_cli_wrapper::process::CleanupScope::DedicatedProcessGroup,
            cleanup_complete: true,
            pipes_complete: true,
            output: Default::default(),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            error: None,
        })
    }
    #[tokio::test]
    async fn zero_delay_inactive_admission_never_calls_handler_before_enable() {
        let scheduler = fixture().await;
        let count = Arc::new(AtomicUsize::new(0));
        let counter = count.clone();
        scheduler
            .set_handler(JobHandler::new(move |_| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(successful())
                }
            }))
            .await;
        let job = scheduler
            .add_paused(
                "inactive".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 0 },
                None,
                None,
            )
            .await
            .unwrap();
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert_eq!(scheduler.list().await[0].status, JobStatus::Paused);
        scheduler.resume(&job.id).await.unwrap();
        wait_status(&scheduler, &job.id, JobStatus::Completed).await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn post_admission_error_retains_binding_until_typed_cleanup_retry() {
        let scheduler = fixture().await;
        scheduler
            .set_handler(JobHandler::new(|_| async {
                Err("possible effect started".into())
            }))
            .await;
        let job = scheduler
            .add(
                "uncertain".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 0 },
                None,
                None,
            )
            .await
            .unwrap();
        wait_status(&scheduler, &job.id, JobStatus::Uncertain).await;
        assert!(scheduler.resume(&job.id).await.is_err());
        assert!(scheduler.cancel(&job.id).await.is_err());
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        scheduler
            .set_cleanup_handler(JobCleanupHandler::new(move |_| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    let mut outcome = successful();
                    outcome.process.as_mut().unwrap().end = ProcessEnd::Cancelled;
                    Ok(outcome)
                }
            }))
            .await;
        scheduler.cancel(&job.id).await.unwrap();
        let saved = scheduler.list().await;
        assert_eq!(saved[0].status, JobStatus::Cancelled);
        assert_eq!(saved[0].run_count, 1);
        assert_eq!(saved[0].runs.len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn failed_cancel_persistence_still_reaches_effect_cleanup() {
        let scheduler = fixture().await;
        let stopped = Arc::new(AtomicUsize::new(0));
        let counter = stopped.clone();
        scheduler
            .set_handler(JobHandler::new(move |run| {
                let counter = counter.clone();
                async move {
                    run.cancel.cancelled().await;
                    counter.fetch_add(1, Ordering::SeqCst);
                    let mut outcome = successful();
                    outcome.process.as_mut().unwrap().end = ProcessEnd::Cancelled;
                    Ok(outcome)
                }
            }))
            .await;
        let job = scheduler
            .add(
                "stop".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 0 },
                None,
                None,
            )
            .await
            .unwrap();
        wait_status(&scheduler, &job.id, JobStatus::Running).await;
        scheduler
            .set_change_hook(|_| Err("disk failed".into()))
            .await;
        assert!(scheduler.cancel(&job.id).await.is_err());
        assert_eq!(stopped.load(Ordering::SeqCst), 1);
        assert_eq!(scheduler.list().await[0].status, JobStatus::Uncertain);
    }
    #[tokio::test(start_paused = true)]
    async fn ignored_cancel_has_deadline_and_retains_active_owner() {
        let scheduler = fixture().await;
        scheduler
            .set_handler(JobHandler::new(|_| async {
                tokio::time::sleep(Duration::from_secs(120)).await;
                Ok(successful())
            }))
            .await;
        let job = scheduler
            .add(
                "hang".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 0 },
                None,
                None,
            )
            .await
            .unwrap();
        wait_status(&scheduler, &job.id, JobStatus::Running).await;
        assert!(scheduler
            .cancel(&job.id)
            .await
            .unwrap_err()
            .to_string()
            .contains("deadline"));
        assert!(scheduler.active.lock().await.contains_key(&job.id));
        assert!(scheduler.resume(&job.id).await.is_err());
    }
    #[tokio::test]
    async fn stale_cleanup_cannot_cancel_a_new_control_or_run_generation() {
        let scheduler = fixture().await;
        scheduler
            .set_handler(JobHandler::new(|_| async { Err("possible effect".into()) }))
            .await;
        let job = scheduler
            .add(
                "retry".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 0 },
                None,
                None,
            )
            .await
            .unwrap();
        wait_status(&scheduler, &job.id, JobStatus::Uncertain).await;
        let entered = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());
        let e = entered.clone();
        let r = release.clone();
        scheduler
            .set_cleanup_handler(JobCleanupHandler::new(move |_| {
                let e = e.clone();
                let r = r.clone();
                async move {
                    if e.fetch_add(1, Ordering::SeqCst) == 0 {
                        r.notified().await;
                    }
                    Ok(successful())
                }
            }))
            .await;
        let first = scheduler.clone();
        let id = job.id.clone();
        let old = tokio::spawn(async move { first.cancel(&id).await });
        while entered.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        scheduler.cancel(&job.id).await.unwrap();
        scheduler
            .set_handler(JobHandler::new(|run| async move {
                run.cancel.cancelled().await;
                let mut outcome = successful();
                outcome.process.as_mut().unwrap().end = ProcessEnd::Cancelled;
                Ok(outcome)
            }))
            .await;
        scheduler.resume(&job.id).await.unwrap();
        wait_status(&scheduler, &job.id, JobStatus::Running).await;
        let current = scheduler.list().await[0]
            .active_run
            .as_ref()
            .unwrap()
            .run_id
            .clone();
        release.notify_one();
        assert!(old.await.unwrap().is_err());
        let saved = scheduler.list().await;
        assert_eq!(saved[0].status, JobStatus::Running);
        assert_eq!(saved[0].active_run.as_ref().unwrap().run_id, current);
        assert!(scheduler.active.lock().await.contains_key(&job.id));
        scheduler.cancel(&job.id).await.unwrap();
    }

    #[tokio::test]
    async fn delayed_effect_is_running_until_actual_outcome_and_history_is_retained() {
        let scheduler = fixture().await;
        let barrier = Arc::new(Notify::new());
        let done = barrier.clone();
        scheduler
            .set_handler(JobHandler::new(move |_| {
                let done = done.clone();
                async move {
                    done.notified().await;
                    Ok(successful())
                }
            }))
            .await;
        let job = scheduler
            .add(
                "one".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 0 },
                None,
                None,
            )
            .await
            .unwrap();
        wait_status(&scheduler, &job.id, JobStatus::Running).await;
        assert_eq!(scheduler.list().await[0].run_count, 1);
        barrier.notify_one();
        wait_status(&scheduler, &job.id, JobStatus::Completed).await;
        let saved = scheduler.list().await;
        assert_eq!(saved[0].runs.len(), 1);
        let restored = fixture().await;
        restored.restore_jobs(saved).await.unwrap();
        assert_eq!(restored.list().await[0].status, JobStatus::Completed);
    }
    #[tokio::test]
    async fn missing_handler_or_error_never_completes() {
        let scheduler = fixture().await;
        let job = scheduler
            .add(
                "one".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 0 },
                None,
                None,
            )
            .await
            .unwrap();
        wait_status(&scheduler, &job.id, JobStatus::Failed).await;
        assert!(scheduler.list().await[0]
            .error
            .as_ref()
            .unwrap()
            .contains("no run handler"));
    }
    #[tokio::test]
    async fn persistence_failure_prevents_dispatch_and_terminal_failure_is_uncertain() {
        let scheduler = Scheduler::new(shared_bus());
        assert!(scheduler
            .add(
                "bad".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 0 },
                None,
                None
            )
            .await
            .is_err());
        assert!(scheduler.list().await.is_empty());
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        scheduler
            .set_handler(JobHandler::new(move |_| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(successful())
                }
            }))
            .await;
        scheduler
            .set_change_hook(|snapshot| {
                if snapshot
                    .jobs
                    .iter()
                    .any(|job| job.status == JobStatus::Running)
                {
                    Err("admission failed".into())
                } else {
                    Ok(())
                }
            })
            .await;
        let job = scheduler
            .add(
                "one".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 0 },
                None,
                None,
            )
            .await
            .unwrap();
        wait_status(&scheduler, &job.id, JobStatus::Uncertain).await;
        assert_eq!(count.load(Ordering::SeqCst), 0);
        let scheduler = fixture().await;
        scheduler
            .set_handler(JobHandler::new(|_| async { Ok(successful()) }))
            .await;
        scheduler
            .set_change_hook(|snapshot| {
                if snapshot
                    .jobs
                    .iter()
                    .any(|job| job.status == JobStatus::Completed)
                {
                    Err("terminal failed".into())
                } else {
                    Ok(())
                }
            })
            .await;
        let job = scheduler
            .add(
                "one".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 0 },
                None,
                None,
            )
            .await
            .unwrap();
        wait_status(&scheduler, &job.id, JobStatus::Uncertain).await;
        assert!(scheduler.list().await[0].active_run.is_some());
    }
    #[tokio::test]
    async fn pause_during_effect_keeps_result_and_cancel_waits_for_cleanup() {
        let scheduler = fixture().await;
        let done = Arc::new(Notify::new());
        let barrier = done.clone();
        scheduler.set_handler(JobHandler::new(move |run| {let barrier=barrier.clone();async move {tokio::select!{_=barrier.notified()=>Ok(successful()),_=run.cancel.cancelled()=>{let mut result=successful();result.process.as_mut().unwrap().end=ProcessEnd::Cancelled;Ok(result)}}}})).await;
        let job = scheduler
            .add(
                "pause".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 0 },
                None,
                None,
            )
            .await
            .unwrap();
        wait_status(&scheduler, &job.id, JobStatus::Running).await;
        scheduler.pause(&job.id).await.unwrap();
        done.notify_one();
        wait_status(&scheduler, &job.id, JobStatus::Paused).await;
        // Wait for outcome, not only the Paused metadata published by pause.
        tokio::time::timeout(Duration::from_secs(2), async {
            while scheduler.list().await[0].runs.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let second = scheduler
            .add(
                "cancel".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 0 },
                None,
                None,
            )
            .await
            .unwrap();
        wait_status(&scheduler, &second.id, JobStatus::Running).await;
        scheduler.cancel(&second.id).await.unwrap();
        assert!(scheduler.list().await.iter().any(|job| job.id == second.id
            && job.status == JobStatus::Cancelled
            && !job.runs.is_empty()));
    }
    #[tokio::test]
    async fn interrupted_restore_never_replays_and_user_mutation_failures_do_not_publish() {
        let scheduler = fixture().await;
        let job = scheduler
            .add(
                "later".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 120 },
                None,
                None,
            )
            .await
            .unwrap();
        let mut saved = scheduler.list().await;
        saved[0].status = JobStatus::Running;
        let restored = fixture().await;
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        restored
            .set_handler(JobHandler::new(move |_| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(successful())
                }
            }))
            .await;
        restored.restore_jobs(saved).await.unwrap();
        assert_eq!(restored.list().await[0].status, JobStatus::Interrupted);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        scheduler
            .set_change_hook(|_| Err("disk failed".into()))
            .await;
        assert!(scheduler.pause(&job.id).await.is_err());
        assert_eq!(scheduler.list().await[0].status, JobStatus::Scheduled);
        assert!(scheduler.cancel(&job.id).await.is_err());
        assert_eq!(scheduler.list().await[0].status, JobStatus::Scheduled);
    }
    #[tokio::test]
    async fn shutdown_fences_timer_wake_and_new_admission_while_cleanup_waits() {
        let scheduler = fixture().await;
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let (a, b, c) = (started.clone(), release.clone(), calls.clone());
        scheduler
            .set_handler(JobHandler::new(move |run| {
                let (a, b, c) = (a.clone(), b.clone(), c.clone());
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    run.cancel.cancelled().await;
                    a.notify_one();
                    b.notified().await;
                    let mut outcome = successful();
                    outcome.process.as_mut().unwrap().end = ProcessEnd::Cancelled;
                    Ok(outcome)
                }
            }))
            .await;
        let active = scheduler
            .add(
                "active".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 0 },
                None,
                None,
            )
            .await
            .unwrap();
        wait_status(&scheduler, &active.id, JobStatus::Running).await;
        let pending = scheduler
            .add_paused(
                "pending".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 0 },
                None,
                None,
            )
            .await
            .unwrap();
        let stopping = scheduler.clone();
        let stop = tokio::spawn(async move { stopping.shutdown().await });
        started.notified().await;
        assert!(!stop.is_finished());
        assert!(scheduler.resume(&pending.id).await.is_err());
        assert!(scheduler
            .add(
                "late".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 0 },
                None,
                None
            )
            .await
            .is_err());
        scheduler.wake.notify_waiters();
        scheduler.fire(&pending.id).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        release.notify_one();
        stop.await.unwrap().unwrap();
        assert_eq!(scheduler.active.lock().await.len(), 0);
        assert_eq!(
            scheduler
                .list()
                .await
                .iter()
                .find(|j| j.id == active.id)
                .unwrap()
                .status,
            JobStatus::Cancelled
        );
        assert_eq!(
            scheduler
                .list()
                .await
                .iter()
                .find(|j| j.id == pending.id)
                .unwrap()
                .run_count,
            0
        );
    }
    #[tokio::test]
    async fn user_stop_all_keeps_explicit_resume_and_new_routines_available() {
        let scheduler = fixture().await;
        let counter = Arc::new(AtomicUsize::new(0));
        let calls = counter.clone();
        scheduler
            .set_handler(JobHandler::new(move |_| {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(successful())
                }
            }))
            .await;
        let job = scheduler
            .add(
                "later".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 60 },
                None,
                None,
            )
            .await
            .unwrap();
        scheduler.stop_all().await.unwrap();
        assert_eq!(scheduler.list().await[0].status, JobStatus::Paused);
        scheduler.resume(&job.id).await.unwrap();
        assert!(!scheduler.shutting_down.load(Ordering::Acquire));
        let now = scheduler
            .add(
                "now".into(),
                "prompt".into(),
                ScheduleKind::Once { delay_secs: 0 },
                None,
                None,
            )
            .await
            .unwrap();
        wait_status(&scheduler, &now.id, JobStatus::Completed).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        scheduler.stop_all().await.unwrap();
    }
}
