//! Project preview / dev-server manager for live testing.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Package manager to drive, chosen by lockfile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pm {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl Pm {
    fn detect(cwd: &Path) -> Pm {
        if cwd.join("pnpm-lock.yaml").is_file() {
            Pm::Pnpm
        } else if cwd.join("yarn.lock").is_file() {
            Pm::Yarn
        } else if cwd.join("bun.lockb").is_file() || cwd.join("bun.lock").is_file() {
            Pm::Bun
        } else {
            Pm::Npm
        }
    }

    /// Run a package.json script, with optional extra args for the script itself.
    /// Only npm needs `--` to stop swallowing the flags.
    fn run(&self, script: &str, args: &str) -> String {
        let (base, sep) = match self {
            Pm::Npm => ("npm run", " --"),
            Pm::Pnpm => ("pnpm", ""),
            Pm::Yarn => ("yarn", ""),
            Pm::Bun => ("bun run", ""),
        };
        if args.is_empty() {
            format!("{base} {script}")
        } else {
            format!("{base} {script}{sep} {args}")
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Pm::Npm => "npm",
            Pm::Pnpm => "pnpm",
            Pm::Yarn => "yarn",
            Pm::Bun => "bun",
        }
    }
}

/// Pull a local dev URL out of a line of server output — vite prints
/// "  ➜  Local:   http://localhost:5173/", next "- Local: http://localhost:3000".
///
/// This is how we learn the *real* port: frameworks silently move to another
/// one when ours is busy, so anything we guessed up front may be a lie.
fn extract_local_url(line: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        // A port is required: a bare "http://localhost" tells us nothing.
        Regex::new(r"https?://(?:localhost|127\.0\.0\.1|0\.0\.0\.0):(\d{2,5})")
            .expect("static regex")
    });
    let port: u16 = re.captures(line)?.get(1)?.as_str().parse().ok()?;
    Some(format!("http://127.0.0.1:{port}"))
}

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
                    let pm = Pm::detect(cwd);
                    let has_dev = scripts.get("dev").is_some();

                    // Framework defaults. We ask for a port we have confirmed is
                    // free, but the server still gets the last word — start()
                    // reads the URL it actually prints.
                    if has_dep(&deps, "next") || (has_dev && raw.contains("next")) {
                        let port = free_port(3000);
                        return Ok(DetectedProject {
                            cwd: cwd.display().to_string(),
                            kind: DevServerKind::Npm,
                            command: shell_cmd(&pm.run("dev", &format!("--port {port}"))),
                            suggested_port: port,
                            label: format!("Next.js · {} run dev", pm.label()),
                        });
                    }
                    if has_dep(&deps, "vite") || (has_dev && raw.contains("vite")) {
                        let port = free_port(5173);
                        return Ok(DetectedProject {
                            cwd: cwd.display().to_string(),
                            kind: DevServerKind::Npm,
                            command: shell_cmd(&pm.run("dev", &format!("--port {port} --host"))),
                            suggested_port: port,
                            label: format!("Vite · {} run dev", pm.label()),
                        });
                    }
                    // Unknown framework: run its script untouched and let it pick
                    // the port. Guessing one from the text of package.json was
                    // never better than a coin flip.
                    if has_dev {
                        return Ok(DetectedProject {
                            cwd: cwd.display().to_string(),
                            kind: DevServerKind::Npm,
                            command: shell_cmd(&pm.run("dev", "")),
                            suggested_port: 0, // unknown until it tells us
                            label: format!("{} run dev", pm.label()),
                        });
                    }
                    if scripts.get("start").is_some() {
                        return Ok(DetectedProject {
                            cwd: cwd.display().to_string(),
                            kind: DevServerKind::Npm,
                            command: shell_cmd(&pm.run("start", "")),
                            suggested_port: 0,
                            label: format!("{} start", pm.label()),
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
        cmd.env("BROWSER", "none"); // don't let vite/next open a browser of their own
        if port != 0 {
            cmd.env("PORT", port.to_string());
        }

        info!(?program, ?args, cwd = %work_dir.display(), port, "starting dev server");
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to start `{cmd_display}`: {e}"))?;

        // Both pipes feed the same ring buffer, and both watch for the URL the
        // server prints — that announcement is the only trustworthy source of
        // the port, since a framework will quietly move off a busy one.
        let log_tail = Arc::new(Mutex::new(Vec::new()));
        let found_url: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        for pipe in [
            child.stdout.take().map(PipeOut::Stdout),
            child.stderr.take().map(PipeOut::Stderr),
        ]
        .into_iter()
        .flatten()
        {
            let logs = log_tail.clone();
            let url_slot = found_url.clone();
            tokio::spawn(async move {
                let mut lines = pipe.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(u) = extract_local_url(&line) {
                        let mut slot = url_slot.lock().await;
                        if slot.is_none() {
                            *slot = Some(u);
                        }
                    }
                    let mut g = logs.lock().await;
                    g.push(line);
                    if g.len() > 80 {
                        let drain = g.len() - 80;
                        g.drain(0..drain);
                    }
                }
            });
        }

        // Wait for the server to announce itself (up to ~25s). Probing the port
        // we asked for is only a fallback: if some *other* process already holds
        // it, a successful connect would have us pointing at the wrong server.
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        let mut url: Option<String> = None;
        while std::time::Instant::now() < deadline {
            if let Ok(Some(status)) = child.try_wait() {
                let logs = log_tail.lock().await.clone();
                return Err(format!(
                    "dev server exited immediately ({status})\n{}",
                    logs.join("\n")
                ));
            }
            if let Some(u) = found_url.lock().await.clone() {
                url = Some(u);
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        if url.is_none() && port != 0 && port_is_bound(port) {
            warn!(port, "dev server printed no URL; falling back to the requested port");
            url = Some(format!("http://127.0.0.1:{port}"));
        }

        let logs = log_tail.lock().await.clone();
        let Some(url) = url else {
            // Do not report a URL we cannot stand behind.
            let _ = child.kill().await;
            return Err(format!(
                "`{cmd_display}` started but never served a URL — is it a dev server?\n{}",
                logs.join("\n")
            ));
        };
        let bound_port = url
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(port);

        {
            let mut guard = self.inner.lock().await;
            *guard = Some(RunningServer {
                child,
                kind: detected.kind.clone(),
                cwd: work_dir,
                url: url.clone(),
                port: bound_port,
                command: cmd_display,
                log_tail: log_tail.clone(),
            });
        }

        // One open, of a URL we have actually seen — no more double-open to
        // paper over a browser racing a server that was not up yet.
        if open_browser {
            let _ = open_browser_url(&url).await;
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

fn port_is_bound(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(200),
    )
    .is_ok()
}

/// stdout and stderr differ in type but not in how we read them; frameworks are
/// inconsistent about which one they announce the URL on.
enum PipeOut {
    Stdout(tokio::process::ChildStdout),
    Stderr(tokio::process::ChildStderr),
}

impl PipeOut {
    fn lines(self) -> Box<dyn LineSource> {
        match self {
            PipeOut::Stdout(s) => Box::new(BufReader::new(s).lines()),
            PipeOut::Stderr(s) => Box::new(BufReader::new(s).lines()),
        }
    }
}

/// Object-safe view over `Lines<BufReader<_>>` for the two pipe types.
trait LineSource: Send {
    fn next_line(
        &mut self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = std::io::Result<Option<String>>> + Send + '_>,
    >;
}

impl<R: tokio::io::AsyncBufRead + Unpin + Send> LineSource for tokio::io::Lines<R> {
    fn next_line(
        &mut self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = std::io::Result<Option<String>>> + Send + '_>,
    > {
        Box::pin(tokio::io::Lines::next_line(self))
    }
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
        // The port must be free, but need not be the canonical one: another
        // vite may already hold 5173.
        assert!(d.suggested_port >= 5173, "got {}", d.suggested_port);
        let cmd = d.command.join(" ");
        assert!(cmd.contains("npm run dev -- --port"), "got {cmd}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lockfile_picks_the_package_manager() {
        let dir = std::env::temp_dir().join(format!("bomb-code-pnpm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("package.json"), r#"{"scripts":{"dev":"astro dev"}}"#).unwrap();
        fs::write(dir.join("pnpm-lock.yaml"), "lockfileVersion: 9").unwrap();

        let d = DevServerManager::detect(&dir).unwrap();
        let cmd = d.command.join(" ");
        assert!(cmd.contains("pnpm dev"), "got {cmd}");
        assert!(!cmd.contains("npm run"), "npm must not be assumed: {cmd}");
        // An unknown framework gets no invented port — it tells us at runtime.
        assert_eq!(d.suggested_port, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_the_url_the_server_prints() {
        // The formats we actually have to survive, ANSI colour and all.
        let cases = [
            ("  ➜  Local:   http://localhost:5173/", 5173),
            ("- Local:        http://localhost:3001", 3001),
            ("\x1b[32m  ➜\x1b[39m  \x1b[1mLocal\x1b[22m:   \x1b[36mhttp://localhost:4321/\x1b[39m", 4321),
            ("Serving HTTP on 127.0.0.1:8765 ...", 0), // no scheme: not a URL
            ("VITE ready in 300 ms", 0),
            ("listening on http://0.0.0.0:8080", 8080),
        ];
        for (line, want) in cases {
            let got = extract_local_url(line);
            if want == 0 {
                assert!(got.is_none(), "{line:?} should not yield a URL, got {got:?}");
            } else {
                assert_eq!(
                    got.as_deref(),
                    Some(format!("http://127.0.0.1:{want}").as_str()),
                    "line: {line:?}"
                );
            }
        }
    }

    /// End-to-end against a real vite server whose preferred port is taken —
    /// the case that used to report a URL nobody was listening on. Needs npm and
    /// a prepared project, so it is opt-in:
    ///   BOMB_DEVSERVER_FIXTURE=/path/to/vite/project \
    ///     cargo test -p grok-build-control-panel -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "needs a real npm project; set BOMB_DEVSERVER_FIXTURE"]
    async fn reports_the_port_vite_actually_took() {
        let Ok(dir) = std::env::var("BOMB_DEVSERVER_FIXTURE") else {
            panic!("set BOMB_DEVSERVER_FIXTURE to a vite project");
        };
        let dir = PathBuf::from(dir);
        // Squat on vite's default so it is forced to move.
        let squat = TcpListener::bind(("127.0.0.1", 5173)).expect("bind 5173");

        let mgr = DevServerManager::new();
        let st = mgr.start(&dir, false).await.expect("dev server should start");
        println!("url={:?} port={:?}", st.url, st.port);

        let url = st.url.clone().expect("a URL");
        let port = st.port.expect("a port");
        assert_ne!(port, 5173, "5173 is squatted; vite must have moved");
        assert!(url.ends_with(&port.to_string()));
        // The decisive check: something is really listening where we point.
        assert!(port_is_bound(port), "nothing is serving {url}");

        mgr.stop().await;
        drop(squat);
    }

    #[test]
    fn ignores_a_url_without_a_port() {
        // "http://localhost" alone cannot tell us where to point the browser.
        assert!(extract_local_url("open http://localhost to continue").is_none());
        // ...and a remote host is never our dev server.
        assert!(extract_local_url("fetching https://registry.npmjs.org:443/x").is_none());
    }
}
