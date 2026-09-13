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
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Search model")
        .click();
    harness.run();
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::TextInput, "Search model")
            .is_some(),
        "clicking the embedded search input must keep the selector open"
    );
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
async fn headless_xkb_search_popup_geometry_and_row_slots_stay_stable() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start_with_size(egui::vec2(800.0, 600.0));
    open_settings_category(&mut harness, "Keyboard interpretation");

    let long_model = format!("unique-model-{}", "very-long-label-".repeat(30));
    {
        let app = harness.state_mut();
        app.available_models = (0..40).map(|index| format!("model-{index:02}")).collect();
        app.available_models.push(long_model.clone());
    }
    harness.step();
    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Model")
        .click();
    harness.run();

    let popup_geometry = |harness: &Harness<'_, App>| {
        let search = harness
            .get_by_role_and_label(egui::accesskit::Role::TextInput, "Search model")
            .accesskit_node()
            .bounding_box()
            .unwrap();
        let search_center = egui::pos2(
            ((search.x0 + search.x1) / 2.0) as f32,
            ((search.y0 + search.y1) / 2.0) as f32,
        );
        let popup = harness.ctx.memory(|memory| {
            memory
                .areas()
                .visible_layer_ids()
                .into_iter()
                .filter(|layer| layer.order == egui::Order::Foreground)
                .filter_map(|layer| memory.area_rect(layer.id))
                .find(|rect| rect.contains(search_center))
                .expect("open model popup should have a foreground Area")
        });
        (popup, search)
    };

    let (initial_popup, initial_search) = popup_geometry(&harness);
    let initial_row = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Default")
        .accesskit_node();
    let initial_row_id = initial_row.id();
    let initial_row_bounds = initial_row.bounding_box().unwrap();

    let search = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Search model");
    search.click();
    search.type_text("unique-model");
    harness.run();
    let (filtered_popup, filtered_search) = popup_geometry(&harness);
    let filtered_row = harness.get_by_label(&long_model).accesskit_node();
    assert_eq!(
        filtered_row.id(),
        initial_row_id,
        "the first selectable row must retain its slot ID"
    );
    assert_eq!(filtered_row.bounding_box().unwrap(), initial_row_bounds);
    assert_eq!(filtered_popup, initial_popup);
    assert_eq!(filtered_search, initial_search);
    assert!(filtered_row.bounding_box().unwrap().x1 <= f64::from(filtered_popup.max.x));

    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::A);
    harness.key_press(egui::Key::Backspace);
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Search model")
        .type_text("no-such-model");
    harness.run();
    let empty_row = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "No matching options")
        .accesskit_node();
    assert!(empty_row.is_disabled());
    assert_eq!(empty_row.id(), initial_row_id);
    assert_eq!(empty_row.bounding_box().unwrap(), initial_row_bounds);
    let (empty_popup, empty_search) = popup_geometry(&harness);
    assert_eq!(empty_popup, initial_popup);
    assert_eq!(empty_search, initial_search);

    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Search model")
        .click();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run();
    harness.key_press(egui::Key::Backspace);
    harness.run();
    let (cleared_popup, cleared_search) = popup_geometry(&harness);
    let cleared_row = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Default")
        .accesskit_node();
    assert_eq!(cleared_row.id(), initial_row_id);
    assert_eq!(cleared_row.bounding_box().unwrap(), initial_row_bounds);
    assert_eq!(cleared_popup, initial_popup);
    assert_eq!(cleared_search, initial_search);

    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_xkb_search_clicks_keep_selectors_open_until_explicit_dismissal() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start_with_size(egui::vec2(800.0, 600.0));
    open_settings_category(&mut harness, "Keyboard interpretation");

    {
        let app = harness.state_mut();
        app.available_models = vec!["pc105".to_owned(), "pc104".to_owned()];
        app.available_layouts = vec!["us".to_owned(), "de".to_owned()];
    }
    harness.step();

    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Model")
        .click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Search model")
        .click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Search model")
        .type_text("105");
    harness.run();
    assert!(harness.query_by_label("pc104").is_none());
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::TextInput, "Search model")
            .is_some(),
        "typing must leave the filtered model selector open"
    );
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::TextInput, "Search model")
            .is_none(),
        "Escape must dismiss the selector"
    );

    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Model")
        .click();
    harness.run();
    assert!(harness.query_by_label("pc104").is_none());
    harness.get_by_label("pc105").click();
    harness.run();
    assert_eq!(harness.state().model, "pc105");
    assert_eq!(harness.state().settings.keyboard_model(), "pc105");
    assert_eq!(harness.state().working_session.keyboard.model, "pc105");
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::TextInput, "Search model")
            .is_none(),
        "selecting a model must close the selector"
    );

    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Layout")
        .click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Search layout")
        .click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Search layout")
        .type_text("us");
    harness.run();
    assert!(harness.query_by_label("de").is_none());
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::TextInput, "Search layout")
            .is_some()
    );
    harness.get_by_label("XKB configuration").click();
    harness.run();
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::TextInput, "Search layout")
            .is_none(),
        "an outside click must dismiss the selector"
    );

    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Layout")
        .click();
    harness.run();
    harness.get_by_label("us").click();
    harness.run();
    assert_eq!(harness.state().layout, "us");
    assert_eq!(harness.state().settings.keyboard_layout(), "us");
    assert_eq!(harness.state().working_session.keyboard.layout, "us");
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::TextInput, "Search layout")
            .is_none(),
        "selecting a layout must close the selector"
    );

    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, "Variant")
        .click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Search variant")
        .click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Search variant")
        .type_text("altgr-intl");
    harness.run();
    assert!(harness.query_by_label("dvorak").is_none());
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::TextInput, "Search variant")
            .is_some()
    );
    harness.get_by_label("altgr-intl").click();
    harness.run();
    assert_eq!(harness.state().variant, "altgr-intl");
    assert_eq!(harness.state().settings.keyboard_variant(), "altgr-intl");
    assert_eq!(
        harness.state().working_session.keyboard.variant,
        "altgr-intl"
    );
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::TextInput, "Search variant")
            .is_none(),
        "selecting a variant must close the selector"
    );
    assert!(!harness.state().working_dirty());
    shutdown_harness(&mut harness);

    let mut restarted = workspace.start();
    assert_eq!(restarted.state().settings.keyboard_model(), "pc105");
    assert_eq!(restarted.state().settings.keyboard_layout(), "us");
    assert_eq!(restarted.state().settings.keyboard_variant(), "altgr-intl");
    shutdown_harness(&mut restarted);
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
