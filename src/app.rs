use std::{
    collections::HashMap,
    path::Path,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use chrono::{Local, TimeZone};
use color_eyre::{Result, eyre::ContextCompat};
use eframe::egui::{self, ScrollArea};
use evdev::KeyCode;
use tracing::{error, info, warn};
use xkbcommon::xkb::{self, Context, Keymap};

use crate::{
    input::{KeyEvent, KeyEventKind, KeyRole, PhysicalKey},
    listener::{self, ListenerHandle},
    metric::{Metric, default_metrics},
    metric_view::render_metric,
    paths::AppPaths,
    scanner::{self, DeviceMetadata, ScannerHandle},
    session::{
        KeyboardContext, MetricRecoveryIssue, SessionId, SessionMetadata, SessionSnapshot,
        SessionSummary, StoredSession, recover_default_metrics,
    },
    settings::{RetentionPolicy, Settings, SettingsStore},
    storage::{
        CheckpointRequest, CheckpointSchedule, DirtyTracker, StorageCommand, StorageEvent,
        StorageOperation, StorageStatus, StorageWorker,
    },
    wake::WakeSignal,
    xkb_helper,
};

const HACK_FONT_NAME: &str = "Hack";
const LISTENER_EXIT_WAIT: Duration = Duration::from_millis(500);
const HISTORY_PAGE_SIZE: u32 = 50;

pub struct App {
    devices: Option<Vec<DeviceMetadata>>,
    selected_device: Option<usize>,
    scan_warning: Option<String>,
    scan_error: Option<String>,
    scanner: ScannerHandle,
    listener: Option<ListenerHandle>,
    listener_state: ListenerState,
    capture_error: Option<String>,
    wake_signal: WakeSignal,
    metrics: Vec<Box<dyn Metric>>,
    physical_keys: HashMap<u16, PhysicalKey>,

    model: String,
    layout: String,
    variant: String,
    keyboard_error: Option<String>,
    available_models: Vec<String>,
    available_layouts: Vec<String>,
    available_variants: Vec<String>,
    xkb_state: xkb::State,

    paths: AppPaths,
    settings_store: SettingsStore,
    settings: Settings,
    settings_error: Option<String>,
    settings_load_failed: bool,
    storage: Option<StorageWorker>,
    storage_tracker: DirtyTracker,
    checkpoint_schedule: CheckpointSchedule,
    storage_error: Option<String>,
    storage_open_intent: Option<StorageOpenIntent>,
    storage_needs_reopen: bool,
    storage_finished: bool,
    checkpoint_when_available: bool,

    session: CurrentSession,
    recovery_messages: Vec<String>,
    pending_finish: Option<PendingFinish>,
    discarding: bool,
    deleting_all_to_disable: bool,
    deleting_all: bool,
    shutting_down_storage: bool,
    confirm_discard: bool,
    confirm_delete_all: bool,
    enable_prompt: Option<EnablePrompt>,
    disable_prompt: bool,

    history_open: bool,
    history_sessions: Vec<SessionSummary>,
    history_offset: u32,
    history_has_more: bool,
    history_loading: bool,
    history_error: Option<String>,
    history_list_request_id: u64,
    history_detail_request_id: u64,
    history_detail: Option<HistoryDetail>,
    confirm_history_delete: Option<SessionId>,
    deleting_completed: Option<SessionId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ListenerState {
    Idle,
    Connecting,
    Listening,
    Stopping,
    Failed,
}

#[derive(Debug, Default)]
struct CurrentSession {
    id: Option<SessionId>,
    created_at_ms: Option<i64>,
    captured_duration: Duration,
    capture_started_at: Option<Instant>,
    keyboard: Option<KeyboardContext>,
    resumed: bool,
}

impl CurrentSession {
    fn is_active(&self) -> bool {
        self.keyboard.is_some()
    }

    fn duration(&self) -> Duration {
        self.capture_started_at
            .map_or(self.captured_duration, |started| {
                self.captured_duration.saturating_add(started.elapsed())
            })
    }

    fn finish_capture_segment(&mut self) {
        if let Some(started) = self.capture_started_at.take() {
            self.captured_duration = self.captured_duration.saturating_add(started.elapsed());
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StorageOpenIntent {
    Restore,
    PreserveCurrent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingFinish {
    disable_after: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EnablePrompt {
    Disclosure,
    ExistingSession,
}

struct HistoryDetail {
    metadata: SessionMetadata,
    metrics: Vec<Box<dyn Metric>>,
    messages: Vec<String>,
}

impl App {
    pub fn new(creation_context: &eframe::CreationContext<'_>, paths: AppPaths) -> Result<Self> {
        creation_context.egui_ctx.set_fonts(font_definitions());

        let repaint_context = creation_context.egui_ctx.clone();
        let wake_signal = WakeSignal::new(move || repaint_context.request_repaint());
        let scanner = scanner::spawn(wake_signal.clone());
        scanner.start_scan()?;

        let settings_store = SettingsStore::new(paths.settings_file());
        let (settings, settings_error, settings_load_failed) = match settings_store.load() {
            Ok(settings) => (settings, None, false),
            Err(error) => (
                Settings::default(),
                Some(format!(
                    "Could not load settings; privacy-preserving defaults are in use and the existing file will not be overwritten: {error}"
                )),
                true,
            ),
        };
        let model = settings.keyboard_model().to_owned();
        let layout = settings.keyboard_layout().to_owned();
        let variant = settings.keyboard_variant().to_owned();
        let available_models = xkb_helper::get_models();
        let available_layouts = xkb_helper::get_layouts();
        let available_variants = xkb_helper::get_variants(&layout);
        let xkb_state = init_keyboard_state(&model, &layout, &variant)?;

        let mut app = Self {
            devices: None,
            selected_device: None,
            scan_warning: None,
            scan_error: None,
            scanner,
            listener: None,
            listener_state: ListenerState::Idle,
            capture_error: None,
            wake_signal,
            metrics: default_metrics(),
            physical_keys: HashMap::new(),
            model,
            layout,
            variant,
            keyboard_error: None,
            available_models,
            available_layouts,
            available_variants,
            xkb_state,
            paths,
            settings_store,
            settings,
            settings_error,
            settings_load_failed,
            storage: None,
            storage_tracker: DirtyTracker::default(),
            checkpoint_schedule: CheckpointSchedule::default(),
            storage_error: None,
            storage_open_intent: None,
            storage_needs_reopen: false,
            storage_finished: false,
            checkpoint_when_available: false,
            session: CurrentSession::default(),
            recovery_messages: Vec::new(),
            pending_finish: None,
            discarding: false,
            deleting_all_to_disable: false,
            deleting_all: false,
            shutting_down_storage: false,
            confirm_discard: false,
            confirm_delete_all: false,
            enable_prompt: None,
            disable_prompt: false,
            history_open: false,
            history_sessions: Vec::new(),
            history_offset: 0,
            history_has_more: false,
            history_loading: false,
            history_error: None,
            history_list_request_id: 0,
            history_detail_request_id: 0,
            history_detail: None,
            confirm_history_delete: None,
            deleting_completed: None,
        };
        if app.settings.persistence_enabled() {
            app.start_storage(StorageOpenIntent::Restore);
        }
        Ok(app)
    }

    fn request_scan(&mut self) {
        self.devices = None;
        self.selected_device = None;
        self.scan_warning = None;
        self.scan_error = None;
        if let Err(error) = self.scanner.start_scan() {
            self.devices = Some(Vec::new());
            self.scan_error = Some(format!("Could not start device scan: {error:#}"));
        }
    }

    fn drain_scanner_events(&mut self) {
        while let Some(event) = self.scanner.try_recv_event() {
            match event {
                scanner::Event::ScanFinished { result } => match result {
                    Ok(report) => {
                        let issue_count = report.issues.len();
                        self.scan_warning = if issue_count == 0 {
                            None
                        } else if report.devices.is_empty() {
                            Some(format!(
                                "No readable keyboard was found. Could not inspect {issue_count} input device(s); check your /dev/input permissions."
                            ))
                        } else {
                            Some(format!(
                                "Could not inspect {issue_count} input device(s); the keyboard list may be incomplete."
                            ))
                        };
                        self.scan_error = None;
                        self.devices = Some(report.devices);
                        self.selected_device = None;
                    }
                    Err(error) => {
                        self.devices = Some(Vec::new());
                        self.selected_device = None;
                        self.scan_warning = None;
                        self.scan_error = Some(format!("Device scan failed: {error}"));
                    }
                },
            }
        }
    }

    fn drain_listener_events(&mut self) {
        while let Some(event) = self
            .listener
            .as_mut()
            .and_then(ListenerHandle::try_recv_event)
        {
            match event {
                listener::Event::Connected => {
                    self.listener_state = ListenerState::Listening;
                    self.capture_error = None;
                    self.session.capture_started_at = Some(Instant::now());
                    info!("listener connected to keyboard");
                }
                listener::Event::Stopped { reason } => {
                    let is_error = reason.is_error();
                    let message = reason.to_string();
                    self.listener = None;
                    self.session.finish_capture_segment();
                    self.listener_state = if is_error {
                        self.capture_error = Some(message.clone());
                        ListenerState::Failed
                    } else {
                        self.capture_error = None;
                        ListenerState::Idle
                    };
                    self.note_session_dirty();
                    if self.pending_finish.is_some() {
                        self.request_finalize();
                    } else {
                        self.request_checkpoint();
                    }
                    info!(%message, "listener stopped");
                }
                listener::Event::Input {
                    timestamp,
                    key_code,
                    kind,
                } => {
                    self.process_input(timestamp, key_code, kind);
                }
            }
        }
    }

    fn process_input(&mut self, timestamp: SystemTime, key_code: KeyCode, kind: KeyEventKind) {
        let code = key_code.code();
        let xkb_code = (code + 8).into();
        let text = self.xkb_state.key_get_utf8(xkb_code);
        let text = (!text.is_empty()).then_some(text);

        match kind {
            KeyEventKind::Press => {
                self.xkb_state.update_key(xkb_code, xkb::KeyDirection::Down);
            }
            KeyEventKind::Release => {
                self.xkb_state.update_key(xkb_code, xkb::KeyDirection::Up);
            }
            KeyEventKind::Repeat => {}
        }

        let key = self
            .physical_keys
            .entry(code)
            .or_insert_with(|| {
                let debug_name = format!("{key_code:?}");
                let label = debug_name
                    .strip_prefix("KEY_")
                    .unwrap_or(&debug_name)
                    .to_owned();
                PhysicalKey::new(code, label)
            })
            .clone();
        let role = if key_code == KeyCode::KEY_BACKSPACE {
            KeyRole::Backspace
        } else {
            KeyRole::Other
        };
        let event = KeyEvent::new(key, text, timestamp, kind, role);
        for metric in &mut self.metrics {
            metric.process(&event);
        }
        self.note_session_dirty();
    }

    fn drain_storage_events(&mut self) {
        loop {
            let event = match self.storage.as_ref().map(StorageWorker::try_recv) {
                Some(Ok(Some(event))) => event,
                Some(Ok(None)) | None => break,
                Some(Err(error)) => {
                    self.storage_error = Some(format!("Storage worker stopped: {error}"));
                    self.storage_tracker.set_failed();
                    break;
                }
            };
            self.handle_storage_event(event);
        }
        if self.storage_finished {
            self.storage = None;
            self.storage_finished = false;
        }
    }

    fn handle_storage_event(&mut self, event: StorageEvent) {
        match event {
            StorageEvent::Opened {
                active,
                retained_sessions,
            } => {
                if retained_sessions > 0 {
                    info!(retained_sessions, "removed expired completed sessions");
                }
                self.storage_tracker.loaded();
                self.storage_needs_reopen = false;
                self.storage_error = None;
                let intent = self
                    .storage_open_intent
                    .take()
                    .unwrap_or(StorageOpenIntent::Restore);
                match (intent, active) {
                    (StorageOpenIntent::PreserveCurrent, Some(_)) if self.session.is_active() => {
                        self.storage_tracker.set_failed();
                        self.storage_error = Some(
                            "A different active session already exists in local storage. The current in-memory session was not changed."
                                .to_owned(),
                        );
                    }
                    (_, Some(active)) => self.restore_active_session(active),
                    (StorageOpenIntent::PreserveCurrent, None) if self.session.is_active() => {
                        self.note_session_dirty();
                        self.request_checkpoint();
                    }
                    (_, None) => {}
                }
                if self.history_open {
                    self.request_history_page(self.history_offset);
                }
            }
            StorageEvent::Checkpointed {
                generation,
                session_id,
            } => {
                self.session.id = Some(session_id);
                if self.storage_tracker.in_flight() == Some(generation)
                    && let Err(error) = self.storage_tracker.acknowledge(generation)
                {
                    self.storage_error = Some(format!("Invalid storage acknowledgement: {error}"));
                }
                if self.pending_finish.is_some()
                    && self.listener.is_none()
                    && self.storage_tracker.in_flight().is_none()
                {
                    self.request_finalize();
                } else if self.checkpoint_when_available
                    && self.storage_tracker.in_flight().is_none()
                {
                    self.request_checkpoint();
                }
            }
            StorageEvent::Finalized {
                generation,
                session_id,
            } => {
                if self.session.id != Some(session_id) {
                    self.storage_error = Some(
                        "Storage finalized an unexpected session; in-memory aggregates were preserved."
                            .to_owned(),
                    );
                    return;
                }
                if self.storage_tracker.in_flight() == Some(generation) {
                    let _ = self.storage_tracker.acknowledge(generation);
                }
                let disable_after = self
                    .pending_finish
                    .take()
                    .is_some_and(|pending| pending.disable_after);
                self.reset_current_session();
                if self.history_open {
                    self.request_history_page(0);
                }
                if disable_after {
                    self.disable_persistence_now();
                }
            }
            StorageEvent::Discarded {
                session_id,
                deleted,
            } => {
                self.discarding = false;
                if self.session.id != Some(session_id) {
                    self.storage_error =
                        Some("Storage acknowledged discard for an unexpected session.".to_owned());
                } else if deleted {
                    self.reset_current_session();
                } else {
                    self.storage_error =
                        Some("The active session no longer exists in storage.".to_owned());
                }
            }
            StorageEvent::RetentionApplied { deleted_sessions } => {
                if deleted_sessions > 0 && self.history_open {
                    self.request_history_page(self.history_offset);
                }
            }
            StorageEvent::AllDeleted { reopened } => {
                let disable_after = self.deleting_all_to_disable;
                if reopened == disable_after {
                    self.storage_error = Some(
                        "Storage deletion completed with an unexpected reopen state.".to_owned(),
                    );
                }
                self.deleting_all_to_disable = false;
                self.deleting_all = false;
                self.confirm_delete_all = false;
                self.history_sessions.clear();
                self.history_detail = None;
                self.confirm_history_delete = None;
                self.deleting_completed = None;
                self.history_open = false;
                self.reset_current_session();
                if disable_after {
                    self.disable_persistence_now();
                }
            }
            StorageEvent::HistoryLoaded {
                request_id,
                offset,
                mut sessions,
            } => {
                if request_id == self.history_list_request_id {
                    self.history_has_more = sessions.len() > HISTORY_PAGE_SIZE as usize;
                    sessions.truncate(HISTORY_PAGE_SIZE as usize);
                    self.history_sessions = sessions;
                    self.history_offset = offset;
                    self.history_loading = false;
                    self.history_error = None;
                }
            }
            StorageEvent::CompletedLoaded {
                request_id,
                session,
            } => {
                if request_id == self.history_detail_request_id {
                    self.history_loading = false;
                    match session {
                        Some(stored) => {
                            let recovered = recover_default_metrics(&stored.metrics);
                            self.history_detail = Some(HistoryDetail {
                                metadata: stored.metadata,
                                metrics: recovered.metrics,
                                messages: recovered
                                    .issues
                                    .iter()
                                    .map(metric_recovery_message)
                                    .collect(),
                            });
                            self.history_error = None;
                        }
                        None => {
                            self.history_detail = None;
                            self.history_error =
                                Some("The selected completed session no longer exists.".to_owned());
                        }
                    }
                }
            }
            StorageEvent::CompletedDeleted {
                session_id,
                deleted,
            } => {
                self.deleting_completed = None;
                self.confirm_history_delete = None;
                if deleted {
                    self.history_sessions
                        .retain(|summary| summary.metadata.id != session_id);
                    if self
                        .history_detail
                        .as_ref()
                        .is_some_and(|detail| detail.metadata.id == session_id)
                    {
                        self.history_detail = None;
                    }
                    if self.history_open {
                        self.request_history_page(self.history_offset);
                    }
                } else {
                    self.history_error =
                        Some("The selected completed session no longer exists.".to_owned());
                }
            }
            StorageEvent::Failed(failure) => {
                if let Some(generation) = failure.generation
                    && self.storage_tracker.in_flight() == Some(generation)
                {
                    let _ = self.storage_tracker.fail(generation);
                    self.checkpoint_schedule.retry_later(Instant::now());
                } else if failure.operation == StorageOperation::Open {
                    let _ = self.storage_tracker.fail_loading();
                    self.storage_needs_reopen = true;
                }
                if failure.operation == StorageOperation::Discard {
                    self.discarding = false;
                }
                if failure.operation == StorageOperation::DeleteAll {
                    self.deleting_all_to_disable = false;
                    self.deleting_all = false;
                }
                if matches!(
                    failure.operation,
                    StorageOperation::HistoryList | StorageOperation::HistoryDetail
                ) {
                    self.history_loading = false;
                    self.history_error = Some(failure.details.clone());
                }
                if failure.operation == StorageOperation::DeleteCompleted {
                    self.deleting_completed = None;
                    self.history_error = Some(failure.details.clone());
                }
                self.storage_error = Some(format!(
                    "{} ({}): {}",
                    storage_operation_label(failure.operation),
                    failure.database_path.display(),
                    failure.details
                ));
            }
            StorageEvent::ShutdownComplete {
                final_generation: _,
                final_checkpoint_saved,
            } => {
                if !final_checkpoint_saved {
                    warn!("storage shutdown completed without saving the final checkpoint");
                }
                self.storage_tracker.disable();
                self.shutting_down_storage = false;
                self.storage_finished = true;
            }
        }
    }

    fn restore_active_session(&mut self, stored: StoredSession) {
        let recovered = recover_default_metrics(&stored.metrics);
        self.recovery_messages = recovered
            .issues
            .iter()
            .map(metric_recovery_message)
            .collect();
        self.metrics = recovered.metrics;
        let metadata = stored.metadata;
        self.model.clone_from(&metadata.keyboard.model);
        self.layout.clone_from(&metadata.keyboard.layout);
        self.variant.clone_from(&metadata.keyboard.variant);
        self.available_variants = xkb_helper::get_variants(&self.layout);
        self.reinit_xkb();
        self.session = CurrentSession {
            id: Some(metadata.id),
            created_at_ms: Some(metadata.created_at_ms),
            captured_duration: Duration::from_nanos(
                u64::try_from(metadata.captured_duration_ns).unwrap_or_default(),
            ),
            capture_started_at: None,
            keyboard: Some(metadata.keyboard),
            resumed: true,
        };
        self.listener = None;
        self.listener_state = ListenerState::Idle;
    }

    fn start_storage(&mut self, intent: StorageOpenIntent) {
        self.storage_tracker.begin_loading();
        self.storage_open_intent = Some(intent);
        self.storage_needs_reopen = false;
        self.storage_error = None;
        let now_ms = unix_now_ms().unwrap_or_default();
        match StorageWorker::spawn(
            self.paths.database_file(),
            self.settings.retention(),
            now_ms,
            self.wake_signal.clone(),
        ) {
            Ok(worker) => self.storage = Some(worker),
            Err(error) => {
                self.storage_tracker.set_failed();
                self.storage_error = Some(format!("Could not start storage: {error}"));
            }
        }
    }

    fn note_session_dirty(&mut self) {
        if !self.settings.persistence_enabled() {
            return;
        }
        match self.storage_tracker.mark_dirty() {
            Ok(_) => self.checkpoint_schedule.note_dirty(Instant::now()),
            Err(_error) if self.storage_tracker.status() == StorageStatus::Loading => {}
            Err(error) => {
                self.storage_error = Some(format!("Could not track unsaved changes: {error}"));
            }
        }
    }

    fn request_checkpoint(&mut self) {
        if !self.settings.persistence_enabled() || !self.session.is_active() {
            return;
        }
        let Some(generation) = self.storage_tracker.begin_checkpoint() else {
            if self.storage_tracker.in_flight().is_some() {
                self.checkpoint_when_available = true;
            }
            return;
        };
        let snapshot = match self.session_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let _ = self.storage_tracker.fail(generation);
                self.storage_error = Some(error);
                return;
            }
        };
        let send_result = self
            .storage
            .as_ref()
            .ok_or_else(|| "storage worker is unavailable".to_owned())
            .and_then(|worker| {
                worker
                    .send(StorageCommand::Checkpoint(CheckpointRequest {
                        generation,
                        snapshot,
                    }))
                    .map_err(|error| error.to_string())
            });
        match send_result {
            Ok(()) => {
                self.checkpoint_when_available = false;
                self.checkpoint_schedule.checkpoint_started();
            }
            Err(error) => {
                let _ = self.storage_tracker.fail(generation);
                self.checkpoint_schedule.retry_later(Instant::now());
                self.storage_error = Some(format!("Could not request checkpoint: {error}"));
            }
        }
    }

    fn request_finalize(&mut self) {
        if !self.settings.persistence_enabled() || !self.session.is_active() {
            return;
        }
        let Some(generation) = self.storage_tracker.begin_checkpoint() else {
            return;
        };
        let snapshot = match self.session_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let _ = self.storage_tracker.fail(generation);
                self.storage_error = Some(error);
                return;
            }
        };
        let completed_at_ms = unix_now_ms().unwrap_or(snapshot.updated_at_ms);
        let command = StorageCommand::Finalize {
            checkpoint: CheckpointRequest {
                generation,
                snapshot,
            },
            completed_at_ms,
            retention: self.settings.retention(),
            retention_now_ms: completed_at_ms,
        };
        let send_result = self
            .storage
            .as_ref()
            .ok_or_else(|| "storage worker is unavailable".to_owned())
            .and_then(|worker| worker.send(command).map_err(|error| error.to_string()));
        match send_result {
            Ok(()) => self.checkpoint_schedule.checkpoint_started(),
            Err(error) => {
                let _ = self.storage_tracker.fail(generation);
                self.storage_error = Some(format!("Could not finish session: {error}"));
            }
        }
    }

    fn session_snapshot(&self) -> Result<SessionSnapshot, String> {
        let keyboard = self
            .session
            .keyboard
            .clone()
            .ok_or_else(|| "current session has no fixed keyboard configuration".to_owned())?;
        let created_at_ms = self
            .session
            .created_at_ms
            .ok_or_else(|| "current session has no creation time".to_owned())?;
        let updated_at_ms = unix_now_ms()?;
        let captured_duration_ns = i64::try_from(self.session.duration().as_nanos())
            .map_err(|_| "captured session duration is too large to save".to_owned())?;
        let metrics = self
            .metrics
            .iter()
            .map(|metric| metric.snapshot().map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SessionSnapshot {
            id: self.session.id,
            created_at_ms,
            updated_at_ms: updated_at_ms.max(created_at_ms),
            captured_duration_ns,
            application_version: env!("CARGO_PKG_VERSION").to_owned(),
            keyboard,
            metrics,
        })
    }

    fn update_variants(&mut self) {
        self.available_variants = xkb_helper::get_variants(&self.layout);
        if !self.available_variants.contains(&self.variant) {
            self.variant.clear();
        }
    }

    fn reinit_xkb(&mut self) {
        match init_keyboard_state(&self.model, &self.layout, &self.variant) {
            Ok(state) => {
                self.xkb_state = state;
                self.keyboard_error = None;
                info!(
                    "Re-initialized XKB: {} / {} / {}",
                    self.model, self.layout, self.variant
                );
            }
            Err(error) => {
                let message = format!("Could not apply keyboard configuration: {error:#}");
                error!(%message);
                self.keyboard_error = Some(message);
            }
        }
    }

    fn save_keyboard_settings(&mut self) {
        self.settings.set_keyboard(
            self.model.clone(),
            self.layout.clone(),
            self.variant.clone(),
        );
        self.save_settings();
    }

    fn save_settings(&mut self) -> bool {
        if self.settings_load_failed {
            self.settings_error = Some(
                "Settings were not changed because the existing settings file could not be read. Fix or remove it before changing preferences."
                    .to_owned(),
            );
            return false;
        }
        match self.settings_store.save(&self.settings) {
            Ok(()) => {
                self.settings_error = None;
                true
            }
            Err(error) => {
                self.settings_error = Some(format!("Could not save settings: {error}"));
                false
            }
        }
    }

    fn begin_session_and_listen(&mut self, device_index: usize) {
        let Some(device) = self
            .devices
            .as_ref()
            .and_then(|devices| devices.get(device_index))
            .cloned()
        else {
            return;
        };
        if let Some(keyboard) = &self.session.keyboard
            && keyboard
                .display_name
                .as_deref()
                .is_some_and(|name| name != device.name)
        {
            self.storage_error = Some(
                "Select the same keyboard model used by the active session, or finish/discard it."
                    .to_owned(),
            );
            return;
        }
        if !self.session.is_active() {
            let now_ms = unix_now_ms().unwrap_or_default();
            self.session = CurrentSession {
                id: None,
                created_at_ms: Some(now_ms),
                captured_duration: Duration::ZERO,
                capture_started_at: None,
                keyboard: Some(KeyboardContext {
                    display_name: Some(device.name.clone()),
                    model: self.model.clone(),
                    layout: self.layout.clone(),
                    variant: self.variant.clone(),
                }),
                resumed: false,
            };
            self.note_session_dirty();
            self.request_checkpoint();
        }
        self.listener = Some(listener::spawn(device.path, self.wake_signal.clone()));
        self.listener_state = ListenerState::Connecting;
        self.capture_error = None;
    }

    fn stop_listener(&mut self) {
        let stop_result = self.listener.as_ref().map(ListenerHandle::stop);
        match stop_result {
            Some(Ok(())) => self.listener_state = ListenerState::Stopping,
            Some(Err(error)) => {
                self.listener = None;
                self.session.finish_capture_segment();
                self.listener_state = ListenerState::Failed;
                self.capture_error = Some(format!("Could not stop listener: {error:#}"));
                self.note_session_dirty();
                if self.pending_finish.is_some() {
                    self.request_finalize();
                } else {
                    self.request_checkpoint();
                }
            }
            None => {}
        }
    }

    fn begin_finish(&mut self, disable_after: bool) {
        self.pending_finish = Some(PendingFinish { disable_after });
        self.note_session_dirty();
        if self.listener.is_some() {
            self.stop_listener();
        } else {
            self.request_finalize();
        }
    }

    fn begin_discard(&mut self) {
        self.confirm_discard = false;
        if self.listener.is_some() || matches!(self.listener_state, ListenerState::Stopping) {
            return;
        }
        if !self.settings.persistence_enabled() {
            self.reset_current_session();
            return;
        }
        if self.storage_tracker.in_flight().is_some() {
            self.storage_error = Some("Wait for the current save before discarding.".to_owned());
            return;
        }
        match self.session.id {
            Some(session_id) => {
                let result = self
                    .storage
                    .as_ref()
                    .ok_or_else(|| "storage worker is unavailable".to_owned())
                    .and_then(|worker| {
                        worker
                            .send(StorageCommand::DiscardActive { session_id })
                            .map_err(|error| error.to_string())
                    });
                match result {
                    Ok(()) => self.discarding = true,
                    Err(error) => {
                        self.storage_error = Some(format!("Could not discard session: {error}"));
                    }
                }
            }
            None => self.reset_current_session(),
        }
    }

    fn reset_current_session(&mut self) {
        for metric in &mut self.metrics {
            metric.reset();
        }
        self.metrics = default_metrics();
        self.physical_keys.clear();
        self.session = CurrentSession::default();
        self.pending_finish = None;
        self.discarding = false;
        self.confirm_discard = false;
        self.recovery_messages.clear();
        self.checkpoint_schedule.clear();
        self.checkpoint_when_available = false;
        if self.settings.persistence_enabled()
            && self.storage_tracker.status() != StorageStatus::Failed
        {
            self.storage_tracker.loaded();
        }
        self.model = self.settings.keyboard_model().to_owned();
        self.layout = self.settings.keyboard_layout().to_owned();
        self.variant = self.settings.keyboard_variant().to_owned();
        self.available_variants = xkb_helper::get_variants(&self.layout);
        self.reinit_xkb();
    }

    fn has_samples(&self) -> bool {
        self.session.duration() > Duration::ZERO
            || self.metrics.iter().any(|metric| metric.has_data())
    }

    fn enable_persistence(&mut self, intent: StorageOpenIntent) {
        self.settings.set_persistence_enabled(true);
        if !self.save_settings() {
            self.settings.set_persistence_enabled(false);
            return;
        }
        self.enable_prompt = None;
        self.start_storage(intent);
    }

    fn disable_persistence_now(&mut self) {
        self.settings.set_persistence_enabled(false);
        if !self.save_settings() {
            self.settings.set_persistence_enabled(true);
            return;
        }
        self.disable_prompt = false;
        self.history_open = false;
        self.history_detail = None;
        self.shutting_down_storage = true;
        match &self.storage {
            Some(worker) => {
                if let Err(error) = worker.request_shutdown(None) {
                    self.storage_error = Some(format!("Could not stop storage: {error}"));
                    self.shutting_down_storage = false;
                }
            }
            None => {
                self.storage_tracker.disable();
                self.shutting_down_storage = false;
            }
        }
    }

    fn delete_all_and_disable(&mut self) {
        let Some(worker) = &self.storage else {
            self.storage_error =
                Some("Storage worker is unavailable; analytics were not deleted.".to_owned());
            return;
        };
        let now_ms = unix_now_ms().unwrap_or_default();
        match worker.send(StorageCommand::DeleteAll {
            reopen: false,
            retention: self.settings.retention(),
            now_ms,
        }) {
            Ok(()) => {
                self.deleting_all_to_disable = true;
                self.deleting_all = true;
                self.disable_prompt = false;
            }
            Err(error) => {
                self.storage_error = Some(format!("Could not request analytics deletion: {error}"));
            }
        }
    }

    fn delete_all_analytics(&mut self) {
        let Some(worker) = &self.storage else {
            self.history_error =
                Some("Storage worker is unavailable; analytics were not deleted.".to_owned());
            return;
        };
        match worker.send(StorageCommand::DeleteAll {
            reopen: true,
            retention: self.settings.retention(),
            now_ms: unix_now_ms().unwrap_or_default(),
        }) {
            Ok(()) => {
                self.deleting_all = true;
                self.confirm_delete_all = false;
            }
            Err(error) => {
                self.history_error = Some(format!("Could not request analytics deletion: {error}"));
            }
        }
    }

    fn change_retention(&mut self, retention: RetentionPolicy) {
        let previous = self.settings.retention();
        if self.settings.set_retention(retention).is_err() {
            return;
        }
        if !self.save_settings() {
            let _ = self.settings.set_retention(previous);
            return;
        }
        if let Some(worker) = &self.storage {
            let _ = worker.send(StorageCommand::ApplyRetention {
                retention,
                now_ms: unix_now_ms().unwrap_or_default(),
            });
        }
    }

    fn request_history_page(&mut self, offset: u32) {
        self.history_list_request_id = self.history_list_request_id.wrapping_add(1).max(1);
        self.history_loading = true;
        self.history_error = None;
        let command = StorageCommand::ListCompleted {
            request_id: self.history_list_request_id,
            limit: HISTORY_PAGE_SIZE + 1,
            offset,
        };
        if let Err(error) = self
            .storage
            .as_ref()
            .ok_or_else(|| "storage worker is unavailable".to_owned())
            .and_then(|worker| worker.send(command).map_err(|error| error.to_string()))
        {
            self.history_loading = false;
            self.history_error = Some(format!("Could not load history: {error}"));
        }
    }

    fn request_history_detail(&mut self, session_id: SessionId) {
        self.history_detail_request_id = self.history_detail_request_id.wrapping_add(1).max(1);
        self.history_loading = true;
        self.history_error = None;
        let command = StorageCommand::LoadCompleted {
            request_id: self.history_detail_request_id,
            session_id,
        };
        if let Err(error) = self
            .storage
            .as_ref()
            .ok_or_else(|| "storage worker is unavailable".to_owned())
            .and_then(|worker| worker.send(command).map_err(|error| error.to_string()))
        {
            self.history_loading = false;
            self.history_error = Some(format!("Could not load session details: {error}"));
        }
    }

    fn delete_completed_session(&mut self, session_id: SessionId) {
        self.confirm_history_delete = None;
        let command = StorageCommand::DeleteCompleted { session_id };
        match self
            .storage
            .as_ref()
            .ok_or_else(|| "storage worker is unavailable".to_owned())
            .and_then(|worker| worker.send(command).map_err(|error| error.to_string()))
        {
            Ok(()) => self.deleting_completed = Some(session_id),
            Err(error) => {
                self.history_error = Some(format!("Could not delete completed session: {error}"));
            }
        }
    }
}

impl App {
    fn render_capture_setup(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.heading("Capture setup");
            self.render_device_picker(ui);
            ui.add_space(4.0);
            self.render_keyboard_configuration(ui);
            ui.add_space(4.0);
            self.render_session_controls(ui);
            self.render_capture_status(ui);
        });
    }

    fn render_device_picker(&mut self, ui: &mut egui::Ui) {
        let configuration_locked = self.session.is_active();
        let needs_runtime_device = configuration_locked && self.selected_device.is_none();
        let picker_enabled = self.listener.is_none()
            && (!configuration_locked || needs_runtime_device)
            && !matches!(self.listener_state, ListenerState::Stopping);
        let required_name = self
            .session
            .keyboard
            .as_ref()
            .and_then(|keyboard| keyboard.display_name.as_deref());
        let mut request_scan = false;
        ui.horizontal_wrapped(|ui| {
            match &self.devices {
                None => {
                    ui.spinner();
                    ui.label("Scanning for keyboards…");
                }
                Some(devices) if devices.is_empty() => {
                    ui.label("No readable keyboards");
                }
                Some(devices) => {
                    let text = self
                        .selected_device
                        .and_then(|index| devices.get(index))
                        .map_or("Select a keyboard", |device| device.name.as_str());
                    ui.add_enabled_ui(picker_enabled, |ui| {
                        egui::ComboBox::from_label("Keyboard")
                            .selected_text(text)
                            .show_ui(ui, |ui| {
                                for (index, device) in devices.iter().enumerate() {
                                    let compatible =
                                        required_name.is_none_or(|name| name == device.name);
                                    ui.add_enabled_ui(compatible, |ui| {
                                        ui.selectable_value(
                                            &mut self.selected_device,
                                            Some(index),
                                            &device.name,
                                        )
                                        .on_hover_ui(
                                            |ui| {
                                                ui.label(format!(
                                                    "{} ({})",
                                                    device.physical_path, device.path
                                                ));
                                            },
                                        );
                                    });
                                }
                            });
                    });
                }
            }
            if ui
                .add_enabled(
                    picker_enabled && self.devices.is_some(),
                    egui::Button::new("Rescan"),
                )
                .clicked()
            {
                request_scan = true;
            }
        });
        if configuration_locked {
            ui.weak("Keyboard and XKB configuration are fixed for the active session.");
        }
        if request_scan {
            self.request_scan();
        }
    }

    fn render_keyboard_configuration(&mut self, ui: &mut egui::Ui) {
        let enabled = self.listener.is_none() && !self.session.is_active();
        let mut save_settings = false;
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal_wrapped(|ui| {
                let mut changed = false;
                egui::ComboBox::from_label("Model")
                    .width(80.0)
                    .selected_text(&self.model)
                    .show_ui(ui, |ui| {
                        for model in &self.available_models {
                            if ui
                                .selectable_value(&mut self.model, model.clone(), model)
                                .clicked()
                            {
                                changed = true;
                            }
                        }
                    });
                egui::ComboBox::from_label("Layout")
                    .selected_text(&self.layout)
                    .show_ui(ui, |ui| {
                        let mut update_variants = false;
                        for layout in &self.available_layouts {
                            if ui
                                .selectable_value(&mut self.layout, layout.clone(), layout)
                                .clicked()
                            {
                                changed = true;
                                update_variants = true;
                            }
                        }
                        if update_variants {
                            self.update_variants();
                        }
                    });
                let variant_text = if self.variant.is_empty() {
                    "Default"
                } else {
                    &self.variant
                };
                egui::ComboBox::from_label("Variant")
                    .selected_text(variant_text)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_value(&mut self.variant, String::new(), "Default")
                            .clicked()
                        {
                            changed = true;
                        }
                        for variant in &self.available_variants {
                            if !variant.is_empty()
                                && ui
                                    .selectable_value(&mut self.variant, variant.clone(), variant)
                                    .clicked()
                            {
                                changed = true;
                            }
                        }
                    });
                if changed {
                    self.reinit_xkb();
                    save_settings = true;
                }
            });
        });
        if save_settings {
            self.save_keyboard_settings();
        }
    }

    fn render_session_controls(&mut self, ui: &mut egui::Ui) {
        let busy = self.pending_finish.is_some()
            || self.discarding
            || self.deleting_all
            || matches!(self.listener_state, ListenerState::Stopping);
        ui.horizontal_wrapped(|ui| {
            if self.listener.is_some() {
                if ui
                    .add_enabled(!busy, egui::Button::new("Stop listening"))
                    .clicked()
                {
                    self.stop_listener();
                }
            } else {
                let selected_index = self.selected_device;
                let storage_ready = self.storage_tracker.status() != StorageStatus::Loading;
                if ui
                    .add_enabled(
                        selected_index.is_some() && !busy && storage_ready,
                        egui::Button::new("Start listening"),
                    )
                    .clicked()
                    && let Some(index) = selected_index
                {
                    self.begin_session_and_listen(index);
                }
            }

            if self.settings.persistence_enabled()
                && ui
                    .add_enabled(
                        self.session.is_active() && !busy,
                        egui::Button::new("Finish session"),
                    )
                    .clicked()
            {
                self.begin_finish(false);
            }
            let discard_label = if self.settings.persistence_enabled() {
                "Discard session"
            } else {
                "Discard current session"
            };
            if ui
                .add_enabled(
                    self.session.is_active()
                        && self.listener.is_none()
                        && !busy
                        && self.storage_tracker.in_flight().is_none(),
                    egui::Button::new(discard_label),
                )
                .clicked()
            {
                if self.has_samples() {
                    self.confirm_discard = true;
                } else {
                    self.begin_discard();
                }
            }
        });

        if self.confirm_discard {
            ui.colored_label(
                egui::Color32::YELLOW,
                "Discard all aggregates in the current session? This cannot be undone.",
            );
            ui.horizontal(|ui| {
                if ui.button("Discard permanently").clicked() {
                    self.begin_discard();
                }
                if ui.button("Cancel").clicked() {
                    self.confirm_discard = false;
                }
            });
        }
    }

    fn render_capture_status(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = &self.scan_error {
            ui.colored_label(egui::Color32::RED, error);
        }
        if let Some(warning) = &self.scan_warning {
            ui.colored_label(egui::Color32::YELLOW, warning);
        }
        if let Some(error) = &self.keyboard_error {
            ui.colored_label(egui::Color32::RED, error);
        }
        if let Some(error) = &self.settings_error {
            ui.colored_label(egui::Color32::RED, error);
        }
        if let Some(error) = &self.capture_error {
            ui.colored_label(egui::Color32::RED, error);
        }

        match self.listener_state {
            ListenerState::Idle => {
                if self.session.resumed {
                    ui.weak("Saved session resumed — capture is paused");
                } else {
                    ui.weak("Not listening");
                }
            }
            ListenerState::Connecting => {
                ui.label("Connecting to keyboard…");
            }
            ListenerState::Listening => {
                ui.colored_label(egui::Color32::GREEN, "Listening");
            }
            ListenerState::Stopping => {
                ui.label("Stopping listener…");
            }
            ListenerState::Failed => {
                ui.colored_label(egui::Color32::RED, "Capture stopped because of an error");
            }
        }
        if self.pending_finish.is_some() {
            ui.label("Finishing session…");
        } else if self.discarding {
            ui.label("Discarding session…");
        }
    }

    fn render_persistence_settings(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.heading("Local aggregate history");
            if self.settings.persistence_enabled() {
                ui.label("Enabled — versioned aggregate snapshots are stored locally in an unencrypted SQLite database.");
                ui.small(format!("Storage: {}", self.paths.database_file().display()));
                ui.small("Privacy details: docs/privacy.md in the evtap source distribution.");
                let mut retention = self.settings.retention();
                egui::ComboBox::from_label("Retention")
                    .selected_text(retention_label(retention))
                    .show_ui(ui, |ui| {
                        for option in [
                            RetentionPolicy::Days(30),
                            RetentionPolicy::Days(90),
                            RetentionPolicy::Days(365),
                            RetentionPolicy::Forever,
                        ] {
                            ui.selectable_value(&mut retention, option, retention_label(option));
                        }
                    });
                if retention != self.settings.retention() {
                    self.change_retention(retention);
                }
                ui.small(format!(
                    "Current disk usage: {}",
                    format_byte_size(database_disk_usage(&self.paths.database_file()))
                ));
                if ui
                    .add_enabled(
                        self.storage_tracker.status() != StorageStatus::Loading,
                        egui::Button::new(if self.history_open {
                            "Hide history"
                        } else {
                            "History"
                        }),
                    )
                    .clicked()
                {
                    self.history_open = !self.history_open;
                    if self.history_open {
                        self.request_history_page(0);
                    } else {
                        self.history_detail = None;
                    }
                }
                if ui
                    .add_enabled(
                        self.listener.is_none()
                            && self.storage_tracker.in_flight().is_none()
                            && !self.deleting_all,
                        egui::Button::new("Delete all stored analytics…"),
                    )
                    .clicked()
                {
                    self.confirm_delete_all = true;
                }
                if ui
                    .add_enabled(
                        self.listener.is_none()
                            && !self.shutting_down_storage
                            && !self.deleting_all,
                        egui::Button::new("Disable persistence…"),
                    )
                    .clicked()
                {
                    if self.session.is_active() {
                        self.disable_prompt = true;
                    } else {
                        self.disable_persistence_now();
                    }
                }
            } else {
                ui.label("Off — session aggregates remain only in memory and are discarded on exit.");
                if ui.button("Enable persistence…").clicked() {
                    self.enable_prompt = Some(EnablePrompt::Disclosure);
                }
            }
            if self.confirm_delete_all {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "Delete the active session, all completed sessions, and every aggregate snapshot? Settings are retained. This cannot be undone.",
                );
                ui.horizontal(|ui| {
                    if ui.button("Delete all analytics permanently").clicked() {
                        self.delete_all_analytics();
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_delete_all = false;
                    }
                });
            }
            if self.deleting_all {
                ui.label("Deleting all stored analytics…");
            }
            self.render_storage_status(ui);
            self.render_persistence_prompts(ui);
        });
    }

    fn render_storage_status(&mut self, ui: &mut egui::Ui) {
        let label = match self.storage_tracker.status() {
            StorageStatus::Disabled => "Persistence off",
            StorageStatus::Loading => "Loading saved session…",
            StorageStatus::Saved => "Saved",
            StorageStatus::Dirty => "Unsaved changes",
            StorageStatus::Saving => "Saving…",
            StorageStatus::Failed => "Could not save",
        };
        ui.weak(label);
        if let Some(error) = &self.storage_error {
            ui.colored_label(egui::Color32::RED, error);
        }
        if self.storage_tracker.status() == StorageStatus::Failed
            && self.settings.persistence_enabled()
            && ui.button("Retry storage operation").clicked()
        {
            if self.storage.is_none() {
                self.start_storage(if self.session.is_active() {
                    StorageOpenIntent::PreserveCurrent
                } else {
                    StorageOpenIntent::Restore
                });
            } else if self.storage_needs_reopen {
                self.storage_tracker.begin_loading();
                self.storage_open_intent = Some(if self.session.is_active() {
                    StorageOpenIntent::PreserveCurrent
                } else {
                    StorageOpenIntent::Restore
                });
                let result = self.storage.as_ref().map_or_else(
                    || Err("storage worker is unavailable".to_owned()),
                    |worker| {
                        worker
                            .send(StorageCommand::RetryOpen {
                                retention: self.settings.retention(),
                                now_ms: unix_now_ms().unwrap_or_default(),
                            })
                            .map_err(|error| error.to_string())
                    },
                );
                if let Err(error) = result {
                    self.storage_tracker.set_failed();
                    self.storage_error = Some(format!("Could not retry storage: {error}"));
                }
            } else if self.pending_finish.is_some() {
                self.request_finalize();
            } else {
                self.request_checkpoint();
            }
        }
    }

    fn render_history(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.heading("Completed-session history");
            ScrollArea::vertical()
                .id_salt("completed-session-history")
                .max_height(600.0)
                .show(ui, |ui| self.render_history_contents(ui));
        });
    }

    fn render_history_contents(&mut self, ui: &mut egui::Ui) {
        if self.history_loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading history…");
            });
        }
        if let Some(error) = &self.history_error {
            ui.colored_label(egui::Color32::RED, error);
        }

        let mut open_session = None;
        let mut request_delete = None;
        if self.history_sessions.is_empty() && !self.history_loading {
            ui.weak("No completed sessions are stored on this page.");
        }
        for summary in &self.history_sessions {
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format_local_timestamp(summary.metadata.created_at_ms));
                    ui.label(format!(
                        "Capture duration: {}",
                        format_duration_ns(summary.metadata.captured_duration_ns)
                    ));
                    ui.label(match summary.total_presses {
                        Some(count) => format!("Physical presses: {count}"),
                        None => "Physical presses unavailable".to_owned(),
                    });
                });
                ui.small(format!(
                    "{} · layout {}{}",
                    summary
                        .metadata
                        .keyboard
                        .display_name
                        .as_deref()
                        .unwrap_or("Unnamed keyboard"),
                    summary.metadata.keyboard.layout,
                    if summary.metadata.keyboard.variant.is_empty() {
                        String::new()
                    } else {
                        format!(" / {}", summary.metadata.keyboard.variant)
                    }
                ));
                ui.horizontal(|ui| {
                    if ui.button("Open").clicked() {
                        open_session = Some(summary.metadata.id);
                    }
                    if ui
                        .add_enabled(
                            self.deleting_completed != Some(summary.metadata.id),
                            egui::Button::new("Delete"),
                        )
                        .clicked()
                    {
                        request_delete = Some(summary.metadata.id);
                    }
                });
            });
            ui.add_space(4.0);
        }

        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.history_offset > 0 && !self.history_loading,
                    egui::Button::new("Previous page"),
                )
                .clicked()
            {
                self.request_history_page(self.history_offset.saturating_sub(HISTORY_PAGE_SIZE));
            }
            ui.label(format!(
                "Page {}",
                self.history_offset / HISTORY_PAGE_SIZE + 1
            ));
            if ui
                .add_enabled(
                    self.history_has_more && !self.history_loading,
                    egui::Button::new("Next page"),
                )
                .clicked()
            {
                self.request_history_page(self.history_offset.saturating_add(HISTORY_PAGE_SIZE));
            }
        });

        if let Some(session_id) = open_session {
            self.request_history_detail(session_id);
        }
        if let Some(session_id) = request_delete {
            self.confirm_history_delete = Some(session_id);
        }

        let mut close_detail = false;
        let mut delete_detail = None;
        if let Some(detail) = &self.history_detail {
            ui.separator();
            ui.heading("Completed session details");
            ui.label(format!(
                "Started: {}",
                format_local_timestamp(detail.metadata.created_at_ms)
            ));
            if let Some(completed_at_ms) = detail.metadata.completed_at_ms {
                ui.label(format!(
                    "Completed: {}",
                    format_local_timestamp(completed_at_ms)
                ));
            }
            ui.label(format!(
                "Captured: {}",
                format_duration_ns(detail.metadata.captured_duration_ns)
            ));
            ui.label(format!(
                "Keyboard: {} · model {} · layout {}{}",
                detail
                    .metadata
                    .keyboard
                    .display_name
                    .as_deref()
                    .unwrap_or("Unnamed keyboard"),
                detail.metadata.keyboard.model,
                detail.metadata.keyboard.layout,
                if detail.metadata.keyboard.variant.is_empty() {
                    String::new()
                } else {
                    format!(" / {}", detail.metadata.keyboard.variant)
                }
            ));
            ui.small(format!(
                "Captured by evtap {}",
                detail.metadata.application_version
            ));
            for message in &detail.messages {
                ui.colored_label(egui::Color32::YELLOW, message);
            }
            for metric in &detail.metrics {
                ui.group(|ui| {
                    ui.set_width(ui.available_width());
                    render_metric(ui, metric.as_ref());
                });
                ui.add_space(4.0);
            }
            ui.horizontal(|ui| {
                if ui.button("Close details").clicked() {
                    close_detail = true;
                }
                if ui.button("Delete this session").clicked() {
                    delete_detail = Some(detail.metadata.id);
                }
            });
        }
        if close_detail {
            self.history_detail = None;
        }
        if let Some(session_id) = delete_detail {
            self.confirm_history_delete = Some(session_id);
        }

        if let Some(session_id) = self.confirm_history_delete {
            ui.separator();
            ui.colored_label(
                    egui::Color32::YELLOW,
                    "Delete this completed session and all of its aggregate snapshots? This cannot be undone.",
                );
            ui.horizontal(|ui| {
                if ui.button("Delete permanently").clicked() {
                    self.delete_completed_session(session_id);
                }
                if ui.button("Cancel").clicked() {
                    self.confirm_history_delete = None;
                }
            });
        }
    }

    fn render_persistence_prompts(&mut self, ui: &mut egui::Ui) {
        match self.enable_prompt {
            Some(EnablePrompt::Disclosure) => {
                ui.separator();
                ui.colored_label(egui::Color32::YELLOW, "Review before enabling");
                ui.label("evtap will store local aggregate character labels, ranked physical keys, bigram pairs, correction pairs, counts, and timing totals. It never stores raw key events, ordered text, event timestamps, or pressed-key state.");
                ui.label("The SQLite database is local and unencrypted. Completed sessions are retained for 90 days by default and can be deleted in evtap.");
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Continue").clicked() {
                        if self.session.is_active() {
                            self.enable_prompt = Some(EnablePrompt::ExistingSession);
                        } else {
                            self.enable_persistence(StorageOpenIntent::Restore);
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        self.enable_prompt = None;
                    }
                });
            }
            Some(EnablePrompt::ExistingSession) => {
                ui.separator();
                ui.label("Choose what to do with the current in-memory session:");
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Save current session").clicked() {
                        self.enable_persistence(StorageOpenIntent::PreserveCurrent);
                    }
                    if ui
                        .add_enabled(
                            self.listener.is_none(),
                            egui::Button::new("Start persistence with a new session"),
                        )
                        .clicked()
                    {
                        self.reset_current_session();
                        self.enable_persistence(StorageOpenIntent::Restore);
                    }
                    if ui.button("Cancel").clicked() {
                        self.enable_prompt = None;
                    }
                });
                if self.listener.is_some() {
                    ui.weak("Stop capture before discarding the current session.");
                }
            }
            None => {}
        }

        if self.disable_prompt {
            ui.separator();
            ui.label("Resolve the active session before disabling persistence:");
            ui.horizontal_wrapped(|ui| {
                if ui.button("Finish session and keep history").clicked() {
                    self.disable_prompt = false;
                    self.begin_finish(true);
                }
                if ui.button("Delete all analytics and disable").clicked() {
                    self.delete_all_and_disable();
                }
                if ui.button("Cancel").clicked() {
                    self.disable_prompt = false;
                }
            });
        }
    }
}

fn font_definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    let proportional = fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default();
    if !proportional.iter().any(|font| font == HACK_FONT_NAME) {
        proportional.push(HACK_FONT_NAME.to_owned());
    }
    fonts
}

fn init_keyboard_state(model: &str, layout: &str, variant: &str) -> Result<xkb::State> {
    let context = Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = Keymap::new_from_names(
        &context,
        "",
        model,
        layout,
        variant,
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .wrap_err("failed to create XKB keymap")?;
    Ok(xkb::State::new(&keymap))
}

fn unix_now_ms() -> Result<i64, String> {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock precedes the Unix epoch".to_owned())?
        .as_millis();
    i64::try_from(milliseconds).map_err(|_| "system time exceeds the storage range".to_owned())
}

fn metric_recovery_message(issue: &MetricRecoveryIssue) -> String {
    match issue {
        MetricRecoveryIssue::Unknown { metric_id } => {
            format!("Unsupported saved metric: {metric_id}")
        }
        MetricRecoveryIssue::Duplicate { metric_id } => {
            format!("Duplicate saved metric was ignored: {metric_id}")
        }
        MetricRecoveryIssue::Invalid { metric_id, details } => {
            format!("Could not restore saved metric {metric_id}: {details}")
        }
    }
}

fn storage_operation_label(operation: StorageOperation) -> &'static str {
    match operation {
        StorageOperation::Open => "Could not open aggregate storage",
        StorageOperation::Checkpoint => "Could not save aggregate checkpoint",
        StorageOperation::Finalize => "Could not finish saved session",
        StorageOperation::Discard => "Could not discard saved session",
        StorageOperation::Retention => "Could not apply retention",
        StorageOperation::DeleteAll => "Could not delete aggregate storage",
        StorageOperation::HistoryList => "Could not load session history",
        StorageOperation::HistoryDetail => "Could not load session details",
        StorageOperation::DeleteCompleted => "Could not delete completed session",
        StorageOperation::Maintenance => "Could not reclaim deleted storage",
        StorageOperation::ShutdownCheckpoint => "Could not save final aggregate checkpoint",
    }
}

fn database_disk_usage(database_path: &Path) -> u64 {
    let sidecar = |suffix: &str| {
        let mut path = database_path.as_os_str().to_os_string();
        path.push(suffix);
        std::path::PathBuf::from(path)
    };
    [
        database_path.to_path_buf(),
        sidecar("-wal"),
        sidecar("-shm"),
    ]
    .into_iter()
    .filter_map(|path| path.metadata().ok().map(|metadata| metadata.len()))
    .fold(0_u64, u64::saturating_add)
}

fn format_byte_size(bytes: u64) -> String {
    const KIBIBYTE: u64 = 1024;
    const MEBIBYTE: u64 = 1024 * KIBIBYTE;
    if bytes >= MEBIBYTE {
        format!("{:.1} MiB", bytes as f64 / MEBIBYTE as f64)
    } else if bytes >= KIBIBYTE {
        format!("{:.1} KiB", bytes as f64 / KIBIBYTE as f64)
    } else {
        format!("{bytes} B")
    }
}

fn format_duration_ns(duration_ns: i64) -> String {
    let Ok(duration_ns) = u64::try_from(duration_ns) else {
        return "Unavailable".to_owned();
    };
    let seconds = duration_ns / 1_000_000_000;
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn format_local_timestamp(timestamp_ms: i64) -> String {
    Local
        .timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|timestamp| timestamp.format("%Y-%m-%d %H:%M:%S %:z").to_string())
        .unwrap_or_else(|| format!("{timestamp_ms} ms since Unix epoch"))
}

fn retention_label(retention: RetentionPolicy) -> &'static str {
    match retention {
        RetentionPolicy::Days(30) => "30 days",
        RetentionPolicy::Days(90) => "90 days",
        RetentionPolicy::Days(365) => "365 days",
        RetentionPolicy::Days(_) => "Unsupported",
        RetentionPolicy::Forever => "Forever",
    }
}

impl eframe::App for App {
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_scanner_events();
        self.drain_listener_events();
        self.drain_storage_events();

        let now = Instant::now();
        if self.listener_state == ListenerState::Listening && self.checkpoint_schedule.is_due(now) {
            self.request_checkpoint();
        }
        if let Some(delay) = self.checkpoint_schedule.time_until_due(now) {
            ui.ctx().request_repaint_after(delay);
        }

        egui::CentralPanel::default().show(ui, |ui| {
            ScrollArea::vertical()
                .id_salt("application-content")
                .show(ui, |ui| {
            ui.heading("evtap");
            ui.label("Understand the mechanics of your everyday typing.");
            if self.settings.persistence_enabled() {
                ui.small("Opt-in aggregate history is stored locally; raw key events and ordered text are never stored.");
            } else {
                ui.small("Persistence is off. Session aggregates stay in memory and are discarded when evtap exits.");
            }
            ui.add_space(8.0);

            self.render_capture_setup(ui);
            ui.add_space(8.0);
            self.render_persistence_settings(ui);
            ui.add_space(8.0);
            if self.history_open {
                self.render_history(ui);
                ui.add_space(8.0);
            }

            for message in &self.recovery_messages {
                ui.colored_label(egui::Color32::YELLOW, message);
            }
            ui.heading("Session analytics");
            ui.small("Timing tables appear as samples arrive; no raw keystroke history is saved.");
            ui.add_space(4.0);

            for metric in &self.metrics {
                ui.group(|ui| {
                    ui.set_width(ui.available_width());
                    render_metric(ui, metric.as_ref());
                });
                ui.add_space(6.0);
            }
                });
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(listener) = &self.listener {
            let _ = listener.stop();
            let deadline = Instant::now() + LISTENER_EXIT_WAIT;
            while self.listener.is_some() && Instant::now() < deadline {
                self.drain_listener_events();
                thread::sleep(Duration::from_millis(10));
            }
        }
        self.session.finish_capture_segment();

        let final_checkpoint = if self.settings.persistence_enabled() && self.session.is_active() {
            let generation = self.storage_tracker.mark_dirty().ok();
            generation.and_then(|generation| {
                self.session_snapshot()
                    .ok()
                    .map(|snapshot| CheckpointRequest {
                        generation,
                        snapshot,
                    })
            })
        } else {
            None
        };
        if let Some(storage) = self.storage.take() {
            match storage.shutdown(final_checkpoint) {
                Ok(result) if result.final_checkpoint_saved => {
                    info!("storage worker shut down after saving aggregate state");
                }
                Ok(_) => warn!("storage worker shut down without a final aggregate checkpoint"),
                Err(error) => warn!(%error, "storage worker shutdown did not complete cleanly"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        database_disk_usage, font_definitions, format_byte_size, format_duration_ns,
        format_local_timestamp, retention_label, unix_now_ms,
    };
    use crate::settings::RetentionPolicy;
    use eframe::egui;

    #[test]
    fn proportional_font_family_supports_bigram_arrow() {
        let context = egui::Context::default();
        context.set_fonts(font_definitions());
        let mut has_arrow = false;

        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            has_arrow =
                ui.fonts_mut(|fonts| fonts.has_glyph(&egui::FontId::proportional(14.0), '→'));
        });

        assert!(has_arrow);
    }

    #[test]
    fn persistence_helpers_use_bounded_values() {
        assert!(unix_now_ms().unwrap() > 0);
        assert_eq!(retention_label(RetentionPolicy::Days(90)), "90 days");
        assert_eq!(retention_label(RetentionPolicy::Forever), "Forever");
        assert_eq!(format_duration_ns(3_661_000_000_000), "01:01:01");
        assert_eq!(format_duration_ns(-1), "Unavailable");
        assert_eq!(format_byte_size(1_536), "1.5 KiB");
        assert!(!format_local_timestamp(0).is_empty());
    }

    #[test]
    fn disk_usage_includes_sqlite_sidecars() {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("evtap.sqlite3");
        fs::write(&database, [0_u8; 5]).unwrap();
        fs::write(temporary.path().join("evtap.sqlite3-wal"), [0_u8; 7]).unwrap();

        assert_eq!(database_disk_usage(&database), 12);
    }
}
