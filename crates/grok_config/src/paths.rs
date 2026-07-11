//! Path discovery for Grok home, config, worktrees, and binary.

use std::path::{Path, PathBuf};

use which::which;

use crate::{ConfigError, Result};

#[derive(Debug, Clone)]
pub struct GrokPaths {
    pub home_dir: PathBuf,
    pub grok_dir: PathBuf,
    pub config_file: PathBuf,
    pub worktrees_dir: PathBuf,
    pub memory_dir: PathBuf,
    pub sessions_dir: PathBuf,
    pub project_config_file: Option<PathBuf>,
    pub project_root: Option<PathBuf>,
}

impl GrokPaths {
    pub fn discover(project_root: Option<&Path>) -> Result<Self> {
        let home_dir = directories::UserDirs::new()
            .map(|u| u.home_dir().to_path_buf())
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .ok_or_else(|| ConfigError::Invalid("cannot resolve home directory".into()))?;

        let grok_dir = home_dir.join(".grok");
        let config_file = grok_dir.join("config.toml");
        let worktrees_dir = grok_dir.join("worktrees");
        let memory_dir = grok_dir.join("memory");
        let sessions_dir = grok_dir.join("sessions");

        let (project_root, project_config_file) = if let Some(root) = project_root {
            let cfg = root.join(".grok").join("config.toml");
            (Some(root.to_path_buf()), Some(cfg))
        } else {
            (None, None)
        };

        Ok(Self {
            home_dir,
            grok_dir,
            config_file,
            worktrees_dir,
            memory_dir,
            sessions_dir,
            project_config_file,
            project_root,
        })
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.grok_dir)?;
        std::fs::create_dir_all(&self.worktrees_dir)?;
        std::fs::create_dir_all(&self.memory_dir)?;
        std::fs::create_dir_all(&self.sessions_dir)?;
        Ok(())
    }
}

/// Locate the `grok` binary via PATH, common install locations, and cargo bin.
pub fn discover_grok_binary() -> Result<PathBuf> {
    if let Ok(p) = which("grok") {
        return Ok(p);
    }

    let candidates = [
        dirs_home().map(|h| h.join(".cargo").join("bin").join("grok")),
        dirs_home().map(|h| h.join(".local").join("bin").join("grok")),
        dirs_home().map(|h| h.join(".grok").join("bin").join("grok")),
        Some(PathBuf::from("/usr/local/bin/grok")),
        Some(PathBuf::from("/opt/homebrew/bin/grok")),
    ];

    for c in candidates.into_iter().flatten() {
        if c.is_file() {
            return Ok(c);
        }
    }

    Err(ConfigError::BinaryNotFound)
}

fn dirs_home() -> Option<PathBuf> {
    directories::UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_paths() {
        let paths = GrokPaths::discover(None).unwrap();
        assert!(paths.config_file.ends_with("config.toml"));
        assert!(paths.worktrees_dir.ends_with("worktrees"));
    }

    #[test]
    fn project_overlay_path() {
        let root = Path::new("/tmp/myproject");
        let paths = GrokPaths::discover(Some(root)).unwrap();
        assert_eq!(
            paths.project_config_file.unwrap(),
            PathBuf::from("/tmp/myproject/.grok/config.toml")
        );
    }
}
