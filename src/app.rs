use std::{
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use color_eyre::{Result, eyre::ContextCompat};
use eframe::egui::{self, ScrollArea};
use evdev::KeyCode;
use tracing::{error, info, warn};
use xkbcommon::xkb::{self, Context, Keymap};

use crate::{
    input::{KeyEvent, KeyEventKind, KeyRole},
    listener::{self, ListenerHandle},
    metric_view::render_metric,
    paths::AppPaths,
    scanner::{self, DeviceMetadata, ScannerHandle},
    session::{
        KeyboardContext, MetricRecoveryIssue, SessionId, SessionMetadata, SessionSnapshot,
        StoredSession,
    },
    settings::{Settings, SettingsStore},
    storage::{
        CheckpointSchedule, DirtyTracker, SaveRequest, StorageCommand, StorageEvent,
        StorageOperation, StorageStatus, StorageWorker,
    },
    wake::WakeSignal,
    xkb_helper,
};

mod view;
mod working_session;

use working_session::WorkingSession;

const HACK_FONT_NAME: &str = "Hack";
const LISTENER_EXIT_WAIT: Duration = Duration::from_millis(500);
const MAX_SESSION_NAME_BYTES: usize = 80;

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
    initial_storage_open_handled: bool,
    storage_tracker: DirtyTracker,
    checkpoint_schedule: CheckpointSchedule,
    storage_error: Option<String>,
    last_failed_operation: Option<StorageOperation>,
    sessions: Vec<SessionMetadata>,
    list_request_id: u64,
    load_request_id: u64,
    loading_session: bool,

    working_session: WorkingSession,
    recovery_messages: Vec<String>,
    pending_boundary_after_save: Option<BoundaryTarget>,
    pending_boundary_after_stop: Option<BoundaryTarget>,
    boundary_prompt: Option<BoundaryTarget>,
    disclosure_prompt: Option<DisclosureIntent>,
    allow_close: bool,

    rename_open: bool,
    rename_buffer: String,
    rename_error: Option<String>,
    confirm_reset: bool,
    confirm_delete: bool,
    deleting_session: bool,
    confirm_delete_all: bool,
    deleting_all: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ListenerState {
    Idle,
    Connecting,
    Listening,
    Stopping,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoundaryTarget {
    New,
    Load(SessionId),
    Exit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DisclosureIntent {
    Save(Option<BoundaryTarget>),
    EnableAutosave,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoundaryPolicy {
    Proceed,
    Save,
    Prompt,
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
                    "Could not load settings; safe defaults are in use and the existing file will not be overwritten: {error}"
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
        let now_ms = unix_now_ms().unwrap_or_default();
        let working_session = WorkingSession::untitled(
            now_ms,
            KeyboardContext {
                display_name: None,
                model: model.clone(),
                layout: layout.clone(),
                variant: variant.clone(),
            },
        );
        let mut storage_tracker = DirtyTracker::default();
        storage_tracker.begin_loading();
        let storage = StorageWorker::spawn(
            paths.database_file(),
            settings.last_session_id(),
            wake_signal.clone(),
        )?;

        Ok(Self {
            devices: None,
            selected_device: None,
            scan_warning: None,
            scan_error: None,
            scanner,
            listener: None,
            listener_state: ListenerState::Idle,
            capture_error: None,
            wake_signal,
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
            storage: Some(storage),
            initial_storage_open_handled: false,
            storage_tracker,
            checkpoint_schedule: CheckpointSchedule::default(),
            storage_error: None,
            last_failed_operation: None,
            sessions: Vec::new(),
            list_request_id: 0,
            load_request_id: 0,
            loading_session: false,
            working_session,
            recovery_messages: Vec::new(),
            pending_boundary_after_save: None,
            pending_boundary_after_stop: None,
            boundary_prompt: None,
            disclosure_prompt: None,
            allow_close: false,
            rename_open: false,
            rename_buffer: String::new(),
            rename_error: None,
            confirm_reset: false,
            confirm_delete: false,
            deleting_session: false,
            confirm_delete_all: false,
            deleting_all: false,
        })
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
                        self.select_remembered_device();
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

    fn select_remembered_device(&mut self) {
        let Some(name) = self.working_session.keyboard.display_name.as_deref() else {
            return;
        };
        let Some(devices) = &self.devices else {
            return;
        };
        let mut matches = devices
            .iter()
            .enumerate()
            .filter(|(_, device)| device.name == name)
            .map(|(index, _)| index);
        let first = matches.next();
        self.selected_device = if first.is_some() && matches.next().is_none() {
            first
        } else {
            None
        };
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
                    self.working_session.start_capture();
                    info!("listener connected to keyboard");
                }
                listener::Event::Stopped { reason } => {
                    let is_error = reason.is_error();
                    let message = reason.to_string();
                    self.listener = None;
                    if self.working_session.finish_capture_segment() {
                        self.note_session_dirty();
                    }
                    self.clear_in_flight();
                    self.listener_state = if is_error {
                        self.capture_error = Some(message.clone());
                        ListenerState::Failed
                    } else {
                        self.capture_error = None;
                        ListenerState::Idle
                    };
                    if let Some(target) = self.pending_boundary_after_stop.take() {
                        self.continue_boundary(target);
                    } else if self.settings.autosave_enabled() && self.working_dirty() {
                        self.request_save(None);
                    }
                    info!(%message, "listener stopped");
                }
                listener::Event::Input {
                    timestamp,
                    key_code,
                    kind,
                } => self.process_input(timestamp, key_code, kind),
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

        let key = self.working_session.physical_key(code, || {
            let debug_name = format!("{key_code:?}");
            debug_name
                .strip_prefix("KEY_")
                .unwrap_or(&debug_name)
                .to_owned()
        });
        let role = if key_code == KeyCode::KEY_BACKSPACE {
            KeyRole::Backspace
        } else {
            KeyRole::Other
        };
        let event = KeyEvent::new(key, text, timestamp, kind, role);
        self.working_session.process(&event);
        self.note_session_dirty();
    }

    fn clear_in_flight(&mut self) {
        self.working_session.clear_in_flight();
        self.reinit_xkb();
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
    }

    fn handle_storage_event(&mut self, event: StorageEvent) {
        match event {
            StorageEvent::Opened { sessions, selected } => {
                let first_open = !self.initial_storage_open_handled;
                self.initial_storage_open_handled = true;
                self.sessions = sessions;
                self.storage_error = None;
                self.last_failed_operation = None;
                if first_open {
                    if let Some(selected) = selected {
                        self.restore_session(selected);
                        self.storage_tracker.reset_saved();
                    } else {
                        self.storage_tracker.reset_unsaved();
                        if self.settings.last_session_id().is_some() {
                            self.settings.set_last_session_id(None);
                            self.save_settings();
                        }
                    }
                } else {
                    if self.working_session.id.is_some() {
                        self.storage_tracker.reset_saved();
                    } else {
                        self.storage_tracker.reset_unsaved();
                    }
                    if self.working_session.id.is_none() && self.session_has_content() {
                        self.note_session_dirty();
                    }
                }
            }
            StorageEvent::Saved {
                generation,
                session_id,
            } => {
                self.working_session.id = Some(session_id);
                if let Err(error) = self.storage_tracker.acknowledge(generation) {
                    self.storage_error = Some(format!("Unexpected save acknowledgement: {error}"));
                    self.storage_tracker.set_failed();
                    self.pending_boundary_after_save = None;
                    return;
                }
                self.settings.set_last_session_id(Some(session_id));
                self.save_settings();
                self.storage_error = None;
                self.last_failed_operation = None;
                self.request_session_list();
                if self.storage_tracker.is_dirty() {
                    if let Some(target) = self.pending_boundary_after_save {
                        self.begin_save(Some(target));
                    }
                } else if let Some(target) = self.pending_boundary_after_save.take() {
                    self.execute_boundary(target);
                }
            }
            StorageEvent::SessionsListed {
                request_id,
                sessions,
            } => {
                if request_id == self.list_request_id {
                    self.sessions = sessions;
                }
            }
            StorageEvent::SessionLoaded {
                request_id,
                session,
            } => {
                if request_id != self.load_request_id {
                    return;
                }
                self.loading_session = false;
                match session {
                    Some(session) => {
                        let id = session.metadata.id;
                        self.restore_session(session);
                        self.storage_tracker.reset_saved();
                        self.settings.set_last_session_id(Some(id));
                        self.save_settings();
                        self.request_session_list();
                    }
                    None => {
                        self.new_working_session();
                        self.storage_error = Some(
                            "The selected session no longer exists; started an untitled session."
                                .to_owned(),
                        );
                    }
                }
            }
            StorageEvent::SessionDeleted {
                session_id,
                deleted,
            } => {
                self.deleting_session = false;
                if deleted && self.working_session.id == Some(session_id) {
                    self.new_working_session();
                }
                self.request_session_list();
            }
            StorageEvent::AllDeleted => {
                self.deleting_all = false;
                self.sessions.clear();
                self.new_working_session();
            }
            StorageEvent::Failed(failure) => {
                let label = storage_operation_label(failure.operation);
                self.storage_error = Some(format!(
                    "{label} at {}: {}",
                    failure.database_path.display(),
                    failure.details
                ));
                self.last_failed_operation = Some(failure.operation);
                if let Some(generation) = failure.generation {
                    let _ = self.storage_tracker.fail(generation);
                    self.pending_boundary_after_save = None;
                } else if failure.operation == StorageOperation::Open {
                    self.initial_storage_open_handled = true;
                    self.storage_tracker.set_failed();
                }
                if failure.operation == StorageOperation::Load {
                    self.loading_session = false;
                }
                if failure.operation == StorageOperation::Delete {
                    self.deleting_session = false;
                }
                if failure.operation == StorageOperation::DeleteAll {
                    self.deleting_all = false;
                }
            }
            StorageEvent::ShutdownComplete { .. } => {}
        }
    }

    fn restore_session(&mut self, stored: StoredSession) {
        let keyboard = stored.metadata.keyboard.clone();
        let (working_session, recovery_issues) = WorkingSession::restore(stored);
        self.recovery_messages = recovery_issues
            .iter()
            .map(metric_recovery_message)
            .collect();
        self.model.clone_from(&keyboard.model);
        self.layout.clone_from(&keyboard.layout);
        self.variant.clone_from(&keyboard.variant);
        self.available_variants = xkb_helper::get_variants(&self.layout);
        self.reinit_xkb();
        self.working_session = working_session;
        self.listener = None;
        self.listener_state = ListenerState::Idle;
        self.clear_in_flight();
        self.select_remembered_device();
    }

    fn new_working_session(&mut self) {
        let now_ms = unix_now_ms().unwrap_or_default();
        self.working_session = WorkingSession::untitled(
            now_ms,
            KeyboardContext {
                display_name: None,
                model: self.model.clone(),
                layout: self.layout.clone(),
                variant: self.variant.clone(),
            },
        );
        self.listener = None;
        self.listener_state = ListenerState::Idle;
        self.storage_tracker.reset_unsaved();
        self.checkpoint_schedule.clear();
        self.recovery_messages.clear();
        self.settings.set_last_session_id(None);
        self.save_settings();
    }

    fn note_session_dirty(&mut self) {
        if let Err(error) = self.storage_tracker.mark_dirty() {
            self.storage_error = Some(format!("Could not track unsaved changes: {error}"));
            return;
        }
        if self.settings.autosave_enabled() {
            self.checkpoint_schedule.note_dirty(Instant::now());
        }
    }

    fn working_dirty(&self) -> bool {
        if self.working_session.id.is_some() {
            self.storage_tracker.is_dirty()
        } else {
            self.session_has_content()
        }
    }

    fn session_has_content(&self) -> bool {
        self.working_session.has_content()
    }

    fn request_save(&mut self, after: Option<BoundaryTarget>) {
        if !self.settings.storage_disclosure_acknowledged() {
            self.disclosure_prompt = Some(DisclosureIntent::Save(after));
            return;
        }
        self.begin_save(after);
    }

    fn begin_save(&mut self, after: Option<BoundaryTarget>) {
        if self.storage_tracker.in_flight().is_some() {
            if after.is_some() {
                self.pending_boundary_after_save = after;
            }
            return;
        }
        if self.working_session.id.is_none()
            && matches!(
                self.storage_tracker.status(),
                StorageStatus::Failed | StorageStatus::Saved
            )
        {
            let _ = self.storage_tracker.mark_dirty();
        }
        let generation = match self.storage_tracker.begin_save() {
            Ok(Some(generation)) => generation,
            Ok(None) => {
                if let Some(target) = after {
                    self.execute_boundary(target);
                }
                return;
            }
            Err(error) => {
                self.storage_error = Some(format!("Could not begin save: {error}"));
                return;
            }
        };
        let snapshot = match self.session_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let _ = self.storage_tracker.fail(generation);
                self.storage_error = Some(error);
                return;
            }
        };
        self.pending_boundary_after_save = after;
        let command = StorageCommand::Save(SaveRequest {
            generation,
            snapshot,
        });
        match self.storage.as_ref().map(|worker| worker.send(command)) {
            Some(Ok(())) => self.checkpoint_schedule.save_started(),
            Some(Err(error)) => {
                let _ = self.storage_tracker.fail(generation);
                self.storage_error = Some(format!("Could not request save: {error}"));
                self.pending_boundary_after_save = None;
            }
            None => {
                let _ = self.storage_tracker.fail(generation);
                self.storage_error = Some("Storage worker is unavailable".to_owned());
                self.pending_boundary_after_save = None;
            }
        }
    }

    fn session_snapshot(&self) -> Result<SessionSnapshot, String> {
        self.working_session.snapshot(unix_now_ms()?)
    }

    fn request_session_list(&mut self) {
        self.list_request_id = self.list_request_id.wrapping_add(1);
        if let Some(worker) = &self.storage {
            let _ = worker.send(StorageCommand::ListSessions {
                request_id: self.list_request_id,
            });
        }
    }

    fn request_boundary(&mut self, target: BoundaryTarget) {
        if self.listener.is_some() {
            self.pending_boundary_after_stop = Some(target);
            self.stop_listener();
        } else {
            self.clear_in_flight();
            self.continue_boundary(target);
        }
    }

    fn continue_boundary(&mut self, target: BoundaryTarget) {
        match boundary_policy(self.working_dirty(), self.settings.autosave_enabled()) {
            BoundaryPolicy::Proceed => self.execute_boundary(target),
            BoundaryPolicy::Save => self.request_save(Some(target)),
            BoundaryPolicy::Prompt => self.boundary_prompt = Some(target),
        }
    }

    fn execute_boundary(&mut self, target: BoundaryTarget) {
        self.pending_boundary_after_save = None;
        self.boundary_prompt = None;
        match target {
            BoundaryTarget::New => self.new_working_session(),
            BoundaryTarget::Load(session_id) => {
                self.load_request_id = self.load_request_id.wrapping_add(1);
                self.loading_session = true;
                let command = StorageCommand::LoadSession {
                    request_id: self.load_request_id,
                    session_id,
                    opened_at_ms: unix_now_ms().unwrap_or_default(),
                };
                if let Some(worker) = &self.storage
                    && let Err(error) = worker.send(command)
                {
                    self.loading_session = false;
                    self.storage_tracker.set_failed();
                    self.storage_error = Some(format!("Could not request session load: {error}"));
                }
            }
            BoundaryTarget::Exit => {
                self.allow_close = true;
            }
        }
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

    fn begin_listening(&mut self, device_index: usize) {
        let Some(device) = self
            .devices
            .as_ref()
            .and_then(|devices| devices.get(device_index))
            .cloned()
        else {
            return;
        };
        self.working_session.keyboard = KeyboardContext {
            display_name: Some(device.name.clone()),
            model: self.model.clone(),
            layout: self.layout.clone(),
            variant: self.variant.clone(),
        };
        self.working_session.last_opened_at_ms =
            unix_now_ms().unwrap_or(self.working_session.last_opened_at_ms);
        self.working_session.restored = false;
        self.note_session_dirty();
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
                if self.working_session.finish_capture_segment() {
                    self.note_session_dirty();
                }
                self.clear_in_flight();
                self.listener_state = ListenerState::Failed;
                self.capture_error = Some(format!("Could not stop listener: {error:#}"));
                if let Some(target) = self.pending_boundary_after_stop.take() {
                    self.continue_boundary(target);
                } else if self.settings.autosave_enabled() && self.working_dirty() {
                    self.request_save(None);
                }
            }
            None => {}
        }
    }

    fn apply_rename(&mut self) {
        let trimmed = self.rename_buffer.trim();
        if trimmed.len() > MAX_SESSION_NAME_BYTES {
            self.rename_error = Some("Session name is longer than 80 UTF-8 bytes.".to_owned());
            return;
        }
        let name = (!trimmed.is_empty()).then(|| trimmed.to_owned());
        if let Some(name) = &name
            && self.sessions.iter().any(|session| {
                Some(session.id) != self.working_session.id && session.name.as_ref() == Some(name)
            })
        {
            self.rename_error = Some("Another saved session already uses that name.".to_owned());
            return;
        }
        if self.working_session.name != name {
            self.working_session.name = name;
            self.note_session_dirty();
        }
        self.rename_open = false;
        self.rename_error = None;
    }

    fn reset_statistics(&mut self) {
        self.working_session.reset_statistics();
        self.note_session_dirty();
        self.confirm_reset = false;
    }

    fn delete_current_session(&mut self) {
        self.confirm_delete = false;
        let Some(session_id) = self.working_session.id else {
            self.new_working_session();
            return;
        };
        self.deleting_session = true;
        if let Some(worker) = &self.storage
            && let Err(error) = worker.send(StorageCommand::DeleteSession { session_id })
        {
            self.deleting_session = false;
            self.storage_error = Some(format!("Could not request session deletion: {error}"));
        }
    }

    fn delete_all_sessions(&mut self) {
        self.confirm_delete_all = false;
        self.deleting_all = true;
        if let Some(worker) = &self.storage
            && let Err(error) = worker.send(StorageCommand::DeleteAll)
        {
            self.deleting_all = false;
            self.storage_error = Some(format!("Could not request complete deletion: {error}"));
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        let close_requested = ctx.input(|input| input.viewport().close_requested());
        if close_requested && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.boundary_prompt.is_none()
                && self.disclosure_prompt.is_none()
                && self.pending_boundary_after_save.is_none()
            {
                self.request_boundary(BoundaryTarget::Exit);
            }
        }
        if self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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
        StorageOperation::Open => "Could not open session storage",
        StorageOperation::Save => "Could not save session",
        StorageOperation::List => "Could not list saved sessions",
        StorageOperation::Load => "Could not load session",
        StorageOperation::Delete => "Could not delete session",
        StorageOperation::DeleteAll => "Could not delete saved sessions",
        StorageOperation::Maintenance => "Could not reclaim deleted storage",
        StorageOperation::ShutdownSave => "Could not save before shutdown",
    }
}

fn boundary_policy(dirty: bool, autosave: bool) -> BoundaryPolicy {
    match (dirty, autosave) {
        (false, _) => BoundaryPolicy::Proceed,
        (true, true) => BoundaryPolicy::Save,
        (true, false) => BoundaryPolicy::Prompt,
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
        if self.settings.autosave_enabled()
            && self.listener_state == ListenerState::Listening
            && self.checkpoint_schedule.is_due(now)
        {
            self.request_save(None);
        }
        if let Some(delay) = self.checkpoint_schedule.time_until_due(now) {
            ui.ctx().request_repaint_after(delay);
        }

        self.handle_close_request(ui.ctx());
        egui::CentralPanel::default().show(ui, |ui| {
            ScrollArea::vertical()
                .id_salt("application-content")
                .show(ui, |ui| {
                    ui.heading("evtap");
                    ui.label("Understand the mechanics of your everyday typing.");
                    ui.small("Sessions always run in memory. Save manually or enable autosave to back aggregate state with local, unencrypted storage.");
                    ui.add_space(8.0);

                    self.render_session_management(ui);
                    ui.add_space(8.0);
                    self.render_capture_setup(ui);
                    ui.add_space(8.0);

                    for message in &self.recovery_messages {
                        ui.colored_label(egui::Color32::YELLOW, message);
                    }
                    ui.heading("Session analytics");
                    ui.small("Only aggregate snapshots are saved; input sequences and unfinished event context remain in memory.");
                    ui.add_space(4.0);
                    for metric in &self.working_session.metrics {
                        ui.group(|ui| {
                            ui.set_width(ui.available_width());
                            render_metric(ui, metric.as_ref());
                        });
                        ui.add_space(6.0);
                    }
                });
        });
        self.render_prompts(ui.ctx());
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
        if self.working_session.finish_capture_segment() {
            self.note_session_dirty();
        }
        self.clear_in_flight();

        let final_save = if self.settings.autosave_enabled() && self.working_dirty() {
            self.storage_tracker
                .begin_save()
                .ok()
                .flatten()
                .and_then(|generation| {
                    self.session_snapshot().ok().map(|snapshot| SaveRequest {
                        generation,
                        snapshot,
                    })
                })
        } else {
            None
        };
        if let Some(storage) = self.storage.take() {
            match storage.shutdown(final_save) {
                Ok(result) if result.final_save_succeeded => {
                    info!("storage worker shut down after saving aggregate state");
                }
                Ok(_) => warn!("storage worker shut down without a final save"),
                Err(error) => warn!(%error, "storage worker shutdown did not complete cleanly"),
            }
        }
    }
}

#[cfg(test)]
mod tests;
