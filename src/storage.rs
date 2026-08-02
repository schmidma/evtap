use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use thiserror::Error;

pub use crate::database::SessionListOrder;
use crate::{
    database::{Repository, RepositoryError},
    session::{SessionId, SessionMetadata, SessionSnapshot, StoredSession},
    wake::WakeSignal,
};

pub const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(30);
pub const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const SESSION_LIST_LIMIT: u32 = 10_000;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct DirtyGeneration(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageStatus {
    Loading,
    Unsaved,
    Saved,
    Dirty,
    Saving,
    Failed,
}

#[derive(Debug)]
pub struct DirtyTracker {
    status: StorageStatus,
    current: DirtyGeneration,
    saved: DirtyGeneration,
    in_flight: Option<DirtyGeneration>,
}

impl Default for DirtyTracker {
    fn default() -> Self {
        Self {
            status: StorageStatus::Unsaved,
            current: DirtyGeneration::default(),
            saved: DirtyGeneration::default(),
            in_flight: None,
        }
    }
}

impl DirtyTracker {
    pub fn begin_loading(&mut self) {
        self.status = StorageStatus::Loading;
        self.current = DirtyGeneration::default();
        self.saved = DirtyGeneration::default();
        self.in_flight = None;
    }

    pub fn reset_saved(&mut self) {
        self.reset(StorageStatus::Saved);
    }

    pub fn reset_unsaved(&mut self) {
        self.reset(StorageStatus::Unsaved);
    }

    fn reset(&mut self, status: StorageStatus) {
        self.status = status;
        self.current = DirtyGeneration::default();
        self.saved = DirtyGeneration::default();
        self.in_flight = None;
    }

    pub fn set_failed(&mut self) {
        self.in_flight = None;
        self.status = StorageStatus::Failed;
    }

    pub fn status(&self) -> StorageStatus {
        self.status
    }

    #[cfg(test)]
    pub fn saved_generation(&self) -> DirtyGeneration {
        self.saved
    }

    pub fn in_flight(&self) -> Option<DirtyGeneration> {
        self.in_flight
    }

    pub fn is_dirty(&self) -> bool {
        self.status == StorageStatus::Unsaved || self.current > self.saved
    }

    pub fn mark_dirty(&mut self) -> Result<DirtyGeneration, DirtyTrackingError> {
        if self.status == StorageStatus::Loading {
            return Err(DirtyTrackingError::NotReady);
        }
        self.current = next_generation(self.current)?;
        if self.in_flight.is_none() {
            self.status = StorageStatus::Dirty;
        }
        Ok(self.current)
    }

    pub fn begin_save(&mut self) -> Result<Option<DirtyGeneration>, DirtyTrackingError> {
        if self.status == StorageStatus::Loading || self.in_flight.is_some() {
            return Ok(None);
        }
        if self.status == StorageStatus::Unsaved && self.current == self.saved {
            self.current = next_generation(self.current)?;
        }
        if self.current <= self.saved {
            return Ok(None);
        }
        let generation = self.current;
        self.in_flight = Some(generation);
        self.status = StorageStatus::Saving;
        Ok(Some(generation))
    }

    pub fn acknowledge(&mut self, generation: DirtyGeneration) -> Result<(), DirtyTrackingError> {
        self.require_in_flight(generation)?;
        self.in_flight = None;
        self.saved = generation;
        self.status = if self.current > self.saved {
            StorageStatus::Dirty
        } else {
            StorageStatus::Saved
        };
        Ok(())
    }

    pub fn fail(&mut self, generation: DirtyGeneration) -> Result<(), DirtyTrackingError> {
        self.require_in_flight(generation)?;
        self.in_flight = None;
        self.status = StorageStatus::Failed;
        Ok(())
    }

    fn require_in_flight(&self, generation: DirtyGeneration) -> Result<(), DirtyTrackingError> {
        if self.in_flight == Some(generation) {
            Ok(())
        } else {
            Err(DirtyTrackingError::UnexpectedAcknowledgement {
                expected: self.in_flight,
                actual: generation,
            })
        }
    }
}

fn next_generation(generation: DirtyGeneration) -> Result<DirtyGeneration, DirtyTrackingError> {
    generation
        .0
        .checked_add(1)
        .map(DirtyGeneration)
        .ok_or(DirtyTrackingError::GenerationOverflow)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CheckpointSchedule {
    deadline: Option<Instant>,
}

impl CheckpointSchedule {
    pub fn note_dirty(&mut self, now: Instant) {
        self.deadline.get_or_insert(now + CHECKPOINT_INTERVAL);
    }

    pub fn is_due(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline)
    }

    pub fn time_until_due(&self, now: Instant) -> Option<Duration> {
        self.deadline
            .map(|deadline| deadline.saturating_duration_since(now))
    }

    pub fn save_started(&mut self) {
        self.deadline = None;
    }

    pub fn clear(&mut self) {
        self.deadline = None;
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum DirtyTrackingError {
    #[error("storage is not ready for dirty tracking")]
    NotReady,
    #[error("dirty generation counter overflowed")]
    GenerationOverflow,
    #[error("storage acknowledgement did not match the in-flight generation")]
    UnexpectedAcknowledgement {
        expected: Option<DirtyGeneration>,
        actual: DirtyGeneration,
    },
}

#[derive(Clone, Debug)]
pub struct SaveRequest {
    pub generation: DirtyGeneration,
    pub snapshot: SessionSnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageOperation {
    Open,
    Save,
    List,
    Load,
    Rename,
    Delete,
    DeleteAll,
    Maintenance,
    ShutdownSave,
}

#[derive(Debug)]
pub enum StorageCommand {
    RetryOpen {
        last_session_id: Option<SessionId>,
    },
    Save(SaveRequest),
    ListSessions {
        request_id: u64,
        order: SessionListOrder,
    },
    LoadSession {
        request_id: u64,
        session_id: SessionId,
        opened_at_ms: i64,
    },
    RenameSession {
        request_id: u64,
        session_id: SessionId,
        name: Option<String>,
        updated_at_ms: i64,
    },
    DeleteSession {
        session_id: SessionId,
    },
    DeleteAll,
    Shutdown {
        final_save: Option<SaveRequest>,
    },
}

#[derive(Debug)]
pub struct StorageFailure {
    pub operation: StorageOperation,
    pub generation: Option<DirtyGeneration>,
    pub database_path: PathBuf,
    pub details: String,
}

#[derive(Debug)]
pub enum StorageEvent {
    Opened {
        sessions: Vec<SessionMetadata>,
        selected: Option<StoredSession>,
    },
    Saved {
        generation: DirtyGeneration,
        session_id: SessionId,
    },
    SessionsListed {
        request_id: u64,
        sessions: Vec<SessionMetadata>,
    },
    SessionListFailed {
        request_id: u64,
        order: SessionListOrder,
        failure: StorageFailure,
    },
    SessionLoaded {
        request_id: u64,
        session: Option<StoredSession>,
    },
    SessionRenamed {
        request_id: u64,
        session: Option<SessionMetadata>,
    },
    SessionRenameFailed {
        request_id: u64,
        session_id: SessionId,
        failure: StorageFailure,
    },
    SessionDeleted {
        session_id: SessionId,
        deleted: bool,
    },
    AllDeleted,
    Failed(StorageFailure),
    ShutdownComplete {
        final_generation: Option<DirtyGeneration>,
        final_save_succeeded: bool,
    },
}

pub struct StorageWorker {
    commands: Sender<StorageCommand>,
    events: Receiver<StorageEvent>,
    thread: Option<JoinHandle<()>>,
}

impl StorageWorker {
    pub fn spawn(
        database_path: PathBuf,
        last_session_id: Option<SessionId>,
        wake: WakeSignal,
    ) -> Result<Self, StorageWorkerError> {
        let (command_sender, command_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("evtap-storage".to_owned())
            .spawn(move || {
                worker_main(
                    database_path,
                    last_session_id,
                    command_receiver,
                    &event_sender,
                    &wake,
                );
            })
            .map_err(StorageWorkerError::Spawn)?;
        Ok(Self {
            commands: command_sender,
            events: event_receiver,
            thread: Some(thread),
        })
    }

    pub fn send(&self, command: StorageCommand) -> Result<(), StorageWorkerError> {
        self.commands
            .send(command)
            .map_err(|_| StorageWorkerError::CommandChannelClosed)
    }

    pub fn rename_session(
        &self,
        request_id: u64,
        session_id: SessionId,
        name: Option<String>,
        updated_at_ms: i64,
    ) -> Result<(), StorageWorkerError> {
        self.send(StorageCommand::RenameSession {
            request_id,
            session_id,
            name,
            updated_at_ms,
        })
    }

    pub fn try_recv(&self) -> Result<Option<StorageEvent>, StorageWorkerError> {
        match self.events.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(StorageWorkerError::EventChannelClosed),
        }
    }

    pub fn shutdown(
        self,
        final_save: Option<SaveRequest>,
    ) -> Result<ShutdownResult, StorageWorkerError> {
        self.shutdown_with_timeout(final_save, GRACEFUL_SHUTDOWN_TIMEOUT)
    }

    fn shutdown_with_timeout(
        mut self,
        final_save: Option<SaveRequest>,
        timeout: Duration,
    ) -> Result<ShutdownResult, StorageWorkerError> {
        self.send(StorageCommand::Shutdown { final_save })?;
        let deadline = Instant::now() + timeout;
        let result = loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or(StorageWorkerError::ShutdownTimedOut { timeout })?;
            match self.events.recv_timeout(remaining) {
                Ok(StorageEvent::ShutdownComplete {
                    final_generation,
                    final_save_succeeded,
                }) => {
                    break ShutdownResult {
                        final_generation,
                        final_save_succeeded,
                    };
                }
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {
                    return Err(StorageWorkerError::ShutdownTimedOut { timeout });
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(StorageWorkerError::EventChannelClosed);
                }
            }
        };

        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| StorageWorkerError::WorkerPanicked)?;
        }
        Ok(result)
    }
}

impl Drop for StorageWorker {
    fn drop(&mut self) {
        let _ = self
            .commands
            .send(StorageCommand::Shutdown { final_save: None });
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShutdownResult {
    pub final_generation: Option<DirtyGeneration>,
    pub final_save_succeeded: bool,
}

#[derive(Debug, Error)]
pub enum StorageWorkerError {
    #[error("failed to spawn storage worker")]
    Spawn(#[source] std::io::Error),
    #[error("storage command channel is closed")]
    CommandChannelClosed,
    #[error("storage event channel is closed")]
    EventChannelClosed,
    #[error("storage worker did not stop within {timeout:?}")]
    ShutdownTimedOut { timeout: Duration },
    #[error("storage worker panicked")]
    WorkerPanicked,
}

#[derive(Debug, Error)]
enum WorkerStateError {
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error("storage is unavailable")]
    Unavailable,
    #[error("failed to remove analytics file {path}")]
    RemoveFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to synchronize analytics directory {path}")]
    SyncDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

struct WorkerState {
    repository: Option<Repository>,
    database_path: PathBuf,
}

impl WorkerState {
    fn new(database_path: PathBuf) -> Self {
        Self {
            repository: None,
            database_path,
        }
    }

    fn open(
        &mut self,
        last_session_id: Option<SessionId>,
    ) -> Result<(Vec<SessionMetadata>, Option<StoredSession>), WorkerStateError> {
        self.repository = Repository::open_existing(self.database_path.clone())?;
        let Some(repository) = &self.repository else {
            return Ok((Vec::new(), None));
        };
        let sessions =
            repository.list_sessions(SESSION_LIST_LIMIT, SessionListOrder::LastOpened)?;
        let selected = last_session_id
            .map(|session_id| repository.load_session(session_id))
            .transpose()?
            .flatten();
        Ok((sessions, selected))
    }

    fn repository_mut(&mut self) -> Result<&mut Repository, WorkerStateError> {
        if self.repository.is_none() {
            self.repository = Some(Repository::open(self.database_path.clone())?);
        }
        self.repository
            .as_mut()
            .ok_or(WorkerStateError::Unavailable)
    }

    fn save(&mut self, request: SaveRequest) -> Result<SessionId, WorkerStateError> {
        Ok(self.repository_mut()?.save(&request.snapshot)?)
    }

    fn list_sessions(
        &self,
        order: SessionListOrder,
    ) -> Result<Vec<SessionMetadata>, WorkerStateError> {
        self.repository.as_ref().map_or_else(
            || Ok(Vec::new()),
            |repository| {
                repository
                    .list_sessions(SESSION_LIST_LIMIT, order)
                    .map_err(Into::into)
            },
        )
    }

    fn load_session(
        &mut self,
        session_id: SessionId,
        opened_at_ms: i64,
    ) -> Result<Option<StoredSession>, WorkerStateError> {
        let Some(repository) = self.repository.as_mut() else {
            return Ok(None);
        };
        repository
            .load_and_mark_opened(session_id, opened_at_ms)
            .map_err(Into::into)
    }

    fn rename_session(
        &mut self,
        session_id: SessionId,
        name: Option<&str>,
        updated_at_ms: i64,
    ) -> Result<Option<SessionMetadata>, WorkerStateError> {
        let Some(repository) = self.repository.as_mut() else {
            return Ok(None);
        };
        repository
            .rename_session(session_id, name, updated_at_ms)
            .map_err(Into::into)
    }

    fn delete_session(&mut self, session_id: SessionId) -> Result<bool, WorkerStateError> {
        let Some(repository) = self.repository.as_mut() else {
            return Ok(false);
        };
        repository.delete_session(session_id).map_err(Into::into)
    }

    fn reclaim_after_deletion(&self) -> Result<(), WorkerStateError> {
        self.repository
            .as_ref()
            .ok_or(WorkerStateError::Unavailable)?
            .reclaim_after_deletion()
            .map_err(Into::into)
    }

    fn delete_all(&mut self) -> Result<(), WorkerStateError> {
        self.repository = None;
        remove_database_files(&self.database_path)
    }
}

fn database_files(database_path: &Path) -> [PathBuf; 4] {
    let sidecar = |suffix: &str| {
        let mut path = database_path.as_os_str().to_os_string();
        path.push(suffix);
        PathBuf::from(path)
    };
    [
        database_path.to_path_buf(),
        sidecar("-wal"),
        sidecar("-shm"),
        sidecar("-journal"),
    ]
}

pub fn database_disk_usage(database_path: &Path) -> u64 {
    database_files(database_path)
        .into_iter()
        .filter_map(|path| path.metadata().ok().map(|metadata| metadata.len()))
        .fold(0_u64, u64::saturating_add)
}

fn remove_database_files(database_path: &Path) -> Result<(), WorkerStateError> {
    for path in database_files(database_path) {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(WorkerStateError::RemoveFile { path, source }),
        }
    }
    if let Some(parent) = database_path.parent()
        && parent.exists()
    {
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| WorkerStateError::SyncDirectory {
                path: parent.to_path_buf(),
                source,
            })?;
    }
    Ok(())
}

fn worker_main(
    database_path: PathBuf,
    last_session_id: Option<SessionId>,
    commands: Receiver<StorageCommand>,
    events: &Sender<StorageEvent>,
    wake: &WakeSignal,
) {
    let mut state = WorkerState::new(database_path);
    let open_result = state.open(last_session_id);
    emit_open_result(events, wake, &state, open_result);

    while let Ok(command) = commands.recv() {
        match command {
            StorageCommand::RetryOpen { last_session_id } => {
                let result = state.open(last_session_id);
                emit_open_result(events, wake, &state, result);
            }
            StorageCommand::Save(request) => {
                let generation = request.generation;
                match state.save(request) {
                    Ok(session_id) => emit(
                        events,
                        wake,
                        StorageEvent::Saved {
                            generation,
                            session_id,
                        },
                    ),
                    Err(details) => emit_failure(
                        events,
                        wake,
                        &state,
                        StorageOperation::Save,
                        Some(generation),
                        details,
                    ),
                }
            }
            StorageCommand::ListSessions { request_id, order } => {
                match state.list_sessions(order) {
                    Ok(sessions) => emit(
                        events,
                        wake,
                        StorageEvent::SessionsListed {
                            request_id,
                            sessions,
                        },
                    ),
                    Err(details) => emit(
                        events,
                        wake,
                        StorageEvent::SessionListFailed {
                            request_id,
                            order,
                            failure: storage_failure(&state, StorageOperation::List, None, details),
                        },
                    ),
                }
            }
            StorageCommand::LoadSession {
                request_id,
                session_id,
                opened_at_ms,
            } => match state.load_session(session_id, opened_at_ms) {
                Ok(session) => emit(
                    events,
                    wake,
                    StorageEvent::SessionLoaded {
                        request_id,
                        session,
                    },
                ),
                Err(details) => {
                    emit_failure(events, wake, &state, StorageOperation::Load, None, details)
                }
            },
            StorageCommand::RenameSession {
                request_id,
                session_id,
                name,
                updated_at_ms,
            } => match state.rename_session(session_id, name.as_deref(), updated_at_ms) {
                Ok(session) => emit(
                    events,
                    wake,
                    StorageEvent::SessionRenamed {
                        request_id,
                        session,
                    },
                ),
                Err(details) => emit(
                    events,
                    wake,
                    StorageEvent::SessionRenameFailed {
                        request_id,
                        session_id,
                        failure: storage_failure(&state, StorageOperation::Rename, None, details),
                    },
                ),
            },
            StorageCommand::DeleteSession { session_id } => {
                match state.delete_session(session_id) {
                    Ok(deleted) => {
                        emit(
                            events,
                            wake,
                            StorageEvent::SessionDeleted {
                                session_id,
                                deleted,
                            },
                        );
                        if deleted && let Err(details) = state.reclaim_after_deletion() {
                            emit_failure(
                                events,
                                wake,
                                &state,
                                StorageOperation::Maintenance,
                                None,
                                details,
                            );
                        }
                    }
                    Err(details) => emit_failure(
                        events,
                        wake,
                        &state,
                        StorageOperation::Delete,
                        None,
                        details,
                    ),
                }
            }
            StorageCommand::DeleteAll => match state.delete_all() {
                Ok(()) => emit(events, wake, StorageEvent::AllDeleted),
                Err(details) => emit_failure(
                    events,
                    wake,
                    &state,
                    StorageOperation::DeleteAll,
                    None,
                    details,
                ),
            },
            StorageCommand::Shutdown { final_save } => {
                let final_generation = final_save.as_ref().map(|save| save.generation);
                let final_save_succeeded = final_save.is_none_or(|request| {
                    let generation = request.generation;
                    match state.save(request) {
                        Ok(session_id) => {
                            emit(
                                events,
                                wake,
                                StorageEvent::Saved {
                                    generation,
                                    session_id,
                                },
                            );
                            true
                        }
                        Err(details) => {
                            emit_failure(
                                events,
                                wake,
                                &state,
                                StorageOperation::ShutdownSave,
                                Some(generation),
                                details,
                            );
                            false
                        }
                    }
                });
                state.repository = None;
                emit(
                    events,
                    wake,
                    StorageEvent::ShutdownComplete {
                        final_generation,
                        final_save_succeeded,
                    },
                );
                break;
            }
        }
    }
}

fn emit_open_result(
    events: &Sender<StorageEvent>,
    wake: &WakeSignal,
    state: &WorkerState,
    result: Result<(Vec<SessionMetadata>, Option<StoredSession>), WorkerStateError>,
) {
    match result {
        Ok((sessions, selected)) => emit(events, wake, StorageEvent::Opened { sessions, selected }),
        Err(details) => emit_failure(events, wake, state, StorageOperation::Open, None, details),
    }
}

fn emit(events: &Sender<StorageEvent>, wake: &WakeSignal, event: StorageEvent) {
    if events.send(event).is_ok() {
        wake.notify();
    }
}

fn storage_failure(
    state: &WorkerState,
    operation: StorageOperation,
    generation: Option<DirtyGeneration>,
    details: impl std::fmt::Display,
) -> StorageFailure {
    StorageFailure {
        operation,
        generation,
        database_path: state.database_path.clone(),
        details: details.to_string(),
    }
}

fn emit_failure(
    events: &Sender<StorageEvent>,
    wake: &WakeSignal,
    state: &WorkerState,
    operation: StorageOperation,
    generation: Option<DirtyGeneration>,
    details: impl std::fmt::Display,
) {
    emit(
        events,
        wake,
        StorageEvent::Failed(storage_failure(state, operation, generation, details)),
    );
}

#[cfg(test)]
mod tests {
    use std::{
        os::unix::fs::PermissionsExt as _,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    use tempfile::tempdir;

    use crate::{
        metric::MetricSnapshot,
        session::{KeyboardContext, SessionSnapshot},
        wake::WakeSignal,
    };

    use super::{
        CheckpointSchedule, DirtyTracker, SaveRequest, SessionListOrder, StorageCommand,
        StorageEvent, StorageOperation, StorageWorker,
    };

    fn wake_signal() -> WakeSignal {
        let wakes = Arc::new(AtomicUsize::new(0));
        WakeSignal::new({
            let wakes = Arc::clone(&wakes);
            move || {
                wakes.fetch_add(1, Ordering::Relaxed);
            }
        })
    }

    fn snapshot(updated_at_ms: i64) -> SessionSnapshot {
        SessionSnapshot {
            id: None,
            name: None,
            created_at_ms: 1,
            updated_at_ms,
            last_opened_at_ms: updated_at_ms,
            captured_duration_ns: 0,
            application_version: "test".to_owned(),
            keyboard: KeyboardContext::default(),
            metrics: vec![
                MetricSnapshot::from_json("total-presses", 1, r#"{"count":1}"#.to_owned()).unwrap(),
            ],
        }
    }

    fn recv_event(worker: &StorageWorker) -> StorageEvent {
        for _ in 0..100 {
            if let Some(event) = worker.try_recv().unwrap() {
                return event;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("storage event did not arrive");
    }

    #[test]
    fn autosave_schedule_waits_thirty_seconds_without_postponing() {
        let start = Instant::now();
        let mut schedule = CheckpointSchedule::default();
        schedule.note_dirty(start);
        schedule.note_dirty(start + Duration::from_secs(20));

        assert!(!schedule.is_due(start + Duration::from_secs(29)));
        assert!(schedule.is_due(start + Duration::from_secs(30)));
    }

    #[test]
    fn dirty_acknowledgement_does_not_save_newer_changes() {
        let mut tracker = DirtyTracker::default();
        tracker.reset_saved();
        let first = tracker.mark_dirty().unwrap();
        assert_eq!(tracker.begin_save().unwrap(), Some(first));
        tracker.mark_dirty().unwrap();
        tracker.acknowledge(first).unwrap();

        assert!(tracker.is_dirty());
        assert_eq!(tracker.saved_generation(), first);
    }

    #[test]
    fn pristine_untitled_session_can_be_saved() {
        let mut tracker = DirtyTracker::default();
        let generation = tracker.begin_save().unwrap().unwrap();
        tracker.acknowledge(generation).unwrap();
        assert!(!tracker.is_dirty());
    }

    #[test]
    fn worker_starts_without_creating_a_database_and_saves_on_request() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("data/evtap.sqlite3");
        let worker = StorageWorker::spawn(path.clone(), None, wake_signal()).unwrap();
        assert!(matches!(recv_event(&worker), StorageEvent::Opened { .. }));
        assert!(!path.exists());

        let mut tracker = DirtyTracker::default();
        let generation = tracker.begin_save().unwrap().unwrap();
        worker
            .send(StorageCommand::Save(SaveRequest {
                generation,
                snapshot: snapshot(10),
            }))
            .unwrap();
        let session_id = match recv_event(&worker) {
            StorageEvent::Saved { session_id, .. } => session_id,
            event => panic!("unexpected event: {event:?}"),
        };
        assert!(path.exists());

        worker
            .send(StorageCommand::LoadSession {
                request_id: 1,
                session_id,
                opened_at_ms: 20,
            })
            .unwrap();
        assert!(matches!(
            recv_event(&worker),
            StorageEvent::SessionLoaded {
                session: Some(_),
                ..
            }
        ));
        worker.shutdown(None).unwrap();
    }

    #[test]
    fn startup_loads_only_the_requested_last_session() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("evtap.sqlite3");
        let worker = StorageWorker::spawn(path.clone(), None, wake_signal()).unwrap();
        assert!(matches!(recv_event(&worker), StorageEvent::Opened { .. }));
        let mut tracker = DirtyTracker::default();

        let first_generation = tracker.begin_save().unwrap().unwrap();
        worker
            .send(StorageCommand::Save(SaveRequest {
                generation: first_generation,
                snapshot: snapshot(10),
            }))
            .unwrap();
        let first = match recv_event(&worker) {
            StorageEvent::Saved { session_id, .. } => session_id,
            event => panic!("unexpected event: {event:?}"),
        };
        tracker.acknowledge(first_generation).unwrap();

        let second_generation = tracker.mark_dirty().unwrap();
        assert_eq!(tracker.begin_save().unwrap(), Some(second_generation));
        worker
            .send(StorageCommand::Save(SaveRequest {
                generation: second_generation,
                snapshot: snapshot(20),
            }))
            .unwrap();
        let second = match recv_event(&worker) {
            StorageEvent::Saved { session_id, .. } => session_id,
            event => panic!("unexpected event: {event:?}"),
        };
        assert_ne!(first, second);
        worker.shutdown(None).unwrap();

        let worker = StorageWorker::spawn(path, Some(first), wake_signal()).unwrap();
        match recv_event(&worker) {
            StorageEvent::Opened { sessions, selected } => {
                assert_eq!(sessions.len(), 2);
                assert_eq!(selected.unwrap().metadata.id, first);
            }
            event => panic!("unexpected event: {event:?}"),
        }
        worker.shutdown(None).unwrap();
    }

    #[test]
    fn worker_renames_unloaded_sessions_and_lists_each_product_order() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("evtap.sqlite3");
        let worker = StorageWorker::spawn(path.clone(), None, wake_signal()).unwrap();
        assert!(matches!(recv_event(&worker), StorageEvent::Opened { .. }));
        let mut tracker = DirtyTracker::default();

        let first_generation = tracker.begin_save().unwrap().unwrap();
        let mut first_snapshot = snapshot(10);
        first_snapshot.name = Some("First".to_owned());
        worker
            .send(StorageCommand::Save(SaveRequest {
                generation: first_generation,
                snapshot: first_snapshot,
            }))
            .unwrap();
        let first = match recv_event(&worker) {
            StorageEvent::Saved { session_id, .. } => session_id,
            event => panic!("unexpected event: {event:?}"),
        };
        tracker.acknowledge(first_generation).unwrap();

        let second_generation = tracker.mark_dirty().unwrap();
        assert_eq!(tracker.begin_save().unwrap(), Some(second_generation));
        let mut second_snapshot = snapshot(20);
        second_snapshot.name = Some("Second".to_owned());
        worker
            .send(StorageCommand::Save(SaveRequest {
                generation: second_generation,
                snapshot: second_snapshot,
            }))
            .unwrap();
        let second = match recv_event(&worker) {
            StorageEvent::Saved { session_id, .. } => session_id,
            event => panic!("unexpected event: {event:?}"),
        };

        worker
            .send(StorageCommand::LoadSession {
                request_id: 1,
                session_id: first,
                opened_at_ms: 30,
            })
            .unwrap();
        assert!(matches!(
            recv_event(&worker),
            StorageEvent::SessionLoaded {
                request_id: 1,
                session: Some(_)
            }
        ));

        worker
            .rename_session(2, second, Some("Project".to_owned()), 40)
            .unwrap();
        match recv_event(&worker) {
            StorageEvent::SessionRenamed {
                request_id: 2,
                session: Some(session),
            } => {
                assert_eq!(session.id, second);
                assert_eq!(session.name.as_deref(), Some("Project"));
                assert_eq!(session.updated_at_ms, 40);
                assert_eq!(session.last_opened_at_ms, 20);
            }
            event => panic!("unexpected event: {event:?}"),
        }

        for (request_id, order, expected) in [
            (3, SessionListOrder::LastOpened, vec![first, second]),
            (4, SessionListOrder::LastUpdated, vec![second, first]),
        ] {
            worker
                .send(StorageCommand::ListSessions { request_id, order })
                .unwrap();
            match recv_event(&worker) {
                StorageEvent::SessionsListed {
                    request_id: actual_request_id,
                    sessions,
                } => {
                    assert_eq!(actual_request_id, request_id);
                    assert_eq!(
                        sessions
                            .into_iter()
                            .map(|session| session.id)
                            .collect::<Vec<_>>(),
                        expected
                    );
                }
                event => panic!("unexpected event: {event:?}"),
            }
        }

        worker
            .rename_session(5, first, Some("Project".to_owned()), 50)
            .unwrap();
        match recv_event(&worker) {
            StorageEvent::SessionRenameFailed {
                request_id: 5,
                session_id,
                failure,
            } => {
                assert_eq!(session_id, first);
                assert_eq!(failure.operation, StorageOperation::Rename);
                assert!(failure.details.contains("already uses that name"));
            }
            event => panic!("unexpected event: {event:?}"),
        }

        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        worker
            .rename_session(
                6,
                crate::session::SessionId::new(999).unwrap(),
                Some("Missing".to_owned()),
                60,
            )
            .unwrap();
        assert!(matches!(
            recv_event(&worker),
            StorageEvent::SessionRenamed {
                request_id: 6,
                session: None
            }
        ));
        worker.shutdown(None).unwrap();
    }

    #[test]
    fn shutdown_flushes_a_fresh_snapshot() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("evtap.sqlite3");
        let worker = StorageWorker::spawn(path.clone(), None, wake_signal()).unwrap();
        assert!(matches!(recv_event(&worker), StorageEvent::Opened { .. }));
        let mut tracker = DirtyTracker::default();
        let generation = tracker.begin_save().unwrap().unwrap();

        let result = worker
            .shutdown(Some(SaveRequest {
                generation,
                snapshot: snapshot(10),
            }))
            .unwrap();

        assert_eq!(result.final_generation, Some(generation));
        assert!(result.final_save_succeeded);
        assert!(path.exists());
    }

    #[test]
    fn incompatible_database_is_reported_without_replacement() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("evtap.sqlite3");
        std::fs::write(&path, b"not sqlite").unwrap();
        let original = std::fs::read(&path).unwrap();
        let worker = StorageWorker::spawn(path.clone(), None, wake_signal()).unwrap();

        assert!(matches!(recv_event(&worker), StorageEvent::Failed(_)));
        assert_eq!(std::fs::read(path).unwrap(), original);
        worker.shutdown(None).unwrap();
    }

    #[test]
    fn delete_all_removes_database_and_sidecars() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("evtap.sqlite3");
        let worker = StorageWorker::spawn(path.clone(), None, wake_signal()).unwrap();
        assert!(matches!(recv_event(&worker), StorageEvent::Opened { .. }));
        let mut tracker = DirtyTracker::default();
        let generation = tracker.begin_save().unwrap().unwrap();
        worker
            .send(StorageCommand::Save(SaveRequest {
                generation,
                snapshot: snapshot(10),
            }))
            .unwrap();
        assert!(matches!(recv_event(&worker), StorageEvent::Saved { .. }));

        worker.send(StorageCommand::DeleteAll).unwrap();
        assert!(matches!(recv_event(&worker), StorageEvent::AllDeleted));
        assert!(!path.exists());
        worker.shutdown(None).unwrap();
    }

    #[test]
    fn event_types_do_not_need_database_paths_from_callers() {
        let _: Option<PathBuf> = None;
    }
}
