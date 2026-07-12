//! Memory service: workspace MEMORY.md + structured entries.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::info;
use uuid::Uuid;

use grok_events::{ControlEvent, EventBus};

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, MemoryError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub scope: String,
    pub content: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct MemoryStore {
    entries: Vec<MemoryEntry>,
}

pub struct MemoryService {
    root: PathBuf,
    store: RwLock<MemoryStore>,
    event_bus: Arc<EventBus>,
}

impl MemoryService {
    pub async fn open(root: impl Into<PathBuf>, event_bus: Arc<EventBus>) -> Result<Arc<Self>> {
        let root = root.into();
        tokio::fs::create_dir_all(&root).await?;
        let path = root.join("memory.json");
        let store = if path.exists() {
            let raw = tokio::fs::read_to_string(&path).await?;
            match serde_json::from_str(&raw) {
                Ok(s) => s,
                Err(e) => {
                    // Don't treat corruption as "empty" — the next persist()
                    // would silently wipe every entry. Keep the file aside.
                    let backup = path.with_extension("json.corrupt");
                    let _ = tokio::fs::rename(&path, &backup).await;
                    return Err(MemoryError::Json(e));
                }
            }
        } else {
            MemoryStore::default()
        };
        Ok(Arc::new(Self {
            root,
            store: RwLock::new(store),
            event_bus,
        }))
    }

    fn json_path(&self) -> PathBuf {
        self.root.join("memory.json")
    }

    pub fn workspace_memory_path(project_root: &Path) -> PathBuf {
        project_root.join("MEMORY.md")
    }

    pub async fn list(&self, scope: Option<&str>) -> Vec<MemoryEntry> {
        let store = self.store.read().await;
        store
            .entries
            .iter()
            .filter(|e| scope.map(|s| e.scope == s).unwrap_or(true))
            .cloned()
            .collect()
    }

    pub async fn add(
        &self,
        scope: impl Into<String>,
        content: impl Into<String>,
        tags: Vec<String>,
    ) -> Result<MemoryEntry> {
        let now = Utc::now();
        let entry = MemoryEntry {
            id: Uuid::new_v4().to_string(),
            scope: scope.into(),
            content: content.into(),
            tags,
            created_at: now,
            updated_at: now,
        };
        {
            let mut store = self.store.write().await;
            store.entries.push(entry.clone());
        }
        self.persist().await?;
        self.event_bus.emit(ControlEvent::MemoryUpdated {
            scope: entry.scope.clone(),
            at: now,
        });
        Ok(entry)
    }

    pub async fn remove(&self, id: &str) -> Result<()> {
        let mut store = self.store.write().await;
        let before = store.entries.len();
        store.entries.retain(|e| e.id != id);
        if store.entries.len() == before {
            return Err(MemoryError::NotFound(id.to_string()));
        }
        drop(store);
        self.persist().await?;
        Ok(())
    }

    /// Flush: write consolidated markdown for injection into sessions.
    pub async fn flush_markdown(&self, scope: &str) -> Result<String> {
        let entries = self.list(Some(scope)).await;
        let mut md = format!("# Memory ({scope})\n\n");
        for e in entries {
            md.push_str(&format!("## {}\n\n{}\n\n", e.id, e.content));
            if !e.tags.is_empty() {
                md.push_str(&format!("_tags: {}_\n\n", e.tags.join(", ")));
            }
        }
        let path = self.root.join(format!("{scope}.md"));
        tokio::fs::write(&path, &md).await?;
        info!(%scope, path = %path.display(), "memory flushed");
        Ok(md)
    }

    /// Dream: compact/summarize by concatenating recent entries (simple heuristic).
    pub async fn dream(&self, scope: &str, max_chars: usize) -> Result<String> {
        let mut entries = self.list(Some(scope)).await;
        entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let mut summary = String::from("# Dream summary\n\n");
        for e in entries {
            if summary.len() >= max_chars {
                break;
            }
            let snippet: String = e.content.chars().take(400).collect();
            summary.push_str(&format!("- {}\n", snippet.replace('\n', " ")));
        }
        self.add(
            format!("{scope}/dreams"),
            summary.clone(),
            vec!["dream".into()],
        )
        .await?;
        Ok(summary)
    }

    /// Write/read project MEMORY.md
    pub async fn write_workspace_memory(project_root: &Path, content: &str) -> Result<()> {
        let path = Self::workspace_memory_path(project_root);
        tokio::fs::write(path, content).await?;
        Ok(())
    }

    pub async fn read_workspace_memory(project_root: &Path) -> Result<String> {
        let path = Self::workspace_memory_path(project_root);
        if !path.exists() {
            return Ok(String::new());
        }
        Ok(tokio::fs::read_to_string(path).await?)
    }

    async fn persist(&self) -> Result<()> {
        let store = self.store.read().await;
        let raw = serde_json::to_string_pretty(&*store)?;
        // tmp + rename: a crash mid-write must not truncate memory.json.
        let path = self.json_path();
        let tmp = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, raw).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grok_events::shared_bus;
    use tempfile::tempdir;

    #[tokio::test]
    async fn add_flush_dream() {
        let dir = tempdir().unwrap();
        let bus = shared_bus();
        let mem = MemoryService::open(dir.path(), bus).await.unwrap();
        mem.add("ws", "Remember to use ACP first", vec!["arch".into()])
            .await
            .unwrap();
        let md = mem.flush_markdown("ws").await.unwrap();
        assert!(md.contains("ACP"));
        let dream = mem.dream("ws", 2000).await.unwrap();
        assert!(dream.contains("Dream"));
    }
}
