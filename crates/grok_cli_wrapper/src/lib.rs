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

pub mod inspect;
pub mod spawn_opts;

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
            default_timeout: Duration::from_secs(60),
            clean_env: true,
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
        }
        if opts.plan_mode {
            // Prefer plan gate when not always-approve
            // (flag names may vary by grok version; keep as optional)
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

    pub async fn mcp_add(&self, name: &str, command: &str, args: &[&str]) -> Result<String> {
        validate_name(name)?;
        let mut a = vec!["mcp", "add", name, command];
        a.extend(args);
        self.run_args(&a, None).await
    }

    /// `grok mcp add --transport http|sse NAME URL`
    pub async fn mcp_add_http(
        &self,
        name: &str,
        url: &str,
        transport: &str,
    ) -> Result<String> {
        validate_name(name)?;
        if !(url.starts_with("https://")
            || url.starts_with("http://localhost")
            || url.starts_with("http://127.0.0.1"))
        {
            return Err(CliError::InvalidArg("url must be https or localhost".into()));
        }
        self.run_args(
            &["mcp", "add", "--transport", transport, name, url],
            None,
        )
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

    pub async fn doctor(&self) -> Result<String> {
        self.run_args(&["doctor"], None).await
    }

    fn apply_env(&self, cmd: &mut Command) {
        if self.clean_env {
            // Inherit PATH/HOME but strip potentially dangerous overrides if needed.
            // Keep XAI_API_KEY for auth.
            if let Ok(key) = std::env::var("XAI_API_KEY") {
                cmd.env("XAI_API_KEY", key);
            }
            if let Ok(path) = std::env::var("PATH") {
                cmd.env("PATH", path);
            }
            if let Ok(home) = std::env::var("HOME") {
                cmd.env("HOME", home);
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
