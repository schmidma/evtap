use std::{
    fs, thread,
    time::{Duration, SystemTime},
};

use super::{
    App, BoundaryPolicy, HACK_FONT_NAME, ListenerState, SessionMetadata, StorageOperation,
    StorageStatus, boundary_policy, font_definitions,
    view::{session_selector_label, storage_status_label},
};
use crate::{
    input::KeyEventKind,
    paths::AppPaths,
    session::{KeyboardContext, SessionId},
    storage::database_disk_usage,
};
use eframe::egui;
use egui_kittest::{Harness, kittest::Queryable};
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
        let paths = self.paths.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 1_200.0))
            .build_eframe(move |creation_context| App::new(creation_context, paths).unwrap());
        wait_for_app(&mut harness, |app| app.initial_storage_open_handled);
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
    panic!("headless app did not reach the expected state");
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

fn rename_session(harness: &mut Harness<'_, App>, name: &str) {
    harness.get_by_label("Rename").click();
    harness.run();
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .focus();
    harness.run();
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .type_text(name);
    harness.run();
    harness.get_by_label("Apply").click();
    harness.run();
}

fn allow_first_save(harness: &mut Harness<'_, App>) {
    harness.get_by_label("Save now").click();
    harness.run();
    harness.get_by_label("Allow local saves").click();
    harness.step();
    wait_for_app(harness, |app| {
        app.working_session.id.is_some() && app.storage_tracker.status() == StorageStatus::Saved
    });
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

        harness.get_by_label("Save now").click();
        harness.run();
        assert!(
            harness
                .query_by_label("Save sensitive aggregate statistics?")
                .is_some()
        );
        assert!(!database.exists());

        harness.get_by_label("Cancel").click();
        harness.run();
        assert!(!database.exists());

        harness.get_by_label("Save now").click();
        harness.run();
        harness.get_by_label("Allow local saves").click();
        harness.run();
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
            .is_some()
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
            .query_by_label("Save changes before exiting?")
            .is_some()
    );
    harness.get_by_label("Cancel").click();
    harness.run();
    assert!(!harness.state().allow_close);
    assert!(harness.state().working_dirty());

    request_headless_close(&mut harness);
    harness.get_by_label("Save and exit").click();
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
    harness.get_by_label("Save now").click();
    harness.step();
    wait_for_app_attempts(&mut harness, 1_200, |app| {
        app.last_failed_operation == Some(StorageOperation::Save)
    });
    assert!(harness.state().working_dirty());
    assert_eq!(
        harness.state().storage_tracker.status(),
        StorageStatus::Failed
    );
    assert!(harness.query_by_label("Retry storage operation").is_some());

    rename_session(&mut harness, " latest");
    drop(lock);
    harness.get_by_label("Retry storage operation").click();
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

    harness.get_by_label("Autosave sessions").click();
    harness.run();
    assert!(
        harness
            .query_by_label("Save sensitive aggregate statistics?")
            .is_some()
    );
    harness.get_by_label("Cancel").click();
    harness.run();
    assert!(!harness.state().settings.autosave_enabled());

    harness.get_by_label("Autosave sessions").click();
    harness.run();
    harness.get_by_label("Allow local saves").click();
    harness.run();
    assert!(harness.state().settings.autosave_enabled());

    rename_session(&mut harness, "Autosaved Home");
    harness.get_by_label("New session").click();
    harness.run();
    wait_for_app(&mut harness, |app| {
        app.working_session.id.is_none()
            && app
                .sessions
                .iter()
                .any(|saved| saved.name.as_deref() == Some("Autosaved Home"))
    });
    assert!(
        harness
            .query_by_label("Save changes before switching sessions?")
            .is_none()
    );

    rename_session(&mut harness, "Autosaved Work");
    request_headless_close(&mut harness);
    assert!(
        harness
            .query_by_label("Save changes before exiting?")
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

    harness.get_by_label("New session").click();
    harness.run();
    assert_eq!(harness.state().working_session.id, None);
    harness.get_by_label("Save now").click();
    harness.run();
    wait_for_app(&mut harness, |app| {
        app.working_session.id.is_some() && app.storage_tracker.status() == StorageStatus::Saved
    });
    let work_id = harness.state().working_session.id.unwrap();

    rename_session(&mut harness, "Work");
    assert!(harness.state().working_dirty());

    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Current")
        .click();
    harness.run();
    harness.get_by_label_contains("Home —").click();
    harness.run();
    assert!(
        harness
            .query_by_label("Save changes before switching sessions?")
            .is_some()
    );
    harness.get_by_label("Cancel").click();
    harness.run();
    assert_eq!(harness.state().working_session.id, Some(work_id));
    assert_eq!(
        harness.state().working_session.name.as_deref(),
        Some("Work")
    );

    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Current")
        .click();
    harness.run();
    harness.get_by_label_contains("Home —").click();
    harness.run();
    harness.get_by_label("Discard changes").click();
    harness.run();
    wait_for_app(&mut harness, |app| {
        app.working_session.id == Some(home_id) && !app.loading_session
    });
    assert_eq!(
        harness.state().working_session.name.as_deref(),
        Some("Home")
    );

    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Current")
        .click();
    harness.run();
    harness.get_by_label_contains("Untitled session —").click();
    harness.run();
    wait_for_app(&mut harness, |app| {
        app.working_session.id == Some(work_id) && !app.loading_session
    });

    rename_session(&mut harness, "Home");
    assert!(harness.state().rename_error.is_some());
    assert_eq!(harness.state().working_session.name, None);
    harness.get_by_label("Cancel").click();
    harness.run();

    rename_session(&mut harness, "Work");
    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Current")
        .click();
    harness.run();
    harness.get_by_label_contains("Home —").click();
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

    harness.get_by_label("Delete session").click();
    harness.run();
    assert!(harness.query_by_label("Delete session?").is_some());
    harness.get_by_label("Delete permanently").click();
    harness.run();
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

    restarted.get_by_label("Delete all saved sessions").click();
    restarted.run();
    restarted.get_by_label("Delete all permanently").click();
    restarted.run();
    wait_for_app(&mut restarted, |app| {
        !app.deleting_all && app.sessions.is_empty()
    });
    assert!(!database.exists());
    shutdown_harness(&mut restarted);
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
    fs::write(temporary.path().join("evtap.sqlite3-journal"), [0_u8; 11]).unwrap();

    assert_eq!(database_disk_usage(&database), 26);
}
