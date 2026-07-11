//! SQLite-backed session persistence for crash recovery.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, PersistenceError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: Uuid,
    pub cwd: String,
    pub mode: String,
    pub model: String,
    pub status: String,
    pub worktree: Option<String>,
    pub acp_session_id: Option<String>,
    pub metadata_json: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptChunk {
    pub session_id: Uuid,
    pub seq: u64,
    pub kind: String,
    pub payload: String,
    pub at: DateTime<Utc>,
}

pub struct Persistence {
    path: PathBuf,
}

impl Persistence {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Self { path };
        db.migrate()?;
        Ok(db)
    }

    fn conn(&self) -> Result<Connection> {
        Ok(Connection::open(&self.path)?)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                cwd TEXT NOT NULL,
                mode TEXT NOT NULL,
                model TEXT NOT NULL,
                status TEXT NOT NULL,
                worktree TEXT,
                acp_session_id TEXT,
                metadata_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS transcripts (
                session_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                kind TEXT NOT NULL,
                payload TEXT NOT NULL,
                at TEXT NOT NULL,
                PRIMARY KEY (session_id, seq)
            );
            CREATE TABLE IF NOT EXISTS kv (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    pub fn upsert_session(&self, rec: &SessionRecord) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            INSERT INTO sessions (id, cwd, mode, model, status, worktree, acp_session_id, metadata_json, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(id) DO UPDATE SET
                cwd=excluded.cwd,
                mode=excluded.mode,
                model=excluded.model,
                status=excluded.status,
                worktree=excluded.worktree,
                acp_session_id=excluded.acp_session_id,
                metadata_json=excluded.metadata_json,
                updated_at=excluded.updated_at
            "#,
            params![
                rec.id.to_string(),
                rec.cwd,
                rec.mode,
                rec.model,
                rec.status,
                rec.worktree,
                rec.acp_session_id,
                rec.metadata_json,
                rec.created_at.to_rfc3339(),
                rec.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionRecord>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, cwd, mode, model, status, worktree, acp_session_id, metadata_json, created_at, updated_at FROM sessions ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SessionRecord {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_else(|_| Uuid::nil()),
                cwd: row.get(1)?,
                mode: row.get(2)?,
                model: row.get(3)?,
                status: row.get(4)?,
                worktree: row.get(5)?,
                acp_session_id: row.get(6)?,
                metadata_json: row.get(7)?,
                created_at: parse_dt(&row.get::<_, String>(8)?),
                updated_at: parse_dt(&row.get::<_, String>(9)?),
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn get_session(&self, id: Uuid) -> Result<SessionRecord> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT id, cwd, mode, model, status, worktree, acp_session_id, metadata_json, created_at, updated_at FROM sessions WHERE id=?1",
            params![id.to_string()],
            |row| {
                Ok(SessionRecord {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_else(|_| Uuid::nil()),
                    cwd: row.get(1)?,
                    mode: row.get(2)?,
                    model: row.get(3)?,
                    status: row.get(4)?,
                    worktree: row.get(5)?,
                    acp_session_id: row.get(6)?,
                    metadata_json: row.get(7)?,
                    created_at: parse_dt(&row.get::<_, String>(8)?),
                    updated_at: parse_dt(&row.get::<_, String>(9)?),
                })
            },
        )
        .map_err(|_| PersistenceError::NotFound(id.to_string()))
    }

    pub fn delete_session(&self, id: Uuid) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM sessions WHERE id=?1", params![id.to_string()])?;
        conn.execute(
            "DELETE FROM transcripts WHERE session_id=?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    pub fn append_transcript(&self, chunk: &TranscriptChunk) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT OR REPLACE INTO transcripts (session_id, seq, kind, payload, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                chunk.session_id.to_string(),
                chunk.seq as i64,
                chunk.kind,
                chunk.payload,
                chunk.at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn transcripts(&self, session_id: Uuid) -> Result<Vec<TranscriptChunk>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT session_id, seq, kind, payload, at FROM transcripts WHERE session_id=?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![session_id.to_string()], |row| {
            Ok(TranscriptChunk {
                session_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_else(|_| Uuid::nil()),
                seq: row.get::<_, i64>(1)? as u64,
                kind: row.get(2)?,
                payload: row.get(3)?,
                at: parse_dt(&row.get::<_, String>(4)?),
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn export_markdown(&self, session_id: Uuid) -> Result<String> {
        let session = self.get_session(session_id)?;
        let chunks = self.transcripts(session_id)?;
        let mut md = format!(
            "# Session {}\n\n- cwd: `{}`\n- mode: {}\n- model: {}\n- status: {}\n\n## Transcript\n\n",
            session.id, session.cwd, session.mode, session.model, session.status
        );
        for c in chunks {
            md.push_str(&format!("### [{}] {}\n\n```\n{}\n```\n\n", c.at, c.kind, c.payload));
        }
        Ok(md)
    }

    pub fn set_kv(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_kv(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT value FROM kv WHERE key=?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn checkpoint(&self) -> Result<()> {
        info!(path = %self.path.display(), "persistence checkpoint");
        self.set_kv("last_checkpoint", &Utc::now().to_rfc3339())?;
        Ok(())
    }
}

fn parse_dt(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn session_roundtrip() {
        let dir = tempdir().unwrap();
        let db = Persistence::open(dir.path().join("t.db")).unwrap();
        let id = Uuid::new_v4();
        let rec = SessionRecord {
            id,
            cwd: "/tmp".into(),
            mode: "acp".into(),
            model: "grok".into(),
            status: "idle".into(),
            worktree: None,
            acp_session_id: Some("s1".into()),
            metadata_json: "{}".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.upsert_session(&rec).unwrap();
        db.append_transcript(&TranscriptChunk {
            session_id: id,
            seq: 1,
            kind: "message".into(),
            payload: "hello".into(),
            at: Utc::now(),
        })
        .unwrap();
        let loaded = db.get_session(id).unwrap();
        assert_eq!(loaded.cwd, "/tmp");
        let md = db.export_markdown(id).unwrap();
        assert!(md.contains("hello"));
    }
}
