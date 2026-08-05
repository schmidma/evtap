use super::{fixtures::*, *};

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
        storage_failure_operation(app) == Some(StorageOperation::Save)
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
    let failure = harness.state().storage_failure.as_ref().unwrap();
    assert_eq!(failure.operation, StorageOperation::List);
    assert_eq!(failure.list_order, Some(SessionListOrder::LastOpened));
    assert_eq!(
        failure.message,
        "Saved session metadata could not be refreshed."
    );
    assert!(failure.details.contains("injected switcher-list failure"));

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
        storage_failure_operation(harness.state()),
        Some(StorageOperation::List),
        "an unrelated ordered-list success must not clear the failed switcher list"
    );
    assert_eq!(
        storage_failure_order(harness.state()),
        Some(SessionListOrder::LastOpened)
    );

    harness.state_mut().session_list_request_id = Some(13);
    harness
        .state_mut()
        .handle_storage_event(StorageEvent::SessionsListed {
            request_id: 13,
            sessions: Vec::new(),
        });
    assert!(harness.state().storage_failure.is_none());
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_list_and_load_responses_cannot_replace_newer_results() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();
    let metadata = |id, name: &str| SessionMetadata {
        id: SessionId::new(id).unwrap(),
        name: Some(name.to_owned()),
        created_at_ms: 1,
        updated_at_ms: id,
        last_opened_at_ms: id,
        captured_duration_ns: 0,
        application_version: env!("CARGO_PKG_VERSION").to_owned(),
        keyboard: KeyboardContext::default(),
    };

    {
        let app = harness.state_mut();
        app.session_list_request_id = Some(42);
        app.handle_storage_event(StorageEvent::SessionsListed {
            request_id: 42,
            sessions: vec![metadata(42, "Newest list")],
        });
        app.handle_storage_event(StorageEvent::SessionsListed {
            request_id: 41,
            sessions: vec![metadata(41, "Stale list")],
        });
        assert_eq!(app.session_list_request_id, None);
        assert_eq!(app.sessions.len(), 1);
        assert_eq!(app.sessions[0].name.as_deref(), Some("Newest list"));

        app.load_request_id = 52;
        app.loading_session = true;
        app.handle_storage_event(StorageEvent::SessionLoaded {
            request_id: 52,
            session: Some(StoredSession {
                metadata: metadata(52, "Newest load"),
                metrics: Vec::new(),
            }),
        });
        app.handle_storage_event(StorageEvent::SessionLoaded {
            request_id: 51,
            session: Some(StoredSession {
                metadata: metadata(51, "Stale load"),
                metrics: Vec::new(),
            }),
        });
        assert!(!app.loading_session);
        assert_eq!(app.working_session.id, SessionId::new(52));
        assert_eq!(app.working_session.name.as_deref(), Some("Newest load"));
    }

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
        storage_failure_operation(harness.state()),
        Some(StorageOperation::Load)
    );
    harness
        .state_mut()
        .handle_storage_event(StorageEvent::SessionLoaded {
            request_id: 7,
            session: Some(stored_session()),
        });
    assert!(harness.state().storage_failure.is_none());

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
        storage_failure_operation(harness.state()),
        Some(StorageOperation::Delete),
        "a success for an older operation must not clear a newer failure"
    );
    harness
        .state_mut()
        .handle_storage_event(StorageEvent::SessionDeleted {
            session_id,
            deleted: false,
        });
    assert!(harness.state().storage_failure.is_none());

    harness
        .state_mut()
        .handle_storage_event(failure(StorageOperation::DeleteAll));
    harness
        .state_mut()
        .handle_storage_event(StorageEvent::AllDeleted);
    assert!(harness.state().storage_failure.is_none());

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
    assert!(harness.state().storage_failure.is_none());
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
            storage_failure_operation(app),
            Some(StorageOperation::Delete),
            "a successful save must not clear an unrelated delete failure"
        );

        app.handle_storage_event(StorageEvent::Opened {
            sessions: Vec::new(),
            selected: None,
        });
        assert_eq!(
            storage_failure_operation(app),
            Some(StorageOperation::Delete),
            "a successful open must not clear an unrelated delete failure"
        );

        app.open_rename_dialog(
            RenameTarget::Saved(session_id),
            Some("Recovered session"),
            egui::Id::new("retry-test-rename-opener"),
        );
        if let Some((dialog, _)) = app.rename_prompt_mut() {
            dialog.request_id = Some(91);
            dialog.submitting = true;
        }
        app.handle_storage_event(StorageEvent::SessionRenamed {
            request_id: 91,
            session: Some(stored_session().metadata),
        });
        assert_eq!(
            storage_failure_operation(app),
            Some(StorageOperation::Delete),
            "a successful rename must not clear an unrelated delete failure"
        );
        app.handle_storage_event(StorageEvent::SessionDeleted {
            session_id,
            deleted: false,
        });
    }
    assert!(harness.state().storage_failure.is_none());
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn unavailable_storage_worker_finishes_requests_and_records_atomic_failures() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();
    let session_id = SessionId::new(88).unwrap();

    {
        let app = harness.state_mut();
        drop(app.storage.take());

        app.request_session_list();
        assert_eq!(app.session_list_request_id, None);
        assert_eq!(storage_failure_operation(app), Some(StorageOperation::List));
        assert_eq!(
            storage_failure_order(app),
            Some(SessionListOrder::LastOpened)
        );

        app.request_manage_session_list();
        assert_eq!(app.manage_list_request_id, None);
        assert!(!app.manage_list_loading);
        assert_eq!(
            storage_failure_order(app),
            Some(SessionListOrder::LastUpdated)
        );

        app.execute_boundary(super::super::BoundaryTarget::Load(session_id));
        assert!(!app.loading_session);
        assert_eq!(storage_failure_operation(app), Some(StorageOperation::Load));

        app.open_rename_dialog(
            RenameTarget::Saved(session_id),
            Some("Archived"),
            egui::Id::new("unavailable-worker-rename"),
        );
        app.submit_rename();
        assert_eq!(
            storage_failure_operation(app),
            Some(StorageOperation::Rename)
        );
        assert_eq!(
            app.rename_prompt()
                .and_then(|prompt| prompt.error.as_deref()),
            Some("The session name could not be saved.")
        );
        let _ = app.finish_prompt();
        app.focus_after_prompt = None;

        app.prompt_delete_session(Some(session_id), "Archived", false, None);
        app.delete_prompted_session();
        assert!(!app.deleting_session);
        assert_eq!(
            storage_failure_operation(app),
            Some(StorageOperation::Delete)
        );

        app.open_prompt(ActivePromptKind::DeleteAll, None);
        app.delete_all_sessions();
        assert!(!app.deleting_all);
        let failure = app.storage_failure.as_ref().unwrap();
        assert_eq!(failure.operation, StorageOperation::DeleteAll);
        assert_eq!(failure.list_order, None);
        assert_eq!(failure.message, "Saved sessions could not be deleted.");
        assert_eq!(failure.details, "Storage worker is unavailable");
    }

    shutdown_harness(&mut harness);
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
