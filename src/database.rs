#![allow(
    dead_code,
    reason = "database repository is consumed by the storage worker milestone"
)]

use std::{
    collections::HashSet,
    fs::{self, DirBuilder, OpenOptions},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Row, Transaction, TransactionBehavior, params,
};
use serde::Deserialize;
use thiserror::Error;

use crate::{
    session::{
        KeyboardContext, SessionId, SessionMetadata, SessionSnapshot, SessionStatus,
        SessionSummary, StoredMetricSnapshot, StoredSession,
    },
    settings::RetentionPolicy,
};

const APPLICATION_ID: i64 = 0x4556_5450; // "EVTP"
const DATABASE_SCHEMA_VERSION: i64 = 1;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_SESSION_METRICS: usize = 1_000;
const MAX_APPLICATION_VERSION_BYTES: usize = 128;
const MAX_KEYBOARD_NAME_BYTES: usize = 1_024;
const MAX_KEYBOARD_VALUE_BYTES: usize = 256;
const MAX_HISTORY_PAGE_SIZE: u32 = 100;
const MILLISECONDS_PER_DAY: i64 = 86_400_000;

const SCHEMA_V1: &str = r#"
CREATE TABLE sessions (
    id                    INTEGER PRIMARY KEY,
    status                TEXT NOT NULL CHECK (status IN ('active', 'completed')),
    created_at_ms         INTEGER NOT NULL,
    updated_at_ms         INTEGER NOT NULL,
    completed_at_ms       INTEGER,
    captured_duration_ns  INTEGER NOT NULL DEFAULT 0
                             CHECK (captured_duration_ns >= 0),
    application_version   TEXT NOT NULL,
    keyboard_name         TEXT,
    xkb_model             TEXT NOT NULL,
    xkb_layout            TEXT NOT NULL,
    xkb_variant           TEXT NOT NULL,
    CHECK (
        (status = 'active' AND completed_at_ms IS NULL) OR
        (status = 'completed' AND completed_at_ms IS NOT NULL)
    )
);

CREATE UNIQUE INDEX sessions_one_active
    ON sessions(status)
    WHERE status = 'active';

CREATE INDEX sessions_completed_at
    ON sessions(completed_at_ms DESC)
    WHERE status = 'completed';

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

PRAGMA user_version = 1;
"#;

pub struct Repository {
    connection: Connection,
    path: Option<PathBuf>,
}

impl Repository {
    pub fn open(path: PathBuf) -> Result<Self, RepositoryError> {
        prepare_database_path(&path)?;
        let connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        set_private_permissions(&path, 0o600)?;

        let mut repository = Self {
            connection,
            path: Some(path),
        };
        repository.configure(true)?;
        Ok(repository)
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

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn checkpoint(&mut self, snapshot: &SessionSnapshot) -> Result<SessionId, RepositoryError> {
        validate_snapshot(snapshot)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let session_id = match snapshot.id {
            Some(session_id) => {
                let changed = transaction.execute(
                    "UPDATE sessions SET
                        created_at_ms = ?2,
                        updated_at_ms = ?3,
                        captured_duration_ns = ?4,
                        application_version = ?5,
                        keyboard_name = ?6,
                        xkb_model = ?7,
                        xkb_layout = ?8,
                        xkb_variant = ?9
                     WHERE id = ?1 AND status = 'active'",
                    params![
                        session_id.get(),
                        snapshot.created_at_ms,
                        snapshot.updated_at_ms,
                        snapshot.captured_duration_ns,
                        snapshot.application_version,
                        snapshot.keyboard.display_name,
                        snapshot.keyboard.model,
                        snapshot.keyboard.layout,
                        snapshot.keyboard.variant,
                    ],
                )?;
                if changed != 1 {
                    return Err(RepositoryError::SessionNotActive(session_id));
                }
                session_id
            }
            None => {
                transaction.execute(
                    "INSERT INTO sessions (
                        status, created_at_ms, updated_at_ms, completed_at_ms,
                        captured_duration_ns, application_version, keyboard_name,
                        xkb_model, xkb_layout, xkb_variant
                     ) VALUES ('active', ?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        snapshot.created_at_ms,
                        snapshot.updated_at_ms,
                        snapshot.captured_duration_ns,
                        snapshot.application_version,
                        snapshot.keyboard.display_name,
                        snapshot.keyboard.model,
                        snapshot.keyboard.layout,
                        snapshot.keyboard.variant,
                    ],
                )?;
                SessionId::new(transaction.last_insert_rowid()).ok_or(
                    RepositoryError::InvalidStoredSession("invalid inserted session ID"),
                )?
            }
        };
        write_metrics(&transaction, session_id, snapshot)?;
        transaction.commit()?;
        Ok(session_id)
    }

    pub fn finalize(
        &mut self,
        snapshot: &SessionSnapshot,
        completed_at_ms: i64,
    ) -> Result<SessionId, RepositoryError> {
        validate_snapshot(snapshot)?;
        if completed_at_ms < snapshot.updated_at_ms {
            return Err(RepositoryError::InvalidSession(
                "completion time precedes the latest update",
            ));
        }
        let session_id = snapshot.id.ok_or(RepositoryError::InvalidSession(
            "cannot finalize a session that has not been checkpointed",
        ))?;

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE sessions SET
                status = 'completed',
                created_at_ms = ?2,
                updated_at_ms = ?3,
                completed_at_ms = ?4,
                captured_duration_ns = ?5,
                application_version = ?6,
                keyboard_name = ?7,
                xkb_model = ?8,
                xkb_layout = ?9,
                xkb_variant = ?10
             WHERE id = ?1 AND status = 'active'",
            params![
                session_id.get(),
                snapshot.created_at_ms,
                snapshot.updated_at_ms,
                completed_at_ms,
                snapshot.captured_duration_ns,
                snapshot.application_version,
                snapshot.keyboard.display_name,
                snapshot.keyboard.model,
                snapshot.keyboard.layout,
                snapshot.keyboard.variant,
            ],
        )?;
        if changed != 1 {
            return Err(RepositoryError::SessionNotActive(session_id));
        }
        write_metrics(&transaction, session_id, snapshot)?;
        transaction.commit()?;
        Ok(session_id)
    }

    pub fn load_active(&self) -> Result<Option<StoredSession>, RepositoryError> {
        let metadata = self
            .connection
            .query_row(
                &format!(
                    "{} WHERE status = 'active' LIMIT 1",
                    session_metadata_query()
                ),
                [],
                raw_metadata_from_row,
            )
            .optional()?
            .map(SessionMetadata::try_from)
            .transpose()?;
        metadata
            .map(|metadata| self.load_stored_session(metadata))
            .transpose()
    }

    pub fn load_session(
        &self,
        session_id: SessionId,
    ) -> Result<Option<StoredSession>, RepositoryError> {
        let metadata = self
            .connection
            .query_row(
                &format!("{} WHERE id = ?1", session_metadata_query()),
                [session_id.get()],
                raw_metadata_from_row,
            )
            .optional()?
            .map(SessionMetadata::try_from)
            .transpose()?;
        metadata
            .map(|metadata| self.load_stored_session(metadata))
            .transpose()
    }

    pub fn list_completed(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SessionSummary>, RepositoryError> {
        if limit == 0 || limit > MAX_HISTORY_PAGE_SIZE {
            return Err(RepositoryError::InvalidPageSize(limit));
        }
        let mut statement = self.connection.prepare(&format!(
            "{} WHERE status = 'completed'
             ORDER BY completed_at_ms DESC, id DESC LIMIT ?1 OFFSET ?2",
            session_metadata_query()
        ))?;
        let rows = statement.query_map(params![limit, offset], raw_metadata_from_row)?;
        let raw_metadata: Vec<_> = rows.collect::<Result<_, _>>()?;
        drop(statement);

        raw_metadata
            .into_iter()
            .map(|raw| {
                let metadata = SessionMetadata::try_from(raw)?;
                let total_presses = self.total_presses(metadata.id)?;
                Ok(SessionSummary {
                    metadata,
                    total_presses,
                })
            })
            .collect()
    }

    pub fn delete_completed(&mut self, session_id: SessionId) -> Result<bool, RepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "DELETE FROM sessions WHERE id = ?1 AND status = 'completed'",
            [session_id.get()],
        )?;
        transaction.commit()?;
        Ok(changed == 1)
    }

    pub fn discard_active(&mut self, session_id: SessionId) -> Result<bool, RepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "DELETE FROM sessions WHERE id = ?1 AND status = 'active'",
            [session_id.get()],
        )?;
        transaction.commit()?;
        Ok(changed == 1)
    }

    pub fn apply_retention(
        &mut self,
        now_ms: i64,
        retention: RetentionPolicy,
    ) -> Result<usize, RepositoryError> {
        let RetentionPolicy::Days(days @ (30 | 90 | 365)) = retention else {
            return match retention {
                RetentionPolicy::Forever => Ok(0),
                RetentionPolicy::Days(_) => Err(RepositoryError::InvalidRetention),
            };
        };
        let retention_ms = i64::from(days)
            .checked_mul(MILLISECONDS_PER_DAY)
            .ok_or(RepositoryError::InvalidRetention)?;
        let cutoff = now_ms
            .checked_sub(retention_ms)
            .ok_or(RepositoryError::InvalidRetention)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "DELETE FROM sessions
             WHERE status = 'completed' AND completed_at_ms < ?1",
            [cutoff],
        )?;
        transaction.commit()?;
        Ok(changed)
    }

    pub fn clear_all(&mut self) -> Result<usize, RepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute("DELETE FROM sessions", [])?;
        transaction.commit()?;
        Ok(changed)
    }

    fn configure(&mut self, persistent: bool) -> Result<(), RepositoryError> {
        self.connection.busy_timeout(BUSY_TIMEOUT)?;

        let application_id: i64 =
            self.connection
                .pragma_query_value(None, "application_id", |row| row.get(0))?;
        if application_id != 0 && application_id != APPLICATION_ID {
            return Err(RepositoryError::WrongApplicationId { application_id });
        }
        let schema_version: i64 =
            self.connection
                .pragma_query_value(None, "user_version", |row| row.get(0))?;
        if schema_version > DATABASE_SCHEMA_VERSION {
            return Err(RepositoryError::NewerSchemaVersion {
                supported: DATABASE_SCHEMA_VERSION,
                actual: schema_version,
            });
        }
        if application_id == 0 {
            let schema_objects: i64 = self.connection.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )?;
            if schema_objects != 0 {
                return Err(RepositoryError::UnidentifiedNonemptyDatabase);
            }
            self.connection
                .pragma_update(None, "application_id", APPLICATION_ID)?;
        }

        self.connection.pragma_update(None, "foreign_keys", "ON")?;
        self.connection.pragma_update(None, "secure_delete", "ON")?;
        self.connection
            .pragma_update(None, "synchronous", "NORMAL")?;
        if persistent {
            let journal_mode: String =
                self.connection
                    .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
            if !journal_mode.eq_ignore_ascii_case("wal") {
                return Err(RepositoryError::UnexpectedJournalMode(journal_mode));
            }
        }

        if schema_version == 0 {
            self.connection
                .pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(SCHEMA_V1)?;
            transaction.commit()?;
        }
        Ok(())
    }

    fn load_stored_session(
        &self,
        metadata: SessionMetadata,
    ) -> Result<StoredSession, RepositoryError> {
        let metrics = self.metric_snapshots(metadata.id)?;
        Ok(StoredSession { metadata, metrics })
    }

    fn metric_snapshots(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<StoredMetricSnapshot>, RepositoryError> {
        let mut statement = self.connection.prepare(
            "SELECT metric_id, metric_schema_version, payload_json
             FROM metric_snapshots WHERE session_id = ?1 ORDER BY metric_id",
        )?;
        let rows = statement.query_map([session_id.get()], |row| {
            Ok(StoredMetricSnapshot {
                metric_id: row.get(0)?,
                schema_version: row.get(1)?,
                payload_json: row.get(2)?,
            })
        })?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    fn total_presses(&self, session_id: SessionId) -> Result<Option<u64>, RepositoryError> {
        let stored: Option<(i64, String)> = self
            .connection
            .query_row(
                "SELECT metric_schema_version, payload_json FROM metric_snapshots
                 WHERE session_id = ?1 AND metric_id = 'total-presses'",
                [session_id.get()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(stored
            .filter(|(schema_version, _)| *schema_version == 1)
            .and_then(|(_, payload)| serde_json::from_str::<TotalPressesPayload>(&payload).ok())
            .map(|payload| payload.count))
    }
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
    if snapshot.created_at_ms < 0 {
        return Err(RepositoryError::InvalidSession(
            "creation time must not precede the Unix epoch",
        ));
    }
    if snapshot.updated_at_ms < snapshot.created_at_ms {
        return Err(RepositoryError::InvalidSession(
            "update time precedes creation time",
        ));
    }
    if snapshot.captured_duration_ns < 0 {
        return Err(RepositoryError::InvalidSession(
            "capture duration must not be negative",
        ));
    }
    if snapshot.application_version.is_empty()
        || snapshot.application_version.len() > MAX_APPLICATION_VERSION_BYTES
    {
        return Err(RepositoryError::InvalidSession(
            "application version is empty or too long",
        ));
    }
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
        .into_iter()
        .any(|value| value.len() > MAX_KEYBOARD_VALUE_BYTES)
    {
        return Err(RepositoryError::InvalidSession(
            "keyboard metadata is too long",
        ));
    }
    if snapshot.metrics.len() > MAX_SESSION_METRICS {
        return Err(RepositoryError::InvalidSession(
            "session contains too many metrics",
        ));
    }
    let mut metric_ids = HashSet::with_capacity(snapshot.metrics.len());
    if snapshot
        .metrics
        .iter()
        .any(|metric| !metric_ids.insert(metric.metric_id()))
    {
        return Err(RepositoryError::InvalidSession(
            "session contains duplicate metric IDs",
        ));
    }
    Ok(())
}

fn session_metadata_query() -> &'static str {
    "SELECT
        id, status, created_at_ms, updated_at_ms, completed_at_ms,
        captured_duration_ns, application_version, keyboard_name,
        xkb_model, xkb_layout, xkb_variant
     FROM sessions"
}

#[derive(Debug)]
struct RawSessionMetadata {
    id: i64,
    status: String,
    created_at_ms: i64,
    updated_at_ms: i64,
    completed_at_ms: Option<i64>,
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
        status: row.get(1)?,
        created_at_ms: row.get(2)?,
        updated_at_ms: row.get(3)?,
        completed_at_ms: row.get(4)?,
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
        let id = SessionId::new(raw.id)
            .ok_or(RepositoryError::InvalidStoredSession("invalid session ID"))?;
        let status = match raw.status.as_str() {
            "active" => SessionStatus::Active,
            "completed" => SessionStatus::Completed,
            _ => {
                return Err(RepositoryError::InvalidStoredSession(
                    "invalid session status",
                ));
            }
        };
        let completion_is_invalid = match (status, raw.completed_at_ms) {
            (SessionStatus::Active, Some(_)) | (SessionStatus::Completed, None) => true,
            (SessionStatus::Completed, Some(completed_at_ms)) => {
                completed_at_ms < raw.updated_at_ms
            }
            (SessionStatus::Active, None) => false,
        };
        let keyboard_is_invalid = raw
            .keyboard_name
            .as_ref()
            .is_some_and(|name| name.len() > MAX_KEYBOARD_NAME_BYTES)
            || [&raw.xkb_model, &raw.xkb_layout, &raw.xkb_variant]
                .into_iter()
                .any(|value| value.len() > MAX_KEYBOARD_VALUE_BYTES);
        if raw.created_at_ms < 0
            || raw.updated_at_ms < raw.created_at_ms
            || raw.captured_duration_ns < 0
            || raw.application_version.is_empty()
            || raw.application_version.len() > MAX_APPLICATION_VERSION_BYTES
            || completion_is_invalid
            || keyboard_is_invalid
        {
            return Err(RepositoryError::InvalidStoredSession(
                "invalid session metadata",
            ));
        }
        Ok(Self {
            id,
            status,
            created_at_ms: raw.created_at_ms,
            updated_at_ms: raw.updated_at_ms,
            completed_at_ms: raw.completed_at_ms,
            captured_duration_ns: raw.captured_duration_ns,
            application_version: raw.application_version,
            keyboard: KeyboardContext {
                display_name: raw.keyboard_name,
                model: raw.xkb_model,
                layout: raw.xkb_layout,
                variant: raw.xkb_variant,
            },
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TotalPressesPayload {
    count: u64,
}

fn prepare_database_path(path: &Path) -> Result<(), RepositoryError> {
    let parent = path.parent().ok_or(RepositoryError::MissingParent)?;
    ensure_private_directory(parent)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(RepositoryError::UnsafeDatabasePath);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .map_err(|source| RepositoryError::CreateDatabase { source })?;
        }
        Err(source) => return Err(RepositoryError::ReadMetadata { source }),
    }
    set_private_permissions(path, 0o600)
}

fn ensure_private_directory(path: &Path) -> Result<(), RepositoryError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(RepositoryError::UnsafeDatabaseDirectory);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder
                .create(path)
                .map_err(|source| RepositoryError::CreateDirectory { source })?;
        }
        Err(source) => return Err(RepositoryError::ReadMetadata { source }),
    }
    set_private_permissions(path, 0o700)
}

fn set_private_permissions(path: &Path, mode: u32) -> Result<(), RepositoryError> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|source| RepositoryError::SetPermissions { source })
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("database path has no parent directory")]
    MissingParent,
    #[error("database path is a symbolic link or not a regular file")]
    UnsafeDatabasePath,
    #[error("database directory is a symbolic link or not a directory")]
    UnsafeDatabaseDirectory,
    #[error("failed to create private database directory")]
    CreateDirectory {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create private database file")]
    CreateDatabase {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read database path metadata")]
    ReadMetadata {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to set private database permissions")]
    SetPermissions {
        #[source]
        source: std::io::Error,
    },
    #[error("SQLite operation failed")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database belongs to another application (application_id={application_id})")]
    WrongApplicationId { application_id: i64 },
    #[error("nonempty database has no evtap application identity")]
    UnidentifiedNonemptyDatabase,
    #[error("database schema version {actual} is newer than supported version {supported}")]
    NewerSchemaVersion { supported: i64, actual: i64 },
    #[error("SQLite selected unexpected journal mode {0}")]
    UnexpectedJournalMode(String),
    #[error("invalid session snapshot: {0}")]
    InvalidSession(&'static str),
    #[error("stored session is invalid: {0}")]
    InvalidStoredSession(&'static str),
    #[error("session {0:?} is not active")]
    SessionNotActive(SessionId),
    #[error("history page size {0} is invalid")]
    InvalidPageSize(u32),
    #[error("retention interval is invalid")]
    InvalidRetention,
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt as _};

    use rusqlite::Connection;
    use tempfile::tempdir;

    use crate::{
        metric::MetricSnapshot,
        session::{KeyboardContext, SessionSnapshot, SessionStatus},
        settings::RetentionPolicy,
    };

    use super::{APPLICATION_ID, DATABASE_SCHEMA_VERSION, Repository, RepositoryError};

    fn snapshot(id: Option<crate::session::SessionId>, updated_at_ms: i64) -> SessionSnapshot {
        SessionSnapshot {
            id,
            created_at_ms: 1_000,
            updated_at_ms,
            captured_duration_ns: 500_000_000,
            application_version: "0.2.0-dev".to_owned(),
            keyboard: KeyboardContext {
                display_name: Some("Test Keyboard".to_owned()),
                model: "pc105".to_owned(),
                layout: "de".to_owned(),
                variant: String::new(),
            },
            metrics: vec![
                MetricSnapshot::from_json("total-presses", 1, r#"{"count":12}"#.to_owned())
                    .unwrap(),
                MetricSnapshot::from_json("unknown-future", 3, "{}".to_owned()).unwrap(),
            ],
        }
    }

    #[test]
    fn creates_private_database_with_expected_schema() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("data/evtap.sqlite3");
        let repository = Repository::open(path.clone()).unwrap();

        assert_eq!(repository.path(), Some(path.as_path()));
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
        let application_id: i64 = repository
            .connection
            .pragma_query_value(None, "application_id", |row| row.get(0))
            .unwrap();
        let schema_version: i64 = repository
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(application_id, APPLICATION_ID);
        assert_eq!(schema_version, DATABASE_SCHEMA_VERSION);
    }

    #[test]
    fn checkpoints_and_restores_active_session_without_dropping_unknown_metrics() {
        let mut repository = Repository::open_in_memory().unwrap();
        let first = snapshot(None, 2_000);
        let session_id = repository.checkpoint(&first).unwrap();

        let stored = repository.load_active().unwrap().unwrap();
        assert_eq!(stored.metadata.id, session_id);
        assert_eq!(stored.metadata.status, SessionStatus::Active);
        assert_eq!(stored.metrics.len(), 2);

        let mut second = snapshot(Some(session_id), 3_000);
        second.metrics.truncate(1);
        repository.checkpoint(&second).unwrap();
        let stored = repository.load_active().unwrap().unwrap();
        assert_eq!(stored.metrics.len(), 2);
        assert!(
            stored
                .metrics
                .iter()
                .any(|metric| metric.metric_id == "unknown-future")
        );
    }

    #[test]
    fn database_enforces_a_single_active_session() {
        let mut repository = Repository::open_in_memory().unwrap();
        let first_id = repository.checkpoint(&snapshot(None, 2_000)).unwrap();

        assert!(matches!(
            repository.checkpoint(&snapshot(None, 3_000)),
            Err(RepositoryError::Sqlite(_))
        ));
        assert_eq!(
            repository.load_active().unwrap().unwrap().metadata.id,
            first_id
        );
    }

    #[test]
    fn checkpoint_validation_is_atomic() {
        let mut repository = Repository::open_in_memory().unwrap();
        let mut invalid = snapshot(None, 2_000);
        invalid.metrics.push(invalid.metrics[0].clone());

        assert!(matches!(
            repository.checkpoint(&invalid),
            Err(RepositoryError::InvalidSession(_))
        ));
        assert!(repository.load_active().unwrap().is_none());
    }

    #[test]
    fn finalizes_lists_retains_and_deletes_sessions() {
        let mut repository = Repository::open_in_memory().unwrap();
        let session_id = repository.checkpoint(&snapshot(None, 2_000)).unwrap();
        repository
            .finalize(&snapshot(Some(session_id), 3_000), 4_000)
            .unwrap();

        assert!(repository.load_active().unwrap().is_none());
        let history = repository.list_completed(50, 0).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].metadata.status, SessionStatus::Completed);
        assert_eq!(history[0].total_presses, Some(12));
        assert_eq!(
            repository
                .apply_retention(4_000, RetentionPolicy::Forever)
                .unwrap(),
            0
        );
        assert_eq!(
            repository
                .apply_retention(90 * 86_400_000 + 4_001, RetentionPolicy::Days(90))
                .unwrap(),
            1
        );
        assert!(repository.load_session(session_id).unwrap().is_none());
    }

    #[test]
    fn rejects_wrong_application_and_newer_schema_without_modifying_them() {
        let temporary = tempdir().unwrap();
        let wrong_path = temporary.path().join("wrong.sqlite3");
        let wrong = Connection::open(&wrong_path).unwrap();
        wrong.pragma_update(None, "application_id", 1234).unwrap();
        drop(wrong);
        assert!(matches!(
            Repository::open(wrong_path.clone()),
            Err(RepositoryError::WrongApplicationId {
                application_id: 1234
            })
        ));
        let wrong = Connection::open(wrong_path).unwrap();
        assert_eq!(
            wrong
                .pragma_query_value::<i64, _>(None, "application_id", |row| row.get(0))
                .unwrap(),
            1234
        );

        let unidentified_path = temporary.path().join("unidentified.sqlite3");
        let unidentified = Connection::open(&unidentified_path).unwrap();
        unidentified
            .execute("CREATE TABLE foreign_data (value INTEGER)", [])
            .unwrap();
        drop(unidentified);
        assert!(matches!(
            Repository::open(unidentified_path),
            Err(RepositoryError::UnidentifiedNonemptyDatabase)
        ));

        let newer_path = temporary.path().join("newer.sqlite3");
        let newer = Connection::open(&newer_path).unwrap();
        newer
            .pragma_update(None, "application_id", APPLICATION_ID)
            .unwrap();
        newer.pragma_update(None, "user_version", 99).unwrap();
        drop(newer);
        assert!(matches!(
            Repository::open(newer_path.clone()),
            Err(RepositoryError::NewerSchemaVersion { actual: 99, .. })
        ));
        let newer = Connection::open(newer_path).unwrap();
        assert_eq!(
            newer
                .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
                .unwrap(),
            99
        );
    }
}
