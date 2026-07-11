//! Worktree manager: create, list, remove, and land isolated agent workspaces.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::process::Command;
use tracing::{info, warn};
use uuid::Uuid;

use grok_cli_wrapper::GrokCli;

#[derive(Debug, Error)]
pub enum WorktreeError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("git failed: {0}")]
    Git(String),
    #[error("cli error: {0}")]
    Cli(#[from] grok_cli_wrapper::CliError),
    #[error("invalid name: {0}")]
    InvalidName(String),
    #[error("worktree not found: {0}")]
    NotFound(String),
    #[error("not a git repository: {0}")]
    NotGitRepo(String),
}

pub type Result<T> = std::result::Result<T, WorktreeError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub created_at: DateTime<Utc>,
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorktreeRequest {
    pub name: String,
    pub base_ref: Option<String>,
    /// If true, try `grok worktree create` first, else pure git.
    pub prefer_grok_cli: bool,
}

pub struct WorktreeManager {
    grok_cli: Arc<GrokCli>,
    /// Root under which managed worktrees are placed (e.g. ~/.grok/worktrees).
    worktrees_root: PathBuf,
}

impl WorktreeManager {
    pub fn new(grok_cli: Arc<GrokCli>, worktrees_root: PathBuf) -> Self {
        Self {
            grok_cli,
            worktrees_root,
        }
    }

    pub fn worktrees_root(&self) -> &Path {
        &self.worktrees_root
    }

    pub async fn ensure_root(&self) -> Result<()> {
        tokio::fs::create_dir_all(&self.worktrees_root).await?;
        Ok(())
    }

    pub async fn create(
        &self,
        repo: &Path,
        req: CreateWorktreeRequest,
    ) -> Result<WorktreeInfo> {
        validate_name(&req.name)?;
        ensure_git_repo(repo).await?;
        self.ensure_root().await?;

        if req.prefer_grok_cli {
            match self
                .grok_cli
                .worktree_create(&req.name, Some(repo), req.base_ref.as_deref())
                .await
            {
                Ok(out) => {
                    info!(name = %req.name, %out, "created worktree via grok cli");
                    // Discover path after CLI create
                    if let Ok(list) = self.list_git(repo).await {
                        if let Some(found) = list.into_iter().find(|w| w.name == req.name || w.path.ends_with(&req.name))
                        {
                            return Ok(found);
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "grok worktree create failed; falling back to git");
                }
            }
        }

        let path = self
            .worktrees_root
            .join(format!("{}-{}", req.name, &Uuid::new_v4().to_string()[..8]));
        let branch = format!("grok/{}", req.name);
        let base = req.base_ref.as_deref().unwrap_or("HEAD");

        // Create branch from base if needed, then worktree
        let _ = run_git(repo, &["branch", &branch, base]).await;
        run_git(
            repo,
            &[
                "worktree",
                "add",
                path.to_str().unwrap_or_default(),
                &branch,
            ],
        )
        .await?;

        let head = run_git(&path, &["rev-parse", "HEAD"])
            .await
            .ok()
            .map(|s| s.trim().to_string());

        let info = WorktreeInfo {
            id: Uuid::new_v4().to_string(),
            name: req.name,
            path,
            branch: Some(branch),
            head,
            created_at: Utc::now(),
            locked: false,
        };
        info!(path = %info.path.display(), "worktree created");
        Ok(info)
    }

    pub async fn list(&self, repo: &Path) -> Result<Vec<WorktreeInfo>> {
        ensure_git_repo(repo).await?;
        self.list_git(repo).await
    }

    async fn list_git(&self, repo: &Path) -> Result<Vec<WorktreeInfo>> {
        let out = run_git(repo, &["worktree", "list", "--porcelain"]).await?;
        Ok(parse_porcelain(&out))
    }

    pub async fn remove(&self, repo: &Path, path_or_name: &str, force: bool) -> Result<()> {
        ensure_git_repo(repo).await?;

        // Try grok CLI first
        if validate_name(path_or_name).is_ok() {
            if let Err(e) = self.grok_cli.worktree_remove(path_or_name, Some(repo)).await {
                warn!(error = %e, "grok worktree rm failed; using git");
            } else {
                return Ok(());
            }
        }

        let list = self.list_git(repo).await?;
        let target = list
            .iter()
            .find(|w| w.name == path_or_name || w.path.to_string_lossy() == path_or_name)
            .ok_or_else(|| WorktreeError::NotFound(path_or_name.to_string()))?;

        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        let path_str = target.path.to_string_lossy();
        args.push(path_str.as_ref());
        run_git(repo, &args).await?;
        info!(path = %target.path.display(), "worktree removed");
        Ok(())
    }

    pub async fn prune(&self, repo: &Path) -> Result<String> {
        ensure_git_repo(repo).await?;
        run_git(repo, &["worktree", "prune", "-v"]).await
    }

    /// Capture a summary of uncommitted changes for landing review.
    pub async fn status_summary(&self, worktree_path: &Path) -> Result<String> {
        run_git(worktree_path, &["status", "--short"]).await
    }

    /// Produce a full diff for the worktree.
    pub async fn diff(&self, worktree_path: &Path) -> Result<String> {
        let staged = run_git(worktree_path, &["diff", "--cached"]).await.unwrap_or_default();
        let unstaged = run_git(worktree_path, &["diff"]).await.unwrap_or_default();
        Ok(format!("{staged}\n{unstaged}"))
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        return Err(WorktreeError::InvalidName(name.to_string()));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '/')
    {
        return Err(WorktreeError::InvalidName(name.to_string()));
    }
    Ok(())
}

async fn ensure_git_repo(path: &Path) -> Result<()> {
    match run_git(path, &["rev-parse", "--is-inside-work-tree"]).await {
        Ok(s) if s.trim() == "true" => Ok(()),
        _ => Err(WorktreeError::NotGitRepo(path.display().to_string())),
    }
}

async fn run_git(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(WorktreeError::Git(stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_porcelain(out: &str) -> Vec<WorktreeInfo> {
    let mut result = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_head: Option<String> = None;
    let mut current_branch: Option<String> = None;
    let mut locked = false;

    let flush = |result: &mut Vec<WorktreeInfo>,
                 path: &mut Option<PathBuf>,
                 head: &mut Option<String>,
                 branch: &mut Option<String>,
                 locked: &mut bool| {
        if let Some(p) = path.take() {
            let name = p
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string());
            result.push(WorktreeInfo {
                id: Uuid::new_v4().to_string(),
                name,
                path: p,
                branch: branch.take(),
                head: head.take(),
                created_at: Utc::now(),
                locked: *locked,
            });
            *locked = false;
        }
    };

    for line in out.lines() {
        if line.is_empty() {
            flush(
                &mut result,
                &mut current_path,
                &mut current_head,
                &mut current_branch,
                &mut locked,
            );
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            flush(
                &mut result,
                &mut current_path,
                &mut current_head,
                &mut current_branch,
                &mut locked,
            );
            current_path = Some(PathBuf::from(rest));
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            current_head = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("branch ") {
            current_branch = Some(rest.trim_start_matches("refs/heads/").to_string());
        } else if line.starts_with("locked") {
            locked = true;
        }
    }
    flush(
        &mut result,
        &mut current_path,
        &mut current_head,
        &mut current_branch,
        &mut locked,
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_worktree_list() {
        let sample = "\
worktree /repo
HEAD abcdef
branch refs/heads/main

worktree /repo/.grok/wt-feature
HEAD 123456
branch refs/heads/grok/feature
locked
";
        let list = parse_porcelain(sample);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "repo");
        assert_eq!(list[1].branch.as_deref(), Some("grok/feature"));
        assert!(list[1].locked);
    }

    #[test]
    fn name_validation() {
        assert!(validate_name("feat-1").is_ok());
        assert!(validate_name("bad name").is_err());
    }
}
