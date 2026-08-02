use std::{
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use color_eyre::{Result, eyre::ContextCompat};
use eframe::egui;
use evdev::KeyCode;
use tracing::{error, info, warn};
use xkbcommon::xkb::{self, Context, Keymap};

use crate::{
    input::{KeyEvent, KeyEventKind, KeyRole},
    listener::{self, ListenerHandle},
    paths::AppPaths,
    scanner::{self, DeviceMetadata, ScannerHandle},
    session::{
        KeyboardContext, MetricRecoveryIssue, SessionId, SessionMetadata, SessionSnapshot,
        StoredSession,
    },
    settings::{Settings, SettingsStore},
    storage::{
        CheckpointSchedule, DirtyTracker, SaveRequest, SessionListOrder, StorageCommand,
        StorageEvent, StorageOperation, StorageStatus, StorageWorker,
    },
    wake::WakeSignal,
    xkb_helper,
};

pub(crate) mod view;
mod working_session;

use working_session::WorkingSession;

const LISTENER_EXIT_WAIT: Duration = Duration::from_millis(500);
const MAX_SESSION_NAME_BYTES: usize = 80;

pub struct App {
    view: AppView,
    timing_view: TimingView,
    session_switcher_open: bool,

    devices: Option<Vec<DeviceMetadata>>,
    selected_device: Option<usize>,
    scan_warning: Option<ScanWarning>,
    scan_error: Option<String>,
    scanner: ScannerHandle,
    select_remembered_after_scan: bool,
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
    storage_error_details: Option<String>,
    last_failed_operation: Option<StorageOperation>,
    failed_list_order: Option<SessionListOrder>,
    sessions: Vec<SessionMetadata>,
    managed_sessions: Vec<SessionMetadata>,
    next_list_request_id: u64,
    session_list_request_id: Option<u64>,
    manage_list_request_id: Option<u64>,
    manage_list_loading: bool,
    load_request_id: u64,
    loading_session: bool,

    working_session: WorkingSession,
    recovery_messages: Vec<String>,
    session_notice: Option<String>,
    pending_boundary_after_save: Option<BoundaryTarget>,
    pending_boundary_after_stop: Option<BoundaryTarget>,
    pending_boundary_opener: Option<egui::Id>,
    boundary_prompt: Option<BoundaryTarget>,
    disclosure_prompt: Option<DisclosureIntent>,
    allow_close: bool,

    rename_dialog: Option<RenameDialog>,
    rename_request_id: u64,
    prompt_opener: Option<egui::Id>,
    prompt_needs_focus: bool,
    focus_after_prompt: Option<egui::Id>,
    focus_renamed_session: Option<SessionId>,
    confirm_reset: bool,
    confirm_delete: Option<DeleteSessionPrompt>,
    deleting_session: bool,
    confirm_delete_all: bool,
    deleting_all: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum AppView {
    Overview,
    KeyUsage,
    Timing(TimingView),
    Corrections,
    Sessions,
    Settings(SettingsSection),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum TimingView {
    Dwell,
    Flight,
    Bigrams,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum SettingsSection {
    Input,
    KeyboardInterpretation,
    StoragePrivacy,
    Appearance,
    About,
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
enum ScanWarning {
    NoKeyboardDetected,
    PermissionDenied {
        count: usize,
    },
    Unavailable {
        count: usize,
    },
    Incomplete {
        issue_count: usize,
        permission_denied: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoundaryTarget {
    New,
    Load(SessionId),
    Exit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RenameTarget {
    Current,
    Saved(SessionId),
}

struct RenameDialog {
    target: RenameTarget,
    buffer: String,
    error: Option<String>,
    request_id: Option<u64>,
    submitting: bool,
    focus_text: bool,
    opener: egui::Id,
}

struct DeleteSessionPrompt {
    session_id: Option<SessionId>,
    display_name: String,
    current: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DisclosureIntent {
    Save(Option<BoundaryTarget>),
    EnableAutosave,
    Review,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoundaryPolicy {
    Proceed,
    Save,
    Prompt,
}

impl App {
    pub fn new(creation_context: &eframe::CreationContext<'_>, paths: AppPaths) -> Result<Self> {
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
        view::theme::install(&creation_context.egui_ctx, settings.appearance_preference());
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
            view: AppView::Overview,
            timing_view: TimingView::Dwell,
            session_switcher_open: false,
            devices: None,
            selected_device: None,
            scan_warning: None,
            scan_error: None,
            scanner,
            select_remembered_after_scan: true,
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
            storage_error_details: None,
            last_failed_operation: None,
            failed_list_order: None,
            sessions: Vec::new(),
            managed_sessions: Vec::new(),
            next_list_request_id: 0,
            session_list_request_id: None,
            manage_list_request_id: None,
            manage_list_loading: false,
            load_request_id: 0,
            loading_session: false,
            working_session,
            recovery_messages: Vec::new(),
            session_notice: None,
            pending_boundary_after_save: None,
            pending_boundary_after_stop: None,
            pending_boundary_opener: None,
            boundary_prompt: None,
            disclosure_prompt: None,
            allow_close: false,
            rename_dialog: None,
            rename_request_id: 0,
            prompt_opener: None,
            prompt_needs_focus: false,
            focus_after_prompt: None,
            focus_renamed_session: None,
            confirm_reset: false,
            confirm_delete: None,
            deleting_session: false,
            confirm_delete_all: false,
            deleting_all: false,
        })
    }

    fn request_scan(&mut self) {
        self.request_scan_with_remembered_selection(true);
    }

    fn request_recovery_scan(&mut self) {
        self.request_scan_with_remembered_selection(false);
    }

    fn request_scan_with_remembered_selection(&mut self, select_remembered: bool) {
        self.devices = None;
        self.selected_device = None;
        self.scan_warning = None;
        self.scan_error = None;
        self.select_remembered_after_scan = select_remembered;
        if let Err(error) = self.scanner.start_scan() {
            self.devices = Some(Vec::new());
            self.scan_error = Some(format!("Could not start device scan: {error:#}"));
        }
    }

    fn apply_scan_report(&mut self, report: scanner::ScanReport) {
        let issue_count = report.issues.len();
        let permission_denied = report
            .issues
            .iter()
            .filter(|issue| issue.kind == scanner::DeviceScanIssueKind::PermissionDenied)
            .count();
        self.scan_warning = if report.devices.is_empty() {
            if permission_denied > 0 {
                Some(ScanWarning::PermissionDenied {
                    count: permission_denied,
                })
            } else if issue_count > 0 {
                Some(ScanWarning::Unavailable { count: issue_count })
            } else {
                Some(ScanWarning::NoKeyboardDetected)
            }
        } else if issue_count > 0 {
            Some(ScanWarning::Incomplete {
                issue_count,
                permission_denied,
            })
        } else {
            None
        };
        self.scan_error = None;
        self.devices = Some(report.devices);
        if self.select_remembered_after_scan {
            self.select_remembered_device();
        } else {
            self.selected_device = None;
            self.select_remembered_after_scan = true;
        }
    }

    fn drain_scanner_events(&mut self) {
        while let Some(event) = self.scanner.try_recv_event() {
            match event {
                scanner::Event::ScanFinished { result } => match result {
                    Ok(report) => self.apply_scan_report(report),
                    Err(error) => {
                        self.devices = Some(Vec::new());
                        self.selected_device = None;
                        self.scan_warning = None;
                        self.scan_error = Some(format!("Device scan failed: {error}"));
                        self.select_remembered_after_scan = true;
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
                    self.finish_listener_stop(message.clone(), is_error);
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

    fn finish_listener_stop(&mut self, message: String, is_error: bool) {
        self.listener = None;
        if self.working_session.finish_capture_segment() {
            self.note_session_dirty();
        }
        self.clear_in_flight();
        self.listener_state = if is_error {
            self.capture_error = Some(message);
            self.request_recovery_scan();
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
                    self.storage_error = Some("The local storage worker stopped unexpectedly. Unsaved changes remain in memory.".to_owned());
                    self.storage_error_details = Some(format!("Storage worker stopped: {error}"));
                    self.last_failed_operation = Some(StorageOperation::Maintenance);
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
                self.clear_storage_failure(StorageOperation::Open);
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
                self.clear_storage_failure(StorageOperation::Save);
                self.clear_storage_failure(StorageOperation::ShutdownSave);
                self.refresh_session_lists();
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
                let matched_order = if self.session_list_request_id == Some(request_id) {
                    self.sessions = sessions;
                    self.session_list_request_id = None;
                    Some(SessionListOrder::LastOpened)
                } else if self.manage_list_request_id == Some(request_id) {
                    self.managed_sessions = sessions;
                    self.manage_list_request_id = None;
                    self.manage_list_loading = false;
                    Some(SessionListOrder::LastUpdated)
                } else {
                    None
                };
                if matched_order.is_some() && self.failed_list_order == matched_order {
                    self.clear_storage_failure(StorageOperation::List);
                }
            }
            StorageEvent::SessionListFailed {
                request_id,
                order,
                failure,
            } => self.handle_session_list_failed(request_id, order, failure),
            StorageEvent::SessionLoaded {
                request_id,
                session,
            } => {
                if request_id != self.load_request_id {
                    return;
                }
                self.loading_session = false;
                self.clear_storage_failure(StorageOperation::Load);
                match session {
                    Some(session) => {
                        let id = session.metadata.id;
                        self.restore_session(session);
                        self.storage_tracker.reset_saved();
                        self.settings.set_last_session_id(Some(id));
                        self.save_settings();
                        self.refresh_session_lists();
                    }
                    None => {
                        self.new_working_session();
                        self.session_notice = Some(
                            "Started an untitled session. The missing saved session and all other saved data were left unchanged."
                                .to_owned(),
                        );
                    }
                }
            }
            StorageEvent::SessionRenamed {
                request_id,
                session,
            } => self.handle_session_renamed(request_id, session),
            StorageEvent::SessionRenameFailed {
                request_id,
                session_id,
                failure,
            } => self.handle_session_rename_failed(request_id, session_id, failure),
            StorageEvent::SessionDeleted {
                session_id,
                deleted,
            } => {
                self.deleting_session = false;
                self.clear_storage_failure(StorageOperation::Delete);
                if deleted && self.working_session.id == Some(session_id) {
                    self.new_working_session();
                }
                self.refresh_session_lists();
            }
            StorageEvent::AllDeleted => {
                self.deleting_all = false;
                self.clear_storage_failure(StorageOperation::DeleteAll);
                self.sessions.clear();
                self.managed_sessions.clear();
                if self.working_session.id.is_some() {
                    self.new_working_session();
                }
            }
            StorageEvent::Failed(failure) => {
                self.storage_error = Some(match failure.operation {
                    StorageOperation::Open => "evtap could not open local storage. In-memory capture remains available.",
                    StorageOperation::Save | StorageOperation::ShutdownSave => "The active session could not be saved. Unsaved changes remain in memory.",
                    StorageOperation::List => "Saved session metadata could not be refreshed.",
                    StorageOperation::Load => "The selected session could not be loaded. The current session remains active.",
                    StorageOperation::Rename => "The session name could not be saved.",
                    StorageOperation::Delete => "The selected saved session could not be deleted.",
                    StorageOperation::DeleteAll => "Saved sessions could not be deleted.",
                    StorageOperation::Maintenance => "Local storage maintenance could not be completed.",
                }.to_owned());
                self.storage_error_details = Some(format!(
                    "{} at {}: {}",
                    storage_operation_label(failure.operation),
                    failure.database_path.display(),
                    failure.details
                ));
                self.last_failed_operation = Some(failure.operation);
                if failure.operation != StorageOperation::List {
                    self.failed_list_order = None;
                }
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

    fn clear_storage_failure(&mut self, operation: StorageOperation) {
        if self.last_failed_operation == Some(operation) {
            self.storage_error = None;
            self.storage_error_details = None;
            self.last_failed_operation = None;
            if operation == StorageOperation::List {
                self.failed_list_order = None;
            }
        }
    }

    fn restore_session(&mut self, stored: StoredSession) {
        let keyboard = stored.metadata.keyboard.clone();
        let (working_session, recovery_issues) = WorkingSession::restore(stored);
        self.session_notice = None;
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
        self.session_notice = None;
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

    fn begin_prompt(&mut self, opener: Option<egui::Id>) {
        self.prompt_opener = opener;
        self.prompt_needs_focus = true;
    }

    fn finish_prompt(&mut self) {
        self.prompt_needs_focus = false;
        if let Some(opener) = self.prompt_opener.take() {
            self.focus_after_prompt = Some(opener);
        }
    }

    fn open_disclosure_prompt(&mut self, intent: DisclosureIntent, opener: Option<egui::Id>) {
        self.begin_prompt(opener);
        self.disclosure_prompt = Some(intent);
    }

    fn request_save(&mut self, after: Option<BoundaryTarget>) {
        self.request_save_from(after, None);
    }

    fn request_save_from(&mut self, after: Option<BoundaryTarget>, opener: Option<egui::Id>) {
        if !self.settings.storage_disclosure_acknowledged() {
            self.open_disclosure_prompt(DisclosureIntent::Save(after), opener);
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

    fn handle_session_list_failed(
        &mut self,
        request_id: u64,
        order: SessionListOrder,
        failure: crate::storage::StorageFailure,
    ) {
        let matched = match order {
            SessionListOrder::LastOpened if self.session_list_request_id == Some(request_id) => {
                self.session_list_request_id = None;
                true
            }
            SessionListOrder::LastUpdated if self.manage_list_request_id == Some(request_id) => {
                self.manage_list_request_id = None;
                self.manage_list_loading = false;
                true
            }
            _ => false,
        };
        if matched {
            self.failed_list_order = Some(order);
            self.handle_storage_event(StorageEvent::Failed(failure));
        } else {
            tracing::debug!(request_id, ?order, "ignored stale session list failure");
        }
    }

    fn next_list_request_id(&mut self) -> u64 {
        self.next_list_request_id = self.next_list_request_id.wrapping_add(1);
        self.next_list_request_id
    }

    fn request_session_list(&mut self) {
        let request_id = self.next_list_request_id();
        self.session_list_request_id = Some(request_id);
        if let Some(worker) = &self.storage
            && let Err(error) = worker.send(StorageCommand::ListSessions {
                request_id,
                order: SessionListOrder::LastOpened,
            })
        {
            self.session_list_request_id = None;
            self.storage_error = Some(format!("Could not request saved sessions: {error}"));
        }
    }

    fn request_manage_session_list(&mut self) {
        let request_id = self.next_list_request_id();
        self.manage_list_request_id = Some(request_id);
        self.manage_list_loading = true;
        if let Some(worker) = &self.storage
            && let Err(error) = worker.send(StorageCommand::ListSessions {
                request_id,
                order: SessionListOrder::LastUpdated,
            })
        {
            self.manage_list_request_id = None;
            self.manage_list_loading = false;
            self.storage_error = Some(format!("Could not request saved sessions: {error}"));
        }
    }

    fn refresh_session_lists(&mut self) {
        self.request_session_list();
        if matches!(self.view, AppView::Sessions) {
            self.request_manage_session_list();
        }
    }

    fn open_manage_sessions(&mut self) {
        self.view = AppView::Sessions;
        self.request_manage_session_list();
    }

    fn request_boundary(&mut self, target: BoundaryTarget) {
        self.request_boundary_from(target, None);
    }

    fn request_boundary_from(&mut self, target: BoundaryTarget, opener: Option<egui::Id>) {
        self.pending_boundary_opener = opener;
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
            BoundaryPolicy::Proceed => {
                self.pending_boundary_opener = None;
                self.execute_boundary(target);
            }
            BoundaryPolicy::Save => {
                self.pending_boundary_opener = None;
                self.request_save(Some(target));
            }
            BoundaryPolicy::Prompt => {
                let opener = self.pending_boundary_opener.take();
                self.begin_prompt(opener);
                self.boundary_prompt = Some(target);
            }
        }
    }

    fn execute_boundary(&mut self, target: BoundaryTarget) {
        self.pending_boundary_after_save = None;
        self.pending_boundary_opener = None;
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

    fn apply_keyboard_settings(&mut self) -> bool {
        let state = match init_keyboard_state(&self.model, &self.layout, &self.variant) {
            Ok(state) => {
                self.keyboard_error = None;
                state
            }
            Err(error) => {
                let message = format!("Could not apply keyboard configuration: {error:#}");
                error!(%message);
                self.keyboard_error = Some(message);
                return false;
            }
        };

        let previous_model = self.settings.keyboard_model().to_owned();
        let previous_layout = self.settings.keyboard_layout().to_owned();
        let previous_variant = self.settings.keyboard_variant().to_owned();
        self.settings.set_keyboard(
            self.model.clone(),
            self.layout.clone(),
            self.variant.clone(),
        );
        if !self.save_settings() {
            self.settings
                .set_keyboard(previous_model, previous_layout, previous_variant);
            return false;
        }

        self.xkb_state = state;
        self.working_session.keyboard.model.clone_from(&self.model);
        self.working_session
            .keyboard
            .layout
            .clone_from(&self.layout);
        self.working_session
            .keyboard
            .variant
            .clone_from(&self.variant);
        if self.working_session.id.is_some() || self.session_has_content() {
            self.note_session_dirty();
        }
        true
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
                self.finish_listener_stop(format!("Could not stop listener: {error:#}"), true);
            }
            None => {}
        }
    }

    fn open_rename_dialog(&mut self, target: RenameTarget, name: Option<&str>, opener: egui::Id) {
        self.session_switcher_open = false;
        self.focus_renamed_session = None;
        self.rename_dialog = Some(RenameDialog {
            target,
            buffer: name.unwrap_or_default().to_owned(),
            error: None,
            request_id: None,
            submitting: false,
            focus_text: true,
            opener,
        });
    }

    fn close_rename_dialog(&mut self) {
        if let Some(dialog) = self.rename_dialog.take() {
            self.focus_after_prompt = Some(dialog.opener);
        }
    }

    fn set_rename_error(&mut self, message: impl Into<String>) {
        if let Some(dialog) = &mut self.rename_dialog {
            dialog.error = Some(message.into());
            dialog.submitting = false;
            dialog.focus_text = true;
        }
    }

    fn submit_rename(&mut self) {
        let Some(dialog) = &self.rename_dialog else {
            return;
        };
        if dialog.submitting {
            return;
        }
        let target = dialog.target;
        let trimmed = dialog.buffer.trim();
        if trimmed.len() > MAX_SESSION_NAME_BYTES {
            self.set_rename_error("Session name is longer than 80 UTF-8 bytes.");
            return;
        }
        let name = (!trimmed.is_empty()).then(|| trimmed.to_owned());
        let excluded_id = match target {
            RenameTarget::Current => self.working_session.id,
            RenameTarget::Saved(session_id) => Some(session_id),
        };
        if let Some(name) = &name {
            let saved_conflict = self.sessions.iter().any(|session| {
                Some(session.id) != excluded_id && session.name.as_ref() == Some(name)
            });
            let active_conflict = matches!(
                target,
                RenameTarget::Saved(session_id)
                    if self.working_session.id != Some(session_id)
                        && self.working_session.name.as_ref() == Some(name)
            );
            if saved_conflict || active_conflict {
                self.set_rename_error("Another session already uses that name.");
                return;
            }
        }

        match target {
            RenameTarget::Current => {
                if self.working_session.name != name {
                    self.working_session.name = name;
                    self.note_session_dirty();
                }
                self.close_rename_dialog();
            }
            RenameTarget::Saved(session_id) => {
                let updated_at_ms = match unix_now_ms() {
                    Ok(value) => value,
                    Err(error) => {
                        self.set_rename_error(error);
                        return;
                    }
                };
                self.rename_request_id = self.rename_request_id.wrapping_add(1);
                let request_id = self.rename_request_id;
                let send_result = self.storage.as_ref().map_or_else(
                    || Err("Storage worker is unavailable".to_owned()),
                    |worker| {
                        worker
                            .rename_session(request_id, session_id, name, updated_at_ms)
                            .map_err(|error| format!("Could not request session rename: {error}"))
                    },
                );
                match send_result {
                    Ok(()) => {
                        if let Some(dialog) = &mut self.rename_dialog {
                            dialog.request_id = Some(request_id);
                            dialog.submitting = true;
                            dialog.error = None;
                        }
                    }
                    Err(error) => self.set_rename_error(error),
                }
            }
        }
    }

    fn handle_session_renamed(&mut self, request_id: u64, session: Option<SessionMetadata>) {
        let matches_dialog = self
            .rename_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.request_id == Some(request_id));
        if matches_dialog {
            if let Some(session) = session {
                self.close_rename_dialog();
                self.focus_after_prompt = None;
                self.focus_renamed_session = Some(session.id);
                self.clear_storage_failure(StorageOperation::Rename);
            } else {
                self.set_rename_error("The saved session no longer exists.");
            }
        }
        self.refresh_session_lists();
    }

    fn handle_session_rename_failed(
        &mut self,
        request_id: u64,
        session_id: SessionId,
        failure: crate::storage::StorageFailure,
    ) {
        let matches_dialog = self.rename_dialog.as_ref().is_some_and(|dialog| {
            dialog.request_id == Some(request_id)
                && dialog.target == RenameTarget::Saved(session_id)
        });
        if matches_dialog {
            self.set_rename_error(failure.details);
        } else {
            tracing::debug!(
                request_id,
                ?session_id,
                "ignored stale session rename failure"
            );
        }
    }

    fn reset_statistics(&mut self) {
        self.working_session.reset_statistics();
        self.note_session_dirty();
        self.confirm_reset = false;
        self.finish_prompt();
    }

    fn prompt_delete_session(
        &mut self,
        session_id: Option<SessionId>,
        display_name: impl Into<String>,
        current: bool,
        opener: Option<egui::Id>,
    ) {
        self.session_switcher_open = false;
        self.begin_prompt(opener);
        self.confirm_delete = Some(DeleteSessionPrompt {
            session_id,
            display_name: display_name.into(),
            current,
        });
    }

    fn delete_prompted_session(&mut self) {
        let Some(prompt) = self.confirm_delete.take() else {
            return;
        };
        self.finish_prompt();
        let Some(session_id) = prompt.session_id else {
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
        self.finish_prompt();
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
        StorageOperation::Rename => "Could not rename session",
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
        if self.listener_state == ListenerState::Listening {
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }

        self.handle_close_request(ui.ctx());
        self.render_shell(ui);
        let text_edit_focused = self.render_prompts(ui.ctx());
        self.handle_global_shortcuts(ui.ctx(), text_edit_focused);
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
