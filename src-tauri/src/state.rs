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
use grok_scheduler::{JobHandler, ScheduledJob, Scheduler};
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
        let persistence_for_jobs = persistence.clone();
        scheduler
            .set_handler(JobHandler::new(move |job: ScheduledJob| {
                let registry = registry_for_jobs.clone();
                let persistence = persistence_for_jobs.clone();
                async move {
                    info!(job_id = %job.id, name = %job.name, "scheduler firing job");
                    // A Finder-launched app's current_dir is `/` — running an
                    // agent from filesystem root is never what anyone wants.
                    let Some(cwd) = job.cwd.clone().filter(|c| !c.trim().is_empty()) else {
                        warn!(job_id = %job.id, "scheduled job has no cwd; skipping run");
                        let _ = persistence.set_kv(
                            &format!("last_job_error_{}", job.id),
                            "job has no cwd configured",
                        );
                        return;
                    };

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
                            let _ = persistence
                                .set_kv(&format!("last_job_{}", job.id), &id.to_string());
                        }
                        Err(e) => {
                            // Offline / no binary: record intent only
                            warn!(error = %e, "scheduler could not spawn agent");
                            let _ = persistence
                                .set_kv(&format!("last_job_error_{}", job.id), &e.to_string());
                        }
                    }
                }
            }))
            .await;

        // Durable routines: persist the job list on every change and reload
        // it at startup (jobs previously lived only in memory).
        {
            let persistence_for_sched = persistence.clone();
            scheduler
                .set_change_hook(move |jobs| {
                    if let Ok(json) = serde_json::to_string(&jobs) {
                        let _ = persistence_for_sched.set_kv("scheduler_jobs", &json);
                    }
                })
                .await;
            if let Ok(Some(json)) = persistence.get_kv("scheduler_jobs") {
                if let Ok(jobs) = serde_json::from_str::<Vec<ScheduledJob>>(&json) {
                    scheduler.restore_jobs(jobs).await;
                }
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
}
