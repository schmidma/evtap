use super::{fixtures::*, *};

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
fn open_settings_category(harness: &mut Harness<'static, App>, category: &str) {
    harness.get_by_label_contains("Settings").click();
    harness.run();
    harness.get_by_label(category).click();
    harness.run();
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_storage_autosave_and_disclosure_persist() {
    let workspace = TestWorkspace::new();
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
    shutdown_harness(&mut harness);

    let mut restarted = workspace.start();
    assert!(restarted.state().settings.autosave_enabled());
    assert!(restarted.state().settings.storage_disclosure_acknowledged());
    shutdown_harness(&mut restarted);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_appearance_rolls_back_failed_write_and_persists_success() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();
    open_settings_category(&mut harness, "Appearance");

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
    shutdown_harness(&mut harness);

    let mut restarted = workspace.start();
    assert_eq!(
        restarted.state().settings.appearance_preference(),
        AppearancePreference::Dark
    );
    shutdown_harness(&mut restarted);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_about_content_makes_offline_no_network_claims() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();
    open_settings_category(&mut harness, "About");

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
