//! Path discovery for Grok home, config, worktrees, and binary.

use std::path::{Path, PathBuf};

use which::which;

use crate::{ConfigError, Result};

#[derive(Debug, Clone)]
pub struct GrokPaths {
    pub home_dir: PathBuf,
    pub grok_dir: PathBuf,
    /// Panel-owned config — never overwrites Grok CLI `~/.grok/config.toml`.
    pub config_file: PathBuf,
    /// Read-only path to the official Grok CLI config (for display/doctor).
    pub grok_cli_config_file: PathBuf,
    pub worktrees_dir: PathBuf,
    pub memory_dir: PathBuf,
    pub sessions_dir: PathBuf,
    pub panel_dir: PathBuf,
    pub project_config_file: Option<PathBuf>,
    pub project_root: Option<PathBuf>,
}

impl GrokPaths {
    pub fn discover(project_root: Option<&Path>) -> Result<Self> {
        let home_dir = directories::UserDirs::new()
            .map(|u| u.home_dir().to_path_buf())
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .ok_or_else(|| ConfigError::Invalid("cannot resolve home directory".into()))?;

        Self::for_home(
            home_dir,
            std::env::var_os("C3_PROFILE_DIR").map(PathBuf::from),
            project_root,
        )
    }

    fn for_home(
        home_dir: PathBuf,
        profile: Option<PathBuf>,
        project_root: Option<&Path>,
    ) -> Result<Self> {
        // An operator-selected profile isolates panel stores, worktrees and MCP
        // credentials. CLI authentication stays at the real user's location.
        let grok_dir = match profile {
            Some(root) => {
                if !root.is_absolute() || (root.exists() && !root.is_dir()) {
                    return Err(ConfigError::Invalid(
                        "C3_PROFILE_DIR must be an absolute directory".into(),
                    ));
                }
                root.join(".grok")
            }
            None => home_dir.join(".grok"),
        };
        let panel_dir = grok_dir.join("control-panel");
        // Isolate panel settings from the CLI's config.toml ([cli]/[ui]/marketplace).
        let config_file = panel_dir.join("config.toml");
        let grok_cli_config_file = home_dir.join(".grok/config.toml");
        let worktrees_dir = grok_dir.join("worktrees");
        let memory_dir = panel_dir.join("memory");
        let sessions_dir = panel_dir.join("sessions");

        let (project_root, project_config_file) = if let Some(root) = project_root {
            let cfg = root.join(".grok").join("control-panel.toml");
            (Some(root.to_path_buf()), Some(cfg))
        } else {
            (None, None)
        };

        Ok(Self {
            home_dir,
            grok_dir,
            config_file,
            grok_cli_config_file,
            worktrees_dir,
            memory_dir,
            sessions_dir,
            panel_dir,
            project_config_file,
            project_root,
        })
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.grok_dir)?;
        std::fs::create_dir_all(&self.panel_dir)?;
        std::fs::create_dir_all(&self.worktrees_dir)?;
        std::fs::create_dir_all(&self.memory_dir)?;
        std::fs::create_dir_all(&self.sessions_dir)?;
        Ok(())
    }
}

/// Locate the `grok` binary via official install locations first, then PATH.
pub fn discover_grok_binary() -> Result<PathBuf> {
    // Prefer official ~/.grok/bin install (GUI apps often lack PATH entries).
    for c in crate::env_bootstrap::preferred_grok_candidates() {
        if c.is_file() {
            // Resolve symlinks when possible for stability.
            return Ok(std::fs::canonicalize(&c).unwrap_or(c));
        }
    }

    if let Ok(p) = which("grok") {
        return Ok(std::fs::canonicalize(&p).unwrap_or(p));
    }

    Err(ConfigError::BinaryNotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_paths() {
        let paths = GrokPaths::discover(None).unwrap();
        assert!(paths.config_file.ends_with("control-panel/config.toml"));
        assert!(paths.grok_cli_config_file.ends_with(".grok/config.toml"));
        assert!(paths.worktrees_dir.ends_with("worktrees"));
    }

    #[test]
    fn project_overlay_path() {
        let root = Path::new("/tmp/myproject");
        let paths = GrokPaths::discover(Some(root)).unwrap();
        assert_eq!(
            paths.project_config_file.unwrap(),
            PathBuf::from("/tmp/myproject/.grok/control-panel.toml")
        );
    }

    #[test]
    fn qa_profile_isolates_all_writable_stores_and_keeps_cli_config_read_only() {
        let paths =
            GrokPaths::for_home("/Users/Tester".into(), Some("/tmp/c3-qa".into()), None).unwrap();
        assert_eq!(paths.home_dir, PathBuf::from("/Users/Tester"));
        assert_eq!(
            paths.grok_cli_config_file,
            PathBuf::from("/Users/Tester/.grok/config.toml")
        );
        for path in [
            &paths.config_file,
            &paths.worktrees_dir,
            &paths.memory_dir,
            &paths.sessions_dir,
            &paths.panel_dir,
        ] {
            assert!(path.starts_with("/tmp/c3-qa/.grok"));
        }
        assert!(
            GrokPaths::for_home("/Users/Tester".into(), Some("relative".into()), None).is_err()
        );
    }
}
