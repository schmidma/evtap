use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Row, Transaction, TransactionBehavior, params,
};
use serde::Deserialize;
use thiserror::Error;

use crate::{
    private_fs::{PrivatePathError, ensure_private_directory, set_private_permissions},
    session::{
        KeyboardContext, SessionId, SessionMetadata, SessionSnapshot, StoredMetricSnapshot,
        StoredSession,
    },
};

const APPLICATION_ID: i64 = 0x4556_5450; // "EVTP"
const DATABASE_SCHEMA_VERSION: i64 = 2;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_SESSION_METRICS: usize = 1_000;
const MAX_APPLICATION_VERSION_BYTES: usize = 128;
const MAX_SESSION_NAME_BYTES: usize = 80;
const MAX_KEYBOARD_NAME_BYTES: usize = 1_024;
const MAX_KEYBOARD_VALUE_BYTES: usize = 256;
const MAX_SESSION_LIST_SIZE: u32 = 10_000;

const SCHEMA_V2: &str = r#"
CREATE TABLE sessions (
    id                    INTEGER PRIMARY KEY,
    name                  TEXT,
    created_at_ms         INTEGER NOT NULL,
    updated_at_ms         INTEGER NOT NULL,
    last_opened_at_ms     INTEGER NOT NULL,
    captured_duration_ns  INTEGER NOT NULL DEFAULT 0
                             CHECK (captured_duration_ns >= 0),
    application_version   TEXT NOT NULL,
    keyboard_name         TEXT,
    xkb_model             TEXT NOT NULL,
    xkb_layout            TEXT NOT NULL,
    xkb_variant           TEXT NOT NULL,
    CHECK (name IS NULL OR length(name) > 0)
);

CREATE UNIQUE INDEX sessions_unique_name
    ON sessions(name)
    WHERE name IS NOT NULL;

CREATE INDEX sessions_recently_opened
    ON sessions(last_opened_at_ms DESC, id DESC);

CREATE TABLE metric_snapshots (
    session_id             INTEGER NOT NULL
                               REFERENCES sessions(id) ON DELETE CASCADE,
    metric_id              TEXT NOT NULL,
    metric_schema_version  INTEGER NOT NULL
                               CHECK (metric_schema_version > 0),
    payload_json           TEXT NOT NULL,
    updated_at_ms          INTEGER NOT NULL,
    PRIMARY KEY (session_id, metric_id)
) WITHOUT ROWID;
"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionListOrder {
    LastOpened,
    LastUpdated,
}

impl SessionListOrder {
    fn sql(self) -> &'static str {
        match self {
            Self::LastOpened => "last_opened_at_ms DESC, id DESC",
            Self::LastUpdated => "updated_at_ms DESC, id DESC",
        }
    }
}

pub struct Repository {
    connection: Connection,
    path: Option<PathBuf>,
}

impl Repository {
    pub fn open(path: PathBuf) -> Result<Self, RepositoryError> {
        prepare_database_path(&path)?;
        let connection = open_connection(&path, true)?;
        let mut repository = Self {
            connection,
            path: Some(path.clone()),
        };
        repository.configure(true)?;
        set_private_permissions(&path, 0o600)?;
        Ok(repository)
    }

    pub fn open_existing(path: PathBuf) -> Result<Option<Self>, RepositoryError> {
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Err(RepositoryError::NotRegularFile(path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(RepositoryError::ReadMetadata { path, source }),
        }
        let connection = open_connection(&path, false)?;
        let mut repository = Self {
            connection,
            path: Some(path.clone()),
        };
        repository.configure(true)?;
        set_private_permissions(&path, 0o600)?;
        Ok(Some(repository))
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, RepositoryError> {
        let connection = Connection::open_in_memory()?;
        let mut repository = Self {
            connection,
            path: None,
        };
        repository.configure(false)?;
        Ok(repository)
    }

    pub fn save(&mut self, snapshot: &SessionSnapshot) -> Result<SessionId, RepositoryError> {
        validate_snapshot(snapshot)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        ensure_unique_session_name(&transaction, snapshot.name.as_deref(), snapshot.id)?;

        let session_id = if let Some(session_id) = snapshot.id {
            let changed = transaction.execute(
                "UPDATE sessions SET
                    name = ?1,
                    created_at_ms = ?2,
                    updated_at_ms = ?3,
                    last_opened_at_ms = ?4,
                    captured_duration_ns = ?5,
                    application_version = ?6,
                    keyboard_name = ?7,
                    xkb_model = ?8,
                    xkb_layout = ?9,
                    xkb_variant = ?10
                 WHERE id = ?11",
                params![
                    snapshot.name,
                    snapshot.created_at_ms,
                    snapshot.updated_at_ms,
                    snapshot.last_opened_at_ms,
                    snapshot.captured_duration_ns,
                    snapshot.application_version,
                    snapshot.keyboard.display_name,
                    snapshot.keyboard.model,
                    snapshot.keyboard.layout,
                    snapshot.keyboard.variant,
                    session_id.get(),
                ],
            )?;
            if changed != 1 {
                return Err(RepositoryError::SessionNotFound(session_id.get()));
            }
            session_id
        } else {
            transaction.execute(
                "INSERT INTO sessions (
                    name, created_at_ms, updated_at_ms, last_opened_at_ms,
                    captured_duration_ns, application_version, keyboard_name,
                    xkb_model, xkb_layout, xkb_variant
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    snapshot.name,
                    snapshot.created_at_ms,
                    snapshot.updated_at_ms,
                    snapshot.last_opened_at_ms,
                    snapshot.captured_duration_ns,
                    snapshot.application_version,
                    snapshot.keyboard.display_name,
                    snapshot.keyboard.model,
                    snapshot.keyboard.layout,
                    snapshot.keyboard.variant,
                ],
            )?;
            SessionId::new(transaction.last_insert_rowid())
                .ok_or(RepositoryError::InvalidGeneratedSessionId)?
        };

        write_metrics(&transaction, session_id, snapshot)?;
        transaction.commit()?;
        Ok(session_id)
    }

    pub fn load_session(
        &self,
        session_id: SessionId,
    ) -> Result<Option<StoredSession>, RepositoryError> {
        load_session(&self.connection, session_id)
    }

    pub fn load_and_mark_opened(
        &mut self,
        session_id: SessionId,
        opened_at_ms: i64,
    ) -> Result<Option<StoredSession>, RepositoryError> {
        validate_timestamp(opened_at_ms)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE sessions SET last_opened_at_ms = ?1 WHERE id = ?2",
            params![opened_at_ms, session_id.get()],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        let session = load_session(&transaction, session_id)?;
        transaction.commit()?;
        Ok(session)
    }

    pub fn list_sessions(
        &self,
        limit: u32,
        order: SessionListOrder,
    ) -> Result<Vec<SessionMetadata>, RepositoryError> {
        let limit = limit.clamp(1, MAX_SESSION_LIST_SIZE);
        let mut statement = self.connection.prepare(&format!(
            "{} ORDER BY {} LIMIT ?1",
            session_metadata_query(),
            order.sql()
        ))?;
        let rows = statement.query_map([limit], raw_metadata_from_row)?;
        rows.map(|row| SessionMetadata::try_from(row?)).collect()
    }

    pub fn rename_session(
        &mut self,
        session_id: SessionId,
        name: Option<&str>,
        updated_at_ms: i64,
    ) -> Result<Option<SessionMetadata>, RepositoryError> {
        validate_session_name(name)?;
        validate_timestamp(updated_at_ms)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1)",
            [session_id.get()],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(None);
        }
        ensure_unique_session_name(&transaction, name, Some(session_id))?;

        let changed = transaction.execute(
            "UPDATE sessions SET name = ?1, updated_at_ms = ?2 WHERE id = ?3",
            params![name, updated_at_ms, session_id.get()],
        )?;
        if changed == 0 {
            return Ok(None);
        }

        let metadata = transaction.query_row(
            &format!("{} WHERE id = ?1", session_metadata_query()),
            [session_id.get()],
            raw_metadata_from_row,
        )?;
        let metadata = SessionMetadata::try_from(metadata)?;
        transaction.commit()?;
        Ok(Some(metadata))
    }

    pub fn delete_session(&mut self, session_id: SessionId) -> Result<bool, RepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let deleted =
            transaction.execute("DELETE FROM sessions WHERE id = ?1", [session_id.get()])?;
        transaction.commit()?;
        Ok(deleted == 1)
    }

    #[cfg(test)]
    pub fn delete_all_sessions(&mut self) -> Result<usize, RepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let deleted = transaction.execute("DELETE FROM sessions", [])?;
        transaction.commit()?;
        Ok(deleted)
    }

    pub fn reclaim_after_deletion(&self) -> Result<(), RepositoryError> {
        self.connection.execute_batch(
            "PRAGMA wal_checkpoint(TRUNCATE);
             VACUUM;
             PRAGMA optimize;",
        )?;
        Ok(())
    }

    fn configure(&mut self, persistent: bool) -> Result<(), RepositoryError> {
        self.connection.busy_timeout(BUSY_TIMEOUT)?;

        let application_id: i64 =
            self.connection
                .pragma_query_value(None, "application_id", |row| row.get(0))?;
        let schema_version: i64 =
            self.connection
                .pragma_query_value(None, "user_version", |row| row.get(0))?;
        let object_count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM sqlite_schema
             WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;

        if application_id == 0 && schema_version == 0 && object_count == 0 {
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(SCHEMA_V2)?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", DATABASE_SCHEMA_VERSION)?;
            transaction.commit()?;
        } else {
            if application_id != APPLICATION_ID {
                return Err(if application_id == 0 {
                    RepositoryError::UnidentifiedDatabase(self.display_path())
                } else {
                    RepositoryError::WrongApplication {
                        path: self.display_path(),
                        actual: application_id,
                    }
                });
            }
            if schema_version != DATABASE_SCHEMA_VERSION {
                return Err(RepositoryError::IncompatibleSchema {
                    path: self.display_path(),
                    actual: schema_version,
                    expected: DATABASE_SCHEMA_VERSION,
                });
            }
        }

        self.connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA secure_delete = ON;
             PRAGMA synchronous = NORMAL;",
        )?;
        if persistent {
            self.connection.pragma_update(None, "journal_mode", "WAL")?;
        }
        Ok(())
    }

    fn display_path(&self) -> PathBuf {
        self.path
            .clone()
            .unwrap_or_else(|| PathBuf::from("<in-memory database>"))
    }
}

fn ensure_unique_session_name(
    transaction: &Transaction<'_>,
    name: Option<&str>,
    session_id: Option<SessionId>,
) -> Result<(), RepositoryError> {
    let Some(name) = name else {
        return Ok(());
    };
    let duplicate: bool = transaction.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sessions
            WHERE name = ?1 AND (?2 IS NULL OR id != ?2)
        )",
        params![name, session_id.map(SessionId::get)],
        |row| row.get(0),
    )?;
    if duplicate {
        Err(RepositoryError::DuplicateSessionName)
    } else {
        Ok(())
    }
}

fn load_session(
    connection: &Connection,
    session_id: SessionId,
) -> Result<Option<StoredSession>, RepositoryError> {
    let metadata = connection
        .query_row(
            &format!("{} WHERE id = ?1", session_metadata_query()),
            [session_id.get()],
            raw_metadata_from_row,
        )
        .optional()?
        .map(SessionMetadata::try_from)
        .transpose()?;
    metadata
        .map(|metadata| {
            let metrics = metric_snapshots(connection, metadata.id)?;
            Ok(StoredSession { metadata, metrics })
        })
        .transpose()
}

fn metric_snapshots(
    connection: &Connection,
    session_id: SessionId,
) -> Result<Vec<StoredMetricSnapshot>, RepositoryError> {
    let mut statement = connection.prepare(
        "SELECT metric_id, metric_schema_version, payload_json
         FROM metric_snapshots
         WHERE session_id = ?1
         ORDER BY metric_id",
    )?;
    let rows = statement.query_map([session_id.get()], |row| {
        Ok(StoredMetricSnapshot {
            metric_id: row.get(0)?,
            schema_version: row.get(1)?,
            payload_json: row.get(2)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn open_connection(path: &Path, create: bool) -> Result<Connection, RepositoryError> {
    let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    if create {
        flags |= OpenFlags::SQLITE_OPEN_CREATE;
    }
    Connection::open_with_flags(path, flags).map_err(Into::into)
}

fn write_metrics(
    transaction: &Transaction<'_>,
    session_id: SessionId,
    snapshot: &SessionSnapshot,
) -> Result<(), RepositoryError> {
    let mut statement = transaction.prepare(
        "INSERT INTO metric_snapshots (
            session_id, metric_id, metric_schema_version, payload_json, updated_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(session_id, metric_id) DO UPDATE SET
            metric_schema_version = excluded.metric_schema_version,
            payload_json = excluded.payload_json,
            updated_at_ms = excluded.updated_at_ms",
    )?;
    for metric in &snapshot.metrics {
        statement.execute(params![
            session_id.get(),
            metric.metric_id(),
            metric.schema_version(),
            metric.payload_json(),
            snapshot.updated_at_ms,
        ])?;
    }
    Ok(())
}

fn validate_snapshot(snapshot: &SessionSnapshot) -> Result<(), RepositoryError> {
    validate_timestamp(snapshot.created_at_ms)?;
    validate_timestamp(snapshot.updated_at_ms)?;
    validate_timestamp(snapshot.last_opened_at_ms)?;
    if snapshot.captured_duration_ns < 0 {
        return Err(RepositoryError::InvalidDuration);
    }
    if snapshot.application_version.is_empty()
        || snapshot.application_version.len() > MAX_APPLICATION_VERSION_BYTES
    {
        return Err(RepositoryError::InvalidApplicationVersion);
    }
    validate_session_name(snapshot.name.as_deref())?;
    if snapshot
        .keyboard
        .display_name
        .as_ref()
        .is_some_and(|name| name.len() > MAX_KEYBOARD_NAME_BYTES)
        || [
            &snapshot.keyboard.model,
            &snapshot.keyboard.layout,
            &snapshot.keyboard.variant,
        ]
        .iter()
        .any(|value| value.len() > MAX_KEYBOARD_VALUE_BYTES)
    {
        return Err(RepositoryError::InvalidKeyboardContext);
    }
    if snapshot.metrics.len() > MAX_SESSION_METRICS {
        return Err(RepositoryError::TooManyMetrics);
    }
    let mut metric_ids = HashSet::with_capacity(snapshot.metrics.len());
    for metric in &snapshot.metrics {
        if !metric_ids.insert(metric.metric_id()) {
            return Err(RepositoryError::DuplicateMetric(
                metric.metric_id().to_owned(),
            ));
        }
        serde_json::from_str::<serde::de::IgnoredAny>(metric.payload_json()).map_err(|source| {
            RepositoryError::InvalidMetricJson {
                metric_id: metric.metric_id().to_owned(),
                source,
            }
        })?;
    }
    Ok(())
}

fn validate_session_name(name: Option<&str>) -> Result<(), RepositoryError> {
    if name.is_some_and(|name| {
        name.trim() != name || name.is_empty() || name.len() > MAX_SESSION_NAME_BYTES
    }) {
        Err(RepositoryError::InvalidSessionName)
    } else {
        Ok(())
    }
}

fn validate_timestamp(value: i64) -> Result<(), RepositoryError> {
    if value < 0 {
        Err(RepositoryError::InvalidTimestamp)
    } else {
        Ok(())
    }
}

fn session_metadata_query() -> &'static str {
    "SELECT
        id, name, created_at_ms, updated_at_ms, last_opened_at_ms,
        captured_duration_ns, application_version, keyboard_name,
        xkb_model, xkb_layout, xkb_variant
     FROM sessions"
}

#[derive(Deserialize)]
struct RawSessionMetadata {
    id: i64,
    name: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
    last_opened_at_ms: i64,
    captured_duration_ns: i64,
    application_version: String,
    keyboard_name: Option<String>,
    xkb_model: String,
    xkb_layout: String,
    xkb_variant: String,
}

fn raw_metadata_from_row(row: &Row<'_>) -> rusqlite::Result<RawSessionMetadata> {
    Ok(RawSessionMetadata {
        id: row.get(0)?,
        name: row.get(1)?,
        created_at_ms: row.get(2)?,
        updated_at_ms: row.get(3)?,
        last_opened_at_ms: row.get(4)?,
        captured_duration_ns: row.get(5)?,
        application_version: row.get(6)?,
        keyboard_name: row.get(7)?,
        xkb_model: row.get(8)?,
        xkb_layout: row.get(9)?,
        xkb_variant: row.get(10)?,
    })
}

impl TryFrom<RawSessionMetadata> for SessionMetadata {
    type Error = RepositoryError;

    fn try_from(raw: RawSessionMetadata) -> Result<Self, Self::Error> {
        let id = SessionId::new(raw.id).ok_or(RepositoryError::InvalidStoredSession)?;
        let metadata = Self {
            id,
            name: raw.name,
            created_at_ms: raw.created_at_ms,
            updated_at_ms: raw.updated_at_ms,
            last_opened_at_ms: raw.last_opened_at_ms,
            captured_duration_ns: raw.captured_duration_ns,
            application_version: raw.application_version,
            keyboard: KeyboardContext {
                display_name: raw.keyboard_name,
                model: raw.xkb_model,
                layout: raw.xkb_layout,
                variant: raw.xkb_variant,
            },
        };
        let validation_snapshot = SessionSnapshot {
            id: Some(metadata.id),
            name: metadata.name.clone(),
            created_at_ms: metadata.created_at_ms,
            updated_at_ms: metadata.updated_at_ms,
            last_opened_at_ms: metadata.last_opened_at_ms,
            captured_duration_ns: metadata.captured_duration_ns,
            application_version: metadata.application_version.clone(),
            keyboard: metadata.keyboard.clone(),
            metrics: Vec::new(),
        };
        validate_snapshot(&validation_snapshot)
            .map_err(|_| RepositoryError::InvalidStoredSession)?;
        Ok(metadata)
    }
}

fn prepare_database_path(path: &Path) -> Result<(), RepositoryError> {
    let parent = path.parent().ok_or(RepositoryError::MissingParent)?;
    ensure_private_directory(parent).map_err(Into::into)
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("database path has no parent directory")]
    MissingParent,
    #[error("database path is not a regular file: {0}")]
    NotRegularFile(PathBuf),
    #[error("failed to read database path metadata: {path}")]
    ReadMetadata {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    PrivatePath(#[from] PrivatePathError),
    #[error("nonempty database has no evtap application identity: {0}")]
    UnidentifiedDatabase(PathBuf),
    #[error("database belongs to another application ({actual:#x}): {path}")]
    WrongApplication { path: PathBuf, actual: i64 },
    #[error(
        "the evtap database at {path} uses incompatible schema {actual}; expected {expected}. Move or delete it to start fresh"
    )]
    IncompatibleSchema {
        path: PathBuf,
        actual: i64,
        expected: i64,
    },
    #[error("session name is invalid")]
    InvalidSessionName,
    #[error("another saved session already uses that name")]
    DuplicateSessionName,
    #[error("session timestamp is invalid")]
    InvalidTimestamp,
    #[error("captured duration is invalid")]
    InvalidDuration,
    #[error("application version is invalid")]
    InvalidApplicationVersion,
    #[error("keyboard context is invalid")]
    InvalidKeyboardContext,
    #[error("session has too many metrics")]
    TooManyMetrics,
    #[error("session contains duplicate metric {0}")]
    DuplicateMetric(String),
    #[error("metric {metric_id} contains invalid JSON")]
    InvalidMetricJson {
        metric_id: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("stored session metadata is invalid")]
    InvalidStoredSession,
    #[error("database generated an invalid session ID")]
    InvalidGeneratedSessionId,
    #[error("saved session {0} does not exist")]
    SessionNotFound(i64),
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt as _, symlink},
    };

    use rusqlite::Connection;
    use tempfile::tempdir;

    use crate::{
        metric::MetricSnapshot,
        session::{KeyboardContext, SessionId, SessionSnapshot},
    };

    use super::{
        APPLICATION_ID, DATABASE_SCHEMA_VERSION, Repository, RepositoryError, SessionListOrder,
    };

    fn snapshot(id: Option<SessionId>, name: Option<&str>, now: i64) -> SessionSnapshot {
        SessionSnapshot {
            id,
            name: name.map(str::to_owned),
            created_at_ms: 1,
            updated_at_ms: now,
            last_opened_at_ms: now,
            captured_duration_ns: 42,
            application_version: env!("CARGO_PKG_VERSION").to_owned(),
            keyboard: KeyboardContext {
                display_name: Some("Keyboard".to_owned()),
                model: "pc105".to_owned(),
                layout: "us".to_owned(),
                variant: String::new(),
            },
            metrics: vec![
                MetricSnapshot::from_json("total-presses", 1, r#"{"count":3}"#.to_owned()).unwrap(),
            ],
        }
    }

    #[test]
    fn creates_private_database_with_redesigned_schema() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("data/evtap/evtap.sqlite3");
        let repository = Repository::open(path.clone()).unwrap();

        assert!(
            repository
                .list_sessions(10, SessionListOrder::LastOpened)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let connection = Connection::open(path).unwrap();
        assert_eq!(
            connection
                .pragma_query_value(None, "application_id", |row| row.get::<_, i64>(0))
                .unwrap(),
            APPLICATION_ID
        );
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            DATABASE_SCHEMA_VERSION
        );
    }

    #[test]
    fn saves_lists_and_restores_multiple_mutable_sessions() {
        let mut repository = Repository::open_in_memory().unwrap();
        let home = repository.save(&snapshot(None, Some("Home"), 10)).unwrap();
        let work = repository.save(&snapshot(None, Some("Work"), 20)).unwrap();

        let mut changed = snapshot(Some(home), Some("Home"), 30);
        changed.metrics[0] =
            MetricSnapshot::from_json("total-presses", 1, r#"{"count":9}"#.to_owned()).unwrap();
        repository.save(&changed).unwrap();

        let sessions = repository
            .list_sessions(10, SessionListOrder::LastOpened)
            .unwrap();
        assert_eq!(
            sessions.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![home, work]
        );
        let restored = repository.load_session(home).unwrap().unwrap();
        assert_eq!(restored.metadata.name.as_deref(), Some("Home"));
        assert_eq!(restored.metrics[0].payload_json, r#"{"count":9}"#);
    }

    #[test]
    fn permits_untitled_sessions_but_rejects_duplicate_nonempty_names() {
        let mut repository = Repository::open_in_memory().unwrap();
        repository.save(&snapshot(None, None, 10)).unwrap();
        repository.save(&snapshot(None, None, 11)).unwrap();
        repository.save(&snapshot(None, Some("Home"), 12)).unwrap();

        assert!(matches!(
            repository.save(&snapshot(None, Some("Home"), 13)),
            Err(RepositoryError::DuplicateSessionName)
        ));
        assert_eq!(
            repository
                .list_sessions(10, SessionListOrder::LastOpened)
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn unloaded_rename_preserves_open_time_and_supports_distinct_list_orders() {
        let mut repository = Repository::open_in_memory().unwrap();
        let home = repository.save(&snapshot(None, Some("Home"), 10)).unwrap();
        let work = repository.save(&snapshot(None, Some("Work"), 20)).unwrap();
        repository.load_and_mark_opened(home, 30).unwrap();

        let renamed = repository
            .rename_session(work, Some("Project"), 40)
            .unwrap()
            .unwrap();
        assert_eq!(renamed.name.as_deref(), Some("Project"));
        assert_eq!(renamed.created_at_ms, 1);
        assert_eq!(renamed.updated_at_ms, 40);
        assert_eq!(renamed.last_opened_at_ms, 20);
        assert_eq!(renamed.captured_duration_ns, 42);
        assert_eq!(renamed.application_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(renamed.keyboard.display_name.as_deref(), Some("Keyboard"));
        assert_eq!(renamed.keyboard.model, "pc105");
        assert_eq!(renamed.keyboard.layout, "us");
        assert_eq!(renamed.keyboard.variant, "");
        assert_eq!(
            repository.load_session(work).unwrap().unwrap().metrics[0].payload_json,
            r#"{"count":3}"#
        );

        let opened = repository
            .list_sessions(10, SessionListOrder::LastOpened)
            .unwrap();
        assert_eq!(
            opened.iter().map(|session| session.id).collect::<Vec<_>>(),
            vec![home, work]
        );
        let updated = repository
            .list_sessions(10, SessionListOrder::LastUpdated)
            .unwrap();
        assert_eq!(
            updated.iter().map(|session| session.id).collect::<Vec<_>>(),
            vec![work, home]
        );
        assert_eq!(
            repository
                .list_sessions(0, SessionListOrder::LastUpdated)
                .unwrap()
                .len(),
            1
        );

        assert!(repository.rename_session(work, None, 50).unwrap().is_some());
        assert!(
            repository
                .rename_session(SessionId::new(999).unwrap(), Some("Missing"), 60)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn session_list_order_ties_use_the_newest_session_id() {
        let mut repository = Repository::open_in_memory().unwrap();
        let first = repository.save(&snapshot(None, Some("First"), 10)).unwrap();
        let second = repository
            .save(&snapshot(None, Some("Second"), 10))
            .unwrap();

        for order in [SessionListOrder::LastOpened, SessionListOrder::LastUpdated] {
            let sessions = repository.list_sessions(10, order).unwrap();
            assert_eq!(
                sessions
                    .into_iter()
                    .map(|session| session.id)
                    .collect::<Vec<_>>(),
                vec![second, first]
            );
        }
    }

    #[test]
    fn session_lists_remain_capped_at_ten_thousand() {
        let mut repository = Repository::open_in_memory().unwrap();
        let transaction = repository.connection.transaction().unwrap();
        {
            let mut statement = transaction
                .prepare(
                    "INSERT INTO sessions (
                        id, name, created_at_ms, updated_at_ms, last_opened_at_ms,
                        captured_duration_ns, application_version, keyboard_name,
                        xkb_model, xkb_layout, xkb_variant
                    ) VALUES (?1, NULL, 1, ?1, ?1, 0, 'test', NULL, '', '', '')",
                )
                .unwrap();
            for id in 1..=10_001_i64 {
                statement.execute([id]).unwrap();
            }
        }
        transaction.commit().unwrap();

        let sessions = repository
            .list_sessions(u32::MAX, SessionListOrder::LastUpdated)
            .unwrap();
        assert_eq!(sessions.len(), 10_000);
        assert_eq!(sessions.first().unwrap().id.get(), 10_001);
        assert_eq!(sessions.last().unwrap().id.get(), 2);
    }

    #[test]
    fn unloaded_rename_validates_names_and_rolls_back_conflicts() {
        let mut repository = Repository::open_in_memory().unwrap();
        let first = repository.save(&snapshot(None, Some("First"), 10)).unwrap();
        repository
            .save(&snapshot(None, Some("Reserved"), 20))
            .unwrap();
        let eighty_bytes = "é".repeat(40);

        let renamed = repository
            .rename_session(first, Some(&eighty_bytes), 30)
            .unwrap()
            .unwrap();
        assert_eq!(renamed.name.as_deref(), Some(eighty_bytes.as_str()));
        assert!(matches!(
            repository.rename_session(first, Some(&"é".repeat(41)), 40),
            Err(RepositoryError::InvalidSessionName)
        ));
        for invalid in ["", " leading", "trailing ", &"x".repeat(81)] {
            assert!(matches!(
                repository.rename_session(first, Some(invalid), 40),
                Err(RepositoryError::InvalidSessionName)
            ));
        }
        assert!(matches!(
            repository.rename_session(first, Some("Reserved"), 40),
            Err(RepositoryError::DuplicateSessionName)
        ));
        assert!(
            repository
                .rename_session(SessionId::new(999).unwrap(), Some("Reserved"), 40,)
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            repository.rename_session(first, Some("Valid"), -1),
            Err(RepositoryError::InvalidTimestamp)
        ));

        let unchanged = repository.load_session(first).unwrap().unwrap().metadata;
        assert_eq!(unchanged.name.as_deref(), Some(eighty_bytes.as_str()));
        assert_eq!(unchanged.updated_at_ms, 30);
        assert_eq!(unchanged.last_opened_at_ms, 10);
    }

    #[test]
    fn preserves_unknown_metrics_during_later_saves() {
        let mut repository = Repository::open_in_memory().unwrap();
        let id = repository.save(&snapshot(None, None, 10)).unwrap();
        repository
            .connection
            .execute(
                "INSERT INTO metric_snapshots VALUES (?1, 'future', 1, '{}', 10)",
                [id.get()],
            )
            .unwrap();

        repository.save(&snapshot(Some(id), None, 20)).unwrap();

        let restored = repository.load_session(id).unwrap().unwrap();
        assert!(
            restored
                .metrics
                .iter()
                .any(|metric| metric.metric_id == "future")
        );
    }

    #[test]
    fn validation_failure_does_not_partially_update() {
        let mut repository = Repository::open_in_memory().unwrap();
        let id = repository
            .save(&snapshot(None, Some("Before"), 10))
            .unwrap();
        let mut invalid = snapshot(Some(id), Some("After"), 20);
        invalid.metrics.push(invalid.metrics[0].clone());

        assert!(matches!(
            repository.save(&invalid),
            Err(RepositoryError::DuplicateMetric(_))
        ));
        assert_eq!(
            repository
                .load_session(id)
                .unwrap()
                .unwrap()
                .metadata
                .name
                .as_deref(),
            Some("Before")
        );
    }

    #[test]
    fn sqlite_failure_rolls_back_metadata_and_all_metric_changes() {
        let mut repository = Repository::open_in_memory().unwrap();
        let id = repository
            .save(&snapshot(None, Some("Before"), 10))
            .unwrap();
        repository
            .connection
            .execute_batch(
                "CREATE TRIGGER fail_metric_update
                 BEFORE UPDATE ON metric_snapshots
                 BEGIN
                     SELECT RAISE(FAIL, 'injected metric failure');
                 END;",
            )
            .unwrap();
        let mut changed = snapshot(Some(id), Some("After"), 20);
        changed.metrics[0] =
            MetricSnapshot::from_json("total-presses", 1, r#"{"count":99}"#.to_owned()).unwrap();

        assert!(repository.save(&changed).is_err());

        let restored = repository.load_session(id).unwrap().unwrap();
        assert_eq!(restored.metadata.name.as_deref(), Some("Before"));
        assert_eq!(restored.metadata.updated_at_ms, 10);
        assert_eq!(restored.metrics[0].payload_json, r#"{"count":3}"#);
    }

    #[test]
    fn saves_preserve_open_time_and_loading_updates_it_atomically() {
        let mut repository = Repository::open_in_memory().unwrap();
        let id = repository.save(&snapshot(None, Some("Home"), 10)).unwrap();

        let mut changed = snapshot(Some(id), Some("Home"), 20);
        changed.last_opened_at_ms = 10;
        repository.save(&changed).unwrap();

        let saved = repository.load_session(id).unwrap().unwrap();
        assert_eq!(saved.metadata.updated_at_ms, 20);
        assert_eq!(saved.metadata.last_opened_at_ms, 10);

        let opened = repository.load_and_mark_opened(id, 30).unwrap().unwrap();
        assert_eq!(opened.metadata.updated_at_ms, 20);
        assert_eq!(opened.metadata.last_opened_at_ms, 30);
        assert!(
            repository
                .load_and_mark_opened(SessionId::new(999).unwrap(), 40)
                .unwrap()
                .is_none()
        );

        repository
            .connection
            .execute(
                "UPDATE sessions SET created_at_ms = -1 WHERE id = ?1",
                [id.get()],
            )
            .unwrap();
        assert!(matches!(
            repository.load_and_mark_opened(id, 40),
            Err(RepositoryError::InvalidStoredSession)
        ));
        let last_opened_at_ms: i64 = repository
            .connection
            .query_row(
                "SELECT last_opened_at_ms FROM sessions WHERE id = ?1",
                [id.get()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(last_opened_at_ms, 30);
    }

    #[test]
    fn deletes_sessions_transactionally() {
        let mut repository = Repository::open_in_memory().unwrap();
        let first = repository.save(&snapshot(None, Some("One"), 10)).unwrap();
        repository.save(&snapshot(None, Some("Two"), 20)).unwrap();

        assert!(repository.delete_session(first).unwrap());
        assert_eq!(
            repository
                .list_sessions(10, SessionListOrder::LastOpened)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(repository.delete_all_sessions().unwrap(), 1);
        assert!(
            repository
                .list_sessions(10, SessionListOrder::LastOpened)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn rejects_any_incompatible_schema_generically_without_modifying_it() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("evtap.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "application_id", APPLICATION_ID)
            .unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        connection
            .execute("CREATE TABLE old_experiment (value)", [])
            .unwrap();
        drop(connection);
        let original = fs::read(&path).unwrap();

        assert!(matches!(
            Repository::open_existing(path.clone()),
            Err(RepositoryError::IncompatibleSchema { actual: 1, .. })
        ));
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn rejects_foreign_future_and_unidentified_databases_non_destructively() {
        let temporary = tempdir().unwrap();
        for (name, application_id, version) in [
            ("foreign.sqlite3", 7, DATABASE_SCHEMA_VERSION),
            (
                "future.sqlite3",
                APPLICATION_ID,
                DATABASE_SCHEMA_VERSION + 1,
            ),
            ("unidentified.sqlite3", 0, 0),
        ] {
            let path = temporary.path().join(name);
            let connection = Connection::open(&path).unwrap();
            connection
                .execute("CREATE TABLE existing_data (value)", [])
                .unwrap();
            connection
                .pragma_update(None, "application_id", application_id)
                .unwrap();
            connection
                .pragma_update(None, "user_version", version)
                .unwrap();
            drop(connection);
            let original = fs::read(&path).unwrap();

            assert!(Repository::open_existing(path.clone()).is_err());
            assert_eq!(fs::read(path).unwrap(), original);
        }
    }

    #[test]
    fn allows_a_symlinked_database_path() {
        let temporary = tempdir().unwrap();
        let target = temporary.path().join("target.sqlite3");
        let link = temporary.path().join("evtap.sqlite3");
        drop(Repository::open(target.clone()).unwrap());
        symlink(&target, &link).unwrap();

        assert!(Repository::open_existing(link).unwrap().is_some());
    }
}
