use super::*;
use fs2::FileExt;
use grok_events::{
    CommittedEvent, ControlEvent, EventError, EventOrigin, EventSink, OperationRecord,
    ProjectionDelta, StatusDelta, StoreIdentity, TranscriptDelta,
};
use rusqlite::OptionalExtension;
use std::fs::{File, OpenOptions};

pub const MAX_EVENT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ROW_BYTES: usize = 64 * 1024;
pub const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EventCursor {
    pub store_id: Uuid,
    pub generation: Uuid,
    pub after_seq: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayPage {
    pub store_id: Uuid,
    pub generation: Uuid,
    pub watermark: u64,
    pub events: Vec<CommittedEvent>,
    pub next: EventCursor,
    pub has_more: bool,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotCursor {
    pub snapshot_id: Uuid,
    pub store_id: Uuid,
    pub generation: Uuid,
    pub session_after: Option<Uuid>,
    pub transcript_after: u64,
    #[serde(default)]
    pub operation_after: Option<Uuid>,
}
pub(crate) struct SnapshotLease {
    conn: Connection,
    watermark: u64,
    session_id: Option<Uuid>,
    cursor: SnapshotCursor,
    created: std::time::Instant,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSnapshot {
    pub store_id: Uuid,
    pub generation: Uuid,
    pub watermark: u64,
    pub session_id: Option<Uuid>,
    pub sessions: Vec<SessionRecord>,
    pub transcripts: Vec<TranscriptChunk>,
    pub operations: Vec<OperationRecord>,
    pub operations_truncated: bool,
    pub sessions_truncated: bool,
    pub transcripts_truncated: bool,
    pub next: Option<SnapshotCursor>,
}
impl EventSnapshot {
    pub fn cursor(&self) -> EventCursor {
        EventCursor {
            store_id: self.store_id,
            generation: self.generation,
            after_seq: self.watermark,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteRuntimeInfo {
    pub version: String,
    pub source_id: String,
    pub journal_mode: String,
    pub synchronous: i64,
    pub foreign_keys: i64,
    pub busy_timeout: i64,
}
pub(crate) fn verify_runtime(conn: &Connection) -> Result<SqliteRuntimeInfo> {
    let info = SqliteRuntimeInfo {
        version: conn.query_row("SELECT sqlite_version()", [], |r| r.get(0))?,
        source_id: conn.query_row("SELECT sqlite_source_id()", [], |r| r.get(0))?,
        journal_mode: conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?,
        synchronous: conn.query_row("PRAGMA synchronous", [], |r| r.get(0))?,
        foreign_keys: conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?,
        busy_timeout: conn.query_row("PRAGMA busy_timeout", [], |r| r.get(0))?,
    };
    if info.version!="3.53.2"||info.source_id!="2026-06-03 19:12:13 d6e03d8c777cfa2d35e3b60d8ec3e0187f3e9f99d8e2ee9cac695fd6fcdf1a24"||info.journal_mode!="wal"||info.synchronous!=2||info.foreign_keys!=1||info.busy_timeout!=5000 {return Err(PersistenceError::InvalidState(format!("unexpected linked SQLite runtime/settings: {info:?}")));}
    Ok(info)
}

/// Process-lifetime locks. The profile lock protects its ancillary stores; the
/// actual main-file lock pins database inode identity across path aliases.
pub struct ProfileOwnership {
    _profile: File,
    _database: File,
    _database_lock: File,
    path: PathBuf,
}
fn open_owned_file(path: &Path) -> Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    Ok(opts.open(path)?)
}
#[cfg(unix)]
fn physical_database_lock(database: &File) -> Result<File> {
    use std::os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        io::{AsRawFd, FromRawFd},
    };
    #[cfg(target_os = "macos")]
    let root = Path::new("/private/tmp");
    #[cfg(not(target_os = "macos"))]
    let root = Path::new("/tmp");
    let uid = unsafe { libc::geteuid() };
    let directory = root.join(format!("c3-database-ownership-{uid}"));
    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700);
    match builder.create(&directory) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    let directory_file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(&directory)?;
    let metadata = directory_file.metadata()?;
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        return Err(PersistenceError::InvalidState(
            "physical ownership namespace must be a private owned directory".into(),
        ));
    }
    let metadata = database.metadata()?;
    let name = std::ffi::CString::new(format!(
        "database-{:x}-{:x}.lock",
        metadata.dev(),
        metadata.ino()
    ))
    .unwrap();
    // openat pins the validated namespace even if a path alias is exchanged.
    let fd = unsafe {
        libc::openat(
            directory_file.as_raw_fd(),
            name.as_ptr(),
            libc::O_CREAT | libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            0o600,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let lock = unsafe { File::from_raw_fd(fd) };
    let metadata = lock.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
    {
        return Err(PersistenceError::InvalidState(
            "physical ownership lock must be a private regular file without aliases".into(),
        ));
    }
    lock.try_lock_exclusive().map_err(|_| {
        PersistenceError::InvalidState(
            "physical database already owned by another profile/process".into(),
        )
    })?;
    // Never unlink these files: another process may already have opened this inode.
    Ok(lock)
}
#[cfg(not(unix))]
fn physical_database_lock(_database: &File) -> Result<File> {
    Err(PersistenceError::InvalidState(
        "physical ownership requires a qualified Unix inode lock namespace".into(),
    ))
}
pub(crate) fn verify_sqlite_companions(path: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        let companion = PathBuf::from(name);
        match std::fs::symlink_metadata(&companion) {
            Ok(metadata) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if metadata.nlink() != 1 {
                        return Err(PersistenceError::InvalidState(
                            "SQLite companion hardlink aliases refused before opening database"
                                .into(),
                        ));
                    }
                }
                if !metadata.is_file() {
                    return Err(PersistenceError::InvalidState(
                        "SQLite companion must be a regular file without symlink aliases".into(),
                    ));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
fn companion_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}
pub(crate) fn pin_owned_companions(path: &Path) -> Result<[File; 2]> {
    let open = |suffix| -> Result<File> {
        let mut opts = OpenOptions::new();
        opts.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.custom_flags(libc::O_NOFOLLOW);
        }
        Ok(opts.open(companion_path(path, suffix))?)
    };
    Ok([open("-wal")?, open("-shm")?])
}
impl ProfileOwnership {
    pub fn acquire(profile_dir: impl AsRef<Path>, database_path: impl AsRef<Path>) -> Result<Self> {
        let profile_dir = profile_dir.as_ref();
        let database_path = database_path.as_ref();
        verify_sqlite_companions(database_path)?;
        // Inspect existing aliases before creating any profile/store files.
        if std::fs::symlink_metadata(database_path).is_ok_and(|m| m.file_type().is_symlink())
            && !database_path.exists()
        {
            return Err(PersistenceError::InvalidState(
                "unresolved database symlink refused before creating its target".into(),
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if database_path.exists() && std::fs::metadata(database_path)?.nlink() > 1 {
                return Err(PersistenceError::InvalidState("database hardlink aliases are ambiguous; ownership refused before opening SQLite".into()));
            }
        }
        std::fs::create_dir_all(profile_dir)?;
        let profile = std::fs::canonicalize(profile_dir)?;
        let path = if database_path.exists() {
            std::fs::canonicalize(database_path)?
        } else {
            let parent = database_path
                .parent()
                .ok_or_else(|| PersistenceError::InvalidState("database needs a parent".into()))?;
            // Resolve missing suffix lexically beneath an existing canonical parent.
            let mut ancestor = parent;
            let mut suffix = Vec::new();
            while !ancestor.exists() {
                suffix.push(
                    ancestor
                        .file_name()
                        .ok_or_else(|| {
                            PersistenceError::InvalidState("invalid database ancestor".into())
                        })?
                        .to_os_string(),
                );
                ancestor = ancestor.parent().ok_or_else(|| {
                    PersistenceError::InvalidState("database has no existing ancestor".into())
                })?;
            }
            let mut resolved = std::fs::canonicalize(ancestor)?;
            for component in suffix.into_iter().rev() {
                resolved.push(component);
            }
            resolved.join(database_path.file_name().ok_or_else(|| {
                PersistenceError::InvalidState("missing database filename".into())
            })?)
        };
        if !path.starts_with(&profile)
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(PersistenceError::InvalidState(
                "database location escapes the owned profile; no store directories created".into(),
            ));
        }

        let lock_path = profile.join(".c3-profile.lock");
        if std::fs::symlink_metadata(&lock_path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(PersistenceError::InvalidState(
                "profile lock symlink refused".into(),
            ));
        }
        let owner = open_owned_file(&lock_path)?;
        owner.try_lock_exclusive().map_err(|_| {
            PersistenceError::InvalidState("profile already owned by another process".into())
        })?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let path = std::fs::canonicalize(path.parent().unwrap())?.join(path.file_name().unwrap());
        if !path.starts_with(&profile) {
            return Err(PersistenceError::InvalidState(
                "database location escapes the owned profile".into(),
            ));
        }
        let database = open_owned_file(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if database.metadata()?.nlink() != 1 {
                return Err(PersistenceError::InvalidState(
                    "physical database hardlink aliases refused".into(),
                ));
            }
        }
        let database_lock = physical_database_lock(&database)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let current = std::fs::metadata(&path)?;
            let pinned = database.metadata()?;
            if current.dev() != pinned.dev()
                || current.ino() != pinned.ino()
                || current.nlink() != 1
            {
                return Err(PersistenceError::InvalidState(
                    "database identity changed during ownership admission".into(),
                ));
            }
        }

        Ok(Self {
            _profile: owner,
            _database: database,
            _database_lock: database_lock,
            path,
        })
    }
    pub fn verify_database_identity(&self) -> Result<()> {
        verify_sqlite_companions(&self.path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let current = std::fs::symlink_metadata(&self.path)?;
            let pinned = self._database.metadata()?;
            if !current.is_file()
                || current.dev() != pinned.dev()
                || current.ino() != pinned.ino()
                || current.nlink() != 1
            {
                return Err(PersistenceError::InvalidState(
                    "owned database path/inode changed before SQLite open".into(),
                ));
            }
        }
        Ok(())
    }
    pub fn db_path(&self) -> &Path {
        &self.path
    }
}
impl Persistence {
    pub(crate) fn verify_owned_writer_identity(&self) -> Result<()> {
        if let Some(owner) = &self._ownership {
            owner.verify_database_identity()?;
        }
        if let Some(pins) = &self.owned_companions {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                for (suffix, pin) in ["-wal", "-shm"].into_iter().zip(pins) {
                    let current=std::fs::symlink_metadata(companion_path(&self.path,suffix)).map_err(|e|PersistenceError::InvalidState(format!("owned SQLite companion disappeared; writer coverage unconfirmed: {e}")))?;
                    let pinned = pin.metadata()?;
                    if !current.is_file()
                        || current.nlink() != 1
                        || current.dev() != pinned.dev()
                        || current.ino() != pinned.ino()
                    {
                        return Err(PersistenceError::InvalidState(
                            "owned SQLite companion identity changed; writer coverage unconfirmed"
                                .into(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
    pub(crate) fn migrate_journal(&self) -> Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS event_journal(seq INTEGER PRIMARY KEY AUTOINCREMENT,envelope TEXT NOT NULL);CREATE TABLE IF NOT EXISTS session_runtimes(session_id TEXT PRIMARY KEY,runtime_id TEXT NOT NULL,active INTEGER NOT NULL);CREATE TABLE IF NOT EXISTS event_operations(operation_id TEXT PRIMARY KEY,session_id TEXT,record_json TEXT NOT NULL);CREATE INDEX IF NOT EXISTS event_operations_session ON event_operations(session_id,operation_id);")?;
        for key in ["event_store_id", "event_generation"] {
            tx.execute(
                "INSERT OR IGNORE INTO kv(key,value) VALUES(?1,?2)",
                params![key, Uuid::new_v4().to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn load_store_identity(&self) -> Result<StoreIdentity> {
        let conn = self.conn()?;
        let read = |key| -> Result<Uuid> {
            let id: String =
                conn.query_row("SELECT value FROM kv WHERE key=?1", [key], |r| r.get(0))?;
            Uuid::parse_str(&id).map_err(|_| {
                PersistenceError::InvalidState(
                    "corrupt event store identity; bytes preserved".into(),
                )
            })
        };
        Ok(StoreIdentity {
            store_id: read("event_store_id")?,
            generation: read("event_generation")?,
        })
    }
    pub fn store_identity(&self) -> StoreIdentity {
        self.identity
    }
    pub fn sqlite_runtime(&self) -> Result<SqliteRuntimeInfo> {
        verify_runtime(&self.conn()?)
    }
    /// Recovery copy of the same logical store. Concurrent restoration requires
    /// an explicit identity-rotation/import step; this API never starts a clone writer.
    pub fn backup_to(&self, destination: impl AsRef<Path>) -> Result<()> {
        let destination = destination.as_ref();
        let parent = destination
            .parent()
            .ok_or_else(|| PersistenceError::InvalidState("backup needs existing parent".into()))?;
        let stage = parent.join(format!(".c3-backup-{}.sqlite", Uuid::new_v4()));
        let mut opts = OpenOptions::new();
        opts.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = opts.open(&stage)?;
        // Keep a failed stage for inspection; never replace any existing destination.
        let conn = self.conn()?;
        conn.backup(rusqlite::MAIN_DB, &stage, None)?;
        file.sync_all()?;
        std::fs::hard_link(&stage, destination)?;
        File::open(parent)?.sync_all()?;
        std::fs::remove_file(&stage)?;
        Ok(())
    }
    pub fn replay_events(&self, cursor: EventCursor, limit: usize) -> Result<ReplayPage> {
        if cursor.store_id != self.identity.store_id
            || cursor.generation != self.identity.generation
        {
            return Err(PersistenceError::InvalidState(
                "event cursor belongs to another store/generation".into(),
            ));
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let watermark: i64 =
            tx.query_row("SELECT COALESCE(MAX(seq),0) FROM event_journal", [], |r| {
                r.get(0)
            })?;
        if cursor.after_seq > watermark as u64 {
            return Err(PersistenceError::InvalidState(
                "event cursor is ahead of committed coverage".into(),
            ));
        }
        let mut statement=tx.prepare("SELECT seq,length(CAST(envelope AS BLOB)) FROM event_journal WHERE seq>?1 AND seq<=?2 ORDER BY seq LIMIT ?3")?;
        let rows = statement.query_map(
            params![
                i64::try_from(cursor.after_seq).map_err(|_| PersistenceError::Bounds(
                    "event cursor exceeds SQLite range".into()
                ))?,
                watermark,
                limit.clamp(1, 512) as i64
            ],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)? as usize)),
        )?;
        let mut bytes = 0;
        let mut events = Vec::new();
        let mut after = cursor.after_seq;
        for row in rows {
            let (seq, size) = row?;
            if size > MAX_PAGE_BYTES {
                return Err(PersistenceError::Bounds(format!(
                    "event {seq} exceeds replay byte budget"
                )));
            }
            if bytes + size > MAX_PAGE_BYTES {
                break;
            }
            let text: String = tx.query_row(
                "SELECT envelope FROM event_journal WHERE seq=?1",
                [seq],
                |r| r.get(0),
            )?;
            events.push(serde_json::from_str(&text)?);
            bytes += size;
            after = seq as u64;
        }
        Ok(ReplayPage {
            store_id: self.identity.store_id,
            generation: self.identity.generation,
            watermark: watermark as u64,
            events,
            next: EventCursor {
                after_seq: after,
                ..cursor
            },
            has_more: after < watermark as u64,
        })
    }
    pub fn event_snapshot(&self, session_id: Option<Uuid>, limit: usize) -> Result<EventSnapshot> {
        self.event_snapshot_page(None, session_id, limit)
    }
    /// Invalidate unfinished UI read leases before final shutdown/checkpoint.
    /// Any later continuation must explicitly rebase, never silently skip rows.
    pub fn release_all_snapshots(&self) {
        self.snapshots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
    pub fn release_snapshot(&self, cursor: SnapshotCursor) {
        self.snapshots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&cursor.snapshot_id);
    }
    pub fn event_snapshot_page(
        &self,
        cursor: Option<SnapshotCursor>,
        session_id: Option<Uuid>,
        limit: usize,
    ) -> Result<EventSnapshot> {
        let mut leases = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
        leases.retain(|_, lease| lease.created.elapsed() < std::time::Duration::from_secs(60));
        let cursor = match cursor {
            Some(cursor) => cursor,
            None => {
                if leases.len() >= 4 {
                    return Err(PersistenceError::Bounds(
                        "too many unfinished snapshots; release or rebase a cursor".into(),
                    ));
                }
                let conn = self.conn()?;
                conn.execute_batch("BEGIN DEFERRED")?;
                let watermark: i64 =
                    conn.query_row("SELECT COALESCE(MAX(seq),0) FROM event_journal", [], |r| {
                        r.get(0)
                    })?;
                let cursor = SnapshotCursor {
                    snapshot_id: Uuid::new_v4(),
                    store_id: self.identity.store_id,
                    generation: self.identity.generation,
                    session_after: None,
                    transcript_after: 0,
                    operation_after: None,
                };
                leases.insert(
                    cursor.snapshot_id,
                    SnapshotLease {
                        conn,
                        watermark: watermark as u64,
                        session_id,
                        cursor,
                        created: std::time::Instant::now(),
                    },
                );
                cursor
            }
        };
        if cursor.store_id != self.identity.store_id
            || cursor.generation != self.identity.generation
        {
            return Err(PersistenceError::InvalidState(
                "snapshot cursor belongs to another store/generation".into(),
            ));
        }
        let lease = leases.get_mut(&cursor.snapshot_id).ok_or_else(|| {
            PersistenceError::InvalidState("snapshot expired or finished; rebase explicitly".into())
        })?;
        if lease.cursor != cursor || lease.session_id != session_id {
            return Err(PersistenceError::InvalidState(
                "snapshot cursor/session changed; rebase explicitly".into(),
            ));
        }
        let limit = limit.clamp(1, 512);
        let mut bytes = 0;
        let mut sessions = Vec::new();
        let mut transcripts = Vec::new();
        let mut sessions_truncated = false;
        let mut transcripts_truncated = false;
        let after = cursor
            .session_after
            .map(|id| id.to_string())
            .unwrap_or_default();
        let mut statement=lease.conn.prepare("SELECT id,length(CAST(metadata_json AS BLOB)),length(CAST(id AS BLOB))+length(CAST(cwd AS BLOB))+length(CAST(model AS BLOB))+length(CAST(mode AS BLOB))+length(CAST(status AS BLOB))+COALESCE(length(CAST(worktree AS BLOB)),0)+COALESCE(length(CAST(acp_session_id AS BLOB)),0)+length(CAST(created_at AS BLOB))+length(CAST(updated_at AS BLOB)) FROM sessions WHERE id>?1 ORDER BY id LIMIT ?2")?;
        for row in statement.query_map(params![after, (limit + 1) as i64], |r| {
            let size = r.get::<_, i64>(1)? as usize;
            let other = r.get::<_, i64>(2)? as usize;
            Ok((
                if size.saturating_add(other) > MAX_PAGE_BYTES {
                    String::new()
                } else {
                    r.get::<_, String>(0)?
                },
                size,
                other,
            ))
        })? {
            let (id, size, other) = row?;
            if size + other > MAX_PAGE_BYTES {
                return Err(PersistenceError::Bounds(
                    "session snapshot row exceeds byte budget".into(),
                ));
            }
            if sessions.len() == limit || bytes + size + other + 512 > MAX_PAGE_BYTES {
                sessions_truncated = true;
                break;
            }
            let rec = read_session(&lease.conn, &id)?;
            let encoded = serde_json::to_vec(&rec)?.len();
            if encoded > MAX_PAGE_BYTES {
                return Err(PersistenceError::Bounds(
                    "encoded session snapshot row exceeds byte budget".into(),
                ));
            }
            if bytes + encoded > MAX_PAGE_BYTES {
                sessions_truncated = true;
                break;
            }
            bytes += encoded;
            lease.cursor.session_after = Some(rec.id);
            sessions.push(rec);
        }
        drop(statement);
        if let Some(id) = session_id {
            let mut statement=lease.conn.prepare("SELECT seq,length(CAST(payload AS BLOB))+length(CAST(kind AS BLOB))+length(CAST(at AS BLOB)) FROM transcripts WHERE session_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3")?;
            for row in statement.query_map(
                params![
                    id.to_string(),
                    cursor.transcript_after as i64,
                    (limit + 1) as i64
                ],
                |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as usize)),
            )? {
                let (seq, size) = row?;
                if size > MAX_PAGE_BYTES {
                    return Err(PersistenceError::Bounds(format!("legacy transcript {seq} exceeds baseline byte budget; explicit oversize history handling required")));
                }
                if transcripts.len() == limit || bytes + size + 256 > MAX_PAGE_BYTES {
                    transcripts_truncated = true;
                    break;
                }
                let entry = lease.conn.query_row(
                    "SELECT kind,payload,at FROM transcripts WHERE session_id=?1 AND seq=?2",
                    params![id.to_string(), seq as i64],
                    |r| {
                        Ok(TranscriptChunk {
                            session_id: id,
                            seq,
                            kind: r.get(0)?,
                            payload: r.get(1)?,
                            at: parse_dt(&r.get::<_, String>(2)?),
                        })
                    },
                )?;
                let encoded = serde_json::to_vec(&entry)?.len();
                if encoded > MAX_PAGE_BYTES {
                    return Err(PersistenceError::Bounds(format!(
                        "legacy transcript {seq} exceeds encoded snapshot budget"
                    )));
                }
                if bytes + encoded > MAX_PAGE_BYTES {
                    transcripts_truncated = true;
                    break;
                }
                bytes += encoded;
                lease.cursor.transcript_after = seq;
                transcripts.push(entry);
            }
        }
        let mut operations = Vec::new();
        let mut operations_truncated = false;
        let after = cursor
            .operation_after
            .map(|id| id.to_string())
            .unwrap_or_default();
        let mut statement=lease.conn.prepare("SELECT operation_id,length(CAST(record_json AS BLOB)) FROM event_operations WHERE operation_id>?1 AND (?2 IS NULL OR session_id=?2) ORDER BY operation_id LIMIT ?3")?;
        for row in statement.query_map(
            params![
                after,
                session_id.map(|id| id.to_string()),
                (limit + 1) as i64
            ],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize)),
        )? {
            let (id, size) = row?;
            if size > MAX_PAGE_BYTES {
                return Err(PersistenceError::Bounds(
                    "operation snapshot row exceeds byte budget".into(),
                ));
            }
            if operations.len() == limit || bytes + size + 256 > MAX_PAGE_BYTES {
                operations_truncated = true;
                break;
            }
            let json: String = lease.conn.query_row(
                "SELECT record_json FROM event_operations WHERE operation_id=?1",
                [&id],
                |r| r.get(0),
            )?;
            let record: OperationRecord = serde_json::from_str(&json)?;
            bytes += size + 256;
            lease.cursor.operation_after = Some(record.operation_id);
            operations.push(record);
        }
        drop(statement);
        let next = (sessions_truncated || transcripts_truncated || operations_truncated)
            .then_some(lease.cursor);
        if next.is_some() && sessions.is_empty() && transcripts.is_empty() && operations.is_empty()
        {
            return Err(PersistenceError::Bounds(
                "snapshot page cannot advance within byte budget".into(),
            ));
        }
        let snapshot = EventSnapshot {
            store_id: self.identity.store_id,
            generation: self.identity.generation,
            watermark: lease.watermark,
            session_id,
            sessions,
            transcripts,
            operations,
            operations_truncated,
            sessions_truncated,
            transcripts_truncated,
            next,
        };
        if next.is_none() {
            leases.remove(&cursor.snapshot_id);
        }
        Ok(snapshot)
    }
}
fn read_session(conn: &Connection, id: &str) -> Result<SessionRecord> {
    let uuid = Uuid::parse_str(id).map_err(|_| {
        PersistenceError::InvalidState(
            "invalid session identity in snapshot; stored bytes preserved".into(),
        )
    })?;
    Ok(conn.query_row("SELECT id,cwd,mode,model,status,worktree,acp_session_id,metadata_json,created_at,updated_at,(SELECT COUNT(*) FROM transcripts WHERE session_id=sessions.id) FROM sessions WHERE id=?1",[id],|r|Ok(SessionRecord{id:uuid,cwd:r.get(1)?,mode:r.get(2)?,model:r.get(3)?,status:r.get(4)?,worktree:r.get(5)?,acp_session_id:r.get(6)?,metadata_json:r.get(7)?,created_at:parse_dt(&r.get::<_,String>(8)?),updated_at:parse_dt(&r.get::<_,String>(9)?),message_count:r.get::<_,i64>(10)? as u64}))?)
}
fn input_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|e| PersistenceError::InvalidInput(e.to_string()))
}
pub(crate) fn ensure_legacy_writer(conn: &Connection, id: Uuid) -> Result<()> {
    let managed: i64 = conn.query_row(
        "SELECT COUNT(*) FROM session_runtimes WHERE session_id=?1",
        [id.to_string()],
        |r| r.get(0),
    )?;
    if managed != 0 {
        return Err(PersistenceError::StaleRuntime);
    }
    let tombstone: i64 = conn.query_row(
        "SELECT COUNT(*) FROM kv WHERE key=?1",
        [format!("tombstone_{id}")],
        |r| r.get(0),
    )?;
    if tombstone != 0 {
        return Err(PersistenceError::NotFound(format!("deleted session {id}")));
    }
    Ok(())
}
pub(crate) fn validate_metadata(conn: &Connection, id: Uuid, metadata_json: &str) -> Result<()> {
    let tombstoned: i64 = conn.query_row(
        "SELECT COUNT(*) FROM kv WHERE key=?1",
        [format!("tombstone_{id}")],
        |r| r.get(0),
    )?;
    if tombstoned != 0 {
        return Err(PersistenceError::NotFound(format!("deleted session {id}")));
    }
    let runtime: Option<(String, bool)> = conn
        .query_row(
            "SELECT runtime_id,active FROM session_runtimes WHERE session_id=?1",
            [id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((runtime, active)) = runtime {
        let metadata: serde_json::Value = input_json(metadata_json)?;
        let supplied = metadata
            .get("metadata")
            .unwrap_or(&metadata)
            .get("runtimeId")
            .and_then(|v| v.as_str());
        if supplied != Some(runtime.as_str()) && (active || supplied.is_some()) {
            return Err(PersistenceError::StaleRuntime);
        }
    }
    Ok(())
}
pub(crate) fn tombstone_session(conn: &Connection, id: Uuid, at: DateTime<Utc>) -> Result<()> {
    conn.execute("INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("tombstone_{id}"),at.to_rfc3339()])?;
    conn.execute(
        "UPDATE session_runtimes SET active=0 WHERE session_id=?1",
        [id.to_string()],
    )?;
    conn.execute("DELETE FROM sessions WHERE id=?1", [id.to_string()])?;
    Ok(())
}
impl EventSink for Persistence {
    fn watermark(&self) -> grok_events::Result<u64> {
        let conn = self.conn().map_err(|e| EventError::Sink(e.to_string()))?;
        let seq: i64 = conn
            .query_row("SELECT COALESCE(MAX(seq),0) FROM event_journal", [], |r| {
                r.get(0)
            })
            .map_err(|e| EventError::Sink(e.to_string()))?;
        Ok(seq as u64)
    }
    fn identity(&self) -> StoreIdentity {
        self.identity
    }
    fn commit(
        &self,
        origin: &EventOrigin,
        event: &ControlEvent,
    ) -> grok_events::Result<CommittedEvent> {
        self.commit_event(origin, event)
            .map_err(|error| match error {
                PersistenceError::NotFound(_) => EventError::Tombstoned,
                PersistenceError::StaleRuntime => EventError::StaleRuntime,
                PersistenceError::Bounds(reason) => EventError::Bounds(reason),
                PersistenceError::InvalidInput(reason)
                | PersistenceError::InvalidConversationRole(reason) => {
                    EventError::InvalidOrigin(reason)
                }
                _ => EventError::Sink(error.to_string()),
            })
    }
}
impl Persistence {
    fn commit_event(&self, origin: &EventOrigin, event: &ControlEvent) -> Result<CommittedEvent> {
        let event_json = serde_json::to_string(event)?;
        if event_json.len() > MAX_EVENT_BYTES {
            return Err(PersistenceError::Bounds(
                "event payload exceeds durable byte budget".into(),
            ));
        }
        let _guard = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        self.verify_owned_writer_identity()?;
        let mut conn = self
            .writer
            .lock()
            .map_err(|_| PersistenceError::InvalidState("writer lock poisoned".into()))?;
        let tx = conn.transaction()?;
        if let Some(id) = event.session_id() {
            let retirement = matches!(event, ControlEvent::RuntimeRetired { .. });
            let matched_outcome = matches!(event, ControlEvent::HostOperationOutcome { .. });
            if !retirement && !matched_outcome && !self.ensure_session_row(&tx, id)? {
                return Err(PersistenceError::NotFound(format!("deleted session {id}")));
            }
            match event {
                ControlEvent::RuntimeActivated {
                    session_id,
                    runtime_id,
                    ..
                } => {
                    if origin.session_id != Some(*session_id)
                        || origin.runtime_id != Some(*runtime_id)
                    {
                        return Err(PersistenceError::StaleRuntime);
                    }
                    tx.execute("INSERT INTO session_runtimes(session_id,runtime_id,active) VALUES(?1,?2,1) ON CONFLICT(session_id) DO UPDATE SET runtime_id=excluded.runtime_id,active=1",params![id.to_string(),runtime_id.to_string()])?;
                }
                _ => {
                    let current: Option<(String, bool)> = tx
                        .query_row(
                            "SELECT runtime_id,active FROM session_runtimes WHERE session_id=?1",
                            [id.to_string()],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;
                    if let Some((runtime, active)) = current {
                        let host_control = origin.runtime_id.is_none() && event.is_host_control();
                        if (!active && !retirement && !host_control)
                            || (!host_control
                                && origin.runtime_id.map(|v| v.to_string()).as_deref()
                                    != Some(&runtime))
                        {
                            return Err(PersistenceError::StaleRuntime);
                        }
                    }
                    if matches!(event, ControlEvent::RuntimeRetired { .. }) {
                        tx.execute(
                            "UPDATE session_runtimes SET active=0 WHERE session_id=?1",
                            [id.to_string()],
                        )?;
                    }
                }
            }
        }
        tx.execute("INSERT INTO event_journal(envelope) VALUES('')", [])?;
        let seq = tx.last_insert_rowid() as u64;
        let projection = project(&tx, event, origin, seq)?;
        let envelope = CommittedEvent {
            store_id: self.identity.store_id,
            generation: self.identity.generation,
            seq,
            origin: *origin,
            event: event.clone(),
            projection,
        };
        let serialized = serde_json::to_string(&envelope)?;
        if serialized.len() > MAX_PAGE_BYTES {
            return Err(PersistenceError::Bounds(
                "event plus projection exceeds replay byte budget".into(),
            ));
        }
        tx.execute(
            "UPDATE event_journal SET envelope=?1 WHERE seq=?2",
            params![serialized, seq as i64],
        )?;
        tx.commit()?;
        #[cfg(test)]
        if let Some(hook) = self.after_commit_test_hook.lock().unwrap().take() {
            hook();
        }
        // A failure here does not roll back the already committed old-inode receipt.
        // It refuses visibility/effect acknowledgement and poisons writer coverage.
        self.verify_owned_writer_identity()?;
        Ok(envelope)
    }
}
fn record_intent(
    conn: &Connection,
    event: &ControlEvent,
    seq: u64,
    delta: &mut ProjectionDelta,
) -> Result<()> {
    let (id, session_id, kind, target, submission) = match event {
        ControlEvent::HostOperationIntent {
            operation_id,
            session_id,
            kind,
            target,
            ..
        } => (
            *operation_id,
            *session_id,
            kind.as_str(),
            target.as_str(),
            false,
        ),
        ControlEvent::UserMessage {
            operation_id,
            session_id,
            ..
        } => (
            *operation_id,
            Some(*session_id),
            "submission",
            "conversation",
            true,
        ),
        _ => {
            return Err(PersistenceError::InvalidInput(
                "operation intent expected".into(),
            ))
        }
    };
    if kind.len() > 128 || target.len() > 4096 {
        return Err(PersistenceError::Bounds(
            "operation kind/target budget exceeded".into(),
        ));
    }
    let existing: i64 = conn.query_row(
        "SELECT COUNT(*) FROM event_operations WHERE operation_id=?1",
        [id.to_string()],
        |r| r.get(0),
    )?;
    if existing != 0 {
        return Err(PersistenceError::StaleRuntime);
    }
    let record = OperationRecord {
        submission,
        operation_id: id,
        session_id,
        kind: kind.into(),
        target: target.into(),
        intent_seq: seq,
        outcome_seq: None,
        result: None,
    };
    conn.execute(
        "INSERT INTO event_operations(operation_id,session_id,record_json) VALUES(?1,?2,?3)",
        params![
            id.to_string(),
            session_id.map(|v| v.to_string()),
            serde_json::to_string(&record)?
        ],
    )?;
    delta.operations.push(record);
    Ok(())
}
fn record_outcome(
    conn: &Connection,
    event: &ControlEvent,
    seq: u64,
    delta: &mut ProjectionDelta,
) -> Result<()> {
    let ControlEvent::HostOperationOutcome {
        operation_id,
        session_id,
        kind,
        target,
        result,
        ..
    } = event
    else {
        return Err(PersistenceError::InvalidInput(
            "operation outcome expected".into(),
        ));
    };
    let id = *operation_id;
    let session_id = *session_id;
    if kind.len() > 128 || target.len() > 4096 || result.len() > 4096 {
        return Err(PersistenceError::Bounds(
            "operation outcome budget exceeded".into(),
        ));
    }
    let json: Option<String> = conn
        .query_row(
            "SELECT record_json FROM event_operations WHERE operation_id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    let Some(json) = json else {
        return Err(PersistenceError::StaleRuntime);
    };
    let mut record: OperationRecord = serde_json::from_str(&json)?;
    if record.session_id != session_id || record.outcome_seq.is_some() {
        return Err(PersistenceError::StaleRuntime);
    }
    if !record.submission && (record.kind != *kind || record.target != *target) {
        return Err(PersistenceError::InvalidInput(
            "operation outcome does not match captured intent kind/target".into(),
        ));
    }
    record.result = Some(result.into());
    record.outcome_seq = Some(seq);
    conn.execute(
        "UPDATE event_operations SET record_json=?1 WHERE operation_id=?2",
        params![serde_json::to_string(&record)?, id.to_string()],
    )?;
    delta.operations.push(record);
    Ok(())
}
fn status(
    conn: &Connection,
    id: Uuid,
    value: &str,
    origin: &EventOrigin,
    delta: &mut ProjectionDelta,
) -> Result<()> {
    conn.execute(
        "UPDATE sessions SET status=?1,updated_at=?2 WHERE id=?3",
        params![value, Utc::now().to_rfc3339(), id.to_string()],
    )?;
    delta.statuses.push(StatusDelta {
        session_id: id,
        status: value.into(),
        runtime_id: origin.runtime_id,
    });
    Ok(())
}
fn append(
    conn: &Connection,
    id: Uuid,
    kind: &str,
    payload: &str,
    at: DateTime<Utc>,
    merge: bool,
    delta: &mut ProjectionDelta,
) -> Result<()> {
    let plain = matches!(kind, "agent" | "thought" | "term" | "prompt");
    if !plain && payload.len() > 1024 * 1024 {
        return Err(PersistenceError::Bounds(format!(
            "structured {kind} row exceeds one MiB"
        )));
    }
    let row_limit = if plain {
        MAX_ROW_BYTES
    } else {
        payload.len().max(1)
    };
    let mut remaining = payload;
    while !remaining.is_empty() {
        let mut target = None;
        if merge && plain {
            let mut statement=conn.prepare("SELECT seq,kind,at,length(CAST(payload AS BLOB)) FROM transcripts WHERE session_id=?1 ORDER BY seq DESC LIMIT 24")?;
            for row in statement.query_map([id.to_string()], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)? as usize,
                ))
            })? {
                let (seq, k, stamp, size) = row?;
                if k == kind {
                    if (at - parse_dt(&stamp)).num_seconds().abs() <= 10 && size < MAX_ROW_BYTES {
                        target = Some((seq, size));
                    }
                    break;
                }
                if matches!(kind, "agent" | "thought") && matches!(k.as_str(), "term" | "system") {
                    continue;
                }
                break;
            }
        }
        let available = row_limit - target.map(|(_, size)| size).unwrap_or(0);
        let mut split = remaining.len().min(available);
        while !remaining.is_char_boundary(split) {
            split -= 1;
        }
        if split == 0 {
            target = None;
            split = remaining.len().min(row_limit);
            while !remaining.is_char_boundary(split) {
                split -= 1;
            }
        }
        let fragment = &remaining[..split];
        let seq = if let Some((seq, _)) = target {
            conn.execute(
                "UPDATE transcripts SET payload=payload||?1,at=?2 WHERE session_id=?3 AND seq=?4",
                params![fragment, at.to_rfc3339(), id.to_string(), seq],
            )?;
            seq
        } else {
            let seq: i64 = conn.query_row(
                "SELECT COALESCE(MAX(seq),0)+1 FROM transcripts WHERE session_id=?1",
                [id.to_string()],
                |r| r.get(0),
            )?;
            conn.execute(
                "INSERT INTO transcripts(session_id,seq,kind,payload,at) VALUES(?1,?2,?3,?4,?5)",
                params![id.to_string(), seq, kind, fragment, at.to_rfc3339()],
            )?;
            seq
        };
        delta.transcripts.push(TranscriptDelta {
            session_id: id,
            seq: seq as u64,
            role: super::kind_to_role(kind),
            body: fragment.into(),
            at: at.to_rfc3339(),
            append: target.is_some(),
        });
        remaining = &remaining[split..];
    }
    Ok(())
}
fn project(
    conn: &Connection,
    event: &ControlEvent,
    origin: &EventOrigin,
    event_seq: u64,
) -> Result<ProjectionDelta> {
    use ControlEvent::*;
    let mut delta = ProjectionDelta::default();
    match event {
        StoreValueUpdated { key, value, .. } => {
            if !matches!(
                key.as_str(),
                "reviewed_builds_v1" | "reviewed_build_concurrency" | "scheduler_jobs_v2"
            ) {
                return Err(PersistenceError::InvalidInput(
                    "reserved or unsupported durable store key".into(),
                ));
            }
            conn.execute("INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value])?;
        }
        StoreRecoveryObserved { at, .. } => {
            conn.execute("UPDATE sessions SET status='recovering',updated_at=?1 WHERE id IN (SELECT session_id FROM session_runtimes WHERE active=1) AND status NOT IN ('completed','failed','cancelled')",[at.to_rfc3339()])?;
            conn.execute("UPDATE session_runtimes SET active=0 WHERE active=1", [])?;
            delta.baseline_changed_all = true;
        }
        ImportedConversation {
            session_id,
            metadata_json,
            entries_json,
            ..
        } => {
            let rec: SessionRecord = input_json(metadata_json)?;
            if rec.id != *session_id {
                return Err(PersistenceError::StaleRuntime);
            }
            let active: i64 = conn.query_row(
                "SELECT COUNT(*) FROM session_runtimes WHERE session_id=?1 AND active=1",
                [session_id.to_string()],
                |r| r.get(0),
            )?;
            let existing: i64 = conn.query_row(
                "SELECT COUNT(*) FROM transcripts WHERE session_id=?1",
                [session_id.to_string()],
                |r| r.get(0),
            )?;
            if active != 0 || existing != 0 {
                return Err(PersistenceError::StaleRuntime);
            }
            conn.execute("UPDATE sessions SET cwd=?1,mode=?2,model=?3,status=?4,worktree=?5,acp_session_id=?6,metadata_json=?7,created_at=?8,updated_at=?9 WHERE id=?10",params![rec.cwd,rec.mode,rec.model,rec.status,rec.worktree,rec.acp_session_id,rec.metadata_json,rec.created_at.to_rfc3339(),rec.updated_at.to_rfc3339(),session_id.to_string()])?;
            let entries: Vec<TranscriptEntry> = input_json(entries_json)?;
            let mut previous = 0;
            for entry in entries {
                let kind = match entry.role.as_str() {
                    "user" => "prompt",
                    "agent" | "assistant" => "agent",
                    "system" => "system",
                    _ => return Err(PersistenceError::InvalidConversationRole(entry.role)),
                };
                if entry.seq <= previous || entry.seq > i64::MAX as u64 {
                    return Err(PersistenceError::Bounds("import sequence must be strictly increasing positive SQLite-range source sequence".into()));
                }
                previous = entry.seq;
                conn.execute("INSERT INTO transcripts(session_id,seq,kind,payload,at) VALUES(?1,?2,?3,?4,?5)",params![session_id.to_string(),entry.seq as i64,kind,entry.body,entry.at])?;
            }
            delta.baseline_changed_sessions.push(*session_id);
        }
        HostOperationIntent { .. } => record_intent(conn, event, event_seq, &mut delta)?,
        HostOperationOutcome { .. } => record_outcome(conn, event, event_seq, &mut delta)?,
        SessionMetadataUpdated {
            session_id,
            metadata_json,
            at,
        } => {
            let rec: SessionRecord = input_json(metadata_json)?;
            if rec.id != *session_id {
                return Err(PersistenceError::StaleRuntime);
            }
            validate_metadata(conn, *session_id, &rec.metadata_json)?;
            conn.execute("UPDATE sessions SET cwd=?1,mode=?2,model=?3,worktree=?4,acp_session_id=?5,metadata_json=?6,updated_at=?7 WHERE id=?8",params![rec.cwd,rec.mode,rec.model,rec.worktree,rec.acp_session_id,rec.metadata_json,at.to_rfc3339(),session_id.to_string()])?;
            let current: String = conn.query_row(
                "SELECT status FROM sessions WHERE id=?1",
                [session_id.to_string()],
                |r| r.get(0),
            )?;
            delta.statuses.push(StatusDelta {
                session_id: *session_id,
                status: current,
                runtime_id: origin.runtime_id,
            });
            delta.baseline_changed_sessions.push(*session_id);
        }
        SessionRemoved { session_id, at } => {
            tombstone_session(conn, *session_id, *at)?;
            delta.deleted_sessions.push(*session_id);
        }
        RuntimeActivated { session_id, .. } => {
            status(conn, *session_id, "starting", origin, &mut delta)?
        }
        SessionCreated {
            session_id,
            cwd,
            mode,
            ..
        } => {
            conn.execute(
                "UPDATE sessions SET cwd=?1,mode=?2 WHERE id=?3",
                params![cwd, mode, session_id.to_string()],
            )?;
        }
        UserMessage {
            session_id,
            text,
            at,
            ..
        } => {
            append(conn, *session_id, "prompt", text, *at, false, &mut delta)?;
            record_intent(conn, event, event_seq, &mut delta)?;
        }
        AgentMessage {
            session_id,
            text,
            at,
        } => {
            let lower = text.to_lowercase();
            if !text.trim().is_empty()
                && !lower.starts_with("prompt sent")
                && lower != "turn complete"
            {
                let (kind, body) = match text.strip_prefix('💭') {
                    Some(rest) => ("thought", rest),
                    None => ("agent", text.as_str()),
                };
                append(conn, *session_id, kind, body, *at, true, &mut delta)?;
            }
        }
        ToolCall { session_id, event } => {
            if !event.tool.to_lowercase().contains("plan")
                && !event.args_summary.contains("\"plan\":")
            {
                let body=serde_json::json!({"id":event.id,"tool":event.tool,"status":event.status,"args":event.args_summary,"result":event.result_summary}).to_string();
                append(
                    conn,
                    *session_id,
                    "tool",
                    &body,
                    event.at,
                    false,
                    &mut delta,
                )?;
            }
            status(conn, *session_id, "running", origin, &mut delta)?;
        }
        PlanUpdate { session_id, event } => append(
            conn,
            *session_id,
            "plan",
            &serde_json::to_string(event)?,
            event.at,
            false,
            &mut delta,
        )?,
        SessionStatusChanged {
            session_id,
            status: current,
            ..
        } => status(
            conn,
            *session_id,
            &format!("{current:?}").to_lowercase(),
            origin,
            &mut delta,
        )?,
        SessionCancelled { session_id, at } | SessionCompleted { session_id, at } => {
            let value = if matches!(event, SessionCancelled { .. }) {
                "cancelled"
            } else {
                "completed"
            };
            status(conn, *session_id, value, origin, &mut delta)?;
            append(
                conn,
                *session_id,
                "system",
                &format!("session {value}"),
                *at,
                false,
                &mut delta,
            )?;
        }
        ApprovalRequired {
            session_id,
            tool,
            summary,
            auto_approved,
            at,
            ..
        } => {
            if *auto_approved {
                append(
                    conn,
                    *session_id,
                    "system",
                    &format!("auto-approved (yolo): {tool}"),
                    *at,
                    false,
                    &mut delta,
                )?;
            } else {
                status(conn, *session_id, "waitingapproval", origin, &mut delta)?;
                append(
                    conn,
                    *session_id,
                    "approval",
                    &format!("{tool} — {summary}"),
                    *at,
                    false,
                    &mut delta,
                )?;
            }
        }
        ApprovalResolved {
            session_id,
            option_id,
            cancelled,
            at,
            ..
        } => {
            status(conn, *session_id, "running", origin, &mut delta)?;
            let body = if *cancelled {
                "approval cancelled".into()
            } else {
                format!(
                    "approval granted: {}",
                    option_id.as_deref().unwrap_or("selected")
                )
            };
            append(conn, *session_id, "system", &body, *at, false, &mut delta)?;
        }
        Raw {
            session_id: Some(id),
            payload,
        } => match payload.get("channel").and_then(|v| v.as_str()) {
            Some("plan_doc") => {
                if let Some(text) = payload.get("text").and_then(|v| v.as_str()) {
                    append(conn, *id, "plan", text, Utc::now(), false, &mut delta)?;
                }
            }
            Some("term") => {
                if let Some(line) = payload
                    .get("line")
                    .and_then(|v| v.as_str())
                    .filter(|text| !text.trim().is_empty())
                {
                    append(
                        conn,
                        *id,
                        "term",
                        &format!("{line}\n"),
                        Utc::now(),
                        true,
                        &mut delta,
                    )?;
                }
            }
            _ => {}
        },
        Error {
            session_id: Some(id),
            message,
            at,
        } => append(conn, *id, "error", message, *at, false, &mut delta)?,
        // AgentOutput boundaries and historical grants stay in journal only.
        _ => {}
    }
    Ok(delta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use grok_events::{EventBus, SessionStatus};
    use std::{collections::BTreeMap, sync::Arc};
    fn fixture() -> (
        tempfile::TempDir,
        Arc<Persistence>,
        Arc<EventBus>,
        Uuid,
        Arc<EventBus>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Persistence::open(dir.path().join("memory.sqlite")).unwrap());
        let bus = Arc::new(EventBus::with_capacity(2));
        bus.install_sink(db.clone()).unwrap();
        let id = Uuid::new_v4();
        let scoped = bus.register_runtime(id).unwrap();
        (dir, db, bus, id, scoped)
    }
    fn rec(id: Uuid, runtime: Option<Uuid>) -> SessionRecord {
        SessionRecord {
            id,
            cwd: "/generated".into(),
            mode: "acp".into(),
            model: "fixture".into(),
            status: "running".into(),
            worktree: None,
            acp_session_id: None,
            metadata_json: serde_json::json!({"metadata":{"runtimeId":runtime}}).to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            message_count: 0,
        }
    }
    fn message(id: Uuid, text: &str) -> ControlEvent {
        ControlEvent::AgentMessage {
            session_id: id,
            text: text.into(),
            at: Utc::now(),
        }
    }
    #[test]
    fn zero_subscriber_commit_and_linked_runtime_are_real() {
        let (_dir, db, bus, id, scoped) = fixture();
        assert_eq!(bus.receiver_count(), 0);
        scoped.emit_checked(message(id, "final byte 🧙")).unwrap();
        scoped
            .emit_checked(ControlEvent::SessionStatusChanged {
                session_id: id,
                status: SessionStatus::Idle,
                at: Utc::now(),
            })
            .unwrap();
        assert_eq!(db.get_session(id).unwrap().status, "idle");
        assert_eq!(db.transcripts(id).unwrap()[0].payload, "final byte 🧙");
        let linked = db.sqlite_runtime().unwrap();
        assert_eq!(linked.version, "3.53.2");
        assert_eq!(linked.synchronous, 2);
        assert_eq!(linked.journal_mode, "wal");
        assert_eq!(bus.health().last_seq, 3);
        drop(scoped);
        drop(bus);
        drop(db);
    }
    #[test]
    fn rollback_is_atomic_and_sticky_without_notification() {
        let (_dir, db, bus, id, scoped) = fixture();
        let mut raw = bus.subscribe();
        let mut committed = bus.subscribe_committed();
        let conn = db.conn().unwrap();
        conn.execute_batch("CREATE TRIGGER injected_abort BEFORE UPDATE ON event_journal BEGIN SELECT RAISE(ABORT,'generated failure'); END;").unwrap();
        assert!(scoped.emit_checked(message(id, "must rollback")).is_err());
        assert!(db.transcripts(id).unwrap().is_empty());
        assert_eq!(db.get_session(id).unwrap().status, "starting");
        assert_eq!(db.watermark().unwrap(), 1);
        assert!(raw.try_recv().is_err());
        assert!(committed.try_recv().is_err());
        assert!(!bus.health().healthy);
        conn.execute_batch("DROP TRIGGER injected_abort").unwrap();
        assert!(bus.ensure_healthy().is_err());
        assert!(scoped.emit_checked(message(id, "still blocked")).is_err());
    }
    #[test]
    fn stale_runtime_metadata_tombstone_and_protective_retirement() {
        let (_dir, db, bus, id, old) = fixture();
        let new = bus.register_runtime(id).unwrap();
        assert!(matches!(
            old.emit_checked(message(id, "old")),
            Err(EventError::StaleRuntime)
        ));
        assert!(bus.health().healthy);
        new.emit_checked(ControlEvent::SessionMetadataUpdated {
            session_id: id,
            metadata_json: serde_json::to_string(&rec(id, new.runtime_id())).unwrap(),
            at: Utc::now(),
        })
        .unwrap();
        new.emit_checked(ControlEvent::SessionStatusChanged {
            session_id: id,
            status: SessionStatus::Idle,
            at: Utc::now(),
        })
        .unwrap();
        new.emit_checked(ControlEvent::SessionMetadataUpdated {
            session_id: id,
            metadata_json: serde_json::to_string(&rec(id, new.runtime_id())).unwrap(),
            at: Utc::now(),
        })
        .unwrap();
        assert_eq!(db.get_session(id).unwrap().status, "idle");
        assert!(matches!(
            db.upsert_session(&rec(id, old.runtime_id())),
            Err(PersistenceError::StaleRuntime)
        ));
        bus.emit_checked(ControlEvent::SessionRemoved {
            session_id: id,
            at: Utc::now(),
        })
        .unwrap();
        assert!(matches!(
            new.emit_checked(message(id, "late")),
            Err(EventError::Tombstoned)
        ));
        assert!(db.upsert_session(&rec(id, new.runtime_id())).is_err());
        assert!(bus.health().healthy);
        new.retire_runtime(new.runtime_id().unwrap()).unwrap();
        assert!(db.get_session(id).is_err());
        assert!(bus.health().healthy);
    }
    #[test]
    fn lagged_receiver_recovers_small_interleaved_pages_and_exact_fragments() {
        let (_dir, db, bus, a, scope_a) = fixture();
        let b = Uuid::new_v4();
        let scope_b = bus.register_runtime(b).unwrap();
        let mut receiver = bus.subscribe_committed();
        for i in 0..40 {
            for (id, scope) in [(a, &scope_a), (b, &scope_b)] {
                let text = format!("{i}λ ");
                scope.emit_checked(message(id, &text)).unwrap();
                scope
                    .emit_checked(ControlEvent::AgentOutput {
                        session_id: id,
                        message_id: None,
                        text: text.clone(),
                        at: Utc::now(),
                    })
                    .unwrap();
            }
        }
        assert!(matches!(
            receiver.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))
        ));
        let identity = db.store_identity();
        let mut cursor = EventCursor {
            store_id: identity.store_id,
            generation: identity.generation,
            after_seq: 0,
        };
        let mut rows: BTreeMap<(Uuid, u64), String> = BTreeMap::new();
        let mut seq = 0;
        loop {
            let page = db.replay_events(cursor, 3).unwrap();
            for event in page.events {
                assert!(event.seq > seq);
                seq = event.seq;
                for patch in event.projection.transcripts {
                    let body = rows.entry((patch.session_id, patch.seq)).or_default();
                    if patch.append {
                        body.push_str(&patch.body);
                    } else {
                        *body = patch.body;
                    }
                }
            }
            cursor = page.next;
            if !page.has_more {
                break;
            }
        }
        for id in [a, b] {
            let expected = db
                .transcripts(id)
                .unwrap()
                .into_iter()
                .map(|e| e.payload)
                .collect::<String>();
            let reconstructed = rows
                .iter()
                .filter(|((sid, _), _)| *sid == id)
                .map(|(_, text)| text.as_str())
                .collect::<String>();
            assert_eq!(reconstructed, expected);
            assert_eq!(expected.matches('λ').count(), 40);
        }
    }
    #[test]
    fn baseline_cursor_pins_all_legacy_rows_while_new_chunks_commit() {
        let (_dir, db, _bus, id, scope) = fixture();
        let conn = db.conn().unwrap();
        for i in 1..=530 {
            conn.execute("INSERT INTO transcripts(session_id,seq,kind,payload,at) VALUES(?1,?2,'agent',?3,?4)",params![id.to_string(),i,format!("legacy{i}"),Utc::now().to_rfc3339()]).unwrap();
        }
        let first = db.event_snapshot(Some(id), 7).unwrap();
        let wm = first.watermark;
        let mut all = first.transcripts;
        let mut next = first.next;
        scope.emit_checked(message(id, " NEW")).unwrap();
        while let Some(cursor) = next {
            let page = db.event_snapshot_page(Some(cursor), Some(id), 7).unwrap();
            assert_eq!(page.watermark, wm);
            all.extend(page.transcripts);
            next = page.next;
        }
        assert_eq!(all.len(), 530);
        assert_eq!(all.last().unwrap().payload, "legacy530");
        let identity = db.store_identity();
        let replay = db
            .replay_events(
                EventCursor {
                    store_id: identity.store_id,
                    generation: identity.generation,
                    after_seq: wm,
                },
                10,
            )
            .unwrap();
        assert_eq!(replay.events.len(), 1);
        assert_eq!(replay.events[0].projection.transcripts[0].body, " NEW");
        assert!(replay.events[0].projection.transcripts[0].append);
    }
    #[test]
    fn unicode_split_structured_rows_and_input_rejection_are_bounded() {
        let (_dir, db, bus, id, scope) = fixture();
        let text = "λ".repeat(MAX_ROW_BYTES);
        let committed = scope.emit_checked(message(id, &text)).unwrap();
        assert!(committed.projection.transcripts.len() >= 2);
        let rows = db.transcripts(id).unwrap();
        assert!(rows.iter().all(|r| r.payload.len() <= MAX_ROW_BYTES));
        assert_eq!(
            rows.iter().map(|r| r.payload.as_str()).collect::<String>(),
            text
        );
        let json = serde_json::json!({"tool":"fixture","long":"a".repeat(100_000)}).to_string();
        let mut delta = ProjectionDelta::default();
        append(
            &db.conn().unwrap(),
            id,
            "tool",
            &json,
            Utc::now(),
            false,
            &mut delta,
        )
        .unwrap();
        assert_eq!(delta.transcripts.len(), 1);
        assert!(serde_json::from_str::<serde_json::Value>(&delta.transcripts[0].body).is_ok());
        assert!(matches!(
            scope.emit_checked(message(id, &"a".repeat(MAX_EVENT_BYTES + 1))),
            Err(EventError::Bounds(_))
        ));
        assert!(bus.health().healthy);
    }
    #[test]
    fn operation_intent_survives_reopen_without_inventing_outcome() {
        let (dir, db, _bus, id, scope) = fixture();
        let op = Uuid::new_v4();
        scope
            .emit_checked(ControlEvent::UserMessage {
                session_id: id,
                operation_id: op,
                text: "new request".into(),
                at: Utc::now(),
            })
            .unwrap();
        let reopened = Persistence::open(dir.path().join("memory.sqlite")).unwrap();
        let page = reopened.event_snapshot(Some(id), 100).unwrap();
        assert_eq!(page.operations.len(), 1);
        assert_eq!(page.operations[0].operation_id, op);
        assert!(page.operations[0].outcome_seq.is_none());
        assert!(page.operations[0].result.is_none());
        scope
            .emit_checked(ControlEvent::HostOperationOutcome {
                session_id: Some(id),
                operation_id: op,
                kind: "session/prompt".into(),
                target: "native conversation".into(),
                result: "uncertain".into(),
                at: Utc::now(),
            })
            .unwrap();
        assert_eq!(
            db.event_snapshot(Some(id), 100).unwrap().operations[0]
                .result
                .as_deref(),
            Some("uncertain")
        );
    }
    #[test]
    fn imported_baseline_and_metadata_are_atomic_and_inert() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Persistence::open(dir.path().join("memory.sqlite")).unwrap());
        let bus = EventBus::new();
        bus.install_sink(db.clone()).unwrap();
        let id = Uuid::new_v4();
        let record = rec(id, None);
        let entries = vec![
            TranscriptEntry {
                role: "user".into(),
                body: "source".into(),
                seq: 42,
                at: Utc::now().to_rfc3339(),
            },
            TranscriptEntry {
                role: "assistant".into(),
                body: "history".into(),
                seq: 99,
                at: Utc::now().to_rfc3339(),
            },
        ];
        let event = ControlEvent::ImportedConversation {
            session_id: id,
            metadata_json: serde_json::to_string(&record).unwrap(),
            entries_json: serde_json::to_string(&entries).unwrap(),
            at: Utc::now(),
        };
        let committed = bus.emit_checked(event).unwrap();
        assert_eq!(committed.projection.baseline_changed_sessions, vec![id]);
        let rows = db.transcripts(id).unwrap();
        assert_eq!(rows[0].seq, 42);
        assert_eq!(rows[1].seq, 99);
        let bad = Uuid::new_v4();
        let invalid = ControlEvent::ImportedConversation {
            session_id: bad,
            metadata_json: serde_json::to_string(&rec(bad, None)).unwrap(),
            entries_json: serde_json::to_string(&vec![TranscriptEntry {
                role: "approval".into(),
                body: "allow forever".into(),
                seq: 1,
                at: Utc::now().to_rfc3339(),
            }])
            .unwrap(),
            at: Utc::now(),
        };
        assert!(bus.emit_checked(invalid).is_err());
        assert!(db.get_session(bad).is_err());
        assert!(bus.health().healthy);
    }
    #[test]
    fn online_backup_includes_wal_and_never_replaces_destination() {
        let (dir, db, _bus, id, scope) = fixture();
        let reader = db.conn().unwrap();
        reader.execute_batch("BEGIN").unwrap();
        let _: i64 = reader
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        scope
            .emit_checked(message(id, "after reader snapshot"))
            .unwrap();
        assert!(db.checkpoint().is_err());
        let backup = dir.path().join("backup.sqlite");
        db.backup_to(&backup).unwrap();
        let restored = Persistence::open(&backup).unwrap();
        assert_eq!(restored.store_identity(), db.store_identity());
        assert_eq!(restored.watermark().unwrap(), db.watermark().unwrap());
        assert_eq!(
            restored.transcripts(id).unwrap()[0].payload,
            "after reader snapshot"
        );
        let before = std::fs::read(&backup).unwrap();
        assert!(db.backup_to(&backup).is_err());
        assert_eq!(std::fs::read(&backup).unwrap(), before);
        reader.execute_batch("ROLLBACK").unwrap();
        db.checkpoint().unwrap();
    }
    #[test]
    fn ownership_child_probe() {
        if let Ok(profile) = std::env::var("C3_GENERATED_LOCK_PROFILE") {
            let path = std::env::var_os("C3_GENERATED_LOCK_DB")
                .map(PathBuf::from)
                .unwrap_or_else(|| Path::new(&profile).join("control.sqlite"));
            let owner = ProfileOwnership::acquire(&profile, &path);
            if std::env::var("C3_GENERATED_LOCK_EXPECT").as_deref() == Ok("success") {
                let db = Persistence::open_owned(Arc::new(owner.unwrap())).unwrap();
                db.set_kv("generated_child_marker", "opened-owned").unwrap();
            } else {
                assert!(owner.is_err());
            }
        }
    }
    #[test]
    fn physical_lock_blocks_second_process_and_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("owned");
        let db = profile.join("control.sqlite");
        let owner = ProfileOwnership::acquire(&profile, &db).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "journal::tests::ownership_child_probe",
                "--nocapture",
            ])
            .env("C3_GENERATED_LOCK_PROFILE", &profile)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        #[cfg(unix)]
        {
            let alias = dir.path().join("alias");
            std::os::unix::fs::symlink(&profile, &alias).unwrap();
            assert!(ProfileOwnership::acquire(&alias, alias.join("control.sqlite")).is_err());
        }
        drop(owner);
        assert!(ProfileOwnership::acquire(&profile, &db).is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn rejected_database_aliases_create_no_external_target() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("owned");
        std::fs::create_dir(&profile).unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let target = outside.join("new.sqlite");
        let db = profile.join("control.sqlite");
        symlink(&target, &db).unwrap();
        assert!(ProfileOwnership::acquire(&profile, &db).is_err());
        assert!(!target.exists());
        assert!(!profile.join(".c3-profile.lock").exists());
        std::fs::remove_file(&db).unwrap();
        std::fs::write(&target, "preserve").unwrap();
        symlink(&target, &db).unwrap();
        assert!(ProfileOwnership::acquire(&profile, &db).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"preserve");
        std::fs::remove_file(&db).unwrap();
        std::fs::hard_link(&target, &db).unwrap();
        assert!(ProfileOwnership::acquire(&profile, &db).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"preserve");
        assert!(ProfileOwnership::acquire(
            &profile,
            outside.join("not-created").join("new.sqlite")
        )
        .is_err());
        assert!(!outside.join("not-created").exists());
    }
    #[test]
    fn owned_restart_retires_prior_runtime_without_claiming_cleanup_or_replay() {
        let (_dir, db, old_bus, id, old_scope) = fixture();
        let op = Uuid::new_v4();
        old_scope
            .emit_checked(ControlEvent::UserMessage {
                session_id: id,
                operation_id: op,
                text: "prior effect".into(),
                at: Utc::now(),
            })
            .unwrap();
        old_scope
            .emit_checked(ControlEvent::SessionStatusChanged {
                session_id: id,
                status: SessionStatus::Running,
                at: Utc::now(),
            })
            .unwrap();
        let new_bus = EventBus::new();
        new_bus.install_sink(db.clone()).unwrap();
        assert_eq!(new_bus.health().last_seq, old_bus.health().last_seq);
        let recovery = new_bus.begin_owned_lifetime().unwrap();
        assert!(recovery.projection.baseline_changed_all);
        assert_eq!(db.get_session(id).unwrap().status, "recovering");
        assert!(old_scope
            .emit_checked(message(id, "stale after crash observation"))
            .is_err());
        let snapshot = db.event_snapshot(Some(id), 100).unwrap();
        assert!(snapshot.operations[0].outcome_seq.is_none());
        assert_eq!(snapshot.operations[0].operation_id, op);
        let mut saved = rec(id, None);
        saved.status = "cancelled".into();
        new_bus
            .emit_checked(ControlEvent::SessionMetadataUpdated {
                session_id: id,
                metadata_json: serde_json::to_string(&saved).unwrap(),
                at: Utc::now(),
            })
            .unwrap();
        assert_eq!(db.get_session(id).unwrap().status, "recovering");
        assert!(new_bus.begin_owned_lifetime().is_err());
    }
    #[test]
    fn generated_stream_latency_and_heartbeat_probe() {
        let (_dir, db, _bus, id, scope, _) = owned_pair();
        let mut samples = Vec::with_capacity(1000);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_stop = stop.clone();
        let gaps = Arc::new(Mutex::new(Vec::new()));
        let worker_gaps = gaps.clone();
        let heartbeat = std::thread::spawn(move || {
            let mut previous = std::time::Instant::now();
            while !worker_stop.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(1));
                let now = std::time::Instant::now();
                worker_gaps
                    .lock()
                    .unwrap()
                    .push(now.duration_since(previous).as_micros());
                previous = now;
            }
        });
        for i in 0..1000 {
            let start = std::time::Instant::now();
            scope
                .emit_checked(message(id, "generated λ chunk "))
                .unwrap();
            samples.push(start.elapsed().as_micros());
            if i % 25 == 0 {
                let snapshot = db.event_snapshot(Some(id), 16).unwrap();
                if let Some(cursor) = snapshot.next {
                    db.release_snapshot(cursor);
                }
                let identity = db.store_identity();
                db.replay_events(
                    EventCursor {
                        store_id: identity.store_id,
                        generation: identity.generation,
                        after_seq: i as u64,
                    },
                    3,
                )
                .unwrap();
            }
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        heartbeat.join().unwrap();
        samples.sort_unstable();
        let gaps = gaps.lock().unwrap();
        let max_gap = gaps.iter().max().copied().unwrap_or(0);
        eprintln!("generated retained-writer commit latency µs p50={} p95={} p99={} max={} independent-thread heartbeat max_gap={} samples=1000",samples[500],samples[950],samples[990],samples[999],max_gap);
        assert_eq!(db.watermark().unwrap(), 1001);
        assert!(
            db.conn()
                .unwrap()
                .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
                .unwrap()
                == "wal"
        );
    }
    #[test]
    fn bounded_snapshot_cursor_recovers_more_than_512_sessions_and_rejects_reuse() {
        let (_dir, db, _bus, _, _) = fixture();
        let mut conn = db.conn().unwrap();
        let tx = conn.transaction().unwrap();
        for _ in 0..530 {
            let id = Uuid::new_v4();
            db.ensure_session_row(&tx, id).unwrap();
        }
        tx.commit().unwrap();
        let first = db.event_snapshot(None, 11).unwrap();
        let mut ids = first.sessions.into_iter().map(|s| s.id).collect::<Vec<_>>();
        let mut next = first.next;
        let used = next.unwrap();
        let mut final_cursor = None;
        while let Some(cursor) = next {
            final_cursor = Some(cursor);
            let page = db.event_snapshot_page(Some(cursor), None, 11).unwrap();
            ids.extend(page.sessions.into_iter().map(|s| s.id));
            next = page.next;
        }
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 531);
        assert!(db.event_snapshot_page(Some(used), None, 11).is_err());
        assert!(db.event_snapshot_page(final_cursor, None, 11).is_err());
        let identity = db.store_identity();
        assert!(db
            .replay_events(
                EventCursor {
                    store_id: Uuid::new_v4(),
                    generation: identity.generation,
                    after_seq: 0
                },
                1
            )
            .is_err());
    }
    #[test]
    fn ownership_lives_with_sink_after_outer_owner_is_dropped_and_reserved_keys_refuse() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("owned");
        let path = profile.join("control.sqlite");
        let owner = Arc::new(ProfileOwnership::acquire(&profile, &path).unwrap());
        let db = Arc::new(Persistence::open_owned(owner.clone()).unwrap());
        drop(owner);
        let bus = EventBus::new();
        bus.install_sink(db.clone()).unwrap();
        drop(db);
        assert!(ProfileOwnership::acquire(&profile, &path).is_err());
        bus.emit_checked(ControlEvent::StoreValueUpdated {
            key: "reviewed_build_concurrency".into(),
            value: "2".into(),
            at: Utc::now(),
        })
        .unwrap();
        assert!(matches!(
            bus.emit_checked(ControlEvent::StoreValueUpdated {
                key: "event_store_id".into(),
                value: Uuid::new_v4().to_string(),
                at: Utc::now()
            }),
            Err(EventError::InvalidOrigin(_))
        ));
        assert!(bus.health().healthy);
        drop(bus);
        assert!(ProfileOwnership::acquire(&profile, &path).is_ok());
    }
    #[test]
    fn synchronous_owner_non_yielding_tokio_burst_stall_is_measured_not_qualified() {
        let (_dir, db, _bus, id, scope, _) = owned_pair();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let observer=tokio::spawn(async {let start=std::time::Instant::now();tokio::time::sleep(std::time::Duration::from_millis(1)).await;start.elapsed()});
            tokio::task::yield_now().await;
            let start=std::time::Instant::now();for i in 0..1000 {scope.emit_checked(message(id,"single-worker generated chunk")).unwrap();if i%25==0 {db.event_snapshot(Some(id),16).unwrap();}}
            let burst=start.elapsed();let gap=observer.await.unwrap();eprintln!("non-yielding single-worker Tokio burst: events=1000 elapsed_ms={} heartbeat_delay_ms={} (sync checked owner; NOT frame-time qualification)",burst.as_millis(),gap.as_millis());
        });
    }
    #[test]
    fn physical_inode_namespace_blocks_nested_profile_and_allows_owned_sqlite_after_release() {
        let dir = tempfile::tempdir().unwrap();
        let outer = dir.path().join("outer");
        let inner = outer.join("nested");
        let path = inner.join("control.sqlite");
        let owner = Arc::new(ProfileOwnership::acquire(&outer, &path).unwrap());
        let db = Persistence::open_owned(owner.clone()).unwrap();
        db.set_kv("generated_parent_marker", "preserved").unwrap();
        let before = std::fs::read(&path).unwrap();
        let command = |expect: &str| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "journal::tests::ownership_child_probe",
                    "--nocapture",
                ])
                .env("C3_GENERATED_LOCK_PROFILE", &inner)
                .env("C3_GENERATED_LOCK_DB", &path)
                .env("C3_GENERATED_LOCK_EXPECT", expect)
                .output()
                .unwrap()
        };
        let denied = command("failure");
        assert!(
            denied.status.success(),
            "{}",
            String::from_utf8_lossy(&denied.stderr)
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(db.get_kv("generated_child_marker").unwrap().is_none());
        drop(db);
        drop(owner);
        let admitted = command("success");
        assert!(
            admitted.status.success(),
            "{}",
            String::from_utf8_lossy(&admitted.stderr)
        );
        let owner = Arc::new(ProfileOwnership::acquire(&outer, &path).unwrap());
        let reopened = Persistence::open_owned(owner).unwrap();
        assert_eq!(
            reopened
                .get_kv("generated_parent_marker")
                .unwrap()
                .as_deref(),
            Some("preserved")
        );
        assert_eq!(
            reopened
                .get_kv("generated_child_marker")
                .unwrap()
                .as_deref(),
            Some("opened-owned")
        );
    }
    #[test]
    fn captured_scope_and_lag_status_mirror_only_accept_current_committed_runtime() {
        let (_dir, _db, bus, id, scope) = fixture();
        assert_eq!(
            bus.current_status(id),
            Some((scope.runtime_id().unwrap(), SessionStatus::Starting))
        );
        let captured = bus.scope_for_origin(scope.origin()).unwrap();
        captured
            .emit_checked(ControlEvent::SessionStatusChanged {
                session_id: id,
                status: SessionStatus::Idle,
                at: Utc::now(),
            })
            .unwrap();
        assert_eq!(
            bus.current_status(id),
            Some((scope.runtime_id().unwrap(), SessionStatus::Idle))
        );
        let new = bus.register_runtime(id).unwrap();
        assert!(bus.scope_for_origin(scope.origin()).is_err());
        assert!(captured.ensure_healthy().is_err());
        assert_eq!(
            bus.current_status(id),
            Some((new.runtime_id().unwrap(), SessionStatus::Starting))
        );
        new.retire_runtime(new.runtime_id().unwrap()).unwrap();
        assert!(bus.current_status(id).is_none());
        assert!(bus.health().healthy);
    }
    #[cfg(unix)]
    #[test]
    fn sqlite_companion_aliases_are_refused_before_original_bytes_or_profile_mutate() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("owned");
        std::fs::create_dir(&profile).unwrap();
        let db = profile.join("control.sqlite");
        let outside = dir.path().join("outside");
        std::fs::write(&outside, b"original companion target").unwrap();
        let wal = profile.join("control.sqlite-wal");
        std::os::unix::fs::symlink(&outside, &wal).unwrap();
        assert!(ProfileOwnership::acquire(&profile, &db).is_err());
        assert_eq!(
            std::fs::read(&outside).unwrap(),
            b"original companion target"
        );
        assert!(!db.exists());
        assert!(!profile.join(".c3-profile.lock").exists());
        std::fs::remove_file(&wal).unwrap();
        std::fs::hard_link(&outside, &wal).unwrap();
        assert!(ProfileOwnership::acquire(&profile, &db).is_err());
        assert_eq!(
            std::fs::read(&outside).unwrap(),
            b"original companion target"
        );
    }
    #[test]
    fn tombstoned_host_outcome_matches_existing_intent_without_resurrecting() {
        let (_dir, db, bus, id, scope) = fixture();
        let op = Uuid::new_v4();
        scope
            .emit_checked(ControlEvent::HostOperationIntent {
                session_id: Some(id),
                operation_id: op,
                kind: "remove_session".into(),
                target: "generated thread".into(),
                at: Utc::now(),
            })
            .unwrap();
        scope.retire_runtime(scope.runtime_id().unwrap()).unwrap();
        bus.emit_checked(ControlEvent::SessionRemoved {
            session_id: id,
            at: Utc::now(),
        })
        .unwrap();
        let outcome = bus
            .emit_checked(ControlEvent::HostOperationOutcome {
                session_id: Some(id),
                operation_id: op,
                kind: "remove_session".into(),
                target: "generated thread".into(),
                result: "completed".into(),
                at: Utc::now(),
            })
            .unwrap();
        assert_eq!(
            outcome.projection.operations[0].result.as_deref(),
            Some("completed")
        );
        assert!(db.get_session(id).is_err());
        assert!(db.transcripts(id).unwrap().is_empty());
        assert!(bus.health().healthy);
        assert!(bus
            .emit_checked(ControlEvent::HostOperationOutcome {
                session_id: Some(id),
                operation_id: Uuid::new_v4(),
                kind: "remove_session".into(),
                target: "generated".into(),
                result: "completed".into(),
                at: Utc::now()
            })
            .is_err());
        assert!(db.get_session(id).is_err());
        assert!(bus.health().healthy);
    }
    #[test]
    fn mismatched_outcome_and_legacy_managed_writes_are_rejected_without_retcon() {
        let (_dir, db, bus, id, scope) = fixture();
        let op = Uuid::new_v4();
        scope
            .emit_checked(ControlEvent::HostOperationIntent {
                session_id: Some(id),
                operation_id: op,
                kind: "file_write".into(),
                target: "reviewed/file.rs".into(),
                at: Utc::now(),
            })
            .unwrap();
        assert!(matches!(
            scope.emit_checked(ControlEvent::HostOperationOutcome {
                session_id: Some(id),
                operation_id: op,
                kind: "file_write".into(),
                target: "unrelated/file.rs".into(),
                result: "completed".into(),
                at: Utc::now()
            }),
            Err(EventError::InvalidOrigin(_))
        ));
        let operation = db
            .event_snapshot(Some(id), 100)
            .unwrap()
            .operations
            .remove(0);
        assert_eq!(operation.target, "reviewed/file.rs");
        assert!(operation.outcome_seq.is_none());
        assert!(bus.health().healthy);
        assert!(db.update_session_status(id, "completed").is_err());
        assert!(db
            .append_message(id, "prompt", "bypass", Utc::now())
            .is_err());
        assert!(db
            .append_message_merged(id, "agent", "bypass", Utc::now(), 10)
            .is_err());
        assert!(db.import_conversation(id, &[]).is_err());
        assert!(db.delete_session(id).is_err());
        assert!(db.upsert_session(&rec(id, scope.runtime_id())).is_err());
        assert_eq!(db.get_session(id).unwrap().status, "starting");
        assert!(db.transcripts(id).unwrap().is_empty());
    }
    fn owned_pair() -> (
        tempfile::TempDir,
        Arc<Persistence>,
        Arc<EventBus>,
        Uuid,
        Arc<EventBus>,
        PathBuf,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("first");
        let owner =
            Arc::new(ProfileOwnership::acquire(&profile, profile.join("main.sqlite")).unwrap());
        let db = Arc::new(Persistence::open_owned(owner).unwrap());
        let bus = Arc::new(EventBus::new());
        bus.install_sink(db.clone()).unwrap();
        let id = Uuid::new_v4();
        let scope = bus.register_runtime(id).unwrap();
        let other = dir.path().join("second");
        let other_owner =
            Arc::new(ProfileOwnership::acquire(&other, other.join("main.sqlite")).unwrap());
        let other_db = Persistence::open_owned(other_owner).unwrap();
        other_db.set_kv("other_marker", "untouched").unwrap();
        let other_path = other_db.path().to_owned();
        drop(other_db);
        (dir, db, bus, id, scope, other_path)
    }
    #[test]
    fn replacement_before_owned_intent_commit_fences_effect_and_preserves_both_stores() {
        let (_dir, db, bus, id, scope, other) = owned_pair();
        let original = db.path().with_extension("preserved.sqlite");
        let old_bytes = std::fs::read(db.path()).unwrap();
        let other_bytes = std::fs::read(&other).unwrap();
        std::fs::rename(db.path(), &original).unwrap();
        std::fs::copy(&other, db.path()).unwrap();
        let replacement = std::fs::read(db.path()).unwrap();
        let mut raw = bus.subscribe();
        let mut committed = bus.subscribe_committed();
        let mut effects = 0;
        if scope
            .emit_checked(ControlEvent::HostOperationIntent {
                session_id: Some(id),
                operation_id: Uuid::new_v4(),
                kind: "native_spawn".into(),
                target: "generated".into(),
                at: Utc::now(),
            })
            .is_ok()
        {
            effects += 1;
        }
        assert_eq!(effects, 0);
        assert!(!bus.health().healthy);
        assert!(bus.ensure_healthy().is_err());
        assert!(raw.try_recv().is_err());
        assert!(committed.try_recv().is_err());
        assert_eq!(std::fs::read(&original).unwrap(), old_bytes);
        assert_eq!(std::fs::read(&other).unwrap(), other_bytes);
        assert_eq!(std::fs::read(db.path()).unwrap(), replacement);
    }
    #[test]
    fn replacement_after_commit_retains_old_receipt_but_refuses_effect_ack_and_notifications() {
        let (_dir, db, bus, id, scope, other) = owned_pair();
        let path = db.path().to_owned();
        let original = path.with_extension("preserved.sqlite");
        let other_bytes = std::fs::read(&other).unwrap();
        let callback_path = path.clone();
        let callback_original = original.clone();
        let callback_other = other.clone();
        *db.after_commit_test_hook.lock().unwrap() = Some(Box::new(move || {
            std::fs::rename(&callback_path, &callback_original).unwrap();
            std::fs::copy(callback_other, callback_path).unwrap();
        }));
        let mut raw = bus.subscribe();
        let mut committed = bus.subscribe_committed();
        let mut effects = 0;
        if scope
            .emit_checked(ControlEvent::HostOperationIntent {
                session_id: Some(id),
                operation_id: Uuid::new_v4(),
                kind: "native_spawn".into(),
                target: "generated".into(),
                at: Utc::now(),
            })
            .is_ok()
        {
            effects += 1;
        }
        assert_eq!(effects, 0);
        assert!(!bus.health().healthy);
        assert!(raw.try_recv().is_err());
        assert!(committed.try_recv().is_err());
        assert_eq!(std::fs::read(&other).unwrap(), other_bytes);
        assert_eq!(std::fs::read(&path).unwrap(), other_bytes);
        assert!(original.exists());
        let retained: i64 = db
            .writer
            .lock()
            .unwrap()
            .query_row("SELECT MAX(seq) FROM event_journal", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            retained, 2,
            "receipt committed to retained old inode; uncertainty is not rollback"
        );
    }
    #[test]
    fn retained_owned_writer_refuses_replaced_or_missing_wal_before_effect() {
        for replace in [false, true] {
            let (_dir, db, bus, id, scope, _) = owned_pair();
            let wal = companion_path(db.path(), "-wal");
            let preserved = wal.with_extension("original-wal");
            std::fs::rename(&wal, &preserved).unwrap();
            if replace {
                std::fs::write(&wal, b"unrelated WAL replacement").unwrap();
            }
            let mut committed = bus.subscribe_committed();
            assert!(scope
                .emit_checked(ControlEvent::HostOperationIntent {
                    session_id: Some(id),
                    operation_id: Uuid::new_v4(),
                    kind: "native_spawn".into(),
                    target: "generated".into(),
                    at: Utc::now()
                })
                .is_err());
            assert!(!bus.health().healthy);
            assert!(committed.try_recv().is_err());
            assert!(preserved.exists());
            if replace {
                assert_eq!(std::fs::read(&wal).unwrap(), b"unrelated WAL replacement");
            }
        }
    }
    #[test]
    fn accepted_native_coverage_failure_is_sticky_but_stale_producer_cannot_poison() {
        let (_dir, _db, bus, id, old) = fixture();
        let current = bus.register_runtime(id).unwrap();
        assert!(matches!(
            old.mark_coverage_failed("late obsolete frame"),
            Err(EventError::StaleRuntime)
        ));
        assert!(bus.health().healthy);
        current
            .mark_coverage_failed("accepted native frame could not be committed")
            .unwrap();
        assert!(!bus.health().healthy);
        assert!(current
            .emit_checked(message(id, "cannot fabricate later completion"))
            .is_err());
        assert!(bus.ensure_durable().is_err());
    }
}
