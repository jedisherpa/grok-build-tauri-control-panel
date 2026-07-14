//! Scheduler for recurring agent jobs (`/loop`, `/goal` style routines).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{info, warn};
use uuid::Uuid;

use grok_events::{ControlEvent, EventBus};

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("job not found: {0}")]
    NotFound(String),
    #[error("invalid schedule: {0}")]
    InvalidSchedule(String),
    #[error("job already running: {0}")]
    AlreadyRunning(String),
}

pub type Result<T> = std::result::Result<T, SchedulerError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleKind {
    /// Fixed interval in seconds.
    Interval { secs: u64 },
    /// Cron expression (UTC).
    Cron { expr: String },
    /// One-shot after delay.
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
    pub run_count: u64,
    pub max_runs: Option<u64>,
}

type JobFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;
type JobFn = dyn Fn(ScheduledJob) -> JobFuture + Send + Sync;

#[derive(Clone)]
pub struct JobHandler {
    inner: Arc<JobFn>,
}

impl JobHandler {
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: Fn(ScheduledJob) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        Self {
            inner: Arc::new(move |job| Box::pin(f(job))),
        }
    }

    pub async fn call(&self, job: ScheduledJob) {
        (self.inner)(job).await;
    }
}

struct LiveJob {
    spec: ScheduledJob,
    handle: Option<JoinHandle<()>>,
}

type ChangeHook = dyn Fn(Vec<ScheduledJob>) + Send + Sync;

pub struct Scheduler {
    jobs: RwLock<HashMap<String, LiveJob>>,
    event_bus: Arc<EventBus>,
    handler: Mutex<Option<JobHandler>>,
    rate_limit: Duration,
    last_fire: Mutex<HashMap<String, DateTime<Utc>>>,
    /// Called with the full job list after every mutation — the app layer
    /// persists it so routines survive a restart.
    on_change: Mutex<Option<Arc<ChangeHook>>>,
}

impl Scheduler {
    pub fn new(event_bus: Arc<EventBus>) -> Arc<Self> {
        Arc::new(Self {
            jobs: RwLock::new(HashMap::new()),
            event_bus,
            handler: Mutex::new(None),
            rate_limit: Duration::from_secs(5),
            last_fire: Mutex::new(HashMap::new()),
            on_change: Mutex::new(None),
        })
    }

    pub async fn set_handler(&self, handler: JobHandler) {
        *self.handler.lock().await = Some(handler);
    }

    pub async fn set_change_hook<F>(&self, hook: F)
    where
        F: Fn(Vec<ScheduledJob>) + Send + Sync + 'static,
    {
        *self.on_change.lock().await = Some(Arc::new(hook));
    }

    async fn notify_change(&self) {
        let hook = self.on_change.lock().await.clone();
        if let Some(hook) = hook {
            let jobs = self.list().await;
            hook(jobs);
        }
    }

    /// Re-register persisted jobs after a restart. Terminal jobs are kept in
    /// the list for history but not re-armed; Running degrades to Scheduled.
    pub async fn restore_jobs(self: &Arc<Self>, specs: Vec<ScheduledJob>) {
        for mut spec in specs {
            if matches!(
                spec.status,
                JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled
            ) {
                continue;
            }
            if spec.status == JobStatus::Running {
                spec.status = JobStatus::Scheduled;
            }
            let handle = if spec.status == JobStatus::Paused {
                None
            } else {
                Some(self.spawn_runner(spec.clone()))
            };
            self.jobs
                .write()
                .await
                .insert(spec.id.clone(), LiveJob { spec, handle });
        }
        info!(count = self.jobs.read().await.len(), "scheduler jobs restored");
    }

    pub async fn list(&self) -> Vec<ScheduledJob> {
        self.jobs
            .read()
            .await
            .values()
            .map(|j| j.spec.clone())
            .collect()
    }

    pub async fn add(
        self: &Arc<Self>,
        name: String,
        prompt: String,
        schedule: ScheduleKind,
        cwd: Option<String>,
        max_runs: Option<u64>,
    ) -> Result<ScheduledJob> {
        if prompt.trim().is_empty() {
            return Err(SchedulerError::InvalidSchedule("empty prompt".into()));
        }
        validate_schedule(&schedule)?;

        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let spec = ScheduledJob {
            id: id.clone(),
            name,
            prompt,
            cwd,
            schedule: schedule.clone(),
            status: JobStatus::Scheduled,
            created_at: now,
            last_run: None,
            next_run: compute_next(&schedule, now),
            run_count: 0,
            max_runs,
        };

        let handle = self.spawn_runner(spec.clone());
        self.jobs.write().await.insert(
            id,
            LiveJob {
                spec: spec.clone(),
                handle: Some(handle),
            },
        );
        info!(job_id = %spec.id, "job scheduled");
        self.notify_change().await;
        Ok(spec)
    }

    fn spawn_runner(self: &Arc<Self>, job: ScheduledJob) -> JoinHandle<()> {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            this.run_loop(job).await;
        })
    }

    async fn run_loop(self: Arc<Self>, mut job: ScheduledJob) {
        loop {
            let delay = match &job.schedule {
                ScheduleKind::Interval { secs } => Duration::from_secs((*secs).max(1)),
                ScheduleKind::Once { delay_secs } => Duration::from_secs(*delay_secs),
                ScheduleKind::Cron { expr } => match cron_delay(expr) {
                    Ok(d) => d,
                    Err(e) => {
                        warn!(error = %e, "cron error");
                        this_fail(&self, &job.id, &e.to_string()).await;
                        return;
                    }
                },
            };

            tokio::time::sleep(delay).await;

            // Rate limit
            {
                let mut last = self.last_fire.lock().await;
                if let Some(prev) = last.get(&job.id) {
                    if Utc::now().signed_duration_since(*prev).to_std().unwrap_or_default()
                        < self.rate_limit
                    {
                        continue;
                    }
                }
                last.insert(job.id.clone(), Utc::now());
            }

            // Still present?
            {
                let jobs = self.jobs.read().await;
                match jobs.get(&job.id) {
                    Some(live) if live.spec.status == JobStatus::Paused => continue,
                    Some(live) if live.spec.status == JobStatus::Cancelled => return,
                    None => return,
                    Some(live) => job = live.spec.clone(),
                }
            }

            self.mark_running(&job.id).await;
            self.event_bus.emit(ControlEvent::SchedulerJob {
                job_id: job.id.clone(),
                message: format!("running: {}", job.name),
                at: Utc::now(),
            });

            if let Some(handler) = self.handler.lock().await.clone() {
                handler.call(job.clone()).await;
            } else {
                info!(job = %job.name, prompt = %job.prompt, "scheduler tick (no handler)");
            }

            job.run_count += 1;
            job.last_run = Some(Utc::now());
            job.next_run = compute_next(&job.schedule, Utc::now());
            job.status = JobStatus::Scheduled;
            self.update_spec(&job).await;

            if matches!(job.schedule, ScheduleKind::Once { .. }) {
                self.mark_completed(&job.id).await;
                return;
            }
            if let Some(max) = job.max_runs {
                if job.run_count >= max {
                    self.mark_completed(&job.id).await;
                    return;
                }
            }
        }
    }

    async fn update_spec(&self, job: &ScheduledJob) {
        if let Some(live) = self.jobs.write().await.get_mut(&job.id) {
            live.spec = job.clone();
        }
        self.notify_change().await;
    }

    async fn mark_running(&self, id: &str) {
        if let Some(live) = self.jobs.write().await.get_mut(id) {
            live.spec.status = JobStatus::Running;
        }
    }

    async fn mark_completed(&self, id: &str) {
        // Take the handle out before aborting — this runs on the job's own
        // task, and aborting while holding the write lock is a footgun the
        // moment anything awaits after it.
        let handle = {
            let mut jobs = self.jobs.write().await;
            match jobs.get_mut(id) {
                Some(live) => {
                    live.spec.status = JobStatus::Completed;
                    live.handle.take()
                }
                None => None,
            }
        };
        self.last_fire.lock().await.remove(id);
        self.notify_change().await;
        if let Some(h) = handle {
            h.abort();
        }
    }

    pub async fn pause(&self, id: &str) -> Result<()> {
        {
            let mut jobs = self.jobs.write().await;
            let live = jobs
                .get_mut(id)
                .ok_or_else(|| SchedulerError::NotFound(id.to_string()))?;
            live.spec.status = JobStatus::Paused;
        }
        self.notify_change().await;
        Ok(())
    }

    pub async fn resume(self: &Arc<Self>, id: &str) -> Result<()> {
        {
            let mut jobs = self.jobs.write().await;
            let live = jobs
                .get_mut(id)
                .ok_or_else(|| SchedulerError::NotFound(id.to_string()))?;
            if live.spec.status != JobStatus::Paused {
                return Ok(());
            }
            live.spec.status = JobStatus::Scheduled;
            let spec = live.spec.clone();
            if let Some(h) = live.handle.take() {
                h.abort();
            }
            live.handle = Some(self.spawn_runner(spec));
        }
        self.notify_change().await;
        Ok(())
    }

    pub async fn cancel(&self, id: &str) -> Result<()> {
        let handle = {
            let mut jobs = self.jobs.write().await;
            let mut live = jobs
                .remove(id)
                .ok_or_else(|| SchedulerError::NotFound(id.to_string()))?;
            live.spec.status = JobStatus::Cancelled;
            live.handle.take()
        };
        self.last_fire.lock().await.remove(id);
        self.notify_change().await;
        if let Some(h) = handle {
            h.abort();
        }
        Ok(())
    }
}

async fn this_fail(sched: &Scheduler, id: &str, msg: &str) {
    if let Some(live) = sched.jobs.write().await.get_mut(id) {
        live.spec.status = JobStatus::Failed;
    }
    sched.notify_change().await;
    sched.event_bus.emit(ControlEvent::SchedulerJob {
        job_id: id.to_string(),
        message: format!("failed: {msg}"),
        at: Utc::now(),
    });
}

fn validate_schedule(s: &ScheduleKind) -> Result<()> {
    match s {
        ScheduleKind::Interval { secs } if *secs == 0 => {
            Err(SchedulerError::InvalidSchedule("interval must be > 0".into()))
        }
        ScheduleKind::Cron { expr } => {
            cron_delay(expr).map(|_| ())
        }
        ScheduleKind::Once { delay_secs: _ } => Ok(()),
        ScheduleKind::Interval { .. } => Ok(()),
    }
}

fn compute_next(s: &ScheduleKind, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match s {
        ScheduleKind::Interval { secs } => {
            Some(from + chrono::Duration::seconds(*secs as i64))
        }
        ScheduleKind::Once { delay_secs } => {
            Some(from + chrono::Duration::seconds(*delay_secs as i64))
        }
        ScheduleKind::Cron { expr } => cron_delay(expr).ok().map(|d| {
            from + chrono::Duration::from_std(d).unwrap_or(chrono::Duration::seconds(60))
        }),
    }
}

fn cron_delay(expr: &str) -> Result<Duration> {
    use std::str::FromStr;
    let schedule = cron::Schedule::from_str(expr)
        .map_err(|e| SchedulerError::InvalidSchedule(e.to_string()))?;
    let next = schedule
        .upcoming(Utc)
        .next()
        .ok_or_else(|| SchedulerError::InvalidSchedule("no upcoming cron tick".into()))?;
    let dur = next
        .signed_duration_since(Utc::now())
        .to_std()
        .unwrap_or(Duration::from_secs(1));
    Ok(dur)
}

#[cfg(test)]
mod tests {
    use super::*;
    use grok_events::shared_bus;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn interval_job_fires() {
        let bus = shared_bus();
        let sched = Scheduler::new(bus);
        let counter = Arc::new(AtomicUsize::new(0));
        let c2 = counter.clone();
        sched
            .set_handler(JobHandler::new(move |_job| {
                let c = c2.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                }
            }))
            .await;

        let job = sched
            .add(
                "t".into(),
                "ping".into(),
                ScheduleKind::Interval { secs: 1 },
                None,
                Some(2),
            )
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert!(counter.load(Ordering::SeqCst) >= 1);
        let _ = sched.cancel(&job.id).await;
    }
}
