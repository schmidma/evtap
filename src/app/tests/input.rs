use super::{fixtures::*, *};

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
