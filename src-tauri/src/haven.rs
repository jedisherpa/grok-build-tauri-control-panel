//! Haven client — process keep-alive + temp files on a remote VPS (e.g. Hetzner).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HavenConfig {
    pub enabled: bool,
    pub base_url: String,
    pub auth_token: String,
    pub label: String,
    pub auto_connect: bool,
    /// Explicit opt-in to send the bearer token over plaintext http to a
    /// public host. The token grants shell execution — anyone on the network
    /// path can read it. Prefer Tailscale or a TLS proxy.
    pub allow_insecure_http: bool,
}

impl Default for HavenConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: String::new(),
            auth_token: String::new(),
            label: "haven".into(),
            auto_connect: true,
            allow_insecure_http: false,
        }
    }
}

impl HavenConfig {
    pub fn config_path(home: &std::path::Path) -> PathBuf {
        home.join(".grok").join("control-panel").join("haven.toml")
    }

    pub fn load(home: &std::path::Path) -> Self {
        let path = Self::config_path(home);
        match std::fs::read_to_string(&path) {
            Ok(raw) => match toml::from_str(&raw) {
                Ok(c) => c,
                Err(e) => {
                    warn!(error = %e, path = %path.display(), "invalid haven.toml");
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, home: &std::path::Path) -> Result<(), String> {
        let path = Self::config_path(home);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let raw = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, raw).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn ready(&self) -> bool {
        self.enabled && !self.base_url.trim().is_empty() && self.auth_token.len() >= 8
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HavenStatus {
    pub configured: bool,
    pub connected: bool,
    pub label: String,
    pub base_url: String,
    pub jobs: Option<u64>,
    pub running: Option<u64>,
    pub files: Option<u64>,
    pub message: String,
    pub last_checked: Option<String>,
}

pub struct HavenClient {
    config: RwLock<HavenConfig>,
    http: reqwest::Client,
    last_status: RwLock<HavenStatus>,
    home: PathBuf,
}

impl HavenClient {
    pub fn new(home: PathBuf) -> Arc<Self> {
        let config = HavenConfig::load(&home);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .user_agent(format!("BombCode/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client");
        Arc::new(Self {
            config: RwLock::new(config),
            http,
            last_status: RwLock::new(HavenStatus {
                configured: false,
                connected: false,
                label: String::new(),
                base_url: String::new(),
                jobs: None,
                running: None,
                files: None,
                message: "Haven not checked yet".into(),
                last_checked: None,
            }),
            home,
        })
    }

    pub async fn config(&self) -> HavenConfig {
        self.config.read().await.clone()
    }

    pub async fn set_config(&self, cfg: HavenConfig) -> Result<(), String> {
        cfg.save(&self.home)?;
        *self.config.write().await = cfg;
        Ok(())
    }

    pub async fn last_status(&self) -> HavenStatus {
        self.last_status.read().await.clone()
    }

    async fn auth_header(&self) -> Result<(String, String), String> {
        let cfg = self.config.read().await;
        if !cfg.ready() {
            return Err("Haven not configured".into());
        }
        Ok((
            cfg.base_url.trim_end_matches('/').to_string(),
            cfg.auth_token.clone(),
        ))
    }

    /// Connect / health + status. Called on app startup when auto_connect.
    pub async fn connect_and_status(&self) -> HavenStatus {
        let cfg = self.config.read().await.clone();
        let mut status = HavenStatus {
            configured: cfg.ready(),
            connected: false,
            label: cfg.label.clone(),
            base_url: cfg.base_url.clone(),
            jobs: None,
            running: None,
            files: None,
            message: String::new(),
            last_checked: Some(chrono::Utc::now().to_rfc3339()),
        };

        if !cfg.ready() {
            status.message = if !cfg.enabled {
                "Haven disabled".into()
            } else {
                "Haven not configured (missing URL or token)".into()
            };
            *self.last_status.write().await = status.clone();
            return status;
        }

        let base = cfg.base_url.trim_end_matches('/');
        // Health (no auth)
        match self
            .http
            .get(format!("{base}/health"))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {}
            Ok(resp) => {
                status.message = format!("Haven health HTTP {}", resp.status());
                *self.last_status.write().await = status.clone();
                return status;
            }
            Err(e) => {
                status.message = format!("Haven unreachable: {e}");
                *self.last_status.write().await = status.clone();
                return status;
            }
        }

        match self
            .http
            .get(format!("{base}/v1/status"))
            .header("Authorization", format!("Bearer {}", cfg.auth_token))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                #[derive(Deserialize)]
                struct Remote {
                    jobs: Option<u64>,
                    running: Option<u64>,
                    files: Option<u64>,
                }
                let body = resp.json::<Remote>().await.ok();
                status.connected = true;
                status.jobs = body.as_ref().and_then(|b| b.jobs);
                status.running = body.as_ref().and_then(|b| b.running);
                status.files = body.as_ref().and_then(|b| b.files);
                status.message = format!(
                    "Haven · {} · {} jobs ({} running) · {} files",
                    cfg.label,
                    status.jobs.unwrap_or(0),
                    status.running.unwrap_or(0),
                    status.files.unwrap_or(0)
                );
                info!(label = %cfg.label, url = %cfg.base_url, "haven connected");
            }
            Ok(resp) => {
                status.message = format!("Haven auth/status HTTP {}", resp.status());
            }
            Err(e) => {
                status.message = format!("Haven status error: {e}");
            }
        }

        *self.last_status.write().await = status.clone();
        status
    }

    pub async fn list_jobs(&self) -> Result<serde_json::Value, String> {
        let (base, token) = self.auth_header().await?;
        let resp = self
            .http
            .get(format!("{base}/v1/jobs"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("haven jobs HTTP {}", resp.status()));
        }
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn start_shell(
        &self,
        name: String,
        command: String,
        cwd: Option<String>,
        keep_alive: bool,
    ) -> Result<serde_json::Value, String> {
        let (base, token) = self.auth_header().await?;
        let resp = self
            .http
            .post(format!("{base}/v1/jobs/shell"))
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({
                "name": name,
                "command": command,
                "cwd": cwd,
                "keep_alive": keep_alive,
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            let t = resp.text().await.unwrap_or_default();
            return Err(format!("haven start shell failed: {t}"));
        }
        resp.json().await.map_err(|e| e.to_string())
    }

    /// Tail a job's log (last `bytes` bytes of combined output).
    pub async fn job_log(&self, id: String, bytes: u64) -> Result<String, String> {
        let (base, token) = self.auth_header().await?;
        let resp = self
            .http
            .get(format!("{base}/v1/jobs/{id}/log?bytes={bytes}"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("haven job log HTTP {}", resp.status()));
        }
        resp.text().await.map_err(|e| e.to_string())
    }

    /// Stop and remove a job.
    pub async fn remove_job(&self, id: String) -> Result<serde_json::Value, String> {
        let (base, token) = self.auth_header().await?;
        let resp = self
            .http
            .delete(format!("{base}/v1/jobs/{id}"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            let t = resp.text().await.unwrap_or_default();
            return Err(format!("haven remove job failed: {t}"));
        }
        resp.json()
            .await
            .or_else(|_| Ok(serde_json::json!({ "removed": true })))
    }

    pub async fn list_files(&self) -> Result<serde_json::Value, String> {
        let (base, token) = self.auth_header().await?;
        let resp = self
            .http
            .get(format!("{base}/v1/files"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("haven files HTTP {}", resp.status()));
        }
        resp.json().await.map_err(|e| e.to_string())
    }
}
