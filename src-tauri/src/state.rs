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
use grok_scheduler::{JobHandler, Scheduler, ScheduledJob};
use grok_worktree::WorktreeManager;

use crate::devserver::DevServerManager;

pub struct AppState {
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
}

impl AppState {
    pub async fn initialize() -> Result<Self> {
        // Critical for macOS .app launches from Finder/Dock.
        grok_config::bootstrap_process_env();

        let paths = GrokPaths::discover(std::env::current_dir().ok().as_deref())
            .context("path discovery")?;
        let _ = paths.ensure_dirs();

        let mut config = GrokConfig::load(&paths).unwrap_or_default();
        // Always re-resolve binary so GUI gets absolute path to ~/.grok/bin/grok.
        match grok_config::discover_grok_binary() {
            Ok(bin) => {
                info!(binary = %bin.display(), "resolved grok binary");
                config.grok_binary = Some(bin);
            }
            Err(e) => warn!(error = %e, "grok binary not found — install Grok Build CLI"),
        }

        // Persist panel settings (never writes Grok CLI ~/.grok/config.toml).
        let _ = config.save(&paths.config_file);

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

        let mcp = McpManager::new(
            config.clone(),
            paths.clone(),
            grok_cli.clone(),
            event_bus.clone(),
        )
        .context("mcp manager")?;

        let memory = MemoryService::open(paths.memory_dir.clone(), event_bus.clone())
            .await
            .context("memory service")?;

        let persistence_path = paths.sessions_dir.join("control_panel.db");
        let persistence =
            Arc::new(Persistence::open(persistence_path).context("persistence open")?);

        let scheduler = Scheduler::new(event_bus.clone());
        let registry_for_jobs = registry.clone();
        let persistence_for_jobs = persistence.clone();
        scheduler
            .set_handler(JobHandler::new(move |job: ScheduledJob| {
                let registry = registry_for_jobs.clone();
                let persistence = persistence_for_jobs.clone();
                async move {
                    info!(job_id = %job.id, name = %job.name, "scheduler firing job");
                    let cwd = job
                        .cwd
                        .clone()
                        .unwrap_or_else(|| std::env::current_dir()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|_| "/tmp".into()));

                    // Prefer headless one-shot for scheduled routines
                    let opts = grok_control_core::SpawnOptions {
                        mode: grok_control_core::AgentMode::Headless,
                        prompt: Some(job.prompt.clone()),
                        plan_mode: true,
                        always_approve: false,
                        ..Default::default()
                    };

                    match registry.spawn_agent(&cwd, opts).await {
                        Ok(id) => {
                            let _ = persistence.set_kv(
                                &format!("last_job_{}", job.id),
                                &id.to_string(),
                            );
                        }
                        Err(e) => {
                            // Offline / no binary: record intent only
                            warn!(error = %e, "scheduler could not spawn agent");
                            let _ = persistence.set_kv(
                                &format!("last_job_error_{}", job.id),
                                &e.to_string(),
                            );
                        }
                    }
                }
            }))
            .await;

        // Discovery log (Phase 0)
        match discover_environment() {
            Ok(report) => info!(?report, "environment discovery"),
            Err(e) => warn!(error = %e, "environment discovery failed"),
        }

        let dev_server = DevServerManager::new();
        let login = LoginManager::new(grok_cli.grok_path.clone());

        Ok(Self {
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
        })
    }
}
