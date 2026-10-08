//! SQLite-backed session + transcript memory for Bomb Code.
//!
//! Survives app quit, reboot, and updates under:
//! `~/.grok/control-panel/sessions/control_panel.db`

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;
use uuid::Uuid;
mod journal;
pub use journal::*;

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
    #[error("unsupported conversation role: {0}")]
    InvalidConversationRole(String),
    #[error("invalid persistence state: {0}")]
    InvalidState(String),
    #[error("persistence bound exceeded: {0}")]
    Bounds(String),
    #[error("invalid event input: {0}")]
    InvalidInput(String),
    #[error("stale runtime metadata")]
    StaleRuntime,
}

pub type Result<T> = std::result::Result<T, PersistenceError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    /// Populated by list queries (not stored as column — computed).
    #[serde(default)]
    pub message_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptChunk {
    pub session_id: Uuid,
    pub seq: u64,
    pub kind: String,
    pub payload: String,
    pub at: DateTime<Utc>,
}

/// Frontend-friendly transcript row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptEntry {
    pub role: String,
    pub body: String,
    pub at: String,
    pub seq: u64,
}

/// Thread list row (live or restored from disk).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDto {
    pub id: String,
    pub cwd: String,
    pub mode: String,
    pub model: String,
    /// Agent backend (grok | claude | codex); old records default to grok.
    #[serde(default = "default_backend_key")]
    pub backend: String,
    pub status: String,
    /// True when an ACP/headless process is currently attached in this process.
    pub live: bool,
    pub message_count: u64,
    pub created_at: String,
    pub updated_at: String,
    pub worktree: Option<String>,
    pub mcp_servers: Vec<String>,
    pub label: Option<String>,
    /// Approval stance this thread runs with (plan | ask | auto | yolo).
    #[serde(default)]
    pub approval_mode: Option<String>,
    /// Original project folder when cwd is an isolated thread worktree.
    #[serde(default)]
    pub project_root: Option<String>,
    /// full_brain | history_only | fresh | null when not live
    pub brain_mode: Option<String>,
}

fn default_backend_key() -> String {
    "grok".into()
}

pub struct Persistence {
    path: PathBuf,
    /// Serialize writes — multiple event-loop tasks may append concurrently.
    write_lock: Mutex<()>,
    /// Retained writer keeps WAL open and avoids a last-connection checkpoint per streamed event.
    writer: Mutex<Connection>,
    identity: grok_events::StoreIdentity,
    owned_companions: Option<[std::fs::File; 2]>,
    snapshots: Mutex<std::collections::HashMap<Uuid, journal::SnapshotLease>>,
    #[cfg(test)]
    after_commit_test_hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    // Last field: physical ownership outlives every connection and the installed sink.
    _ownership: Option<std::sync::Arc<ProfileOwnership>>,
}

impl Persistence {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(PersistenceError::InvalidState(
                "terminal database symlink refused before opening SQLite".into(),
            ));
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let filename = path
            .file_name()
            .ok_or_else(|| PersistenceError::InvalidState("database filename required".into()))?;
        // SQLite NOFOLLOW rejects ancestor aliases too; normalize trusted parent
        // while retaining nofollow protection on the actual database/companions.
        let path = std::fs::canonicalize(parent)?.join(filename);
        let db = Self {
            writer: Mutex::new(Self::open_connection(&path)?),
            path,
            write_lock: Mutex::new(()),
            identity: grok_events::StoreIdentity {
                store_id: Uuid::nil(),
                generation: Uuid::nil(),
            },
            owned_companions: None,
            snapshots: Mutex::new(std::collections::HashMap::new()),
            #[cfg(test)]
            after_commit_test_hook: Mutex::new(None),
            _ownership: None,
        };
        let mut db = db;
        db.migrate()?;
        db.migrate_journal()?;
        db.identity = db.load_store_identity()?;
        info!(path = %db.path.display(), "session memory database open");
        Ok(db)
    }

    /// Production constructor keeps physical ownership alive with every sink clone.
    pub fn open_owned(owner: std::sync::Arc<ProfileOwnership>) -> Result<Self> {
        owner.verify_database_identity()?;
        let mut db = Self::open(owner.db_path())?;
        // Migrations used temporary connections. Open the WAL on the retained
        // writer before pinning its active companion identities.
        {
            let writer = db
                .writer
                .lock()
                .map_err(|_| PersistenceError::InvalidState("writer lock poisoned".into()))?;
            let _: i64 =
                writer.query_row("SELECT COALESCE(MAX(seq),0) FROM event_journal", [], |r| {
                    r.get(0)
                })?;
        }
        db.owned_companions = Some(journal::pin_owned_companions(db.path())?);
        db._ownership = Some(owner);
        Ok(db)
    }
    fn conn(&self) -> Result<Connection> {
        self.verify_owned_writer_identity()?;
        Self::open_connection(&self.path)
    }

    fn open_connection(path: &Path) -> Result<Connection> {
        journal::verify_sqlite_companions(path)?;
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::default() | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
        )?;
        verify_runtime(&conn)?;
        Ok(conn)
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
                PRIMARY KEY (session_id, seq),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_transcripts_session_seq
                ON transcripts(session_id, seq);
            CREATE INDEX IF NOT EXISTS idx_sessions_updated
                ON sessions(updated_at DESC);
            CREATE TABLE IF NOT EXISTS kv (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn upsert_session(&self, rec: &SessionRecord) -> Result<()> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let conn = self.conn()?;
        journal::ensure_legacy_writer(&conn, rec.id)?;
        journal::validate_metadata(&conn, rec.id, &rec.metadata_json)?;
        conn.execute(
            r#"
            INSERT INTO sessions (id, cwd, mode, model, status, worktree, acp_session_id, metadata_json, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(id) DO UPDATE SET
                cwd=excluded.cwd,
                mode=excluded.mode,
                model=excluded.model,
                status=CASE WHEN EXISTS(SELECT 1 FROM session_runtimes WHERE session_id=excluded.id) THEN sessions.status ELSE excluded.status END,
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

    /// Update status + touch updated_at without rewriting full metadata.
    pub fn update_session_status(&self, id: Uuid, status: &str) -> Result<()> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let conn = self.conn()?;
        journal::ensure_legacy_writer(&conn, id)?;
        let n = conn.execute(
            "UPDATE sessions SET status=?1, updated_at=?2 WHERE id=?3",
            params![status, Utc::now().to_rfc3339(), id.to_string()],
        )?;
        if n == 0 {
            return Err(PersistenceError::NotFound(id.to_string()));
        }
        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionRecord>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT s.id, s.cwd, s.mode, s.model, s.status, s.worktree, s.acp_session_id,
                   s.metadata_json, s.created_at, s.updated_at,
                   (SELECT COUNT(*) FROM transcripts t WHERE t.session_id = s.id) AS msg_count
            FROM sessions s
            ORDER BY s.updated_at DESC
            "#,
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
                message_count: row.get::<_, i64>(10)? as u64,
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
            r#"
            SELECT id, cwd, mode, model, status, worktree, acp_session_id, metadata_json,
                   created_at, updated_at,
                   (SELECT COUNT(*) FROM transcripts t WHERE t.session_id = sessions.id)
            FROM sessions WHERE id=?1
            "#,
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
                    message_count: row.get::<_, i64>(10)? as u64,
                })
            },
        )
        .map_err(|_| PersistenceError::NotFound(id.to_string()))
    }

    pub fn delete_session(&self, id: Uuid) -> Result<()> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut conn = self.conn()?;
        journal::ensure_legacy_writer(&conn, id)?;
        let tx = conn.transaction()?;
        journal::tombstone_session(&tx, id, Utc::now())?;
        tx.commit()?;
        Ok(())
    }

    /// Idempotent parent-row creation (caller must hold write_lock).
    fn ensure_session_row(&self, conn: &Connection, session_id: Uuid) -> Result<bool> {
        // Deleted sessions stay deleted.
        let tombstoned: i64 = conn.query_row(
            "SELECT COUNT(1) FROM kv WHERE key=?1",
            params![format!("tombstone_{session_id}")],
            |r| r.get(0),
        )?;
        if tombstoned > 0 {
            return Ok(false);
        }
        let now = Utc::now().to_rfc3339();
        conn.execute(
            r#"
            INSERT OR IGNORE INTO sessions
              (id, cwd, mode, model, status, worktree, acp_session_id, metadata_json, created_at, updated_at)
            VALUES (?1, '', 'acp', '', 'unknown', NULL, NULL, '{}', ?2, ?2)
            "#,
            params![session_id.to_string(), now],
        )?;
        Ok(true)
    }

    /// Append a transcript line with auto-incrementing seq.
    ///
    /// MAX(seq) and the INSERT happen under one write lock/connection — two
    /// concurrent appenders previously computed the same seq and the second
    /// silently replaced the first row.
    pub fn append_message(
        &self,
        session_id: Uuid,
        kind: &str,
        payload: impl Into<String>,
        at: DateTime<Utc>,
    ) -> Result<u64> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let conn = self.conn()?;
        journal::ensure_legacy_writer(&conn, session_id)?;
        if !self.ensure_session_row(&conn, session_id)? {
            return Err(PersistenceError::NotFound(format!(
                "session {session_id} was deleted"
            )));
        }
        let max: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM transcripts WHERE session_id=?1",
            params![session_id.to_string()],
            |r| r.get(0),
        )?;
        let seq = (max as u64).saturating_add(1);
        conn.execute(
            "INSERT INTO transcripts (session_id, seq, kind, payload, at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session_id.to_string(),
                seq as i64,
                kind,
                payload.into(),
                at.to_rfc3339(),
            ],
        )?;
        let _ = conn.execute(
            "UPDATE sessions SET updated_at=?1 WHERE id=?2",
            params![Utc::now().to_rfc3339(), session_id.to_string()],
        );
        Ok(seq)
    }

    /// Import visible conversation messages atomically. Historical tool grants
    /// and approval records are deliberately not valid input roles.
    pub fn import_conversation(&self, session_id: Uuid, entries: &[TranscriptEntry]) -> Result<()> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut conn = self.conn()?;
        journal::ensure_legacy_writer(&conn, session_id)?;
        if !self.ensure_session_row(&conn, session_id)? {
            return Err(PersistenceError::NotFound(session_id.to_string()));
        }
        let tx = conn.transaction()?;
        let mut seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq),0) FROM transcripts WHERE session_id=?1",
            [session_id.to_string()],
            |r| r.get(0),
        )?;
        for entry in entries {
            let kind = match entry.role.as_str() {
                "user" => "prompt",
                "assistant" | "agent" => "agent",
                _ => {
                    return Err(PersistenceError::InvalidConversationRole(
                        entry.role.clone(),
                    ))
                }
            };
            seq += 1;
            tx.execute(
                "INSERT INTO transcripts (session_id,seq,kind,payload,at) VALUES (?1,?2,?3,?4,?5)",
                params![session_id.to_string(), seq, kind, entry.body, entry.at],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Append a streamed chunk, concatenating onto the previous row when it
    /// has the same kind and arrived within `window_secs`. Keeps token-level
    /// streaming deltas from becoming one DB row (and one UI line) each.
    pub fn append_message_merged(
        &self,
        session_id: Uuid,
        kind: &str,
        payload: &str,
        at: DateTime<Utc>,
        window_secs: i64,
    ) -> Result<u64> {
        // Soft kinds (ACP term spam / system notices) may land between stream
        // deltas. Look past them for the same-kind merge target — otherwise
        // every interleaved term row fractures one reply into many AGENT bubbles
        // (full-tip Play A S5, 2026-10-06).
        let soft = matches!(kind, "agent" | "thought");
        let merged = {
            let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
            let conn = self.conn()?;
            journal::ensure_legacy_writer(&conn, session_id)?;
            let mut stmt = conn.prepare(
                "SELECT seq, kind, at FROM transcripts WHERE session_id=?1 ORDER BY seq DESC LIMIT 24",
            )?;
            let recent: Vec<(i64, String, String)> = stmt
                .query_map(params![session_id.to_string()], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            let mut target: Option<(i64, String)> = None;
            for (seq, k, at_s) in recent {
                if k == kind {
                    if (at - parse_dt(&at_s)).num_seconds().abs() <= window_secs {
                        target = Some((seq, at_s));
                    }
                    break;
                }
                if soft && matches!(k.as_str(), "term" | "system") {
                    continue;
                }
                break;
            }
            match target {
                Some((seq, _)) => {
                    conn.execute(
                        "UPDATE transcripts SET payload = payload || ?1, at = ?2 WHERE session_id=?3 AND seq=?4",
                        params![payload, at.to_rfc3339(), session_id.to_string(), seq],
                    )?;
                    Some(seq as u64)
                }
                None => None,
            }
        };
        match merged {
            Some(seq) => Ok(seq),
            // New row starts the message; drop leading stream whitespace.
            None => self.append_message(session_id, kind, payload.trim_start(), at),
        }
    }

    pub fn transcripts(&self, session_id: Uuid) -> Result<Vec<TranscriptChunk>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT session_id, seq, kind, payload, at FROM transcripts WHERE session_id=?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![session_id.to_string()], |row| {
            Ok(TranscriptChunk {
                session_id: Uuid::parse_str(&row.get::<_, String>(0)?)
                    .unwrap_or_else(|_| Uuid::nil()),
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

    pub fn transcript_entries(&self, session_id: Uuid) -> Result<Vec<TranscriptEntry>> {
        let chunks = self.transcripts(session_id)?;
        // Repair sessions recorded before write-side merging: token-level
        // streaming deltas were stored one row each. Fold same-role
        // agent/thought rows within a short window, looking past soft
        // term/system noise that interleaved between deltas.
        let mut out: Vec<TranscriptEntry> = Vec::new();
        for c in chunks {
            let role = kind_to_role(&c.kind);
            let mergeable = matches!(role.as_str(), "agent" | "thought");
            if mergeable {
                let mut merge_idx: Option<usize> = None;
                for i in (0..out.len()).rev() {
                    let prev_role = out[i].role.as_str();
                    if prev_role == role {
                        let prev_at = parse_dt(&out[i].at);
                        if (c.at - prev_at).num_seconds().abs() <= 10 {
                            merge_idx = Some(i);
                        }
                        break;
                    }
                    if prev_role == "term" || prev_role == "system" {
                        continue;
                    }
                    break;
                }
                if let Some(i) = merge_idx {
                    let prev = &mut out[i];
                    // Legacy rows lost their leading spaces to trim; add a
                    // space unless punctuation continues the previous word.
                    let needs_space = !prev.body.ends_with(char::is_whitespace)
                        && !c
                            .payload
                            .chars()
                            .next()
                            .map(|ch| ".,!?;:)]}%'\"".contains(ch) || ch.is_whitespace())
                            .unwrap_or(true);
                    if needs_space {
                        prev.body.push(' ');
                    }
                    prev.body.push_str(&c.payload);
                    prev.at = c.at.to_rfc3339();
                    continue;
                }
            }
            out.push(TranscriptEntry {
                role,
                body: c.payload,
                at: c.at.to_rfc3339(),
                seq: c.seq,
            });
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
            md.push_str(&format!(
                "### [{}] {}\n\n```\n{}\n```\n\n",
                c.at, c.kind, c.payload
            ));
        }
        Ok(md)
    }

    pub fn set_kv(&self, key: &str, value: &str) -> Result<()> {
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
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

    pub fn checkpoint(&self) -> Result<()> {
        info!(path = %self.path.display(), "persistence checkpoint");
        let _g = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let conn = self.conn()?;
        let (busy, log, checkpointed): (i64, i64, i64) =
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
        if busy != 0 || log != checkpointed {
            return Err(PersistenceError::InvalidState(format!("WAL checkpoint blocked: busy={busy}, log={log}, checkpointed={checkpointed}; committed WAL preserved")));
        }
        // Report successful checkpoint only after observing the pragma result.
        // This marker itself is a new durable commit and may create a small WAL.
        conn.execute("INSERT INTO kv(key,value) VALUES('last_checkpoint',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [Utc::now().to_rfc3339()])?;
        Ok(())
    }
}

fn kind_to_role(kind: &str) -> String {
    match kind {
        "prompt" | "user" => "user".into(),
        "agent" | "message" | "assistant" => "agent".into(),
        "thought" => "thought".into(),
        "tool" | "tool_call" => "tool".into(),
        "plan" => "plan".into(),
        "error" => "error".into(),
        // Raw ACP protocol lines — hidden by default in the UI, revealed by
        // the View toggle.
        "term" => "term".into(),
        // Permission requests render as (inert, post-restart) approval cards.
        "approval" => "approval".into(),
        _ => "system".into(),
    }
}

fn parse_dt(s: &str) -> DateTime<Utc> {
    // Epoch, not now(): a garbage stored timestamp must not float a stale
    // thread to the top of the recency-sorted list.
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn conversation_import_is_complete_and_never_imports_approval_authority() {
        let dir = tempdir().unwrap();
        let db = Persistence::open(dir.path().join("m.db")).unwrap();
        let id = Uuid::new_v4();
        let entries = vec![
            TranscriptEntry {
                role: "user".into(),
                body: "prior request".into(),
                at: Utc::now().to_rfc3339(),
                seq: 0,
            },
            TranscriptEntry {
                role: "assistant".into(),
                body: "prior reply".into(),
                at: Utc::now().to_rfc3339(),
                seq: 1,
            },
        ];
        db.import_conversation(id, &entries).unwrap();
        assert_eq!(db.transcript_entries(id).unwrap().len(), 2);
        let mut forbidden = entries.clone();
        forbidden[1].role = "approval".into();
        assert!(db.import_conversation(id, &forbidden).is_err());
        assert_eq!(
            db.transcript_entries(id).unwrap().len(),
            2,
            "failed import must roll back atomically"
        );
    }

    #[test]
    fn streamed_chunks_merge_into_one_row() {
        let dir = tempdir().unwrap();
        let db = Persistence::open(dir.path().join("m.db")).unwrap();
        let id = Uuid::new_v4();
        let t = Utc::now();
        db.append_message(id, "prompt", "hi", t).unwrap();
        for chunk in ["Sup", " —", " what", " are", " we", " building", "?"] {
            db.append_message_merged(id, "agent", chunk, t, 10).unwrap();
        }
        let entries = db.transcript_entries(id).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].role, "agent");
        assert_eq!(entries[1].body, "Sup — what are we building?");
        // Different kind starts a new row.
        db.append_message_merged(id, "thought", "pondering", t, 10)
            .unwrap();
        assert_eq!(db.transcript_entries(id).unwrap().len(), 3);
    }

    #[test]
    fn legacy_fragmented_rows_are_merged_on_read() {
        let dir = tempdir().unwrap();
        let db = Persistence::open(dir.path().join("l.db")).unwrap();
        let id = Uuid::new_v4();
        let t = Utc::now();
        // Simulate pre-fix rows: one token per row, leading spaces lost.
        for tok in ["Sup", "—", "what", "are", "we", "building", "?"] {
            db.append_message(id, "agent", tok, t).unwrap();
        }
        db.append_message(id, "prompt", "next", t).unwrap();
        let entries = db.transcript_entries(id).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].body, "Sup — what are we building?");
        assert_eq!(entries[1].role, "user");
    }

    #[test]
    fn streamed_chunks_merge_past_interleaved_term_noise() {
        let dir = tempdir().unwrap();
        let db = Persistence::open(dir.path().join("term.db")).unwrap();
        let id = Uuid::new_v4();
        let t = Utc::now();
        db.append_message(id, "prompt", "hi", t).unwrap();
        db.append_message_merged(id, "agent", "The `add`", t, 10)
            .unwrap();
        db.append_message(id, "term", "sampling.request sse_chunk noise", t)
            .unwrap();
        db.append_message_merged(id, "agent", " function", t, 10)
            .unwrap();
        db.append_message(id, "term", "more noise", t).unwrap();
        db.append_message_merged(id, "agent", " returns a - b.", t, 10)
            .unwrap();
        let raw = db.transcripts(id).unwrap();
        let agent_rows: Vec<_> = raw.iter().filter(|c| c.kind == "agent").collect();
        assert_eq!(
            agent_rows.len(),
            1,
            "write-side must merge past term: {:?}",
            agent_rows
        );
        assert_eq!(agent_rows[0].payload, "The `add` function returns a - b.");
        // Read-side also folds legacy agent/term/agent fragmentation.
        let dir2 = tempdir().unwrap();
        let db2 = Persistence::open(dir2.path().join("legacy-term.db")).unwrap();
        let id2 = Uuid::new_v4();
        db2.append_message(id2, "agent", "The `add`", t).unwrap();
        db2.append_message(id2, "term", "noise", t).unwrap();
        db2.append_message(id2, "agent", "function", t).unwrap();
        db2.append_message(id2, "term", "noise2", t).unwrap();
        db2.append_message(id2, "agent", "returns a - b.", t)
            .unwrap();
        let entries = db2.transcript_entries(id2).unwrap();
        let agents: Vec<_> = entries.iter().filter(|e| e.role == "agent").collect();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].body, "The `add` function returns a - b.");
    }

    #[test]
    fn session_and_transcript_roundtrip() {
        let dir = tempdir().unwrap();
        let db = Persistence::open(dir.path().join("t.db")).unwrap();
        let id = Uuid::new_v4();
        let rec = SessionRecord {
            id,
            cwd: "/tmp/proj".into(),
            mode: "acp".into(),
            model: "grok-4".into(),
            status: "idle".into(),
            worktree: None,
            acp_session_id: Some("s1".into()),
            metadata_json: "{}".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            message_count: 0,
        };
        db.upsert_session(&rec).unwrap();
        db.append_message(id, "prompt", "hello world", Utc::now())
            .unwrap();
        db.append_message(id, "agent", "hi back", Utc::now())
            .unwrap();
        let loaded = db.get_session(id).unwrap();
        assert_eq!(loaded.cwd, "/tmp/proj");
        assert_eq!(loaded.message_count, 2);
        let entries = db.transcript_entries(id).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].role, "user");
        assert_eq!(entries[1].role, "agent");
        let list = db.list_sessions().unwrap();
        assert_eq!(list[0].message_count, 2);
    }

    #[test]
    fn survives_reopen() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("mem.db");
        let id = Uuid::new_v4();
        {
            let db = Persistence::open(&path).unwrap();
            db.upsert_session(&SessionRecord {
                id,
                cwd: "/Users/me/app".into(),
                mode: "acp".into(),
                model: "grok".into(),
                status: "idle".into(),
                worktree: None,
                acp_session_id: None,
                metadata_json: "{}".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                message_count: 0,
            })
            .unwrap();
            db.append_message(id, "prompt", "build a game", Utc::now())
                .unwrap();
            db.append_message(id, "agent", "sure, starting…", Utc::now())
                .unwrap();
        }
        let db2 = Persistence::open(&path).unwrap();
        let entries = db2.transcript_entries(id).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[1].body.contains("starting"));
    }
}
