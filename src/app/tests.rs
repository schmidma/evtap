use std::{
    fs, thread,
    time::{Duration, SystemTime},
};

use super::{
    App, AppView, BoundaryPolicy, ListenerState, RenameTarget, ScanWarning, SessionMetadata,
    StorageOperation, StorageStatus, TimingView, boundary_policy,
    view::{
        session_selector_label, storage_status_label, storage_status_label_for_operation,
        theme::{HACK_FONT_NAME, font_definitions},
    },
};
use crate::{
    input::KeyEventKind,
    listener::StopReason,
    metric::{MetricSnapshot, SessionMetrics},
    paths::AppPaths,
    scanner::{DeviceMetadata, DeviceScanIssue, DeviceScanIssueKind, ScanReport},
    session::{KeyboardContext, SessionId, StoredSession},
    settings::AppearancePreference,
    storage::{SessionListOrder, StorageEvent, StorageFailure, database_disk_usage},
};
use eframe::egui;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use evdev::KeyCode;
use tempfile::TempDir;

struct TestWorkspace {
    _temporary: TempDir,
    paths: AppPaths,
}

impl TestWorkspace {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let paths = AppPaths::with_roots(
            temporary.path().join("config"),
            temporary.path().join("data"),
        );
        paths.prepare_data_dir().unwrap();
        Self {
            _temporary: temporary,
            paths,
        }
    }

    fn start(&self) -> Harness<'static, App> {
        self.start_with_size(egui::vec2(900.0, 1_200.0))
    }

    fn start_with_size(&self, size: egui::Vec2) -> Harness<'static, App> {
        self.start_with_size_and_scale(size, 1.0)
    }

    fn start_with_size_and_scale(
        &self,
        physical_size: egui::Vec2,
        scale: f32,
    ) -> Harness<'static, App> {
        let paths = self.paths.clone();
        let mut harness = Harness::builder()
            .with_size(physical_size / scale)
            .with_pixels_per_point(scale)
            .build_eframe(move |creation_context| App::new(creation_context, paths).unwrap());
        wait_for_app(&mut harness, |app| {
            app.initial_storage_open_handled && app.devices.is_some()
        });
        harness
    }
}

fn wait_for_app(harness: &mut Harness<'_, App>, predicate: impl Fn(&App) -> bool) {
    wait_for_app_attempts(harness, 200, predicate);
}

fn wait_for_app_attempts(
    harness: &mut Harness<'_, App>,
    attempts: usize,
    predicate: impl Fn(&App) -> bool,
) {
    for _ in 0..attempts {
        harness.step();
        if predicate(harness.state()) {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "headless app did not reach the expected state: view={:?}, working_id={:?}, name={:?}, storage={:?}, autosave={}, boundary={:?}, pending_save={:?}, sessions={:?}, managed={:?}, error={:?}",
        harness.state().view,
        harness.state().working_session.id,
        harness.state().working_session.name,
        harness.state().storage_tracker.status(),
        harness.state().settings.autosave_enabled(),
        harness.state().boundary_prompt,
        harness.state().pending_boundary_after_save,
        harness
            .state()
            .sessions
            .iter()
            .map(|session| (session.id, session.name.as_deref()))
            .collect::<Vec<_>>(),
        harness
            .state()
            .managed_sessions
            .iter()
            .map(|session| (session.id, session.name.as_deref()))
            .collect::<Vec<_>>(),
        harness.state().storage_error,
    );
}

fn request_headless_close(harness: &mut Harness<'_, App>) {
    harness
        .input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    harness.step();
    harness
        .input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .clear();
}

fn shutdown_harness(harness: &mut Harness<'_, App>) {
    eframe::App::on_exit(harness.state_mut(), None);
}

fn open_session_switcher(harness: &mut Harness<'_, App>) {
    assert!(
        !harness.state().session_switcher_open,
        "session switcher unexpectedly remained open before its opener was activated"
    );
    harness
        .get_by_label("Switch active session")
        .click_accesskit();
    harness.run();
}

fn open_manage_sessions(harness: &mut Harness<'_, App>) {
    if !matches!(harness.state().view, AppView::Sessions) {
        open_session_switcher(harness);
        harness.get_by_label("Manage sessions").click();
        harness.run();
        wait_for_app(harness, |app| !app.manage_list_loading);
    }
}

fn open_storage_settings(harness: &mut Harness<'_, App>) {
    harness.get_by_label_contains("Settings").click();
    harness.run();
    harness.get_by_label("Storage & privacy").click();
    harness.run();
}

fn rename_session(harness: &mut Harness<'_, App>, name: &str) {
    open_session_switcher(harness);
    assert!(
        harness.query_by_label("Rename").is_some(),
        "rename action unavailable for {name:?}: switcher_open={}, switch_focused={}, current_id={:?}, current_name={:?}, dirty={}, boundary={:?}, disclosure={:?}, rename_dialog={}, reset={}, delete={}, delete_all={}",
        harness.state().session_switcher_open,
        harness.get_by_label("Switch active session").is_focused(),
        harness.state().working_session.id,
        harness.state().working_session.name,
        harness.state().working_dirty(),
        harness.state().boundary_prompt,
        harness.state().disclosure_prompt,
        harness.state().rename_dialog.is_some(),
        harness.state().confirm_reset,
        harness.state().confirm_delete.is_some(),
        harness.state().confirm_delete_all,
    );
    harness.get_by_label("Rename").click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Session name")
        .focus();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Session name")
        .type_text(name);
    harness.run();
    harness.get_by_label("Apply").click();
    harness.run();
}

fn allow_first_save(harness: &mut Harness<'_, App>) {
    harness.get_by_label("Save").click();
    harness.run();
    harness.get_by_label("Allow local saves").click();
    harness.step();
    wait_for_app(harness, |app| {
        app.working_session.id.is_some() && app.storage_tracker.status() == StorageStatus::Saved
    });
}

fn install_analytics_fixture(app: &mut App, key_rows: u16) {
    app.working_session.metrics = SessionMetrics::default();
    let keys = (0..key_rows)
        .map(|index| {
            serde_json::json!({
                "code": 30 + index,
                "label": format!("KEY_{index}"),
                "count": u64::from(key_rows - index),
            })
        })
        .collect::<Vec<_>>();
    let total_presses = u64::from(key_rows) * u64::from(key_rows + 1) / 2;
    let fixtures = [
        (
            "total-presses",
            serde_json::json!({ "count": total_presses }),
        ),
        ("key-usage", serde_json::json!({ "keys": keys })),
        (
            "dwell-time",
            serde_json::json!({
                "entries": [
                    { "text": "a", "total_ns": 240_000_000_u64, "samples": 2 },
                    { "text": " ", "total_ns": 180_000_000_u64, "samples": 3 }
                ]
            }),
        ),
        (
            "flight-time",
            serde_json::json!({
                "entries": [
                    { "text": "b", "total_ns": 150_000_000_u64, "samples": 3 },
                    { "text": "\t", "total_ns": 320_000_000_u64, "samples": 4 }
                ]
            }),
        ),
        (
            "bigram-speed",
            serde_json::json!({
                "pairs": [
                    { "first": "a", "second": "b", "total_ns": 210_000_000_u64, "samples": 3 },
                    { "first": " ", "second": "🜁", "total_ns": 480_000_000_u64, "samples": 4 }
                ]
            }),
        ),
        (
            "corrections",
            serde_json::json!({
                "deletions": [
                    { "text": "a", "count": 4 },
                    { "text": " ", "count": 2 }
                ],
                "corrections": [
                    { "deleted": "a", "typed": "b", "count": 3 },
                    { "deleted": " ", "typed": "🜁", "count": 1 }
                ]
            }),
        ),
    ];

    for (metric_id, payload) in fixtures {
        let snapshot = MetricSnapshot::from_json(metric_id, 1, payload.to_string()).unwrap();
        app.working_session
            .metrics
            .restore_snapshot(&snapshot)
            .unwrap();
    }
}

fn assert_top_bar_controls_are_ordered(harness: &Harness<'_, App>, viewport_width: f64) {
    let bounds = |node: egui_kittest::Node<'_>| node.accesskit_node().bounding_box().unwrap();
    let session = bounds(
        harness.get_by_role_and_label(egui::accesskit::Role::Button, "Switch active session"),
    );
    let save_status = bounds(harness.get_by_label_contains("Save status:"));
    let save = bounds(harness.get_by_role_and_label(egui::accesskit::Role::Button, "Save"));
    let keyboard =
        bounds(harness.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Keyboard"));
    let rescan = bounds(
        harness
            .get_all_by_role_and_label(egui::accesskit::Role::Button, "Rescan keyboards")
            .min_by(|left, right| {
                left.accesskit_node()
                    .bounding_box()
                    .unwrap()
                    .y0
                    .total_cmp(&right.accesskit_node().bounding_box().unwrap().y0)
            })
            .unwrap(),
    );
    let status = bounds(harness.get_by_label_contains("Capture status:"));
    let capture = bounds(
        harness.get_by_role_and_label(egui::accesskit::Role::Button, "Start keyboard capture"),
    );

    assert!(
        session.x1 <= save_status.x0,
        "session={session:?}, save_status={save_status:?}"
    );
    assert!(
        save_status.x1 <= save.x0,
        "save_status={save_status:?}, save={save:?}"
    );
    assert!(
        save_status.y0 < save.y1 && save.y0 < save_status.y1,
        "save status and action should share one row: save_status={save_status:?}, save={save:?}"
    );
    assert!(
        save.x1 <= keyboard.x0,
        "save={save:?}, keyboard={keyboard:?}"
    );
    assert!(
        keyboard.x1 <= rescan.x0,
        "keyboard={keyboard:?}, rescan={rescan:?}"
    );
    assert!(
        rescan.x1 <= status.x0,
        "rescan={rescan:?}, status={status:?}"
    );
    assert!(
        status.x1 <= capture.x0,
        "status={status:?}, capture={capture:?}"
    );
    assert!(
        capture.x1 <= viewport_width,
        "capture={capture:?}, viewport_width={viewport_width}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_manual_save_disclosure_and_restart() {
    let workspace = TestWorkspace::new();
    let database = workspace.paths.database_file();

    let saved_session_id = {
        let mut harness = workspace.start();

        assert_eq!(
            harness.state().working_session.display_name(),
            "Untitled session"
        );
        assert_eq!(harness.state().listener_state, ListenerState::Idle);
        assert!(!database.exists());

        harness
            .state_mut()
            .process_input(SystemTime::now(), KeyCode::KEY_A, KeyEventKind::Press);
        harness.step();
        assert!(harness.state().working_dirty());
        assert!(harness.query_by_label("Presses: 1").is_some());

        harness.get_by_label("Save").click();
        harness.run();
        assert!(
            harness
                .query_by_role_and_label(
                    egui::accesskit::Role::Dialog,
                    "Save sensitive aggregate statistics?",
                )
                .is_some()
        );
        assert!(!database.exists());
        assert!(harness.get_by_label("Allow local saves").is_focused());

        harness.get_by_label("Cancel").click();
        harness.run();
        assert!(!database.exists());

        harness.get_by_label("Save").click();
        harness.run();
        harness.get_by_label("Allow local saves").click();
        harness.step();
        wait_for_app(&mut harness, |app| {
            app.working_session.id.is_some() && app.storage_tracker.status() == StorageStatus::Saved
        });

        let saved_session_id = harness.state().working_session.id.unwrap();
        assert!(database.exists());
        assert!(harness.state().settings.storage_disclosure_acknowledged());
        shutdown_harness(&mut harness);
        saved_session_id
    };

    let mut restarted = workspace.start();
    wait_for_app(&mut restarted, |app| {
        !app.loading_session && app.working_session.restored
    });

    assert_eq!(restarted.state().working_session.id, Some(saved_session_id));
    assert_eq!(restarted.state().listener_state, ListenerState::Idle);
    assert_eq!(
        restarted.state().storage_tracker.status(),
        StorageStatus::Saved
    );
    assert!(restarted.query_by_label("Presses: 1").is_some());
    assert!(
        restarted
            .query_by_label_contains("Restored from disk; capture is paused")
            .is_none()
    );
    shutdown_harness(&mut restarted);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_dirty_close_can_cancel_or_save() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();

    allow_first_save(&mut harness);
    rename_session(&mut harness, "Closing test");
    assert!(harness.state().working_dirty());

    request_headless_close(&mut harness);
    assert!(
        harness
            .query_by_role_and_label(
                egui::accesskit::Role::Dialog,
                "Save changes before exiting?",
            )
            .is_some()
    );
    harness.get_by_label("Cancel").click();
    harness.run();
    assert!(!harness.state().allow_close);
    assert!(harness.state().working_dirty());

    request_headless_close(&mut harness);
    assert!(harness.get_by_label("Save and exit").is_focused());
    harness.key_press(egui::Key::Enter);
    harness.step();
    wait_for_app(&mut harness, |app| {
        app.allow_close && app.storage_tracker.status() == StorageStatus::Saved
    });
    assert_eq!(
        harness.state().working_session.name.as_deref(),
        Some("Closing test")
    );
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_failed_save_stays_dirty_and_retries_latest_state() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();

    allow_first_save(&mut harness);

    let lock = rusqlite::Connection::open(workspace.paths.database_file()).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE; UPDATE sessions SET updated_at_ms = updated_at_ms;")
        .unwrap();

    rename_session(&mut harness, "Retry test");
    harness.get_by_label("Save").click();
    harness.step();
    wait_for_app_attempts(&mut harness, 1_200, |app| {
        app.last_failed_operation == Some(StorageOperation::Save)
    });
    assert!(harness.state().working_dirty());
    assert_eq!(
        harness.state().storage_tracker.status(),
        StorageStatus::Failed
    );
    assert!(harness.query_by_label("Retry save").is_some());

    rename_session(&mut harness, "Retry test latest");
    drop(lock);
    harness.get_by_label("Retry save").click();
    harness.step();
    wait_for_app(&mut harness, |app| {
        app.storage_tracker.status() == StorageStatus::Saved && app.working_session.id.is_some()
    });
    assert_eq!(
        harness.state().working_session.name.as_deref(),
        Some("Retry test latest")
    );
    shutdown_harness(&mut harness);

    let mut restarted = workspace.start();
    wait_for_app(&mut restarted, |app| app.working_session.restored);
    assert_eq!(
        restarted.state().working_session.name.as_deref(),
        Some("Retry test latest")
    );
    shutdown_harness(&mut restarted);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_autosave_switch_and_close_need_no_prompt() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();

    open_storage_settings(&mut harness);
    harness.get_by_label("Autosave sessions").click();
    harness.run();
    assert!(
        harness
            .query_by_role_and_label(
                egui::accesskit::Role::Dialog,
                "Save sensitive aggregate statistics?",
            )
            .is_some()
    );
    harness.get_by_label("Cancel").click();
    harness.run();
    assert!(!harness.state().settings.autosave_enabled());

    harness.get_by_label("Autosave sessions").click();
    harness.run();
    harness.get_by_label("Allow local saves").click();
    harness.step();
    wait_for_app(&mut harness, |app| {
        app.settings.autosave_enabled() && app.storage_tracker.status() != StorageStatus::Saving
    });

    rename_session(&mut harness, "Autosaved Home");
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::N);
    harness.step();
    wait_for_app(&mut harness, |app| {
        app.working_session.id.is_none()
            && app
                .sessions
                .iter()
                .any(|saved| saved.name.as_deref() == Some("Autosaved Home"))
    });
    assert!(
        harness
            .query_by_role_and_label(
                egui::accesskit::Role::Dialog,
                "Save changes before switching sessions?",
            )
            .is_none()
    );

    rename_session(&mut harness, "Autosaved Work");
    request_headless_close(&mut harness);
    assert!(
        harness
            .query_by_role_and_label(
                egui::accesskit::Role::Dialog,
                "Save changes before exiting?",
            )
            .is_none()
    );
    wait_for_app(&mut harness, |app| {
        app.allow_close
            && app.working_session.id.is_some()
            && app.storage_tracker.status() == StorageStatus::Saved
    });
    assert_eq!(
        harness.state().working_session.name.as_deref(),
        Some("Autosaved Work")
    );
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_session_switch_prompts_and_deletion() {
    let workspace = TestWorkspace::new();
    let database = workspace.paths.database_file();
    let mut harness = workspace.start();

    rename_session(&mut harness, "Home");
    assert_eq!(
        harness.state().working_session.name.as_deref(),
        Some("Home")
    );

    allow_first_save(&mut harness);
    let home_id = harness.state().working_session.id.unwrap();

    open_session_switcher(&mut harness);
    harness.get_by_label("New session").click();
    harness.run();
    assert_eq!(harness.state().working_session.id, None);
    harness.get_by_label("Save").click();
    harness.step();
    wait_for_app(&mut harness, |app| {
        app.working_session.id.is_some() && app.storage_tracker.status() == StorageStatus::Saved
    });
    let work_id = harness.state().working_session.id.unwrap();

    rename_session(&mut harness, "Work");
    assert!(harness.state().working_dirty());

    open_session_switcher(&mut harness);
    harness.get_by_label_contains("Home").click();
    harness.run();
    assert!(
        harness
            .query_by_role_and_label(
                egui::accesskit::Role::Dialog,
                "Save changes before switching sessions?",
            )
            .is_some()
    );
    harness.get_by_label("Cancel").click();
    harness.run();
    assert_eq!(harness.state().working_session.id, Some(work_id));
    assert_eq!(
        harness.state().working_session.name.as_deref(),
        Some("Work")
    );

    open_session_switcher(&mut harness);
    harness.get_by_label_contains("Home").click();
    harness.run();
    harness.get_by_label("Discard changes").click();
    harness.step();
    wait_for_app(&mut harness, |app| {
        app.working_session.id == Some(home_id) && !app.loading_session
    });
    assert_eq!(
        harness.state().working_session.name.as_deref(),
        Some("Home")
    );

    open_session_switcher(&mut harness);
    harness.get_by_label_contains("Untitled session").click();
    harness.step();
    wait_for_app(&mut harness, |app| {
        app.working_session.id == Some(work_id) && !app.loading_session
    });

    rename_session(&mut harness, "Home");
    assert!(
        harness
            .state()
            .rename_dialog
            .as_ref()
            .and_then(|dialog| dialog.error.as_ref())
            .is_some()
    );
    assert_eq!(harness.state().working_session.name, None);
    harness.get_by_label("Cancel").click();
    harness.run();

    rename_session(&mut harness, "Work");
    open_session_switcher(&mut harness);
    harness.get_by_label_contains("Home").click();
    harness.run();
    harness.get_by_label("Save and switch").click();
    harness.step();
    wait_for_app(&mut harness, |app| {
        app.working_session.id == Some(home_id)
            && !app.loading_session
            && app
                .sessions
                .iter()
                .any(|saved| saved.id == work_id && saved.name.as_deref() == Some("Work"))
    });

    open_session_switcher(&mut harness);
    harness.get_by_label_contains("Session actions").click();
    harness.run();
    harness.get_by_label("Reset statistics").click();
    harness.run();
    harness
        .get_all_by_role_and_label(egui::accesskit::Role::Button, "Reset statistics")
        .last()
        .unwrap()
        .click();
    harness.run();
    assert!(harness.state().working_dirty());
    assert_eq!(
        harness.state().working_session.name.as_deref(),
        Some("Home")
    );

    open_session_switcher(&mut harness);
    harness.get_by_label_contains("Session actions").click();
    harness.run();
    harness.get_by_label("Delete session").click();
    harness.run();
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::Dialog, "Delete session?")
            .is_some()
    );
    assert!(harness.get_by_label("Cancel").is_focused());
    harness.get_by_label("Delete permanently").click();
    harness.step();
    wait_for_app(&mut harness, |app| {
        !app.deleting_session && app.working_session.id.is_none()
    });
    assert_eq!(
        harness.state().working_session.display_name(),
        "Untitled session"
    );
    shutdown_harness(&mut harness);

    let mut restarted = workspace.start();
    assert_eq!(restarted.state().working_session.id, None);
    assert!(!restarted.state().working_session.restored);
    assert!(
        restarted
            .state()
            .sessions
            .iter()
            .any(|saved| saved.id == work_id)
    );

    rename_session(&mut restarted, "Unsaved draft");
    open_manage_sessions(&mut restarted);
    restarted
        .get_by_role_and_label(egui::accesskit::Role::Button, "Delete all saved sessions")
        .click();
    restarted.run();
    restarted.get_by_label("Delete all permanently").click();
    restarted.run();
    wait_for_app(&mut restarted, |app| {
        !app.deleting_all && app.sessions.is_empty()
    });
    assert!(!database.exists());
    assert_eq!(
        restarted.state().working_session.name.as_deref(),
        Some("Unsaved draft"),
        "deleting saved sessions must preserve an unsaved in-memory session"
    );
    shutdown_harness(&mut restarted);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_manage_sessions_renames_and_deletes_unloaded_sessions() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();

    rename_session(&mut harness, "Alpha");
    allow_first_save(&mut harness);
    let alpha_id = harness.state().working_session.id.unwrap();

    open_session_switcher(&mut harness);
    harness.get_by_label("New session").click();
    harness.run();
    rename_session(&mut harness, "Beta");
    harness.get_by_label("Save").click();
    harness.step();
    wait_for_app(&mut harness, |app| {
        app.working_session.id.is_some() && app.storage_tracker.status() == StorageStatus::Saved
    });
    let beta_id = harness.state().working_session.id.unwrap();

    open_manage_sessions(&mut harness);
    harness.get_by_label_contains("Rename Beta").click();
    harness.run();
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .type_text("Beta active");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert!(harness.state().rename_dialog.is_none());
    assert_eq!(
        harness.state().working_session.name.as_deref(),
        Some("Beta active")
    );
    assert!(harness.state().working_dirty());
    assert!(
        harness
            .query_by_label_contains("Current · Unsaved changes")
            .is_some()
    );
    assert_eq!(
        harness
            .state()
            .managed_sessions
            .iter()
            .map(|session| session.id)
            .collect::<Vec<_>>(),
        vec![beta_id, alpha_id]
    );
    assert_eq!(
        harness
            .state()
            .sessions
            .iter()
            .map(|session| session.id)
            .collect::<Vec<_>>(),
        vec![beta_id, alpha_id]
    );

    std::thread::sleep(Duration::from_millis(2));
    harness.get_by_label_contains("Rename Alpha").click();
    harness.run();
    let input = harness.get_by_role(egui::accesskit::Role::TextInput);
    assert!(input.is_focused());
    input.type_text("Archive");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.step();
    wait_for_app(&mut harness, |app| {
        app.rename_dialog.is_none()
            && !app.manage_list_loading
            && app.managed_sessions.first().is_some_and(|session| {
                session.id == alpha_id && session.name.as_deref() == Some("Archive")
            })
    });
    assert_eq!(harness.state().working_session.id, Some(beta_id));
    assert_eq!(
        harness.state().working_session.name.as_deref(),
        Some("Beta active")
    );
    assert!(harness.get_by_label_contains("Rename Archive").is_focused());
    assert_eq!(
        harness
            .state()
            .sessions
            .iter()
            .map(|session| session.id)
            .collect::<Vec<_>>(),
        vec![beta_id, alpha_id],
        "renaming must not change last-opened switcher order"
    );

    harness.get_by_label_contains("Rename Archive").click();
    harness.run();
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .type_text("Beta active");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert!(
        harness
            .state()
            .rename_dialog
            .as_ref()
            .and_then(|dialog| dialog.error.as_deref())
            .is_some_and(|error| error.contains("already uses"))
    );
    assert!(
        harness
            .get_by_role(egui::accesskit::Role::TextInput)
            .is_focused()
    );

    let over_limit = "é".repeat(41);
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .type_text(&over_limit);
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert!(
        harness
            .state()
            .rename_dialog
            .as_ref()
            .and_then(|dialog| dialog.error.as_deref())
            .is_some_and(|error| error.contains("80 UTF-8 bytes"))
    );
    assert!(
        harness
            .query_by_label_contains("0 of 80 UTF-8 bytes")
            .is_some()
    );

    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(harness.state().rename_dialog.is_none());
    assert!(harness.get_by_label_contains("Rename Archive").is_focused());

    harness.get_by_label_contains("Rename Archive").click();
    harness.run();
    harness.key_press(egui::Key::Backspace);
    harness.key_press(egui::Key::Enter);
    harness.step();
    wait_for_app(&mut harness, |app| {
        app.rename_dialog.is_none()
            && app
                .managed_sessions
                .iter()
                .any(|session| session.id == alpha_id && session.name.is_none())
    });

    harness
        .get_by_label_contains("Delete Untitled session")
        .click();
    harness.run();
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::Dialog, "Delete session?")
            .is_some()
    );
    assert!(harness.get_by_label("Cancel").is_focused());
    harness.get_by_label("Delete permanently").click();
    harness.step();
    wait_for_app(&mut harness, |app| {
        !app.deleting_session
            && !app
                .managed_sessions
                .iter()
                .any(|session| session.id == alpha_id)
    });
    assert_eq!(harness.state().working_session.id, Some(beta_id));
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_ordered_list_failures_keep_other_requests_live() {
    let workspace = TestWorkspace::new();
    let database_path = workspace.paths.database_file();
    let mut harness = workspace.start();
    let metadata = SessionMetadata {
        id: SessionId::new(42).unwrap(),
        name: Some("Managed".to_owned()),
        created_at_ms: 1,
        updated_at_ms: 2,
        last_opened_at_ms: 1,
        captured_duration_ns: 0,
        application_version: env!("CARGO_PKG_VERSION").to_owned(),
        keyboard: KeyboardContext::default(),
    };
    harness.state_mut().session_list_request_id = Some(11);
    harness.state_mut().manage_list_request_id = Some(12);
    harness.state_mut().manage_list_loading = true;

    harness
        .state_mut()
        .handle_storage_event(StorageEvent::SessionListFailed {
            request_id: 11,
            order: SessionListOrder::LastOpened,
            failure: StorageFailure {
                operation: StorageOperation::List,
                generation: None,
                database_path,
                details: "injected switcher-list failure".to_owned(),
            },
        });
    assert_eq!(harness.state().session_list_request_id, None);
    assert_eq!(harness.state().manage_list_request_id, Some(12));
    assert!(harness.state().manage_list_loading);

    harness
        .state_mut()
        .handle_storage_event(StorageEvent::SessionsListed {
            request_id: 12,
            sessions: vec![metadata],
        });
    assert_eq!(harness.state().manage_list_request_id, None);
    assert!(!harness.state().manage_list_loading);
    assert_eq!(harness.state().managed_sessions[0].id.get(), 42);
    assert_eq!(
        harness.state().last_failed_operation,
        Some(StorageOperation::List),
        "an unrelated ordered-list success must not clear the failed switcher list"
    );
    assert_eq!(
        harness.state().failed_list_order,
        Some(SessionListOrder::LastOpened)
    );

    harness.state_mut().session_list_request_id = Some(13);
    harness
        .state_mut()
        .handle_storage_event(StorageEvent::SessionsListed {
            request_id: 13,
            sessions: Vec::new(),
        });
    assert!(harness.state().storage_error.is_none());
    assert!(harness.state().storage_error_details.is_none());
    assert_eq!(harness.state().last_failed_operation, None);
    assert_eq!(harness.state().failed_list_order, None);
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn successful_storage_retries_clear_only_the_matching_failure() {
    let workspace = TestWorkspace::new();
    let database_path = workspace.paths.database_file();
    let mut harness = workspace.start();
    let session_id = SessionId::new(77).unwrap();
    let stored_session = || StoredSession {
        metadata: SessionMetadata {
            id: session_id,
            name: Some("Recovered session".to_owned()),
            created_at_ms: 1,
            updated_at_ms: 2,
            last_opened_at_ms: 3,
            captured_duration_ns: 0,
            application_version: env!("CARGO_PKG_VERSION").to_owned(),
            keyboard: KeyboardContext::default(),
        },
        metrics: Vec::new(),
    };
    let failure = |operation| {
        StorageEvent::Failed(StorageFailure {
            operation,
            generation: None,
            database_path: database_path.clone(),
            details: format!("injected {operation:?} failure"),
        })
    };

    harness.state_mut().load_request_id = 7;
    harness.state_mut().loading_session = true;
    harness
        .state_mut()
        .handle_storage_event(failure(StorageOperation::Load));
    assert_eq!(
        harness.state().last_failed_operation,
        Some(StorageOperation::Load)
    );
    harness
        .state_mut()
        .handle_storage_event(StorageEvent::SessionLoaded {
            request_id: 7,
            session: Some(stored_session()),
        });
    assert!(harness.state().storage_error.is_none());
    assert!(harness.state().storage_error_details.is_none());

    harness.state_mut().load_request_id = 8;
    harness
        .state_mut()
        .handle_storage_event(failure(StorageOperation::Load));
    harness
        .state_mut()
        .handle_storage_event(failure(StorageOperation::Delete));
    harness
        .state_mut()
        .handle_storage_event(StorageEvent::SessionLoaded {
            request_id: 8,
            session: Some(stored_session()),
        });
    assert_eq!(
        harness.state().last_failed_operation,
        Some(StorageOperation::Delete),
        "a success for an older operation must not clear a newer failure"
    );
    harness
        .state_mut()
        .handle_storage_event(StorageEvent::SessionDeleted {
            session_id,
            deleted: false,
        });
    assert!(harness.state().storage_error.is_none());
    assert!(harness.state().storage_error_details.is_none());

    harness
        .state_mut()
        .handle_storage_event(failure(StorageOperation::DeleteAll));
    harness
        .state_mut()
        .handle_storage_event(StorageEvent::AllDeleted);
    assert!(harness.state().storage_error.is_none());
    assert!(harness.state().storage_error_details.is_none());
    assert_eq!(harness.state().last_failed_operation, None);

    harness.state_mut().load_request_id = 9;
    harness
        .state_mut()
        .handle_storage_event(failure(StorageOperation::Load));
    harness
        .state_mut()
        .handle_storage_event(StorageEvent::SessionLoaded {
            request_id: 9,
            session: None,
        });
    assert!(harness.state().storage_error.is_none());
    assert!(harness.state().storage_error_details.is_none());
    assert_eq!(harness.state().last_failed_operation, None);
    assert!(
        harness
            .state()
            .session_notice
            .as_deref()
            .is_some_and(|notice| notice.contains("Started an untitled session"))
    );

    {
        let app = harness.state_mut();
        app.process_input(SystemTime::now(), KeyCode::KEY_A, KeyEventKind::Press);
        app.begin_save(None);
        let generation = app.storage_tracker.in_flight().unwrap();
        app.handle_storage_event(failure(StorageOperation::Delete));
        app.handle_storage_event(StorageEvent::Saved {
            generation,
            session_id,
        });
        assert_eq!(
            app.last_failed_operation,
            Some(StorageOperation::Delete),
            "a successful save must not clear an unrelated delete failure"
        );

        app.handle_storage_event(StorageEvent::Opened {
            sessions: Vec::new(),
            selected: None,
        });
        assert_eq!(
            app.last_failed_operation,
            Some(StorageOperation::Delete),
            "a successful open must not clear an unrelated delete failure"
        );

        app.open_rename_dialog(
            RenameTarget::Saved(session_id),
            Some("Recovered session"),
            egui::Id::new("retry-test-rename-opener"),
        );
        if let Some(dialog) = &mut app.rename_dialog {
            dialog.request_id = Some(91);
            dialog.submitting = true;
        }
        app.handle_storage_event(StorageEvent::SessionRenamed {
            request_id: 91,
            session: Some(stored_session().metadata),
        });
        assert_eq!(
            app.last_failed_operation,
            Some(StorageOperation::Delete),
            "a successful rename must not clear an unrelated delete failure"
        );
        app.handle_storage_event(StorageEvent::SessionDeleted {
            session_id,
            deleted: false,
        });
    }
    assert!(harness.state().storage_error.is_none());
    assert!(harness.state().storage_error_details.is_none());
    assert_eq!(harness.state().last_failed_operation, None);
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_manage_sessions_is_bounded_at_minimum_size() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start_with_size(egui::vec2(800.0, 600.0));
    let now = 1_785_584_000_000_i64;
    let sessions = (1..=5)
        .map(|index| SessionMetadata {
            id: SessionId::new(index).unwrap(),
            name: Some(format!("Session {index}")),
            created_at_ms: now - index * 10_000,
            updated_at_ms: now - index * 1_000,
            last_opened_at_ms: now - index * 2_000,
            captured_duration_ns: index * 1_000_000_000,
            application_version: env!("CARGO_PKG_VERSION").to_owned(),
            keyboard: KeyboardContext {
                display_name: Some(format!("Keyboard {index}")),
                model: "pc105".to_owned(),
                layout: "us".to_owned(),
                variant: String::new(),
            },
        })
        .collect::<Vec<_>>();
    harness.state_mut().view = AppView::Sessions;
    harness.state_mut().working_session.id = Some(sessions[0].id);
    harness.state_mut().working_session.name = sessions[0].name.clone();
    harness.state_mut().storage_tracker.reset_saved();
    harness.state_mut().sessions.clone_from(&sessions);
    harness.state_mut().managed_sessions = sessions;
    harness.run();

    assert!(harness.query_by_label("Manage Sessions").is_some());
    assert!(
        harness
            .query_by_label_contains("Rename Session 1")
            .is_some()
    );
    let delete_all =
        harness.get_by_role_and_label(egui::accesskit::Role::Button, "Delete all saved sessions");
    let bounds = delete_all.accesskit_node().bounding_box().unwrap();
    assert!(bounds.x0 >= 168.0 && bounds.x1 <= 800.0, "{bounds:?}");
    assert!(bounds.y0 >= 64.0 && bounds.y1 <= 600.0, "{bounds:?}");
    assert_eq!(
        harness
            .get_all_by_role(egui::accesskit::Role::ScrollBar)
            .count(),
        1,
        "Manage Sessions should own one bounded list without an outer page scrollbar"
    );
    assert!(harness.query_by_label_contains("Search").is_none());

    for _ in 0..10 {
        harness
            .get_all_by_role(egui::accesskit::Role::Button)
            .find(|node| {
                node.accesskit_node()
                    .label()
                    .is_some_and(|label| label.starts_with("Rename Session"))
            })
            .unwrap()
            .scroll_down();
        harness.run();
    }
    harness.get_by_label_contains("Rename Session 5").click();
    harness.run();
    assert!(matches!(
        harness
            .state()
            .rename_dialog
            .as_ref()
            .map(|dialog| dialog.target),
        Some(super::RenameTarget::Saved(session_id)) if session_id.get() == 5
    ));
    harness.key_press(egui::Key::Escape);
    harness.run();
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_ten_thousand_session_lists_render_bounded_rows() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start_with_size(egui::vec2(800.0, 600.0));
    let sessions = (1..=10_000)
        .map(|id| SessionMetadata {
            id: SessionId::new(id).unwrap(),
            name: Some(format!("Session {id}")),
            created_at_ms: id,
            updated_at_ms: id,
            last_opened_at_ms: id,
            captured_duration_ns: 0,
            application_version: "test".to_owned(),
            keyboard: KeyboardContext::default(),
        })
        .collect::<Vec<_>>();
    harness.state_mut().view = AppView::Sessions;
    harness.state_mut().sessions.clone_from(&sessions);
    harness.state_mut().managed_sessions = sessions;
    harness.run();

    let manage_buttons = harness
        .get_all_by_role(egui::accesskit::Role::Button)
        .count();
    assert!(
        manage_buttons < 50,
        "virtualized Manage Sessions rendered {manage_buttons} buttons"
    );
    open_session_switcher(&mut harness);
    let popup_buttons = harness
        .get_all_by_role(egui::accesskit::Role::Button)
        .count();
    assert!(
        popup_buttons < 80,
        "virtualized switcher rendered {popup_buttons} buttons"
    );
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_shell_navigation_shortcuts_and_view_state_are_stable() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start_with_size(egui::vec2(800.0, 600.0));

    assert_eq!(harness.state().view, AppView::Overview);
    for label in ["Overview", "Key Usage", "Timing", "Corrections", "Settings"] {
        assert!(
            harness
                .query_by_role_and_label(egui::accesskit::Role::Button, label)
                .is_some(),
            "missing navigation button {label}"
        );
    }
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::Button, "Switch active session")
            .is_some()
    );
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::Button, "Start keyboard capture")
            .is_some()
    );
    assert_top_bar_controls_are_ordered(&harness, 800.0);
    assert!(
        harness
            .query_by_label_contains("Storage is local")
            .is_none()
    );

    let bounds = |node: egui_kittest::Node<'_>| node.accesskit_node().bounding_box().unwrap();
    let brand = bounds(harness.get_by_label("evtap"));
    let overview = bounds(harness.get_by_role_and_label(egui::accesskit::Role::Button, "Overview"));
    let session =
        harness.get_by_role_and_label(egui::accesskit::Role::Button, "Switch active session");
    let session_bounds = bounds(session);
    assert!(
        brand.y1 <= overview.y0,
        "brand={brand:?}, overview={overview:?}"
    );
    assert!(
        overview.x1 <= session_bounds.x0,
        "navigation={overview:?}, session={session_bounds:?}"
    );
    assert!(overview.height() >= 44.0, "overview={overview:?}");

    session.focus();
    harness.run();
    assert!(harness.query_by_label_contains("captured ·").is_some());
    assert!(harness.query_by_label_contains("Captured:").is_none());

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Key Usage")
        .click();
    harness.run();
    assert_eq!(harness.state().view, AppView::KeyUsage);

    harness.state_mut().new_working_session();
    harness.step();
    assert_eq!(harness.state().view, AppView::KeyUsage);

    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Comma);
    harness.run();
    assert_eq!(
        harness.state().view,
        AppView::Settings(super::SettingsSection::Input)
    );

    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::K);
    harness.run();
    assert!(harness.state().session_switcher_open);
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(!harness.state().session_switcher_open);

    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_settings_keyboard_search_reset_and_capture_lock_are_accessible() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start_with_size(egui::vec2(800.0, 600.0));

    harness.get_by_label_contains("Settings").click();
    harness.run();
    for category in [
        "Input",
        "Keyboard interpretation",
        "Storage & privacy",
        "Appearance",
        "About",
    ] {
        assert!(
            harness
                .query_by_role_and_label(egui::accesskit::Role::Button, category)
                .is_some(),
            "missing Settings category {category}"
        );
    }
    assert!(harness.query_by_label("Capture keyboard").is_some());
    assert!(
        harness
            .get_all_by_role(egui::accesskit::Role::Button)
            .filter(|node| node.accesskit_node().label().as_deref() == Some("Rescan keyboards"))
            .count()
            >= 2,
        "the top bar and Input card should both expose keyboard rescanning"
    );

    harness.get_by_label("Keyboard interpretation").click();
    harness.run();
    {
        let app = harness.state_mut();
        app.available_models = vec!["fixture-model".to_owned(), "other-model".to_owned()];
        app.available_layouts = vec!["fixture-layout".to_owned()];
    }
    harness.step();

    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Model")
        .click();
    harness.run();
    let search = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Search model");
    search.focus();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Search model")
        .type_text("fixture");
    harness.run();
    assert!(harness.query_by_label("other-model").is_none());
    harness.get_by_label("fixture-model").click();
    harness.run();
    assert_eq!(harness.state().model, "fixture-model");
    assert_eq!(harness.state().settings.keyboard_model(), "fixture-model");
    assert!(!harness.state().working_dirty());

    harness.get_by_label("Reset to defaults").click_accesskit();
    harness.run();
    assert!(harness.state().model.is_empty());
    assert!(harness.state().layout.is_empty());
    assert!(harness.state().variant.is_empty());
    assert!(harness.state().settings.keyboard_model().is_empty());

    {
        let app = harness.state_mut();
        app.model = "fixture-model".to_owned();
        app.listener_state = ListenerState::Connecting;
    }
    harness.step();
    for label in ["Model", "Layout", "Variant"] {
        assert!(
            harness
                .get_by_role_and_label(egui::accesskit::Role::ComboBox, label)
                .accesskit_node()
                .is_disabled(),
            "{label} should be disabled while capture is transitioning"
        );
    }
    assert!(
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Reset to defaults")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        harness
            .query_by_label_contains("Stop capture before changing keyboard interpretation")
            .is_some()
    );

    harness.state_mut().listener_state = ListenerState::Idle;
    harness.get_by_label("Storage & privacy").click();
    harness.run();
    assert_eq!(
        harness
            .get_all_by_role(egui::accesskit::Role::ScrollBar)
            .count(),
        1,
        "Settings should use the shell's single content scrollbar"
    );
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Delete all saved sessions")
        .scroll_to_me();
    harness.run();
    let delete_bounds = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Delete all saved sessions")
        .accesskit_node()
        .bounding_box()
        .unwrap();
    assert!(
        delete_bounds.y0 >= 0.0 && delete_bounds.y1 <= 600.0,
        "destructive data actions should be reachable at 800×600: {delete_bounds:?}"
    );

    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_keyboard_configuration_failures_preserve_active_and_saved_context() {
    let workspace = TestWorkspace::new();
    {
        let mut harness = workspace.start();
        harness.get_by_label_contains("Settings").click();
        harness.run();
        harness.get_by_label("Keyboard interpretation").click();
        harness.run();

        let invalid_layout = "evtap-definitely-invalid-layout";
        harness.state_mut().available_layouts = vec![invalid_layout.to_owned()];
        harness.step();
        let text_before = harness
            .state()
            .xkb_state
            .key_get_utf8((KeyCode::KEY_A.code() + 8).into());
        harness
            .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Layout")
            .click();
        harness.run();
        harness.get_by_label(invalid_layout).click();
        harness.run();

        assert!(harness.state().layout.is_empty());
        assert!(harness.state().settings.keyboard_layout().is_empty());
        assert!(harness.state().working_session.keyboard.layout.is_empty());
        assert_eq!(
            harness
                .state()
                .xkb_state
                .key_get_utf8((KeyCode::KEY_A.code() + 8).into()),
            text_before,
            "a rejected XKB choice must not replace the active interpreter"
        );
        assert!(harness.state().keyboard_error.is_some());
        assert!(!harness.state().working_dirty());

        harness.state_mut().available_models = vec!["pc105".to_owned()];
        harness.state_mut().settings_load_failed = true;
        harness.step();
        harness
            .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Model")
            .click();
        harness.run();
        harness.get_by_label("pc105").click();
        harness.run();

        assert!(harness.state().model.is_empty());
        assert!(harness.state().settings.keyboard_model().is_empty());
        assert!(harness.state().working_session.keyboard.model.is_empty());
        assert!(harness.state().settings_error.is_some());
        assert!(harness.state().keyboard_error.is_none());
        assert!(!harness.state().working_dirty());

        harness.state_mut().settings_load_failed = false;
        harness
            .state_mut()
            .settings
            .set_appearance_preference(AppearancePreference::Dark);
        assert!(harness.state_mut().save_settings());
        shutdown_harness(&mut harness);
    }

    let mut restarted = workspace.start();
    assert!(restarted.state().settings.keyboard_model().is_empty());
    assert!(restarted.state().settings.keyboard_layout().is_empty());
    assert!(restarted.state().settings.keyboard_variant().is_empty());
    assert_eq!(
        restarted.state().settings.appearance_preference(),
        AppearancePreference::Dark,
        "a later successful settings write must not leak a rejected keyboard choice"
    );
    shutdown_harness(&mut restarted);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_settings_storage_appearance_and_about_persist_without_network() {
    let workspace = TestWorkspace::new();
    {
        let mut harness = workspace.start();
        open_storage_settings(&mut harness);

        assert!(harness.query_by_label_contains("Database path").is_some());
        assert!(harness.query_by_label_contains("Disk usage:").is_some());
        assert!(
            harness
                .query_by_label_contains("immediately when enabled")
                .is_some()
        );
        assert!(
            harness
                .query_by_label_contains("creating a new session")
                .is_some()
        );
        assert!(harness.query_by_label("Save now").is_none());
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Copy database path")
            .click();
        harness.run();
        assert!(harness.query_by_label("Database path copied").is_some());

        harness.get_by_label("Autosave sessions").click();
        harness.run();
        assert!(
            harness
                .query_by_role_and_label(
                    egui::accesskit::Role::Dialog,
                    "Save sensitive aggregate statistics?",
                )
                .is_some()
        );
        harness.get_by_label("Allow local saves").click();
        harness.run();
        assert!(harness.state().settings.autosave_enabled());
        assert!(harness.state().settings.storage_disclosure_acknowledged());

        harness
            .get_by_label("Review local storage disclosure")
            .click();
        harness.run();
        assert!(
            harness
                .query_by_role_and_label(egui::accesskit::Role::Dialog, "Local storage disclosure",)
                .is_some()
        );
        assert!(harness.get_by_label("Close").is_focused());
        assert!(harness.query_by_label("Allow local saves").is_none());
        harness.get_by_label("Close").click();
        harness.run();
        assert!(harness.state().settings.autosave_enabled());

        harness.get_by_label("Appearance").click();
        harness.run();
        harness.get_by_label("Dark").click();
        harness.run();
        assert_eq!(
            harness.state().settings.appearance_preference(),
            AppearancePreference::Dark
        );

        harness.state_mut().settings_load_failed = true;
        harness.get_by_label("Light").click();
        harness.run();
        assert_eq!(
            harness.state().settings.appearance_preference(),
            AppearancePreference::Dark,
            "failed preference writes must restore the previous appearance"
        );
        assert!(
            harness
                .query_by_label("Settings could not be saved")
                .is_some()
        );
        harness.state_mut().settings_load_failed = false;

        harness.get_by_label("About").click();
        harness.run();
        assert!(
            harness
                .query_by_label_contains(concat!("Version ", env!("CARGO_PKG_VERSION")))
                .is_some()
        );
        for link in [
            "Repository",
            "Releases",
            "Documentation",
            "MIT",
            "Apache-2.0",
            "Inter font (SIL OFL 1.1)",
        ] {
            assert!(
                harness.query_by_label(link).is_some(),
                "missing About link {link}"
            );
        }
        assert!(
            harness
                .query_by_label_contains("no automatic update checks")
                .is_some()
        );
        shutdown_harness(&mut harness);
    }

    let mut restarted = workspace.start();
    assert!(restarted.state().settings.autosave_enabled());
    assert!(restarted.state().settings.storage_disclosure_acknowledged());
    assert_eq!(
        restarted.state().settings.appearance_preference(),
        AppearancePreference::Dark
    );
    shutdown_harness(&mut restarted);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_overview_spacing_is_exact_at_compact_width() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start_with_size(egui::vec2(800.0, 900.0));
    install_analytics_fixture(harness.state_mut(), 9);
    harness.step();

    let mut usage_bounds = harness
        .get_all_by_role_and_label(egui::accesskit::Role::Button, "Open Key Usage")
        .map(|node| node.accesskit_node().bounding_box().unwrap())
        .collect::<Vec<_>>();
    usage_bounds.sort_by(|left, right| left.y0.total_cmp(&right.y0));
    assert_eq!(usage_bounds.len(), 3);
    let top_row = &usage_bounds[..2];
    let dwell = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Open Dwell timing")
        .accesskit_node()
        .bounding_box()
        .unwrap();
    let flight = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Open Flight timing")
        .accesskit_node()
        .bounding_box()
        .unwrap();
    let preview = usage_bounds[2];

    assert!(top_row.iter().all(|bounds| {
        (bounds.y0 - top_row[0].y0).abs() < 1.0 && (bounds.y1 - top_row[0].y1).abs() < 1.0
    }));
    assert!((dwell.y0 - flight.y0).abs() < 1.0 && (dwell.y1 - flight.y1).abs() < 1.0);
    assert!((dwell.y0 - top_row[0].y1 - 8.0).abs() < 1.0);
    assert!((preview.y0 - dwell.y1 - 12.0).abs() < 1.0);

    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_analytics_pages_compose_metrics_and_preserve_temporary_state() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start_with_size(egui::vec2(1_000.0, 900.0));

    assert!(
        harness
            .query_by_label("Start with a short typing session")
            .is_some()
    );
    {
        let app = harness.state_mut();
        install_analytics_fixture(app, 9);
        app.recovery_messages = vec![
            "Dwell time could not be restored.".to_owned(),
            "One bigram payload was skipped.".to_owned(),
        ];
    }
    harness.step();
    assert!(
        harness
            .query_by_label("Start with a short typing session")
            .is_none()
    );
    let mut summary_card_bounds = harness
        .get_all_by_role_and_label(egui::accesskit::Role::Button, "Open Key Usage")
        .map(|node| node.accesskit_node().bounding_box().unwrap())
        .collect::<Vec<_>>();
    summary_card_bounds.sort_by(|left, right| left.y0.total_cmp(&right.y0));
    summary_card_bounds.truncate(2);
    summary_card_bounds.push(
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Open Dwell timing")
            .accesskit_node()
            .bounding_box()
            .unwrap(),
    );
    summary_card_bounds.push(
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Open Flight timing")
            .accesskit_node()
            .bounding_box()
            .unwrap(),
    );
    let first_summary = summary_card_bounds[0];
    assert!(
        summary_card_bounds.iter().all(|bounds| {
            (bounds.y0 - first_summary.y0).abs() < 1.0 && (bounds.y1 - first_summary.y1).abs() < 1.0
        }),
        "overview summary tiles should share one top edge and height"
    );
    assert!(
        harness
            .query_by_label("Some analytics could not be restored")
            .is_some()
    );
    assert!(
        harness
            .query_all_by_label_contains("Affected analytics restarted empty")
            .next()
            .is_some()
    );
    harness.get_by_label("Affected analytics").click();
    harness.run();
    assert!(
        harness
            .query_by_label("Dwell time could not be restored.")
            .is_some()
    );
    assert!(
        harness
            .query_by_label("One bigram payload was skipped.")
            .is_some()
    );
    harness.get_by_label("Dismiss").click();
    harness.step();
    harness.step();
    assert!(harness.state().recovery_messages.is_empty());

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Open Dwell timing")
        .click();
    harness.run();
    assert_eq!(harness.state().view, AppView::Timing(TimingView::Dwell));
    assert!(harness.query_by_label("Dwell time").is_some());
    assert!(harness.query_by_label("About this metric").is_none());
    assert!(harness.query_by_label_contains("Captured:").is_none());

    harness.get_by_label("Flight").click();
    harness.run();
    assert_eq!(harness.state().view, AppView::Timing(TimingView::Flight));
    assert!(harness.query_by_label("Flight time").is_some());

    harness.get_by_label("Bigrams").click();
    harness.run();
    assert_eq!(harness.state().view, AppView::Timing(TimingView::Bigrams));
    assert!(harness.query_by_label("Fastest pairs").is_some());
    assert!(harness.query_by_label("Slowest pairs").is_some());

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Corrections")
        .click();
    harness.run();
    assert!(harness.query_by_label("Most-deleted text").is_some());
    assert!(harness.query_by_label("Inferred corrections").is_some());

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Timing")
        .click();
    harness.run();
    assert_eq!(harness.state().view, AppView::Timing(TimingView::Bigrams));

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Key Usage")
        .click();
    harness.run();
    assert!(harness.query_by_label("Physical key ranking").is_some());
    assert!(harness.query_by_label("Showing 8 of 9").is_some());
    harness.ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::new(("key-usage", "analysis-expansion-state")),
            true,
        );
    });
    harness.step();
    assert!(harness.query_by_label("Showing 9 of 9").is_some());

    install_analytics_fixture(harness.state_mut(), 9);
    harness.step();
    assert!(harness.query_by_label("Showing 9 of 9").is_some());
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Overview")
        .click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Key Usage")
        .click();
    harness.run();
    assert!(harness.query_by_label("Showing 9 of 9").is_some());

    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_shell_shortcuts_do_not_cross_open_prompts() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();
    let created_at_ms = harness.state().working_session.created_at_ms;

    harness.state_mut().confirm_delete_all = true;
    harness.step();
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::N);
    harness.run();

    assert!(harness.state().confirm_delete_all);
    assert_eq!(harness.state().working_session.created_at_ms, created_at_ms);
    assert_eq!(harness.state().view, AppView::Overview);

    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(!harness.state().confirm_delete_all);

    harness.state_mut().loading_session = true;
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::S);
    harness.step();
    assert!(harness.state().disclosure_prompt.is_none());
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::N);
    harness.step();
    assert_eq!(harness.state().working_session.created_at_ms, created_at_ms);
    harness.state_mut().loading_session = false;

    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_shell_capture_controls_reflect_device_and_busy_states() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();

    {
        let app = harness.state_mut();
        app.devices = Some(Vec::new());
        app.selected_device = None;
        app.scan_warning = None;
        app.scan_error = None;
        app.listener_state = ListenerState::Idle;
    }
    harness.step();

    assert!(
        harness
            .query_by_label_contains("Capture status: Unavailable")
            .is_some()
    );
    assert_eq!(
        harness
            .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Keyboard")
            .accesskit_node()
            .value(),
        Some("No readable keyboard".to_owned())
    );
    let start =
        harness.get_by_role_and_label(egui::accesskit::Role::Button, "Start keyboard capture");
    assert!(start.accesskit_node().is_disabled());

    {
        let app = harness.state_mut();
        app.devices = Some(vec![DeviceMetadata {
            path: "/dev/input/event-test".into(),
            name: "Fixture keyboard".to_owned(),
            physical_path: "fixture/input0".to_owned(),
        }]);
        app.selected_device = Some(0);
    }
    harness.step();

    assert!(
        harness
            .query_by_label_contains("Capture status: Ready")
            .is_some()
    );
    let start =
        harness.get_by_role_and_label(egui::accesskit::Role::Button, "Start keyboard capture");
    assert!(!start.accesskit_node().is_disabled());

    harness.state_mut().listener_state = ListenerState::Connecting;
    harness.step();

    assert!(
        harness
            .query_by_label_contains("Capture status: Connecting…")
            .is_some()
    );
    assert!(
        harness
            .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Keyboard")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Rescan keyboards")
            .accesskit_node()
            .is_disabled()
    );

    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn scanner_outcomes_distinguish_absence_permissions_and_incomplete_lists() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();
    let issue = |kind| DeviceScanIssue {
        path: "/dev/input/event-fixture".to_owned(),
        message: "fixture failure".to_owned(),
        kind,
    };

    harness.state_mut().apply_scan_report(ScanReport {
        devices: Vec::new(),
        issues: Vec::new(),
    });
    assert_eq!(
        harness.state().scan_warning,
        Some(ScanWarning::NoKeyboardDetected)
    );

    harness.state_mut().apply_scan_report(ScanReport {
        devices: Vec::new(),
        issues: vec![issue(DeviceScanIssueKind::PermissionDenied)],
    });
    assert_eq!(
        harness.state().scan_warning,
        Some(ScanWarning::PermissionDenied { count: 1 })
    );

    harness.state_mut().apply_scan_report(ScanReport {
        devices: Vec::new(),
        issues: vec![issue(DeviceScanIssueKind::Unavailable)],
    });
    assert_eq!(
        harness.state().scan_warning,
        Some(ScanWarning::Unavailable { count: 1 })
    );

    harness.state_mut().apply_scan_report(ScanReport {
        devices: vec![DeviceMetadata {
            path: "/dev/input/event-readable".into(),
            name: "Readable fixture".to_owned(),
            physical_path: "fixture/input0".to_owned(),
        }],
        issues: vec![issue(DeviceScanIssueKind::PermissionDenied)],
    });
    assert_eq!(
        harness.state().scan_warning,
        Some(ScanWarning::Incomplete {
            issue_count: 1,
            permission_denied: 1,
        })
    );
    harness.step();
    assert!(
        harness
            .query_by_label("Keyboard list may be incomplete")
            .is_some()
    );
    assert!(harness.query_by_label("Permission help").is_some());
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn listener_failure_stops_autosaves_invalidates_and_rescans_without_restart() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();
    wait_for_app(&mut harness, |app| app.devices.is_some());
    {
        let app = harness.state_mut();
        app.devices = Some(vec![DeviceMetadata {
            path: "/dev/input/event-fixture".into(),
            name: "Failure fixture".to_owned(),
            physical_path: "fixture/input0".to_owned(),
        }]);
        app.selected_device = Some(0);
        app.settings.set_storage_disclosure_acknowledged(true);
        app.settings.set_autosave_enabled(true);
        assert!(app.save_settings());
        app.working_session.start_capture();
        app.process_input(SystemTime::now(), KeyCode::KEY_A, KeyEventKind::Press);
        app.finish_listener_stop(
            StopReason::ReadFailed("fixture read error".to_owned()).to_string(),
            true,
        );

        assert_eq!(app.listener_state, ListenerState::Failed);
        assert!(app.listener.is_none());
        assert!(app.devices.is_none());
        assert!(app.selected_device.is_none());
        assert!(!app.select_remembered_after_scan);
        assert!(
            app.capture_error
                .as_deref()
                .is_some_and(|error| error.contains("fixture read error"))
        );
        assert_eq!(app.storage_tracker.status(), StorageStatus::Saving);
    }
    harness.step();
    assert!(harness.query_by_label("Capture stopped").is_some());
    assert!(
        harness
            .query_all_by_label_contains("capture will not restart automatically")
            .next()
            .is_some()
    );
    assert!(harness.state().listener.is_none());
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn true_modals_keep_safe_focus_block_background_and_fit_scaled_minimum() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start_with_size_and_scale(egui::vec2(800.0, 600.0), 1.25);

    open_session_switcher(&mut harness);
    harness
        .get_by_label_contains("Session actions")
        .click_accesskit();
    harness.run();
    harness.get_by_label("Reset statistics").click_accesskit();
    harness.run();

    let dialog = harness.get_by_role_and_label(egui::accesskit::Role::Dialog, "Reset statistics?");
    assert!(
        dialog.children().next().is_some(),
        "modal dialog should own its reading-order content"
    );
    let cancel = harness.get_by_label("Cancel");
    assert!(cancel.is_focused());
    let cancel_bounds = cancel.accesskit_node().bounding_box().unwrap();
    assert!(
        cancel_bounds.x0 >= 0.0
            && cancel_bounds.y0 >= 0.0
            && cancel_bounds.x1 <= 640.0
            && cancel_bounds.y1 <= 480.0,
        "scaled modal action should remain in the viewport: {cancel_bounds:?}"
    );

    harness.get_by_label_contains("Settings").click();
    harness.run();
    assert_eq!(harness.state().view, AppView::Overview);
    assert!(!harness.state().confirm_reset);
    harness.run();
    assert!(harness.get_by_label("Switch active session").is_focused());

    open_session_switcher(&mut harness);
    harness
        .get_by_label_contains("Session actions")
        .click_accesskit();
    harness.run();
    harness.get_by_label("Reset statistics").click_accesskit();
    harness.run();
    assert!(harness.get_by_label("Cancel").is_focused());
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert!(!harness.state().confirm_reset);
    harness.run();
    assert!(harness.get_by_label("Switch active session").is_focused());

    harness
        .state_mut()
        .process_input(SystemTime::now(), KeyCode::KEY_A, KeyEventKind::Press);
    harness.step();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Save")
        .click_accesskit();
    harness.run();
    assert!(
        harness
            .query_by_role_and_label(
                egui::accesskit::Role::Dialog,
                "Save sensitive aggregate statistics?",
            )
            .is_some()
    );
    for label in ["Allow local saves", "Cancel"] {
        let bounds = harness
            .get_by_label(label)
            .accesskit_node()
            .bounding_box()
            .unwrap();
        assert!(
            bounds.x0 >= 0.0 && bounds.y0 >= 0.0 && bounds.x1 <= 640.0 && bounds.y1 <= 480.0,
            "scaled disclosure action should remain in the viewport: {bounds:?}"
        );
    }
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(harness.state().disclosure_prompt.is_none());

    shutdown_harness(&mut harness);
}

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
        "Not saved"
    );
    assert_eq!(storage_status_label(StorageStatus::Saved, true), "Saved");
    assert_eq!(
        storage_status_label(StorageStatus::Dirty, true),
        "Unsaved changes"
    );
    assert_eq!(storage_status_label(StorageStatus::Saving, true), "Saving…");
    assert_eq!(
        storage_status_label(StorageStatus::Failed, true),
        "Storage failed"
    );
    assert_eq!(
        storage_status_label_for_operation(
            StorageStatus::Failed,
            true,
            Some(StorageOperation::Save),
        ),
        "Save failed"
    );
    assert_eq!(
        storage_status_label_for_operation(
            StorageStatus::Failed,
            true,
            Some(StorageOperation::Load),
        ),
        "Load failed"
    );
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
    fs::write(temporary.path().join("evtap.sqlite3-journal"), [0_u8; 11]).unwrap();

    assert_eq!(database_disk_usage(&database), 26);
}
