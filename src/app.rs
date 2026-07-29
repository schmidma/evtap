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
        StoredSession, recover_default_metrics,
    },
    settings::{Settings, SettingsStore},
    storage::{
        CheckpointSchedule, DirtyTracker, SaveRequest, StorageCommand, StorageEvent,
        StorageOperation, StorageStatus, StorageWorker,
    },
    wake::WakeSignal,
    xkb_helper,
};

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
    storage_initialized: bool,
    storage_tracker: DirtyTracker,
    checkpoint_schedule: CheckpointSchedule,
    storage_error: Option<String>,
    last_failed_operation: Option<StorageOperation>,
    sessions: Vec<SessionMetadata>,
    list_request_id: u64,
    load_request_id: u64,
    loading_session: bool,

    session: CurrentSession,
    recovery_messages: Vec<String>,
    after_save: Option<BoundaryTarget>,
    deferred_boundary: Option<BoundaryTarget>,
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

#[derive(Debug)]
struct CurrentSession {
    id: Option<SessionId>,
    name: Option<String>,
    created_at_ms: i64,
    last_opened_at_ms: i64,
    captured_duration: Duration,
    capture_started_at: Option<Instant>,
    keyboard: KeyboardContext,
    restored: bool,
}

impl CurrentSession {
    fn untitled(now_ms: i64, model: String, layout: String, variant: String) -> Self {
        Self {
            id: None,
            name: None,
            created_at_ms: now_ms,
            last_opened_at_ms: now_ms,
            captured_duration: Duration::ZERO,
            capture_started_at: None,
            keyboard: KeyboardContext {
                display_name: None,
                model,
                layout,
                variant,
            },
            restored: false,
        }
    }

    fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("Untitled session")
    }

    fn duration(&self) -> Duration {
        self.capture_started_at
            .map_or(self.captured_duration, |started| {
                self.captured_duration.saturating_add(started.elapsed())
            })
    }

    fn finish_capture_segment(&mut self) -> bool {
        let Some(started) = self.capture_started_at.take() else {
            return false;
        };
        self.captured_duration = self.captured_duration.saturating_add(started.elapsed());
        true
    }
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
            metrics: default_metrics(),
            physical_keys: HashMap::new(),
            model: model.clone(),
            layout: layout.clone(),
            variant: variant.clone(),
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
            storage_initialized: false,
            storage_tracker,
            checkpoint_schedule: CheckpointSchedule::default(),
            storage_error: None,
            last_failed_operation: None,
            sessions: Vec::new(),
            list_request_id: 0,
            load_request_id: 0,
            loading_session: false,
            session: CurrentSession::untitled(now_ms, model, layout, variant),
            recovery_messages: Vec::new(),
            after_save: None,
            deferred_boundary: None,
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
        let Some(name) = self.session.keyboard.display_name.as_deref() else {
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
                    self.session.capture_started_at = Some(Instant::now());
                    info!("listener connected to keyboard");
                }
                listener::Event::Stopped { reason } => {
                    let is_error = reason.is_error();
                    let message = reason.to_string();
                    self.listener = None;
                    if self.session.finish_capture_segment() {
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
                    if let Some(target) = self.deferred_boundary.take() {
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

    fn clear_in_flight(&mut self) {
        for metric in &mut self.metrics {
            metric.clear_in_flight();
        }
        self.physical_keys.clear();
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
                let first_open = !self.storage_initialized;
                self.storage_initialized = true;
                self.sessions = sessions;
                self.storage_error = None;
                self.last_failed_operation = None;
                if first_open {
                    if let Some(selected) = selected {
                        self.restore_session(selected);
                        self.storage_tracker.loaded(true);
                    } else {
                        self.storage_tracker.loaded(false);
                        if self.settings.last_session_id().is_some() {
                            self.settings.set_last_session_id(None);
                            self.save_settings();
                        }
                    }
                } else {
                    self.storage_tracker.loaded(self.session.id.is_some());
                    if self.session.id.is_none() && self.session_has_content() {
                        self.note_session_dirty();
                    }
                }
            }
            StorageEvent::Saved {
                generation,
                session_id,
            } => {
                self.session.id = Some(session_id);
                if let Err(error) = self.storage_tracker.acknowledge(generation) {
                    self.storage_error = Some(format!("Unexpected save acknowledgement: {error}"));
                    self.storage_tracker.set_failed();
                    self.after_save = None;
                    return;
                }
                self.settings.set_last_session_id(Some(session_id));
                self.save_settings();
                self.storage_error = None;
                self.last_failed_operation = None;
                self.request_session_list();
                if self.storage_tracker.is_dirty() {
                    if let Some(target) = self.after_save {
                        self.begin_save(Some(target));
                    }
                } else if let Some(target) = self.after_save.take() {
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
                        self.storage_tracker.loaded(true);
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
                if deleted && self.session.id == Some(session_id) {
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
                    self.after_save = None;
                } else if failure.operation == StorageOperation::Open {
                    self.storage_initialized = true;
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
            name: metadata.name,
            created_at_ms: metadata.created_at_ms,
            last_opened_at_ms: metadata.last_opened_at_ms,
            captured_duration: Duration::from_nanos(
                u64::try_from(metadata.captured_duration_ns).unwrap_or_default(),
            ),
            capture_started_at: None,
            keyboard: metadata.keyboard,
            restored: true,
        };
        self.listener = None;
        self.listener_state = ListenerState::Idle;
        self.clear_in_flight();
        self.select_remembered_device();
    }

    fn new_working_session(&mut self) {
        let now_ms = unix_now_ms().unwrap_or_default();
        self.metrics = default_metrics();
        self.physical_keys.clear();
        self.session = CurrentSession::untitled(
            now_ms,
            self.model.clone(),
            self.layout.clone(),
            self.variant.clone(),
        );
        self.listener = None;
        self.listener_state = ListenerState::Idle;
        self.storage_tracker.loaded(false);
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
        if self.session.id.is_some() {
            self.storage_tracker.is_dirty()
        } else {
            self.session_has_content()
        }
    }

    fn session_has_content(&self) -> bool {
        self.session.name.is_some()
            || !self.session.captured_duration.is_zero()
            || self.session.capture_started_at.is_some()
            || self.metrics.iter().any(|metric| metric.has_data())
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
                self.after_save = after;
            }
            return;
        }
        if self.session.id.is_none()
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
        self.after_save = after;
        let command = StorageCommand::Save(SaveRequest {
            generation,
            snapshot,
        });
        match self.storage.as_ref().map(|worker| worker.send(command)) {
            Some(Ok(())) => self.checkpoint_schedule.save_started(),
            Some(Err(error)) => {
                let _ = self.storage_tracker.fail(generation);
                self.storage_error = Some(format!("Could not request save: {error}"));
                self.after_save = None;
            }
            None => {
                let _ = self.storage_tracker.fail(generation);
                self.storage_error = Some("Storage worker is unavailable".to_owned());
                self.after_save = None;
            }
        }
    }

    fn session_snapshot(&self) -> Result<SessionSnapshot, String> {
        let duration_ns = i64::try_from(self.session.duration().as_nanos())
            .map_err(|_| "Capture duration exceeds the storage range".to_owned())?;
        let metrics = self
            .metrics
            .iter()
            .map(|metric| metric.snapshot().map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let now_ms = unix_now_ms()?;
        Ok(SessionSnapshot {
            id: self.session.id,
            name: self.session.name.clone(),
            created_at_ms: self.session.created_at_ms,
            updated_at_ms: now_ms,
            last_opened_at_ms: self.session.last_opened_at_ms.max(now_ms),
            captured_duration_ns: duration_ns,
            application_version: env!("CARGO_PKG_VERSION").to_owned(),
            keyboard: self.session.keyboard.clone(),
            metrics,
        })
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
            self.deferred_boundary = Some(target);
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
        self.after_save = None;
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
        self.session.keyboard = KeyboardContext {
            display_name: Some(device.name.clone()),
            model: self.model.clone(),
            layout: self.layout.clone(),
            variant: self.variant.clone(),
        };
        self.session.last_opened_at_ms = unix_now_ms().unwrap_or(self.session.last_opened_at_ms);
        self.session.restored = false;
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
                if self.session.finish_capture_segment() {
                    self.note_session_dirty();
                }
                self.clear_in_flight();
                self.listener_state = ListenerState::Failed;
                self.capture_error = Some(format!("Could not stop listener: {error:#}"));
                if let Some(target) = self.deferred_boundary.take() {
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
                Some(session.id) != self.session.id && session.name.as_ref() == Some(name)
            })
        {
            self.rename_error = Some("Another saved session already uses that name.".to_owned());
            return;
        }
        if self.session.name != name {
            self.session.name = name;
            self.note_session_dirty();
        }
        self.rename_open = false;
        self.rename_error = None;
    }

    fn reset_statistics(&mut self) {
        for metric in &mut self.metrics {
            metric.reset();
        }
        self.physical_keys.clear();
        self.session.captured_duration = Duration::ZERO;
        self.session.capture_started_at = None;
        self.note_session_dirty();
        self.confirm_reset = false;
    }

    fn delete_current_session(&mut self) {
        self.confirm_delete = false;
        let Some(session_id) = self.session.id else {
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

    fn render_session_management(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.heading("Session");
            let busy = self.loading_session
                || self.storage_tracker.in_flight().is_some()
                || self.deleting_session
                || self.deleting_all
                || self.listener_state == ListenerState::Stopping;
            let mut requested_target = None;
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    egui::ComboBox::from_label("Current")
                        .selected_text(self.session.display_name())
                        .show_ui(ui, |ui| {
                            for saved in &self.sessions {
                                let selected = self.session.id == Some(saved.id);
                                let label = session_selector_label(saved);
                                if ui.selectable_label(selected, label).clicked() && !selected {
                                    requested_target = Some(BoundaryTarget::Load(saved.id));
                                }
                            }
                        });
                });
                if ui
                    .add_enabled(!busy, egui::Button::new("New session"))
                    .clicked()
                {
                    requested_target = Some(BoundaryTarget::New);
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Save now"))
                    .clicked()
                {
                    self.request_save(None);
                }
                if ui.add_enabled(!busy, egui::Button::new("Rename")).clicked() {
                    self.rename_buffer = self.session.name.clone().unwrap_or_default();
                    self.rename_open = true;
                    self.rename_error = None;
                }
                if ui
                    .add_enabled(
                        self.listener.is_none() && !busy,
                        egui::Button::new("Reset statistics"),
                    )
                    .clicked()
                {
                    self.confirm_reset = true;
                }
                if ui
                    .add_enabled(
                        self.listener.is_none() && !busy,
                        egui::Button::new("Delete session"),
                    )
                    .clicked()
                {
                    self.confirm_delete = true;
                }
            });
            if let Some(target) = requested_target {
                self.request_boundary(target);
            }

            ui.horizontal_wrapped(|ui| {
                let mut autosave = self.settings.autosave_enabled();
                if ui.checkbox(&mut autosave, "Autosave sessions").changed() {
                    if autosave {
                        if self.settings.storage_disclosure_acknowledged() {
                            self.settings.set_autosave_enabled(true);
                            if self.save_settings() {
                                if self.working_dirty() {
                                    self.request_save(None);
                                }
                            } else {
                                self.settings.set_autosave_enabled(false);
                            }
                        } else {
                            self.disclosure_prompt = Some(DisclosureIntent::EnableAutosave);
                        }
                    } else {
                        self.settings.set_autosave_enabled(false);
                        if self.save_settings() {
                            self.checkpoint_schedule.clear();
                        } else {
                            self.settings.set_autosave_enabled(true);
                        }
                    }
                }
                ui.label(storage_status_label(
                    self.storage_tracker.status(),
                    self.session.id.is_some(),
                ));
                let retryable = matches!(
                    self.last_failed_operation,
                    Some(StorageOperation::Open | StorageOperation::Save)
                );
                if retryable && ui.button("Retry storage operation").clicked() {
                    if self.last_failed_operation == Some(StorageOperation::Open) {
                        if let Some(worker) = &self.storage {
                            let _ = worker.send(StorageCommand::RetryOpen {
                                last_session_id: self.settings.last_session_id(),
                            });
                        }
                    } else {
                        self.request_save(None);
                    }
                }
            });
            ui.small(format!(
                "Capture duration: {} · Created: {}",
                format_duration(self.session.duration()),
                format_local_timestamp(self.session.created_at_ms)
            ));
            if self.session.restored {
                ui.weak("Restored from disk; capture is paused until you start listening.");
            }
            if let Some(error) = &self.storage_error {
                ui.colored_label(egui::Color32::RED, error);
            }
            ui.horizontal_wrapped(|ui| {
                ui.small(format!(
                    "Storage: {} ({})",
                    self.paths.database_file().display(),
                    format_byte_size(database_disk_usage(&self.paths.database_file()))
                ));
                if ui
                    .add_enabled(
                        !self.sessions.is_empty() && !busy,
                        egui::Button::new("Delete all saved sessions"),
                    )
                    .clicked()
                {
                    self.confirm_delete_all = true;
                }
            });
        });
    }

    fn render_capture_setup(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.heading("Capture setup");
            self.render_device_picker(ui);
            ui.add_space(4.0);
            self.render_keyboard_configuration(ui);
            ui.add_space(4.0);
            self.render_capture_controls(ui);
            self.render_capture_status(ui);
        });
    }

    fn render_device_picker(&mut self, ui: &mut egui::Ui) {
        let picker_enabled = self.listener.is_none()
            && !matches!(self.listener_state, ListenerState::Stopping)
            && !self.loading_session;
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
                                    ui.selectable_value(
                                        &mut self.selected_device,
                                        Some(index),
                                        &device.name,
                                    )
                                    .on_hover_ui(|ui| {
                                        ui.label(format!(
                                            "{} ({})",
                                            device.physical_path, device.path
                                        ));
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
        if self.session.keyboard.display_name.is_some() {
            ui.weak("The session's remembered keyboard is a suggestion; any readable keyboard may be used.");
        }
        if request_scan {
            self.request_scan();
        }
    }

    fn render_keyboard_configuration(&mut self, ui: &mut egui::Ui) {
        let enabled = self.listener.is_none() && !self.loading_session;
        let mut changed = false;
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_label("Model")
                    .width(80.0)
                    .selected_text(&self.model)
                    .show_ui(ui, |ui| {
                        for model in &self.available_models {
                            changed |= ui
                                .selectable_value(&mut self.model, model.clone(), model)
                                .clicked();
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
                        changed |= ui
                            .selectable_value(&mut self.variant, String::new(), "Default")
                            .clicked();
                        for variant in &self.available_variants {
                            if !variant.is_empty() {
                                changed |= ui
                                    .selectable_value(&mut self.variant, variant.clone(), variant)
                                    .clicked();
                            }
                        }
                    });
            });
        });
        if changed {
            self.reinit_xkb();
            self.save_keyboard_settings();
            self.session.keyboard.model.clone_from(&self.model);
            self.session.keyboard.layout.clone_from(&self.layout);
            self.session.keyboard.variant.clone_from(&self.variant);
            if self.session.id.is_some() || self.session_has_content() {
                self.note_session_dirty();
            }
        }
    }

    fn render_capture_controls(&mut self, ui: &mut egui::Ui) {
        let busy = self.loading_session
            || self.deleting_session
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
                if ui
                    .add_enabled(
                        selected_index.is_some() && !busy,
                        egui::Button::new("Start listening"),
                    )
                    .clicked()
                    && let Some(index) = selected_index
                {
                    self.begin_listening(index);
                }
            }
        });
    }

    fn render_capture_status(&self, ui: &mut egui::Ui) {
        for error in [
            self.scan_error.as_ref(),
            self.keyboard_error.as_ref(),
            self.settings_error.as_ref(),
            self.capture_error.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            ui.colored_label(egui::Color32::RED, error);
        }
        if let Some(warning) = &self.scan_warning {
            ui.colored_label(egui::Color32::YELLOW, warning);
        }
        match self.listener_state {
            ListenerState::Idle => ui.weak("Not listening"),
            ListenerState::Connecting => ui.label("Connecting to keyboard…"),
            ListenerState::Listening => ui.colored_label(egui::Color32::GREEN, "Listening"),
            ListenerState::Stopping => ui.label("Stopping listener…"),
            ListenerState::Failed => {
                ui.colored_label(egui::Color32::RED, "Capture stopped because of an error")
            }
        };
    }

    fn render_prompts(&mut self, ctx: &egui::Context) {
        if let Some(intent) = self.disclosure_prompt {
            egui::Window::new("Save sensitive aggregate statistics?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("evtap reads global keyboard input while listening. Saving writes sensitive character, bigram, correction, key-usage, count, and timing aggregates to a local, unencrypted SQLite database.");
                    ui.label("Raw event sequences, device paths, pressed-key state, and unfinished timing or correction context are not stored. No telemetry or synchronization is performed.");
                    ui.colored_label(egui::Color32::YELLOW, "Anyone who can read your files, including backups or privileged processes, may read the saved aggregates.");
                    ui.horizontal(|ui| {
                        if ui.button("Allow local saves").clicked() {
                            self.settings.acknowledge_storage_disclosure();
                            if self.save_settings() {
                                self.disclosure_prompt = None;
                                match intent {
                                    DisclosureIntent::Save(after) => self.begin_save(after),
                                    DisclosureIntent::EnableAutosave => {
                                        self.settings.set_autosave_enabled(true);
                                        if self.save_settings() {
                                            if self.working_dirty() {
                                                self.begin_save(None);
                                            }
                                        } else {
                                            self.settings.set_autosave_enabled(false);
                                        }
                                    }
                                }
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.disclosure_prompt = None;
                        }
                    });
                });
        }

        if let Some(target) = self.boundary_prompt {
            let exiting = target == BoundaryTarget::Exit;
            egui::Window::new(if exiting {
                "Save changes before exiting?"
            } else {
                "Save changes before switching sessions?"
            })
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} has unsaved changes.",
                    self.session.display_name()
                ));
                ui.horizontal(|ui| {
                    if ui
                        .button(if exiting {
                            "Save and exit"
                        } else {
                            "Save and switch"
                        })
                        .clicked()
                    {
                        self.boundary_prompt = None;
                        self.request_save(Some(target));
                    }
                    if ui
                        .button(if self.session.id.is_some() {
                            "Discard changes"
                        } else {
                            "Discard session"
                        })
                        .clicked()
                    {
                        self.boundary_prompt = None;
                        self.execute_boundary(target);
                    }
                    if ui.button("Cancel").clicked() {
                        self.boundary_prompt = None;
                    }
                });
            });
        }

        if self.rename_open {
            egui::Window::new("Rename session")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Leave the name empty to keep this session untitled.");
                    ui.text_edit_singleline(&mut self.rename_buffer);
                    if let Some(error) = &self.rename_error {
                        ui.colored_label(egui::Color32::RED, error);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Apply").clicked() {
                            self.apply_rename();
                        }
                        if ui.button("Cancel").clicked() {
                            self.rename_open = false;
                            self.rename_error = None;
                        }
                    });
                });
        }

        if self.confirm_reset {
            egui::Window::new("Reset statistics?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("All aggregate statistics and capture duration in the current session will be reset.");
                    ui.horizontal(|ui| {
                        if ui.button("Reset statistics").clicked() {
                            self.reset_statistics();
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_reset = false;
                        }
                    });
                });
        }

        if self.confirm_delete {
            egui::Window::new("Delete session?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    if self.session.id.is_some() {
                        ui.label("The saved copy and all current unsaved changes will be deleted. This cannot be undone.");
                    } else {
                        ui.label("The current in-memory session will be discarded. This cannot be undone.");
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Delete permanently").clicked() {
                            self.delete_current_session();
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_delete = false;
                        }
                    });
                });
        }

        if self.confirm_delete_all {
            egui::Window::new("Delete all saved sessions?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Every saved session and the current working state will be removed. Filesystem backups and snapshots may retain copies.");
                    ui.horizontal(|ui| {
                        if ui.button("Delete all permanently").clicked() {
                            self.delete_all_sessions();
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_delete_all = false;
                        }
                    });
                });
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        let close_requested = ctx.input(|input| input.viewport().close_requested());
        if close_requested && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.boundary_prompt.is_none()
                && self.disclosure_prompt.is_none()
                && self.after_save.is_none()
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

fn session_selector_label(metadata: &SessionMetadata) -> String {
    let keyboard = metadata
        .keyboard
        .display_name
        .as_deref()
        .unwrap_or("No keyboard");
    format!(
        "{} — {} — {}",
        metadata.display_name(),
        keyboard,
        format_local_timestamp(metadata.updated_at_ms)
    )
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

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
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

fn boundary_policy(dirty: bool, autosave: bool) -> BoundaryPolicy {
    match (dirty, autosave) {
        (false, _) => BoundaryPolicy::Proceed,
        (true, true) => BoundaryPolicy::Save,
        (true, false) => BoundaryPolicy::Prompt,
    }
}

fn storage_status_label(status: StorageStatus, has_id: bool) -> &'static str {
    match status {
        StorageStatus::Loading => "Loading saved sessions…",
        StorageStatus::Unsaved => "Unsaved session",
        StorageStatus::Saved if has_id => "Saved",
        StorageStatus::Saved => "Unsaved session",
        StorageStatus::Dirty => "Unsaved changes",
        StorageStatus::Saving => "Saving…",
        StorageStatus::Failed => "Storage operation failed",
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
                    for metric in &self.metrics {
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
        if self.session.finish_capture_segment() {
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
mod tests {
    use std::fs;

    use super::{
        BoundaryPolicy, HACK_FONT_NAME, SessionMetadata, StorageStatus, boundary_policy,
        database_disk_usage, font_definitions, session_selector_label, storage_status_label,
    };
    use crate::session::{KeyboardContext, SessionId};
    use eframe::egui;

    #[test]
    fn proportional_font_family_supports_bigram_arrow() {
        let fonts = font_definitions();
        assert!(fonts.font_data.contains_key(HACK_FONT_NAME));
        assert!(
            fonts
                .families
                .get(&egui::FontFamily::Proportional)
                .unwrap()
                .iter()
                .any(|font| font == HACK_FONT_NAME)
        );
    }

    #[test]
    fn dirty_boundaries_follow_editor_style_save_policy() {
        assert_eq!(boundary_policy(false, false), BoundaryPolicy::Proceed);
        assert_eq!(boundary_policy(false, true), BoundaryPolicy::Proceed);
        assert_eq!(boundary_policy(true, false), BoundaryPolicy::Prompt);
        assert_eq!(boundary_policy(true, true), BoundaryPolicy::Save);
    }

    #[test]
    fn storage_labels_cover_unsaved_and_saved_sessions() {
        assert_eq!(
            storage_status_label(StorageStatus::Unsaved, false),
            "Unsaved session"
        );
        assert_eq!(storage_status_label(StorageStatus::Saved, true), "Saved");
    }

    #[test]
    fn selector_distinguishes_untitled_sessions_with_metadata() {
        let metadata = SessionMetadata {
            id: SessionId::new(1).unwrap(),
            name: None,
            created_at_ms: 1,
            updated_at_ms: 1,
            last_opened_at_ms: 1,
            captured_duration_ns: 0,
            application_version: "test".to_owned(),
            keyboard: KeyboardContext {
                display_name: Some("Work keyboard".to_owned()),
                ..Default::default()
            },
        };
        let label = session_selector_label(&metadata);
        assert!(label.contains("Untitled session"));
        assert!(label.contains("Work keyboard"));
    }

    #[test]
    fn disk_usage_includes_sqlite_sidecars() {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("evtap.sqlite3");
        fs::write(&database, [0_u8; 3]).unwrap();
        fs::write(temporary.path().join("evtap.sqlite3-wal"), [0_u8; 5]).unwrap();
        fs::write(temporary.path().join("evtap.sqlite3-shm"), [0_u8; 7]).unwrap();

        assert_eq!(database_disk_usage(&database), 15);
    }
}
