//! Host-owned semantic paths. A UI request never chooses a runtime or reference.
use std::path::{Path, PathBuf};

pub(crate) struct SemanticRuntime {
    pub reference: PathBuf,
    pub python: PathBuf,
    pub node: PathBuf,
}

impl SemanticRuntime {
    fn for_home(home: &Path, node: PathBuf) -> Self {
        let root = home.join(".grok/control-panel/wizard-joe");
        Self {
            reference: root.join("reference"),
            python: root.join("python-env/bin/python3"),
            node,
        }
    }

    pub(crate) fn discover() -> Result<Self, String> {
        // QA profiles isolate writable stores; the frozen semantic package stays
        // at the actual user's host-owned location and is read-only to workers.
        let home = grok_config::GrokPaths::discover(None)
            .map_err(|e| e.to_string())?
            .home_dir;
        let node = std::env::var_os("PATH")
            .and_then(|paths| {
                std::env::split_paths(&paths)
                    .filter(|p| p.is_absolute())
                    .map(|p| p.join("node"))
                    .find(|p| p.is_file())
            })
            .unwrap_or_else(|| home.join(".grok/control-panel/wizard-joe/node/bin/node"));
        Ok(Self::for_home(&home, node))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn another_user_and_spaces_keep_host_owned_paths() {
        let runtime = SemanticRuntime::for_home(
            Path::new("/Users/Release Tester"),
            "/opt/homebrew/bin/node".into(),
        );
        assert_eq!(
            runtime.reference,
            PathBuf::from("/Users/Release Tester/.grok/control-panel/wizard-joe/reference")
        );
        assert_eq!(
            runtime.python,
            PathBuf::from(
                "/Users/Release Tester/.grok/control-panel/wizard-joe/python-env/bin/python3"
            )
        );
        assert_eq!(runtime.node, PathBuf::from("/opt/homebrew/bin/node"));
    }
}
