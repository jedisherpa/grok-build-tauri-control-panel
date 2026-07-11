//! Credential store for MCP secrets (never in plain config.toml).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::security::{looks_like_secret_key, mask_secret};

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, CredentialError>;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CredentialStoreFile {
    /// key -> secret value
    pub secrets: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCredential {
    pub key: String,
    /// Masked value for UI.
    pub masked: String,
    pub present: bool,
}

pub struct CredentialStore {
    path: PathBuf,
}

impl CredentialStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(Self { path })
    }

    pub fn default_path(grok_dir: &Path) -> PathBuf {
        grok_dir.join("mcp_credentials.json")
    }

    fn load(&self) -> Result<CredentialStoreFile> {
        if !self.path.exists() {
            return Ok(CredentialStoreFile::default());
        }
        let raw = std::fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&raw).unwrap_or_default())
    }

    fn save(&self, file: &CredentialStoreFile) -> Result<()> {
        let raw = serde_json::to_string_pretty(file)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, raw)?;
        std::fs::rename(&tmp, &self.path)?;
        // Best-effort restrict permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        let mut file = self.load()?;
        file.secrets.insert(key.to_string(), value.to_string());
        self.save(&file)
    }

    pub fn get(&self, key: &str) -> Result<Option<String>> {
        let file = self.load()?;
        Ok(file.secrets.get(key).cloned())
    }

    pub fn remove(&self, key: &str) -> Result<()> {
        let mut file = self.load()?;
        file.secrets.remove(key);
        self.save(&file)
    }

    pub fn list_masked(&self) -> Result<Vec<McpCredential>> {
        let file = self.load()?;
        let mut out: Vec<_> = file
            .secrets
            .iter()
            .map(|(k, v)| McpCredential {
                key: k.clone(),
                masked: mask_secret(v),
                present: true,
            })
            .collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    }

    /// Resolve `${VAR}` or credential-store keys in env map for spawn.
    pub fn resolve_env(&self, env: &HashMap<String, String>) -> Result<HashMap<String, String>> {
        let store = self.load()?;
        let mut out = HashMap::new();
        for (k, v) in env {
            let resolved = if let Some(inner) = v.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
                store
                    .secrets
                    .get(inner)
                    .cloned()
                    .or_else(|| std::env::var(inner).ok())
                    .unwrap_or_else(|| v.clone())
            } else if looks_like_secret_key(k) && v.starts_with("cred:") {
                let key = v.trim_start_matches("cred:");
                store
                    .secrets
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| v.clone())
            } else {
                v.clone()
            };
            out.insert(k.clone(), resolved);
        }
        Ok(out)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn set_get_mask() {
        let dir = tempdir().unwrap();
        let store = CredentialStore::open(dir.path().join("creds.json")).unwrap();
        store.set("GITHUB_TOKEN", "ghp_supersecrettoken99").unwrap();
        let list = store.list_masked().unwrap();
        assert_eq!(list.len(), 1);
        assert!(!list[0].masked.contains("supersecret"));
        assert_eq!(
            store.get("GITHUB_TOKEN").unwrap().as_deref(),
            Some("ghp_supersecrettoken99")
        );
    }

    #[test]
    fn resolve_placeholder() {
        let dir = tempdir().unwrap();
        let store = CredentialStore::open(dir.path().join("creds.json")).unwrap();
        store.set("GITHUB_TOKEN", "abc123").unwrap();
        let mut env = HashMap::new();
        env.insert("GITHUB_PERSONAL_ACCESS_TOKEN".into(), "${GITHUB_TOKEN}".into());
        let resolved = store.resolve_env(&env).unwrap();
        assert_eq!(
            resolved.get("GITHUB_PERSONAL_ACCESS_TOKEN").map(String::as_str),
            Some("abc123")
        );
    }
}
