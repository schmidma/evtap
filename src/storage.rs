#![allow(
    dead_code,
    reason = "storage worker integration is completed by the lifecycle milestone"
)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use thiserror::Error;

use crate::{
    database::Repository,
    session::{SessionId, SessionSnapshot, SessionStatus, SessionSummary, StoredSession},
    settings::RetentionPolicy,
    wake::WakeSignal,
};

pub const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(30);
pub const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct DirtyGeneration(u64);

impl DirtyGeneration {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageStatus {
    Disabled,
    Loading,
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
            status: StorageStatus::Disabled,
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

    pub fn loaded(&mut self) {
        self.status = StorageStatus::Saved;
        self.current = DirtyGeneration::default();
        self.saved = DirtyGeneration::default();
        self.in_flight = None;
    }

    pub fn fail_loading(&mut self) -> Result<(), DirtyTrackingError> {
        if self.status != StorageStatus::Loading {
            return Err(DirtyTrackingError::NotLoading);
        }
        self.status = StorageStatus::Failed;
        Ok(())
    }

    pub fn set_failed(&mut self) {
        self.in_flight = None;
        self.status = StorageStatus::Failed;
    }

    pub fn disable(&mut self) {
        self.status = StorageStatus::Disabled;
        self.in_flight = None;
    }

    pub fn status(&self) -> StorageStatus {
        self.status
    }

    pub fn current(&self) -> DirtyGeneration {
        self.current
    }

    pub fn saved(&self) -> DirtyGeneration {
        self.saved
    }

    pub fn in_flight(&self) -> Option<DirtyGeneration> {
        self.in_flight
    }

    pub fn mark_dirty(&mut self) -> Result<DirtyGeneration, DirtyTrackingError> {
        if matches!(
            self.status,
            StorageStatus::Disabled | StorageStatus::Loading
        ) {
            return Err(DirtyTrackingError::NotReady);
        }
        self.current = DirtyGeneration(
            self.current
                .0
                .checked_add(1)
                .ok_or(DirtyTrackingError::GenerationOverflow)?,
        );
        if self.in_flight.is_none() {
            self.status = StorageStatus::Dirty;
        }
        Ok(self.current)
    }

    pub fn begin_checkpoint(&mut self) -> Option<DirtyGeneration> {
        if matches!(
            self.status,
            StorageStatus::Disabled | StorageStatus::Loading
        ) || self.in_flight.is_some()
            || self.current <= self.saved
        {
            return None;
        }
        let generation = self.current;
        self.in_flight = Some(generation);
        self.status = StorageStatus::Saving;
        Some(generation)
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

    pub fn checkpoint_started(&mut self) {
        self.deadline = None;
    }

    pub fn retry_later(&mut self, now: Instant) {
        self.deadline = Some(now + CHECKPOINT_INTERVAL);
    }

    pub fn clear(&mut self) {
        self.deadline = None;
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum DirtyTrackingError {
    #[error("storage is not ready for dirty tracking")]
    NotReady,
    #[error("storage is not loading")]
    NotLoading,
    #[error("dirty generation counter overflowed")]
    GenerationOverflow,
    #[error("storage acknowledgement did not match the in-flight generation")]
    UnexpectedAcknowledgement {
        expected: Option<DirtyGeneration>,
        actual: DirtyGeneration,
    },
}

#[derive(Clone, Debug)]
pub struct CheckpointRequest {
    pub generation: DirtyGeneration,
    pub snapshot: SessionSnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageOperation {
    Open,
    Checkpoint,
    Finalize,
    Discard,
    Retention,
    DeleteAll,
    HistoryList,
    HistoryDetail,
    DeleteCompleted,
    Maintenance,
    ShutdownCheckpoint,
}

#[derive(Debug)]
pub enum StorageCommand {
    RetryOpen {
        retention: RetentionPolicy,
        now_ms: i64,
    },
    Checkpoint(CheckpointRequest),
    Finalize {
        checkpoint: CheckpointRequest,
        completed_at_ms: i64,
        retention: RetentionPolicy,
        retention_now_ms: i64,
    },
    DiscardActive {
        session_id: SessionId,
    },
    ApplyRetention {
        retention: RetentionPolicy,
        now_ms: i64,
    },
    DeleteAll {
        reopen: bool,
        retention: RetentionPolicy,
        now_ms: i64,
    },
    ListCompleted {
        request_id: u64,
        limit: u32,
        offset: u32,
    },
    LoadCompleted {
        request_id: u64,
        session_id: SessionId,
    },
    DeleteCompleted {
        session_id: SessionId,
    },
    Shutdown {
        final_checkpoint: Option<CheckpointRequest>,
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
        active: Option<StoredSession>,
        retained_sessions: usize,
    },
    Checkpointed {
        generation: DirtyGeneration,
        session_id: SessionId,
    },
    Finalized {
        generation: DirtyGeneration,
        session_id: SessionId,
    },
    Discarded {
        session_id: SessionId,
        deleted: bool,
    },
    RetentionApplied {
        deleted_sessions: usize,
    },
    AllDeleted {
        reopened: bool,
    },
    HistoryLoaded {
        request_id: u64,
        offset: u32,
        sessions: Vec<SessionSummary>,
    },
    CompletedLoaded {
        request_id: u64,
        session: Option<StoredSession>,
    },
    CompletedDeleted {
        session_id: SessionId,
        deleted: bool,
    },
    Failed(StorageFailure),
    ShutdownComplete {
        final_generation: Option<DirtyGeneration>,
        final_checkpoint_saved: bool,
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
        retention: RetentionPolicy,
        now_ms: i64,
        wake: WakeSignal,
    ) -> Result<Self, StorageWorkerError> {
        let (command_sender, command_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("evtap-storage".to_owned())
            .spawn(move || {
                worker_main(
                    database_path,
                    retention,
                    now_ms,
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

    pub fn try_recv(&self) -> Result<Option<StorageEvent>, StorageWorkerError> {
        match self.events.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(StorageWorkerError::EventChannelClosed),
        }
    }

    pub fn request_shutdown(
        &self,
        final_checkpoint: Option<CheckpointRequest>,
    ) -> Result<(), StorageWorkerError> {
        self.send(StorageCommand::Shutdown { final_checkpoint })
    }

    pub fn shutdown(
        self,
        final_checkpoint: Option<CheckpointRequest>,
    ) -> Result<ShutdownResult, StorageWorkerError> {
        self.shutdown_with_timeout(final_checkpoint, GRACEFUL_SHUTDOWN_TIMEOUT)
    }

    fn shutdown_with_timeout(
        mut self,
        final_checkpoint: Option<CheckpointRequest>,
        timeout: Duration,
    ) -> Result<ShutdownResult, StorageWorkerError> {
        self.send(StorageCommand::Shutdown { final_checkpoint })?;
        let deadline = Instant::now() + timeout;
        let result = loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or(StorageWorkerError::ShutdownTimedOut { timeout })?;
            match self.events.recv_timeout(remaining) {
                Ok(StorageEvent::ShutdownComplete {
                    final_generation,
                    final_checkpoint_saved,
                }) => {
                    break ShutdownResult {
                        final_generation,
                        final_checkpoint_saved,
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
        let _ = self.commands.send(StorageCommand::Shutdown {
            final_checkpoint: None,
        });
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShutdownResult {
    pub final_generation: Option<DirtyGeneration>,
    pub final_checkpoint_saved: bool,
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

struct WorkerState {
    repository: Option<Repository>,
    active_session_id: Option<SessionId>,
    database_path: PathBuf,
}

impl WorkerState {
    fn new(database_path: PathBuf) -> Self {
        Self {
            repository: None,
            active_session_id: None,
            database_path,
        }
    }

    fn open(
        &mut self,
        retention: RetentionPolicy,
        now_ms: i64,
    ) -> Result<(Option<StoredSession>, usize), String> {
        self.repository = None;
        self.active_session_id = None;
        let mut repository =
            Repository::open(self.database_path.clone()).map_err(|error| error.to_string())?;
        let retained_sessions = repository
            .apply_retention(now_ms, retention)
            .map_err(|error| error.to_string())?;
        let active = repository
            .load_active()
            .map_err(|error| error.to_string())?;
        self.active_session_id = active.as_ref().map(|session| session.metadata.id);
        self.repository = Some(repository);
        Ok((active, retained_sessions))
    }

    fn checkpoint(&mut self, mut request: CheckpointRequest) -> Result<SessionId, String> {
        request.snapshot.id = request.snapshot.id.or(self.active_session_id);
        let repository = self
            .repository
            .as_mut()
            .ok_or_else(|| "storage is unavailable".to_owned())?;
        let session_id = repository
            .checkpoint(&request.snapshot)
            .map_err(|error| error.to_string())?;
        self.active_session_id = Some(session_id);
        Ok(session_id)
    }

    fn finalize(
        &mut self,
        mut request: CheckpointRequest,
        completed_at_ms: i64,
    ) -> Result<SessionId, String> {
        request.snapshot.id = request.snapshot.id.or(self.active_session_id);
        let repository = self
            .repository
            .as_mut()
            .ok_or_else(|| "storage is unavailable".to_owned())?;
        let session_id = repository
            .finalize(&request.snapshot, completed_at_ms)
            .map_err(|error| error.to_string())?;
        self.active_session_id = None;
        Ok(session_id)
    }

    fn discard_active(&mut self, session_id: SessionId) -> Result<bool, String> {
        let repository = self
            .repository
            .as_mut()
            .ok_or_else(|| "storage is unavailable".to_owned())?;
        let deleted = repository
            .discard_active(session_id)
            .map_err(|error| error.to_string())?;
        if deleted && self.active_session_id == Some(session_id) {
            self.active_session_id = None;
        }
        Ok(deleted)
    }

    fn apply_retention(
        &mut self,
        now_ms: i64,
        retention: RetentionPolicy,
    ) -> Result<usize, String> {
        self.repository
            .as_mut()
            .ok_or_else(|| "storage is unavailable".to_owned())?
            .apply_retention(now_ms, retention)
            .map_err(|error| error.to_string())
    }

    fn list_completed(&self, limit: u32, offset: u32) -> Result<Vec<SessionSummary>, String> {
        self.repository
            .as_ref()
            .ok_or_else(|| "storage is unavailable".to_owned())?
            .list_completed(limit, offset)
            .map_err(|error| error.to_string())
    }

    fn load_completed(&self, session_id: SessionId) -> Result<Option<StoredSession>, String> {
        self.repository
            .as_ref()
            .ok_or_else(|| "storage is unavailable".to_owned())?
            .load_session(session_id)
            .map(|session| {
                session.filter(|stored| stored.metadata.status == SessionStatus::Completed)
            })
            .map_err(|error| error.to_string())
    }

    fn delete_completed(&mut self, session_id: SessionId) -> Result<bool, String> {
        self.repository
            .as_mut()
            .ok_or_else(|| "storage is unavailable".to_owned())?
            .delete_completed(session_id)
            .map_err(|error| error.to_string())
    }

    fn reclaim_after_deletion(&self) -> Result<(), String> {
        self.repository
            .as_ref()
            .ok_or_else(|| "storage is unavailable".to_owned())?
            .reclaim_after_deletion()
            .map_err(|error| error.to_string())
    }

    fn delete_all(
        &mut self,
        reopen: bool,
        retention: RetentionPolicy,
        now_ms: i64,
    ) -> Result<(), String> {
        self.repository = None;
        self.active_session_id = None;
        remove_database_files(&self.database_path)?;
        if reopen {
            let (active, _) = self.open(retention, now_ms)?;
            if active.is_some() {
                return Err("fresh analytics database unexpectedly contained a session".to_owned());
            }
        }
        Ok(())
    }
}

fn remove_database_files(database_path: &Path) -> Result<(), String> {
    let sidecar = |suffix: &str| {
        let mut path = database_path.as_os_str().to_os_string();
        path.push(suffix);
        PathBuf::from(path)
    };
    for path in [
        database_path.to_path_buf(),
        sidecar("-wal"),
        sidecar("-shm"),
        sidecar("-journal"),
    ] {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to remove analytics file {}: {error}",
                    path.display()
                ));
            }
        }
    }
    if let Some(parent) = database_path.parent() {
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("failed to synchronize analytics directory: {error}"))?;
    }
    Ok(())
}

fn worker_main(
    database_path: PathBuf,
    retention: RetentionPolicy,
    now_ms: i64,
    commands: Receiver<StorageCommand>,
    events: &Sender<StorageEvent>,
    wake: &WakeSignal,
) {
    let mut state = WorkerState::new(database_path);
    match state.open(retention, now_ms) {
        Ok((active, retained_sessions)) => emit(
            events,
            wake,
            StorageEvent::Opened {
                active,
                retained_sessions,
            },
        ),
        Err(details) => emit_failure(events, wake, &state, StorageOperation::Open, None, details),
    }

    while let Ok(command) = commands.recv() {
        match command {
            StorageCommand::RetryOpen { retention, now_ms } => {
                match state.open(retention, now_ms) {
                    Ok((active, retained_sessions)) => emit(
                        events,
                        wake,
                        StorageEvent::Opened {
                            active,
                            retained_sessions,
                        },
                    ),
                    Err(details) => {
                        emit_failure(events, wake, &state, StorageOperation::Open, None, details)
                    }
                }
            }
            StorageCommand::Checkpoint(request) => {
                let generation = request.generation;
                match state.checkpoint(request) {
                    Ok(session_id) => emit(
                        events,
                        wake,
                        StorageEvent::Checkpointed {
                            generation,
                            session_id,
                        },
                    ),
                    Err(details) => emit_failure(
                        events,
                        wake,
                        &state,
                        StorageOperation::Checkpoint,
                        Some(generation),
                        details,
                    ),
                }
            }
            StorageCommand::Finalize {
                checkpoint,
                completed_at_ms,
                retention,
                retention_now_ms,
            } => {
                let generation = checkpoint.generation;
                match state.finalize(checkpoint, completed_at_ms) {
                    Ok(session_id) => {
                        emit(
                            events,
                            wake,
                            StorageEvent::Finalized {
                                generation,
                                session_id,
                            },
                        );
                        match state.apply_retention(retention_now_ms, retention) {
                            Ok(deleted_sessions) => {
                                emit(
                                    events,
                                    wake,
                                    StorageEvent::RetentionApplied { deleted_sessions },
                                );
                                if deleted_sessions > 0
                                    && let Err(details) = state.reclaim_after_deletion()
                                {
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
                                StorageOperation::Retention,
                                None,
                                details,
                            ),
                        }
                    }
                    Err(details) => emit_failure(
                        events,
                        wake,
                        &state,
                        StorageOperation::Finalize,
                        Some(generation),
                        details,
                    ),
                }
            }
            StorageCommand::DiscardActive { session_id } => {
                match state.discard_active(session_id) {
                    Ok(deleted) => {
                        emit(
                            events,
                            wake,
                            StorageEvent::Discarded {
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
                        StorageOperation::Discard,
                        None,
                        details,
                    ),
                }
            }
            StorageCommand::ApplyRetention { retention, now_ms } => {
                match state.apply_retention(now_ms, retention) {
                    Ok(deleted_sessions) => {
                        emit(
                            events,
                            wake,
                            StorageEvent::RetentionApplied { deleted_sessions },
                        );
                        if deleted_sessions > 0
                            && let Err(details) = state.reclaim_after_deletion()
                        {
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
                        StorageOperation::Retention,
                        None,
                        details,
                    ),
                }
            }
            StorageCommand::DeleteAll {
                reopen,
                retention,
                now_ms,
            } => match state.delete_all(reopen, retention, now_ms) {
                Ok(()) => emit(events, wake, StorageEvent::AllDeleted { reopened: reopen }),
                Err(details) => emit_failure(
                    events,
                    wake,
                    &state,
                    StorageOperation::DeleteAll,
                    None,
                    details,
                ),
            },
            StorageCommand::ListCompleted {
                request_id,
                limit,
                offset,
            } => match state.list_completed(limit, offset) {
                Ok(sessions) => emit(
                    events,
                    wake,
                    StorageEvent::HistoryLoaded {
                        request_id,
                        offset,
                        sessions,
                    },
                ),
                Err(details) => emit_failure(
                    events,
                    wake,
                    &state,
                    StorageOperation::HistoryList,
                    None,
                    details,
                ),
            },
            StorageCommand::LoadCompleted {
                request_id,
                session_id,
            } => match state.load_completed(session_id) {
                Ok(session) => emit(
                    events,
                    wake,
                    StorageEvent::CompletedLoaded {
                        request_id,
                        session,
                    },
                ),
                Err(details) => emit_failure(
                    events,
                    wake,
                    &state,
                    StorageOperation::HistoryDetail,
                    None,
                    details,
                ),
            },
            StorageCommand::DeleteCompleted { session_id } => {
                match state.delete_completed(session_id) {
                    Ok(deleted) => {
                        emit(
                            events,
                            wake,
                            StorageEvent::CompletedDeleted {
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
                        StorageOperation::DeleteCompleted,
                        None,
                        details,
                    ),
                }
            }
            StorageCommand::Shutdown { final_checkpoint } => {
                let final_generation = final_checkpoint
                    .as_ref()
                    .map(|checkpoint| checkpoint.generation);
                let final_checkpoint_saved = final_checkpoint.is_none_or(|checkpoint| {
                    let generation = checkpoint.generation;
                    match state.checkpoint(checkpoint) {
                        Ok(session_id) => {
                            emit(
                                events,
                                wake,
                                StorageEvent::Checkpointed {
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
                                StorageOperation::ShutdownCheckpoint,
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
                        final_checkpoint_saved,
                    },
                );
                break;
            }
        }
    }
}

fn emit(events: &Sender<StorageEvent>, wake: &WakeSignal, event: StorageEvent) {
    if events.send(event).is_ok() {
        wake.notify();
    }
}

fn emit_failure(
    events: &Sender<StorageEvent>,
    wake: &WakeSignal,
    state: &WorkerState,
    operation: StorageOperation,
    generation: Option<DirtyGeneration>,
    details: String,
) {
    emit(
        events,
        wake,
        StorageEvent::Failed(StorageFailure {
            operation,
            generation,
            database_path: state.database_path.clone(),
            details,
        }),
    );
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use tempfile::tempdir;

    use crate::{
        metric::MetricSnapshot,
        session::{KeyboardContext, SessionSnapshot},
        settings::RetentionPolicy,
        wake::WakeSignal,
    };

    use super::{
        CHECKPOINT_INTERVAL, CheckpointRequest, CheckpointSchedule, DirtyGeneration, DirtyTracker,
        StorageCommand, StorageEvent, StorageOperation, StorageStatus, StorageWorker,
    };

    fn wake_signal() -> (WakeSignal, Arc<AtomicUsize>) {
        let wakes = Arc::new(AtomicUsize::new(0));
        let callback_wakes = Arc::clone(&wakes);
        (
            WakeSignal::new(move || {
                callback_wakes.fetch_add(1, Ordering::Relaxed);
            }),
            wakes,
        )
    }

    fn snapshot(updated_at_ms: i64) -> SessionSnapshot {
        SessionSnapshot {
            id: None,
            created_at_ms: 1_000,
            updated_at_ms,
            captured_duration_ns: 100,
            application_version: "0.2.0-dev".to_owned(),
            keyboard: KeyboardContext {
                display_name: None,
                model: String::new(),
                layout: "us".to_owned(),
                variant: String::new(),
            },
            metrics: vec![
                MetricSnapshot::from_json("total-presses", 1, r#"{"count":4}"#.to_owned()).unwrap(),
            ],
        }
    }

    fn recv_event(worker: &StorageWorker) -> StorageEvent {
        for _ in 0..100 {
            if let Some(event) = worker.try_recv().unwrap() {
                return event;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("storage worker did not produce an event");
    }

    #[test]
    fn checkpoint_schedule_waits_thirty_seconds_without_postponing_on_each_event() {
        let start = std::time::Instant::now();
        let mut schedule = CheckpointSchedule::default();
        schedule.note_dirty(start);
        schedule.note_dirty(start + Duration::from_secs(20));

        assert!(!schedule.is_due(start + CHECKPOINT_INTERVAL - Duration::from_millis(1)));
        assert!(schedule.is_due(start + CHECKPOINT_INTERVAL));
        schedule.checkpoint_started();
        assert!(!schedule.is_due(start + Duration::from_secs(60)));
    }

    #[test]
    fn dirty_acknowledgement_does_not_save_newer_changes() {
        let mut tracker = DirtyTracker::default();
        tracker.begin_loading();
        tracker.loaded();
        let first = tracker.mark_dirty().unwrap();
        assert_eq!(tracker.begin_checkpoint(), Some(first));
        let second = tracker.mark_dirty().unwrap();
        assert!(second > first);
        assert_eq!(tracker.status(), StorageStatus::Saving);
        assert_eq!(tracker.begin_checkpoint(), None);

        tracker.acknowledge(first).unwrap();

        assert_eq!(tracker.status(), StorageStatus::Dirty);
        assert_eq!(tracker.saved(), first);
        assert_eq!(tracker.begin_checkpoint(), Some(second));
    }

    #[test]
    fn failed_checkpoint_remains_dirty_and_can_retry_latest_generation() {
        let mut tracker = DirtyTracker::default();
        tracker.begin_loading();
        tracker.loaded();
        let generation = tracker.mark_dirty().unwrap();
        assert_eq!(tracker.begin_checkpoint(), Some(generation));
        tracker.fail(generation).unwrap();
        assert_eq!(tracker.status(), StorageStatus::Failed);
        assert_eq!(tracker.saved(), DirtyGeneration::default());
        assert_eq!(tracker.begin_checkpoint(), Some(generation));
    }

    #[test]
    fn worker_checkpoints_and_recovers_active_aggregates() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("evtap.sqlite3");
        let (wake, wakes) = wake_signal();
        let worker =
            StorageWorker::spawn(path.clone(), RetentionPolicy::Days(90), 2_000, wake).unwrap();
        assert!(matches!(
            recv_event(&worker),
            StorageEvent::Opened { active: None, .. }
        ));

        let generation = DirtyGeneration(1);
        worker
            .send(StorageCommand::Checkpoint(CheckpointRequest {
                generation,
                snapshot: snapshot(2_000),
            }))
            .unwrap();
        assert!(matches!(
            recv_event(&worker),
            StorageEvent::Checkpointed {
                generation: saved,
                ..
            } if saved == generation
        ));
        assert!(wakes.load(Ordering::Relaxed) >= 2);
        worker.shutdown(None).unwrap();

        let (wake, _) = wake_signal();
        let worker = StorageWorker::spawn(path, RetentionPolicy::Days(90), 3_000, wake).unwrap();
        let StorageEvent::Opened {
            active: Some(active),
            ..
        } = recv_event(&worker)
        else {
            panic!("expected restored active session");
        };
        assert_eq!(active.metrics.len(), 1);
        assert_eq!(active.metrics[0].metric_id, "total-presses");
        worker.shutdown(None).unwrap();
    }

    #[test]
    fn shutdown_flushes_a_fresh_snapshot_and_uses_the_worker_session_id() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("evtap.sqlite3");
        let (wake, _) = wake_signal();
        let worker =
            StorageWorker::spawn(path.clone(), RetentionPolicy::Days(90), 2_000, wake).unwrap();
        assert!(matches!(recv_event(&worker), StorageEvent::Opened { .. }));
        worker
            .send(StorageCommand::Checkpoint(CheckpointRequest {
                generation: DirtyGeneration(1),
                snapshot: snapshot(2_000),
            }))
            .unwrap();

        let result = worker
            .shutdown(Some(CheckpointRequest {
                generation: DirtyGeneration(2),
                snapshot: snapshot(3_000),
            }))
            .unwrap();
        assert_eq!(result.final_generation, Some(DirtyGeneration(2)));
        assert!(result.final_checkpoint_saved);

        let (wake, _) = wake_signal();
        let worker = StorageWorker::spawn(path, RetentionPolicy::Days(90), 4_000, wake).unwrap();
        let StorageEvent::Opened {
            active: Some(active),
            ..
        } = recv_event(&worker)
        else {
            panic!("expected restored active session");
        };
        assert_eq!(active.metadata.updated_at_ms, 3_000);
        worker.shutdown(None).unwrap();
    }

    #[test]
    fn worker_lists_loads_and_deletes_completed_sessions() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("evtap.sqlite3");
        let (wake, _) = wake_signal();
        let worker =
            StorageWorker::spawn(path.clone(), RetentionPolicy::Days(90), 2_000, wake).unwrap();
        assert!(matches!(recv_event(&worker), StorageEvent::Opened { .. }));
        worker
            .send(StorageCommand::Checkpoint(CheckpointRequest {
                generation: DirtyGeneration(1),
                snapshot: snapshot(2_000),
            }))
            .unwrap();
        assert!(matches!(
            recv_event(&worker),
            StorageEvent::Checkpointed { .. }
        ));
        worker
            .send(StorageCommand::Finalize {
                checkpoint: CheckpointRequest {
                    generation: DirtyGeneration(2),
                    snapshot: snapshot(3_000),
                },
                completed_at_ms: 4_000,
                retention: RetentionPolicy::Days(90),
                retention_now_ms: 4_000,
            })
            .unwrap();
        let session_id = match recv_event(&worker) {
            StorageEvent::Finalized {
                generation: DirtyGeneration(2),
                session_id,
            } => session_id,
            event => panic!("unexpected event: {event:?}"),
        };
        assert!(matches!(
            recv_event(&worker),
            StorageEvent::RetentionApplied { .. }
        ));

        worker
            .send(StorageCommand::ListCompleted {
                request_id: 7,
                limit: 50,
                offset: 0,
            })
            .unwrap();
        let StorageEvent::HistoryLoaded {
            request_id: 7,
            sessions,
            ..
        } = recv_event(&worker)
        else {
            panic!("expected history page");
        };
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].metadata.id, session_id);
        assert_eq!(sessions[0].total_presses, Some(4));

        worker
            .send(StorageCommand::LoadCompleted {
                request_id: 8,
                session_id,
            })
            .unwrap();
        let StorageEvent::CompletedLoaded {
            request_id: 8,
            session: Some(stored),
        } = recv_event(&worker)
        else {
            panic!("expected completed-session detail");
        };
        assert_eq!(stored.metadata.id, session_id);
        assert_eq!(stored.metrics.len(), 1);

        worker
            .send(StorageCommand::DeleteCompleted { session_id })
            .unwrap();
        assert!(matches!(
            recv_event(&worker),
            StorageEvent::CompletedDeleted {
                session_id: deleted_id,
                deleted: true,
            } if deleted_id == session_id
        ));
        worker.shutdown(None).unwrap();

        let (wake, _) = wake_signal();
        let worker = StorageWorker::spawn(path, RetentionPolicy::Days(90), 5_000, wake).unwrap();
        assert!(matches!(
            recv_event(&worker),
            StorageEvent::Opened { active: None, .. }
        ));
        worker.shutdown(None).unwrap();
    }

    #[test]
    fn delete_all_closes_removes_and_optionally_reopens_storage() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("evtap.sqlite3");
        let (wake, _) = wake_signal();
        let worker =
            StorageWorker::spawn(path.clone(), RetentionPolicy::Days(90), 2_000, wake).unwrap();
        assert!(matches!(recv_event(&worker), StorageEvent::Opened { .. }));
        worker
            .send(StorageCommand::Checkpoint(CheckpointRequest {
                generation: DirtyGeneration(1),
                snapshot: snapshot(2_000),
            }))
            .unwrap();
        assert!(matches!(
            recv_event(&worker),
            StorageEvent::Checkpointed { .. }
        ));

        worker
            .send(StorageCommand::DeleteAll {
                reopen: true,
                retention: RetentionPolicy::Days(90),
                now_ms: 3_000,
            })
            .unwrap();
        assert!(matches!(
            recv_event(&worker),
            StorageEvent::AllDeleted { reopened: true }
        ));
        assert!(path.exists());

        worker
            .send(StorageCommand::DeleteAll {
                reopen: false,
                retention: RetentionPolicy::Days(90),
                now_ms: 4_000,
            })
            .unwrap();
        assert!(matches!(
            recv_event(&worker),
            StorageEvent::AllDeleted { reopened: false }
        ));
        assert!(!path.exists());
        worker.shutdown(None).unwrap();
    }

    #[test]
    fn open_failure_is_reported_without_replacing_the_database() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("evtap.sqlite3");
        std::fs::write(&path, b"not a database").unwrap();
        let original = std::fs::read(&path).unwrap();
        let (wake, _) = wake_signal();
        let worker =
            StorageWorker::spawn(PathBuf::from(&path), RetentionPolicy::Days(90), 2_000, wake)
                .unwrap();

        assert!(matches!(recv_event(&worker), StorageEvent::Failed(_)));
        assert_eq!(std::fs::read(&path).unwrap(), original);

        worker
            .send(StorageCommand::Checkpoint(CheckpointRequest {
                generation: DirtyGeneration(1),
                snapshot: snapshot(2_000),
            }))
            .unwrap();
        assert!(matches!(
            recv_event(&worker),
            StorageEvent::Failed(failure)
                if failure.operation == StorageOperation::Checkpoint
                    && failure.generation == Some(DirtyGeneration(1))
        ));

        std::fs::remove_file(&path).unwrap();
        worker
            .send(StorageCommand::RetryOpen {
                retention: RetentionPolicy::Days(90),
                now_ms: 3_000,
            })
            .unwrap();
        assert!(matches!(recv_event(&worker), StorageEvent::Opened { .. }));
        worker
            .send(StorageCommand::Checkpoint(CheckpointRequest {
                generation: DirtyGeneration(2),
                snapshot: snapshot(3_000),
            }))
            .unwrap();
        assert!(matches!(
            recv_event(&worker),
            StorageEvent::Checkpointed {
                generation: DirtyGeneration(2),
                ..
            }
        ));
        worker.shutdown(None).unwrap();
    }
}
