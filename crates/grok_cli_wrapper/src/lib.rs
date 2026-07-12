//! Typed async CLI wrapper for Grok Build (`grok` binary).
//!
//! Prefer ACP (`grok agent stdio`) for interactive sessions; use this wrapper
//! for inspect, version, worktree, mcp, plugins, sessions, and headless one-shots.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::process::{Child, Command};
use tracing::{debug, warn};

pub mod auth;
pub mod inspect;
pub mod spawn_opts;

pub use auth::{AuthStatus, LoginManager, LoginPhase, LoginResult, LoginSessionState};
pub use inspect::{GrokInspect, InspectReport};
pub use spawn_opts::HeadlessSpawnOptions;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("command failed ({status}): {stderr}")]
    CommandFailed { status: i32, stderr: String },
    #[error("timeout after {0:?}")]
    Timeout(Duration),
    #[error("binary not found: {0}")]
    BinaryNotFound(String),
    #[error("invalid argument: {0}")]
    InvalidArg(String),
    #[error("utf8 error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

pub type Result<T> = std::result::Result<T, CliError>;

#[derive(Debug, Clone)]
pub struct GrokCli {
    pub grok_path: PathBuf,
    pub default_timeout: Duration,
    pub clean_env: bool,
}

impl GrokCli {
    pub fn new(grok_path: impl Into<PathBuf>) -> Self {
        Self {
            grok_path: grok_path.into(),
            default_timeout: Duration::from_secs(120),
            // Inherit host env so auth tokens, git, node/npx remain available.
            clean_env: false,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// `grok version` — returns stdout text.
    pub async fn version(&self) -> Result<String> {
        let out = self.run(&["version"], None, self.default_timeout).await?;
        Ok(out.stdout.trim().to_string())
    }

    /// `grok inspect --json` structured output.
    pub async fn inspect_json(&self) -> Result<InspectReport> {
        let out = self
            .run(&["inspect", "--json"], None, self.default_timeout)
            .await?;
        if out.stdout.trim().is_empty() {
            // Fallback: some builds may not support --json yet
            return Ok(InspectReport::from_text(&out.stdout));
        }
        match serde_json::from_str(&out.stdout) {
            Ok(v) => Ok(v),
            Err(_) => Ok(InspectReport::from_text(&out.stdout)),
        }
    }

    /// Run arbitrary grok subcommand with args; returns stdout.
    pub async fn run_args(&self, args: &[&str], cwd: Option<&Path>) -> Result<String> {
        let out = self.run(args, cwd, self.default_timeout).await?;
        Ok(out.stdout)
    }

    /// Like `run_args` with an explicit timeout (long one-shot LLM calls).
    pub async fn run_args_timeout(
        &self,
        args: &[&str],
        cwd: Option<&Path>,
        timeout: Duration,
    ) -> Result<String> {
        let out = self.run(args, cwd, timeout).await?;
        Ok(out.stdout)
    }

    /// Spawn headless one-shot: `grok -p '...'` (and optional flags).
    pub async fn spawn_headless(
        &self,
        cwd: &Path,
        prompt: &str,
        opts: &HeadlessSpawnOptions,
    ) -> Result<Child> {
        validate_cwd(cwd)?;
        validate_prompt(prompt)?;

        let mut cmd = Command::new(&self.grok_path);
        cmd.current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        self.apply_env(&mut cmd);

        if opts.always_approve {
            cmd.arg("--always-approve");
        } else if opts.plan_mode {
            cmd.args(["--permission-mode", "plan"]);
        }
        if let Some(ref model) = opts.model {
            cmd.args(["--model", model]);
        }
        if let Some(ref worktree) = opts.worktree {
            cmd.args(["-w", worktree]);
        }
        for rule in &opts.rules {
            // Rules may be passed via env or config; keep as metadata for higher layers.
            debug!(?rule, "headless rule (applied at ACP/config layer)");
        }

        cmd.arg("-p").arg(prompt);

        debug!(binary = %self.grok_path.display(), cwd = %cwd.display(), "spawn headless");
        let child = cmd.spawn()?;
        Ok(child)
    }

    /// Worktree: create via `grok worktree create` or git fallback helpers (higher layer).
    pub async fn worktree_list(&self, cwd: Option<&Path>) -> Result<String> {
        self.run_args(&["worktree", "list"], cwd).await
    }

    pub async fn worktree_create(
        &self,
        name: &str,
        cwd: Option<&Path>,
        r#ref: Option<&str>,
    ) -> Result<String> {
        validate_name(name)?;
        let mut args = vec!["worktree", "create", name];
        if let Some(r) = r#ref {
            args.push("--ref");
            args.push(r);
        }
        self.run_args(&args, cwd).await
    }

    pub async fn worktree_remove(&self, name: &str, cwd: Option<&Path>) -> Result<String> {
        validate_name(name)?;
        self.run_args(&["worktree", "rm", name], cwd).await
    }

    pub async fn mcp_list(&self) -> Result<String> {
        self.run_args(&["mcp", "list"], None).await
    }

    /// `grok mcp add [-e K=V]... NAME COMMAND -- ARGS...`
    /// Env vars ride along so the CLI-registered copy can actually
    /// authenticate; `--` keeps server args (e.g. `-y`) away from grok's parser.
    pub async fn mcp_add(
        &self,
        name: &str,
        command: &str,
        args: &[&str],
        env: &[(String, String)],
    ) -> Result<String> {
        validate_name(name)?;
        let env_flags: Vec<String> = env
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let mut a: Vec<&str> = vec!["mcp", "add"];
        for pair in &env_flags {
            a.push("-e");
            a.push(pair);
        }
        a.push(name);
        a.push(command);
        if !args.is_empty() {
            a.push("--");
            a.extend(args);
        }
        self.run_args(&a, None).await
    }

    /// `grok mcp add --transport http|sse [-H 'Name: value']... NAME URL`
    pub async fn mcp_add_http(
        &self,
        name: &str,
        url: &str,
        transport: &str,
        headers: &[(String, String)],
    ) -> Result<String> {
        validate_name(name)?;
        // Parse the host — prefix checks accepted `http://127.0.0.1.evil.com`.
        let allowed = url::Url::parse(url)
            .map(|u| match u.scheme() {
                "https" => true,
                "http" => matches!(
                    u.host_str(),
                    Some("localhost") | Some("127.0.0.1") | Some("[::1]") | Some("::1")
                ),
                _ => false,
            })
            .unwrap_or(false);
        if !allowed {
            return Err(CliError::InvalidArg("url must be https or loopback http".into()));
        }
        let header_flags: Vec<String> = headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect();
        let mut a: Vec<&str> = vec!["mcp", "add", "--transport", transport];
        for h in &header_flags {
            a.push("-H");
            a.push(h);
        }
        a.push(name);
        a.push(url);
        self.run_args(&a, None)
        .await
    }

    pub async fn mcp_remove(&self, name: &str) -> Result<String> {
        validate_name(name)?;
        self.run_args(&["mcp", "remove", name], None).await
    }

    /// `grok mcp doctor [NAME]` when supported by the CLI.
    pub async fn mcp_doctor(&self, name: Option<&str>) -> Result<String> {
        match name {
            Some(n) => {
                validate_name(n)?;
                self.run_args(&["mcp", "doctor", n], None).await
            }
            None => self.run_args(&["mcp", "doctor"], None).await,
        }
    }

    pub async fn mcp_tools(&self, name: Option<&str>) -> Result<String> {
        match name {
            Some(n) => {
                validate_name(n)?;
                self.run_args(&["mcp", "tools", n], None).await
            }
            None => self.run_args(&["mcp", "tools"], None).await,
        }
    }

    pub async fn sessions_list(&self) -> Result<String> {
        self.run_args(&["sessions", "list"], None).await
    }

    /// Live model catalog from `grok models`: (model_id, is_default).
    /// The CLI's list is the source of truth — built-in catalogs go stale
    /// and a stale id fails every `-m` call with "unknown model id".
    pub async fn list_models(&self) -> Result<Vec<(String, bool)>> {
        let out = self.run_args(&["models"], None).await?;
        let mut models = Vec::new();
        for line in out.lines() {
            let t = line.trim();
            let (rest, is_default_marker) = if let Some(r) = t.strip_prefix("* ") {
                (r, true)
            } else if let Some(r) = t.strip_prefix("- ") {
                (r, false)
            } else {
                continue;
            };
            let name = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if name.is_empty() {
                continue;
            }
            let is_default = is_default_marker || rest.contains("(default)");
            models.push((name, is_default));
        }
        Ok(models)
    }

    pub async fn doctor(&self) -> Result<String> {
        self.run_args(&["doctor"], None).await
    }

    fn apply_env(&self, cmd: &mut Command) {
        // Always give children a usable PATH for GUI-launched apps.
        let path = grok_config::child_path_env();
        cmd.env("PATH", path);

        if let Ok(home) = std::env::var("HOME") {
            cmd.env("HOME", &home);
            // Grok stores auth/session under ~/.grok — preserve USER for git identity.
            cmd.env("USER", std::env::var("USER").unwrap_or_default());
        }

        // Auth: process env first (user may export XAI_API_KEY).
        if let Ok(key) = std::env::var("XAI_API_KEY") {
            if !key.is_empty() {
                cmd.env("XAI_API_KEY", key);
            }
        }

        if !self.clean_env {
            // Inherit remaining env for full CLI fidelity.
            for (k, v) in std::env::vars() {
                if k == "PATH" || k == "HOME" || k == "XAI_API_KEY" {
                    continue;
                }
                cmd.env(k, v);
            }
        }
    }

    async fn run(&self, args: &[&str], cwd: Option<&Path>, timeout: Duration) -> Result<CommandOutput> {
        if !self.grok_path.exists() {
            return Err(CliError::BinaryNotFound(self.grok_path.display().to_string()));
        }

        let mut cmd = Command::new(&self.grok_path);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(c) = cwd {
            validate_cwd(c)?;
            cmd.current_dir(c);
        }
        self.apply_env(&mut cmd);

        debug!(?args, "running grok");
        let fut = cmd.output();
        let output = tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| CliError::Timeout(timeout))??;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            // Soft-fail for missing subcommands during discovery
            if stderr.contains("unrecognized") || stderr.contains("unknown") {
                warn!(?args, %stderr, "grok subcommand may be unsupported");
            }
            return Err(CliError::CommandFailed {
                status: code,
                stderr: if stderr.is_empty() { stdout.clone() } else { stderr },
            });
        }

        Ok(CommandOutput { stdout, stderr })
    }
}

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
}

fn validate_cwd(cwd: &Path) -> Result<()> {
    if !cwd.is_absolute() {
        return Err(CliError::InvalidArg(format!(
            "cwd must be absolute: {}",
            cwd.display()
        )));
    }
    if !cwd.exists() {
        return Err(CliError::InvalidArg(format!(
            "cwd does not exist: {}",
            cwd.display()
        )));
    }
    Ok(())
}

fn validate_prompt(prompt: &str) -> Result<()> {
    if prompt.trim().is_empty() {
        return Err(CliError::InvalidArg("prompt must not be empty".into()));
    }
    if prompt.len() > 500_000 {
        return Err(CliError::InvalidArg("prompt too large".into()));
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 128 {
        return Err(CliError::InvalidArg("invalid name length".into()));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(CliError::InvalidArg(format!(
            "name contains invalid characters: {name}"
        )));
    }
    Ok(())
}

/// Baseline discovery helpers used in Phase 0.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineSnapshot {
    pub version: Option<String>,
    pub inspect: Option<InspectReport>,
    pub binary: PathBuf,
    pub errors: Vec<String>,
}

impl GrokCli {
    pub async fn capture_baseline(&self) -> BaselineSnapshot {
        let mut errors = Vec::new();
        let version = match self.version().await {
            Ok(v) => Some(v),
            Err(e) => {
                errors.push(format!("version: {e}"));
                None
            }
        };
        let inspect = match self.inspect_json().await {
            Ok(i) => Some(i),
            Err(e) => {
                errors.push(format!("inspect: {e}"));
                None
            }
        };
        BaselineSnapshot {
            version,
            inspect,
            binary: self.grok_path.clone(),
            errors,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_names() {
        assert!(validate_name("good-name_1").is_ok());
        assert!(validate_name("bad name").is_err());
        assert!(validate_name("").is_err());
    }

    #[test]
    fn validates_prompt() {
        assert!(validate_prompt("hello").is_ok());
        assert!(validate_prompt("   ").is_err());
    }
}
