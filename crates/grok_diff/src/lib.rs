//! Diff capture: snapshot files before/after plan execution and emit clean diffs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::process::Command;
use tracing::debug;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum DiffError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("git error: {0}")]
    Git(String),
    #[error("utf8 error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

pub type Result<T> = std::result::Result<T, DiffError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSnapshot {
    pub path: String,
    pub hash: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffCapture {
    pub id: String,
    pub cwd: String,
    pub created_at: DateTime<Utc>,
    pub before: Vec<FileSnapshot>,
    pub after: Option<Vec<FileSnapshot>>,
    pub unified_diff: Option<String>,
    pub summary: Option<DiffSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffSummary {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub files: Vec<String>,
}

pub struct DiffEngine;

impl DiffEngine {
    /// Capture git-based before state (status + file hashes of dirty paths).
    pub async fn capture_before(cwd: &Path) -> Result<DiffCapture> {
        let status = git(cwd, &["status", "--porcelain"]).await?;
        let mut before = Vec::new();
        for line in status.lines() {
            if line.len() < 4 {
                continue;
            }
            let path = line[3..].trim();
            if path.is_empty() {
                continue;
            }
            let full = cwd.join(path);
            if let Ok(snap) = snapshot_file(&full, path).await {
                before.push(snap);
            }
        }
        Ok(DiffCapture {
            id: Uuid::new_v4().to_string(),
            cwd: cwd.display().to_string(),
            created_at: Utc::now(),
            before,
            after: None,
            unified_diff: None,
            summary: None,
        })
    }

    /// Finalize with after state + unified diff from git.
    pub async fn capture_after(mut capture: DiffCapture) -> Result<DiffCapture> {
        let cwd = PathBuf::from(&capture.cwd);
        let status = git(&cwd, &["status", "--porcelain"]).await.unwrap_or_default();
        let mut after = Vec::new();
        for line in status.lines() {
            if line.len() < 4 {
                continue;
            }
            let path = line[3..].trim();
            let full = cwd.join(path);
            if let Ok(snap) = snapshot_file(&full, path).await {
                after.push(snap);
            }
        }

        let unified = git(
            &cwd,
            &["diff", "HEAD", "--", ".", ":(exclude).git"],
        )
        .await
        .unwrap_or_else(|_| String::new());

        // Also include untracked content summary
        let unstaged = git(&cwd, &["diff"]).await.unwrap_or_default();
        let combined = if unified.is_empty() {
            unstaged
        } else {
            format!("{unified}\n{unstaged}")
        };

        let summary = summarize_diff(&combined, &capture.before, &after);
        capture.after = Some(after);
        capture.unified_diff = Some(combined);
        capture.summary = Some(summary);
        Ok(capture)
    }

    /// Quick path: just current git diff.
    pub async fn current_diff(cwd: &Path) -> Result<String> {
        let a = git(cwd, &["diff"]).await.unwrap_or_default();
        let b = git(cwd, &["diff", "--cached"]).await.unwrap_or_default();
        Ok(format!("{b}\n{a}"))
    }

    pub async fn current_summary(cwd: &Path) -> Result<DiffSummary> {
        let diff = Self::current_diff(cwd).await?;
        Ok(summarize_diff(&diff, &[], &[]))
    }
}

async fn snapshot_file(full: &Path, rel: &str) -> Result<FileSnapshot> {
    let data = tokio::fs::read(full).await?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let hash = hex::encode(hasher.finalize());
    Ok(FileSnapshot {
        path: rel.to_string(),
        hash,
        size: data.len() as u64,
    })
}

async fn git(cwd: &Path, args: &[&str]) -> Result<String> {
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
        debug!(%stderr, "git command failed");
        return Err(DiffError::Git(stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn summarize_diff(diff: &str, before: &[FileSnapshot], after: &[FileSnapshot]) -> DiffSummary {
    let mut insertions = 0usize;
    let mut deletions = 0usize;
    let mut files: HashMap<String, ()> = HashMap::new();

    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            files.insert(rest.to_string(), ());
        } else if line.starts_with('+') && !line.starts_with("+++") {
            insertions += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            deletions += 1;
        }
    }

    for s in before.iter().chain(after.iter()) {
        files.entry(s.path.clone()).or_insert(());
    }

    let mut file_list: Vec<String> = files.into_keys().collect();
    file_list.sort();
    DiffSummary {
        files_changed: file_list.len(),
        insertions,
        deletions,
        files: file_list,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_counts() {
        let diff = "\
--- a/foo.rs
+++ b/foo.rs
@@ -1,2 +1,3 @@
-line
+line2
+line3
";
        let s = summarize_diff(diff, &[], &[]);
        assert_eq!(s.insertions, 2);
        assert_eq!(s.deletions, 1);
        assert!(s.files.iter().any(|f| f == "foo.rs"));
    }
}
