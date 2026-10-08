//! Shared application state wired from all backend crates.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::RwLock;
use tracing::{info, warn};

use grok_cli_wrapper::{GrokCli, LoginManager};
use grok_config::{discover_environment, GrokConfig, GrokPaths};
use grok_control_core::SessionRegistry;
use grok_events::{shared_bus, EventBus};
use grok_extensions::ExtensionsService;
use grok_mcp::McpManager;
use grok_memory::MemoryService;
use grok_persistence::Persistence;
use grok_scheduler::{JobCleanupHandler, JobHandler, JobOutcome, JobRunContext, ScheduledJob, Scheduler, SchedulerSnapshot};
use grok_worktree::WorktreeManager;

use crate::devserver::DevServerManager;
use crate::explainer::ExplainerService;
use crate::haven::HavenClient;

pub struct AppState {
    pub builds: Arc<crate::builds::BuildService>,
    pub paths: GrokPaths,
    pub config: Arc<RwLock<GrokConfig>>,
    pub event_bus: Arc<EventBus>,
    pub grok_cli: Arc<GrokCli>,
    pub registry: Arc<SessionRegistry>,
    pub worktrees: Arc<WorktreeManager>,
    pub extensions: Arc<ExtensionsService>,
    pub mcp: Arc<McpManager>,
    pub memory: Arc<MemoryService>,
    pub scheduler: Arc<Scheduler>,
    pub persistence: Arc<Persistence>,
    pub dev_server: Arc<DevServerManager>,
    pub login: Arc<LoginManager>,
    pub haven: Arc<HavenClient>,
    pub explainer: Arc<ExplainerService>,
}

fn load_startup_config(paths: &GrokPaths, resolved_binary: Option<PathBuf>) -> Result<GrokConfig> {
    // Validate both layers before saving discovery. Missing is distinct from corrupt.
    let mut config = GrokConfig::load(paths)
        .context("load panel/project configuration; original files preserved")?;
    let mut base =
        GrokConfig::load_base(paths).context("load base configuration; original file preserved")?;
    if let Some(root) = &config.worktrees_root {
        crate::qa_profile::validate_worktree_root(paths, root)?;
    }
    if let Some(binary) = resolved_binary {
        // Discovery may fill a missing program; it must not replace an explicit
        // operator-selected backend, including a generated QA adapter.
        if base.grok_binary.is_none() {
            base.grok_binary = Some(binary.clone());
        }
        if config.grok_binary.is_none() {
            config.grok_binary = Some(binary);
        }
    }
    base.save(&paths.config_file)
        .context("save resolved panel configuration")?;
    Ok(config)
}

impl AppState {
    /// Only app exit uses the lifetime admission fence. User Stop-all keeps the
    /// running app usable through the separate scheduler stop_all operation.
    pub async fn shutdown_for_exit(&self) -> Result<()> {
        let cleanup=shutdown_owned_runtime(&self.scheduler,&self.registry).await;
        let checkpoint=self.persistence.checkpoint();
        let errors=[cleanup.err().map(|e|e.to_string()),checkpoint.err().map(|e|e.to_string())].into_iter().flatten().collect::<Vec<_>>();
        if errors.is_empty() {Ok(())} else {Err(anyhow::anyhow!(errors.join("; ")))}
    }
    pub async fn initialize() -> Result<Self> {
        // Critical for macOS .app launches from Finder/Dock.
        grok_config::bootstrap_process_env();

        let paths = GrokPaths::discover(std::env::current_dir().ok().as_deref())
            .context("path discovery")?;
        paths.ensure_dirs().context("create panel directories")?;

        // Resolve the binary against the BASE (global-only) config and save
        // that — saving the overlay-merged view would silently promote
        // project-scoped settings into the user's global config.
        let resolved_binary = match grok_config::discover_grok_binary() {
            Ok(bin) => {
                info!(binary = %bin.display(), "resolved grok binary");
                Some(bin)
            }
            Err(e) => {
                warn!(error = %e, "grok binary not found — install Grok Build CLI");
                None
            }
        };
        let config = load_startup_config(&paths, resolved_binary)?;

        let binary = config
            .resolve_grok_binary()
            .unwrap_or_else(|_| PathBuf::from("grok"));

        let config = Arc::new(RwLock::new(config));
        let event_bus = shared_bus();
        let grok_cli = Arc::new(GrokCli::new(binary));

        let registry = SessionRegistry::new(event_bus.clone(), config.clone(), grok_cli.clone());

        let worktrees_root = {
            let cfg = config.read().await;
            cfg.worktrees_root
                .clone()
                .unwrap_or_else(|| paths.worktrees_dir.clone())
        };
        let worktrees = Arc::new(WorktreeManager::new(grok_cli.clone(), worktrees_root));
        let _ = worktrees.ensure_root().await;

        let extensions = Arc::new(ExtensionsService::new(
            config.clone(),
            paths.clone(),
            grok_cli.clone(),
            event_bus.clone(),
        ));

        let mut mcp = McpManager::new(
            config.clone(),
            paths.clone(),
            grok_cli.clone(),
            event_bus.clone(),
        )
        .context("mcp manager")?;
        if paths.grok_dir != paths.home_dir.join(".grok") {
            Arc::get_mut(&mut mcp)
                .context("configure isolated MCP manager")?
                .set_prefer_cli(false);
        }

        let memory = MemoryService::open(paths.memory_dir.clone(), event_bus.clone())
            .await
            .context("memory service")?;

        let persistence_path = paths.sessions_dir.join("control_panel.db");
        let persistence =
            Arc::new(Persistence::open(persistence_path).context("persistence open")?);

        let builds = crate::builds::BuildService::open(
            registry.clone(),
            worktrees.clone(),
            persistence.clone(),
            event_bus.clone(),
        )?;

        let scheduler = Scheduler::new(event_bus.clone());
        let registry_for_jobs = registry.clone();
        scheduler.set_handler(JobHandler::new(move |run: JobRunContext| {
            let registry = registry_for_jobs.clone();
            async move {
                let Some(cwd) = run.job.cwd.as_ref().filter(|cwd| !cwd.trim().is_empty()) else {
                    return Ok(JobOutcome::failed("scheduled job has no cwd configured; no worker launched"));
                };
                if run.cancel.requested() { return Ok(JobOutcome::failed("run cancelled before launch")); }
                let opts = grok_control_core::SpawnOptions {
                    mode: grok_control_core::AgentMode::Headless,
                    prompt: Some(run.job.prompt.clone()), plan_mode:true, always_approve:false,
                    ..Default::default()
                };
                // The snapshot already durably binds this run and session ID.
                // Unsupported native Plan capability is a visible failed run.
                if let Err(error) = registry.spawn_agent_preallocated(run.session_id, cwd, opts, Default::default()).await {
                    return Ok(JobOutcome {process:None,error:Some(error.to_string()),cleanup_complete:!registry.is_live(run.session_id)});
                }
                let outcome = tokio::select! {
                    result = registry.wait_headless(run.session_id) => result.map_err(|error| error.to_string())?,
                    _ = run.cancel.cancelled() => {
                        if let Err(error) = registry.cancel_session(run.session_id).await {
                            return Ok(JobOutcome { process:None, error:Some(error.to_string()), cleanup_complete:false });
                        }
                        registry.wait_headless(run.session_id).await.map_err(|error| error.to_string())?
                    }
                };
                Ok(JobOutcome::from_process(outcome))
            }
        })).await;
        let registry_for_cleanup=registry.clone();
        scheduler.set_cleanup_handler(JobCleanupHandler::new(move |id| {
            let registry=registry_for_cleanup.clone();async move {
                registry.cancel_session(id).await.map_err(|error|error.to_string())?;
                let outcome=registry.wait_headless(id).await.map_err(|error|error.to_string())?;
                Ok(JobOutcome::from_process(outcome))
            }
        })).await;
        {
            let persistence_for_sched = persistence.clone();
            scheduler.set_change_hook(move |snapshot| {
                let json = serde_json::to_string(&snapshot).map_err(|error|error.to_string())?;
                persistence_for_sched.set_kv("scheduler_jobs_v2", &json).map_err(|error|error.to_string())
            }).await;
            if let Some(json) = persistence.get_kv("scheduler_jobs_v2")? {
                let snapshot: SchedulerSnapshot = serde_json::from_str(&json)
                    .context("saved scheduler snapshot is corrupt; original bytes preserved")?;
                scheduler.restore_snapshot(snapshot).await?;
            } else if let Some(json) = persistence.get_kv("scheduler_jobs")? {
                let jobs: Vec<ScheduledJob> = serde_json::from_str(&json)
                    .context("saved legacy scheduler jobs are corrupt; original bytes preserved")?;
                scheduler.restore_jobs(jobs).await?;
            }
        }

        // Discovery log (Phase 0)
        match discover_environment() {
            Ok(report) => info!(?report, "environment discovery"),
            Err(e) => warn!(error = %e, "environment discovery failed"),
        }

        let dev_server = DevServerManager::new();
        let login = LoginManager::new(grok_cli.grok_path.clone());
        let haven_home = paths
            .grok_dir
            .parent()
            .context("profile home")?
            .to_path_buf();
        let haven = HavenClient::new(haven_home);

        // ELI12 narrator for the right panel (selected-thread side LLM calls).
        let explainer = {
            let cfg = config.read().await;
            ExplainerService::start(
                grok_cli.clone(),
                config.clone(),
                event_bus.clone(),
                cfg.explainer_enabled,
                cfg.explainer_backend.clone(),
                cfg.explainer_model.clone(),
            )
        };

        // Auto-link Haven (Hetzner process/temp host) on startup.
        {
            let haven_bg = haven.clone();
            tauri::async_runtime::spawn(async move {
                let cfg = haven_bg.config().await;
                if cfg.enabled && cfg.auto_connect {
                    let st = haven_bg.connect_and_status().await;
                    if st.connected {
                        info!(msg = %st.message, "haven linked on startup");
                    } else {
                        warn!(msg = %st.message, "haven auto-connect failed");
                    }
                }
            });
        }

        Ok(Self {
            builds,
            paths,
            config,
            event_bus,
            grok_cli,
            registry,
            worktrees,
            extensions,
            mcp,
            memory,
            scheduler,
            persistence,
            dev_server,
            login,
            haven,
            explainer,
        })
    }
}

async fn shutdown_owned_runtime(scheduler:&Arc<Scheduler>,registry:&Arc<SessionRegistry>) -> Result<()> {
    scheduler.fence_admission();registry.fence_admission()?;
    // The scheduler must save its typed result before its session can be removed.
    let scheduled=scheduler.shutdown().await;
    let retained=scheduler.retained_session_ids().await;
    let registered=registry.shutdown_preserving(retained).await;
    let errors=[scheduled.err().map(|e|e.to_string()),registered.err().map(|e|e.to_string())].into_iter().flatten().collect::<Vec<_>>();
    if errors.is_empty() {Ok(())} else {Err(anyhow::anyhow!(errors.join("; ")))}
}

#[cfg(test)]
mod release_tests {
    use super::*;

    #[test]
    fn discovery_cannot_replace_explicit_base_or_project_backend() {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = GrokPaths::discover(None).unwrap();
        paths.config_file = dir.path().join("config.toml");
        paths.project_config_file = Some(dir.path().join("project.toml"));
        let base = GrokConfig { grok_binary: Some("/generated/base-adapter".into()), ..Default::default() };
        base.save(&paths.config_file).unwrap();
        let loaded = load_startup_config(&paths, Some("/generated/discovered-real-cli".into())).unwrap();
        assert_eq!(loaded.grok_binary, base.grok_binary);
        assert_eq!(GrokConfig::load_base(&paths).unwrap().grok_binary, base.grok_binary);
        let overlay = GrokConfig { grok_binary: Some("/generated/project-adapter".into()), ..Default::default() };
        overlay.save(paths.project_config_file.as_ref().unwrap()).unwrap();
        let loaded = load_startup_config(&paths, Some("/generated/discovered-real-cli".into())).unwrap();
        assert_eq!(loaded.grok_binary, overlay.grok_binary);
        assert_eq!(GrokConfig::load_base(&paths).unwrap().grok_binary, base.grok_binary);
    }

    #[test]
    fn corrupt_base_or_overlay_never_overwrites_existing_config() {
        let dir = std::env::temp_dir().join(format!("c3-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let mut paths = GrokPaths::discover(None).unwrap();
        paths.config_file = dir.join("config.toml");
        paths.project_config_file = Some(dir.join("project.toml"));
        std::fs::write(&paths.config_file, "invalid=[").unwrap();
        assert!(load_startup_config(&paths, Some("/generated/grok".into())).is_err());
        assert_eq!(
            std::fs::read_to_string(&paths.config_file).unwrap(),
            "invalid=["
        );
        let valid = toml::to_string(&GrokConfig::default()).unwrap();
        std::fs::write(&paths.config_file, &valid).unwrap();
        std::fs::write(paths.project_config_file.as_ref().unwrap(), "invalid=[").unwrap();
        assert!(load_startup_config(&paths, Some("/generated/grok".into())).is_err());
        assert_eq!(std::fs::read_to_string(&paths.config_file).unwrap(), valid);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[cfg(target_os="macos")]
    #[tokio::test]
    async fn exit_preserves_scheduler_binding_on_failed_save_and_retries_verified_cleanup() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicBool,Ordering};
        let cwd=tempfile::tempdir().unwrap();let script=cwd.path().join("worker");
        std::fs::write(&script,"#!/bin/sh\nsleep 120 & wait\n").unwrap();std::fs::set_permissions(&script,std::fs::Permissions::from_mode(0o700)).unwrap();
        let bus=grok_events::shared_bus();let config=Arc::new(tokio::sync::RwLock::new(GrokConfig::default()));config.write().await.permissions.deny.clear();
        let registry=SessionRegistry::new(bus.clone(),config,Arc::new(GrokCli::new(script)));
        let scheduler=Scheduler::new(bus);let fail=Arc::new(AtomicBool::new(false));let failing=fail.clone();
        scheduler.set_change_hook(move |_|{if failing.load(Ordering::SeqCst){Err("injected full disk".into())}else{Ok(())}}).await;
        let runtime=registry.clone();scheduler.set_handler(JobHandler::new(move |run|{let runtime=runtime.clone();async move {
            let opts=grok_control_core::SpawnOptions{mode:grok_control_core::AgentMode::Headless,prompt:Some("offline generated".into()),approval_mode:Some(grok_acp::ApprovalMode::Ask),plan_mode:false,sandbox_profile:Some("unrestricted".into()),..Default::default()};
            runtime.spawn_agent_preallocated(run.session_id,run.job.cwd.as_deref().unwrap(),opts,Default::default()).await.map_err(|e|e.to_string())?;
            run.cancel.cancelled().await;runtime.cancel_session(run.session_id).await.map_err(|e|e.to_string())?;
            Ok(JobOutcome::from_process(runtime.wait_headless(run.session_id).await.map_err(|e|e.to_string())?))
        }})).await;
        let runtime=registry.clone();scheduler.set_cleanup_handler(JobCleanupHandler::new(move |id|{let runtime=runtime.clone();async move {runtime.cancel_session(id).await.map_err(|e|e.to_string())?;Ok(JobOutcome::from_process(runtime.wait_headless(id).await.map_err(|e|e.to_string())?))}})).await;
        let job=scheduler.add("quit".into(),"prompt".into(),grok_scheduler::ScheduleKind::Once{delay_secs:0},Some(cwd.path().to_str().unwrap().into()),None).await.unwrap();
        let session=tokio::time::timeout(std::time::Duration::from_secs(2),async {loop{if let Some(run)=scheduler.list().await[0].active_run.clone(){if registry.is_live(run.session_id){break run.session_id;}}tokio::task::yield_now().await;}}).await.unwrap();
        fail.store(true,Ordering::SeqCst);
        assert!(shutdown_owned_runtime(&scheduler,&registry).await.is_err());
        assert!(registry.is_live(session));assert!(scheduler.retained_session_ids().await.contains(&session));
        fail.store(false,Ordering::SeqCst);
        shutdown_owned_runtime(&scheduler,&registry).await.unwrap();
        assert!(!registry.is_live(session));
        let saved=scheduler.list().await;
        assert_eq!(saved[0].id,job.id);assert_eq!(saved[0].run_count,1);assert_eq!(saved[0].runs[0].session_id,session);
        assert!(saved[0].runs[0].outcome.as_ref().unwrap().cleanup_complete);
    }

}
