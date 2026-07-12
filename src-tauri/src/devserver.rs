//! Project preview / dev-server manager for live testing.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DevServerKind {
    /// package.json script (dev/start/vite/next)
    Npm,
    /// python -m http.server (static fallback)
    Static,
    /// cargo run
    Cargo,
    /// Unknown / not started
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevServerStatus {
    pub running: bool,
    pub kind: DevServerKind,
    pub cwd: Option<String>,
    pub url: Option<String>,
    pub port: Option<u16>,
    pub command: Option<String>,
    pub pid: Option<u32>,
    pub message: String,
    pub log_tail: Vec<String>,
}

impl Default for DevServerStatus {
    fn default() -> Self {
        Self {
            running: false,
            kind: DevServerKind::None,
            cwd: None,
            url: None,
            port: None,
            command: None,
            pid: None,
            message: "Dev server stopped".into(),
            log_tail: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedProject {
    pub cwd: String,
    pub kind: DevServerKind,
    pub command: Vec<String>,
    pub suggested_port: u16,
    pub label: String,
}

struct RunningServer {
    child: Child,
    kind: DevServerKind,
    cwd: PathBuf,
    url: String,
    port: u16,
    command: String,
    log_tail: Arc<Mutex<Vec<String>>>,
}

pub struct DevServerManager {
    inner: Mutex<Option<RunningServer>>,
}

impl DevServerManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(None),
        })
    }

    pub async fn status(&self) -> DevServerStatus {
        let mut guard = self.inner.lock().await;
        if let Some(ref mut running) = *guard {
            // Check if still alive
            match running.child.try_wait() {
                Ok(Some(status)) => {
                    let logs = running.log_tail.lock().await.clone();
                    *guard = None;
                    return DevServerStatus {
                        running: false,
                        kind: DevServerKind::None,
                        message: format!("Dev server exited ({status})"),
                        log_tail: logs,
                        ..Default::default()
                    };
                }
                Ok(None) => {
                    let logs = running.log_tail.lock().await.clone();
                    return DevServerStatus {
                        running: true,
                        kind: running.kind.clone(),
                        cwd: Some(running.cwd.display().to_string()),
                        url: Some(running.url.clone()),
                        port: Some(running.port),
                        command: Some(running.command.clone()),
                        pid: running.child.id(),
                        message: format!("Running · {}", running.url),
                        log_tail: logs,
                    };
                }
                Err(e) => {
                    warn!(error = %e, "try_wait failed");
                }
            }
        }
        DevServerStatus::default()
    }

    pub async fn stop(&self) -> DevServerStatus {
        let mut guard = self.inner.lock().await;
        if let Some(mut running) = guard.take() {
            let _ = running.child.kill().await;
            let _ = running.child.wait().await;
            info!(cwd = %running.cwd.display(), "dev server stopped");
            return DevServerStatus {
                running: false,
                message: "Dev server stopped".into(),
                cwd: Some(running.cwd.display().to_string()),
                ..Default::default()
            };
        }
        DevServerStatus::default()
    }

    pub fn detect(cwd: &Path) -> Result<DetectedProject, String> {
        if !cwd.is_absolute() {
            return Err("cwd must be absolute".into());
        }
        if !cwd.is_dir() {
            return Err(format!("not a directory: {}", cwd.display()));
        }

        let pkg = cwd.join("package.json");
        if pkg.is_file() {
            if let Ok(raw) = std::fs::read_to_string(&pkg) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                    let scripts = v.get("scripts").cloned().unwrap_or_default();
                    let deps = merge_deps(&v);
                    // Prefer framework defaults
                    if has_dep(&deps, "next")
                        || (scripts.get("dev").is_some() && raw.contains("next"))
                    {
                        return Ok(DetectedProject {
                            cwd: cwd.display().to_string(),
                            kind: DevServerKind::Npm,
                            command: shell_cmd("npm run dev -- --port 3000"),
                            suggested_port: 3000,
                            label: "Next.js / npm run dev".into(),
                        });
                    }
                    if has_dep(&deps, "vite")
                        || (scripts.get("dev").is_some() && raw.contains("vite"))
                    {
                        return Ok(DetectedProject {
                            cwd: cwd.display().to_string(),
                            kind: DevServerKind::Npm,
                            command: shell_cmd("npm run dev -- --port 5173 --host"),
                            suggested_port: 5173,
                            label: "Vite / npm run dev".into(),
                        });
                    }
                    if scripts.get("dev").is_some() {
                        let port = guess_port_from_package(&raw).unwrap_or(5173);
                        return Ok(DetectedProject {
                            cwd: cwd.display().to_string(),
                            kind: DevServerKind::Npm,
                            command: shell_cmd("npm run dev"),
                            suggested_port: port,
                            label: "npm run dev".into(),
                        });
                    }
                    if scripts.get("start").is_some() {
                        return Ok(DetectedProject {
                            cwd: cwd.display().to_string(),
                            kind: DevServerKind::Npm,
                            command: shell_cmd("npm start"),
                            suggested_port: 3000,
                            label: "npm start".into(),
                        });
                    }
                }
            }
        }

        if cwd.join("Cargo.toml").is_file() {
            // Prefer static if there's also frontend; else cargo run is heavy — still offer static for docs
            if cwd.join("index.html").is_file()
                || cwd.join("frontend").join("index.html").is_file()
                || cwd.join("public").is_dir()
            {
                let root = if cwd.join("frontend").join("index.html").is_file() {
                    cwd.join("frontend")
                } else if cwd.join("public").is_dir() {
                    cwd.join("public")
                } else {
                    cwd.to_path_buf()
                };
                let port = free_port(8765);
                return Ok(DetectedProject {
                    cwd: root.display().to_string(),
                    kind: DevServerKind::Static,
                    command: vec![
                        "python3".into(),
                        "-m".into(),
                        "http.server".into(),
                        port.to_string(),
                        "--bind".into(),
                        "127.0.0.1".into(),
                    ],
                    suggested_port: port,
                    label: format!("static · {}", root.display()),
                });
            }
            return Ok(DetectedProject {
                cwd: cwd.display().to_string(),
                kind: DevServerKind::Cargo,
                command: shell_cmd("cargo run"),
                suggested_port: 8080,
                label: "cargo run".into(),
            });
        }

        // Static fallback: serve cwd (or frontend/public/dist)
        let serve_root = ["frontend", "public", "dist", "build", "out"]
            .iter()
            .map(|s| cwd.join(s))
            .find(|p| p.is_dir() || p.join("index.html").is_file())
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| cwd.to_path_buf());

        let port = free_port(8765);
        Ok(DetectedProject {
            cwd: serve_root.display().to_string(),
            kind: DevServerKind::Static,
            command: vec![
                "python3".into(),
                "-m".into(),
                "http.server".into(),
                port.to_string(),
                "--bind".into(),
                "127.0.0.1".into(),
            ],
            suggested_port: port,
            label: format!("static file server · {}", serve_root.display()),
        })
    }

    pub async fn start(&self, cwd: &Path, open_browser: bool) -> Result<DevServerStatus, String> {
        // Replace any existing server
        let _ = self.stop().await;

        let detected = Self::detect(cwd)?;
        let port = detected.suggested_port;
        let url = format!("http://127.0.0.1:{port}");
        let work_dir = PathBuf::from(&detected.cwd);

        let (program, args) = split_command(&detected.command)?;
        let cmd_display = detected.command.join(" ");

        let mut cmd = Command::new(&program);
        cmd.args(&args)
            .current_dir(&work_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // GUI PATH
        let path = std::env::var("PATH").unwrap_or_default();
        let home = std::env::var("HOME").unwrap_or_default();
        cmd.env(
            "PATH",
            format!(
                "{home}/.grok/bin:{home}/.cargo/bin:{home}/.local/bin:/opt/homebrew/bin:/usr/local/bin:{path}"
            ),
        );
        if !home.is_empty() {
            cmd.env("HOME", &home);
        }
        cmd.env("BROWSER", "none"); // don't auto-open second browser from vite/next
        cmd.env("PORT", port.to_string());

        info!(?program, ?args, cwd = %work_dir.display(), %url, "starting dev server");
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to start `{cmd_display}`: {e}"))?;

        let log_tail = Arc::new(Mutex::new(Vec::new()));
        if let Some(stdout) = child.stdout.take() {
            let logs = log_tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut g = logs.lock().await;
                    g.push(line);
                    if g.len() > 80 {
                        let drain = g.len() - 80;
                        g.drain(0..drain);
                    }
                }
            });
        }
        if let Some(stderr) = child.stderr.take() {
            let logs = log_tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut g = logs.lock().await;
                    g.push(line);
                    if g.len() > 80 {
                        let drain = g.len() - 80;
                        g.drain(0..drain);
                    }
                }
            });
        }

        // Poll for the port to actually bind (up to ~12s) instead of a fixed
        // 900ms sleep declaring success before the server exists. Frameworks
        // that auto-increment a busy port are reported as such.
        let mut bound = false;
        for _ in 0..40 {
            if let Ok(Some(status)) = child.try_wait() {
                let logs = log_tail.lock().await.clone();
                return Err(format!(
                    "dev server exited immediately ({status})\n{}",
                    logs.join("\n")
                ));
            }
            if std::net::TcpStream::connect_timeout(
                &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
                Duration::from_millis(150),
            )
            .is_ok()
            {
                bound = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        if !bound {
            let logs = log_tail.lock().await.clone();
            warn!(port, "dev server has not bound its expected port yet; it may be using another port\n{}", logs.join("\n"));
        }

        {
            let mut guard = self.inner.lock().await;
            *guard = Some(RunningServer {
                child,
                kind: detected.kind.clone(),
                cwd: work_dir,
                url: url.clone(),
                port,
                command: cmd_display.clone(),
                log_tail: log_tail.clone(),
            });
        }

        if open_browser {
            let _ = open_browser_url(&url).await;
        }

        // Give frameworks a bit longer, then re-open if first open was early
        if matches!(detected.kind, DevServerKind::Npm) {
            let url2 = url.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let _ = open_browser_url(&url2).await;
            });
        }

        Ok(self.status().await)
    }

    pub async fn open_in_browser(&self) -> Result<String, String> {
        let st = self.status().await;
        let url = st
            .url
            .ok_or_else(|| "Dev server is not running".to_string())?;
        open_browser_url(&url).await?;
        Ok(url)
    }

    pub async fn reveal_project(cwd: &Path) -> Result<(), String> {
        if !cwd.is_dir() {
            return Err(format!("not a directory: {}", cwd.display()));
        }
        #[cfg(target_os = "macos")]
        {
            Command::new("open")
                .arg(cwd)
                .status()
                .await
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = cwd;
            Err("reveal not supported on this OS".into())
        }
    }
}

fn shell_cmd(s: &str) -> Vec<String> {
    // $SHELL with sane fallbacks — hardcoded zsh broke non-zsh setups.
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|sh| !sh.trim().is_empty() && std::path::Path::new(sh).exists())
        .or_else(|| {
            ["/bin/zsh", "/bin/bash", "/bin/sh"]
                .iter()
                .find(|c| std::path::Path::new(c).exists())
                .map(|c| c.to_string())
        })
        .unwrap_or_else(|| "/bin/sh".into());
    vec![shell, "-lc".into(), s.to_string()]
}

fn split_command(cmd: &[String]) -> Result<(String, Vec<String>), String> {
    if cmd.is_empty() {
        return Err("empty command".into());
    }
    Ok((cmd[0].clone(), cmd[1..].to_vec()))
}

fn free_port(preferred: u16) -> u16 {
    for p in preferred..preferred + 40 {
        if TcpListener::bind(("127.0.0.1", p)).is_ok() {
            return p;
        }
    }
    preferred
}

fn merge_deps(v: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    for key in ["dependencies", "devDependencies"] {
        if let Some(obj) = v.get(key).and_then(|x| x.as_object()) {
            for (k, val) in obj {
                m.insert(k.clone(), val.clone());
            }
        }
    }
    m
}

fn has_dep(deps: &serde_json::Map<String, serde_json::Value>, name: &str) -> bool {
    deps.contains_key(name)
}

fn guess_port_from_package(raw: &str) -> Option<u16> {
    [5173u16, 3000, 4173, 8080, 8000, 4200]
        .into_iter()
        .find(|&p| raw.contains(&p.to_string()))
}

async fn open_browser_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .status()
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = url;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detects_static() {
        let dir = std::env::temp_dir().join(format!("bomb-code-static-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("index.html"), "<h1>hi</h1>").unwrap();
        let d = DevServerManager::detect(&dir).unwrap();
        assert_eq!(d.kind, DevServerKind::Static);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_npm_dev() {
        let dir = std::env::temp_dir().join(format!("bomb-code-npm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"dev":"vite"},"devDependencies":{"vite":"^5"}}"#,
        )
        .unwrap();
        let d = DevServerManager::detect(&dir).unwrap();
        assert_eq!(d.kind, DevServerKind::Npm);
        assert_eq!(d.suggested_port, 5173);
        let _ = fs::remove_dir_all(&dir);
    }
}
