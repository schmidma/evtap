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

mod capture;
mod keyboard_settings;
mod persistence;
mod session_lifecycle;
pub(crate) mod view;
mod working_session;

use keyboard_settings::init_keyboard_state;
#[cfg(test)]
use session_lifecycle::boundary_policy;
use session_lifecycle::unix_now_ms;
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
    storage_failure: Option<StorageFailureNotice>,
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
    active_prompt: Option<ActivePrompt>,
    allow_close: bool,

    rename_request_id: u64,
    focus_after_prompt: Option<egui::Id>,
    focus_renamed_session: Option<SessionId>,
    deleting_session: bool,
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

struct ActivePrompt {
    kind: ActivePromptKind,
    opener: Option<egui::Id>,
    needs_initial_focus: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActivePromptTag {
    Disclosure,
    Boundary,
    Rename,
    Reset,
    DeleteSession,
    DeleteAll,
}

enum ActivePromptKind {
    Disclosure(DisclosureIntent),
    Boundary(BoundaryTarget),
    Rename(RenamePrompt),
    Reset,
    DeleteSession(DeleteSessionPrompt),
    DeleteAll,
}

struct RenamePrompt {
    target: RenameTarget,
    buffer: String,
    error: Option<String>,
    request_id: Option<u64>,
    submitting: bool,
}

struct DeleteSessionPrompt {
    session_id: Option<SessionId>,
    display_name: String,
    current: bool,
}

#[derive(Clone, Debug)]
struct StorageFailureNotice {
    operation: StorageOperation,
    list_order: Option<SessionListOrder>,
    message: String,
    details: String,
}

impl ActivePrompt {
    fn tag(&self) -> ActivePromptTag {
        match self.kind {
            ActivePromptKind::Disclosure(_) => ActivePromptTag::Disclosure,
            ActivePromptKind::Boundary(_) => ActivePromptTag::Boundary,
            ActivePromptKind::Rename(_) => ActivePromptTag::Rename,
            ActivePromptKind::Reset => ActivePromptTag::Reset,
            ActivePromptKind::DeleteSession(_) => ActivePromptTag::DeleteSession,
            ActivePromptKind::DeleteAll => ActivePromptTag::DeleteAll,
        }
    }
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
            storage_failure: None,
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
            active_prompt: None,
            allow_close: false,
            rename_request_id: 0,
            focus_after_prompt: None,
            focus_renamed_session: None,
            deleting_session: false,
            deleting_all: false,
        })
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
