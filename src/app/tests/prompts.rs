use super::{fixtures::*, *};

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
async fn headless_shell_shortcuts_do_not_cross_open_prompts() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();
    let created_at_ms = harness.state().working_session.created_at_ms;

    harness
        .state_mut()
        .open_prompt(ActivePromptKind::DeleteAll, None);
    harness.step();
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::N);
    harness.run();

    assert_eq!(
        harness.state().active_prompt_tag(),
        Some(ActivePromptTag::DeleteAll)
    );
    assert_eq!(harness.state().working_session.created_at_ms, created_at_ms);
    assert_eq!(harness.state().view, AppView::Overview);

    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(harness.state().active_prompt.is_none());

    harness.state_mut().loading_session = true;
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::S);
    harness.step();
    assert_ne!(
        harness.state().active_prompt_tag(),
        Some(ActivePromptTag::Disclosure)
    );
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::N);
    harness.step();
    assert_eq!(harness.state().working_session.created_at_ms, created_at_ms);
    harness.state_mut().loading_session = false;

    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_window_close_waits_for_the_active_prompt() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();

    open_session_switcher(&mut harness);
    harness
        .get_by_label_contains("Session actions")
        .click_accesskit();
    harness.run();
    harness.get_by_label("Reset statistics").click_accesskit();
    harness.run();
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::Dialog, "Reset statistics?")
            .is_some()
    );

    request_headless_close(&mut harness);
    assert!(!harness.state().allow_close);
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::Dialog, "Reset statistics?")
            .is_some()
    );
    assert!(
        harness
            .query_by_role_and_label(
                egui::accesskit::Role::Dialog,
                "Save changes before exiting?",
            )
            .is_none()
    );

    harness.get_by_label("Cancel").click();
    harness.run();
    request_headless_close(&mut harness);
    assert!(harness.state().allow_close);
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_boundary_disclosure_chain_restores_the_original_focus() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();
    let created_at_ms = harness.state().working_session.created_at_ms;

    harness
        .state_mut()
        .process_input(SystemTime::now(), KeyCode::KEY_A, KeyEventKind::Press);
    harness.step();
    open_session_switcher(&mut harness);
    harness.get_by_label("New session").click();
    harness.run();
    assert!(
        harness
            .query_by_role_and_label(
                egui::accesskit::Role::Dialog,
                "Save changes before switching sessions?",
            )
            .is_some()
    );

    harness.get_by_label("Save and switch").click();
    harness.run();
    assert!(
        harness
            .query_by_role_and_label(
                egui::accesskit::Role::Dialog,
                "Save sensitive aggregate statistics?",
            )
            .is_some()
    );
    assert!(harness.get_by_label("Allow local saves").is_focused());

    harness.get_by_label("Cancel").click();
    harness.run();
    harness.run();
    assert!(harness.state().active_prompt.is_none());
    assert_eq!(harness.state().working_session.created_at_ms, created_at_ms);
    assert!(harness.get_by_label("Switch active session").is_focused());
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn deferred_boundary_waits_for_the_active_prompt() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();

    {
        let app = harness.state_mut();
        app.working_session.name = Some("Unsaved work".to_owned());
        app.pending_boundary_after_stop = Some(super::super::BoundaryTarget::New);
        app.open_disclosure_prompt(DisclosureIntent::Review, None);
        app.finish_listener_stop("Stopped".to_owned(), false);
        assert_eq!(app.active_prompt_tag(), Some(ActivePromptTag::Disclosure));
        assert_eq!(
            app.pending_boundary_after_stop,
            Some(super::super::BoundaryTarget::New)
        );
    }
    harness.step();
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::Dialog, "Local storage disclosure",)
            .is_some()
    );

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Close")
        .click_accesskit();
    harness.run();
    assert_eq!(
        harness.state().active_prompt_tag(),
        Some(ActivePromptTag::Boundary)
    );
    assert_eq!(harness.state().pending_boundary_after_stop, None);
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
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_rename_result_does_not_close_a_newer_prompt() {
    let workspace = TestWorkspace::new();
    let mut harness = workspace.start();
    let session = SessionMetadata {
        id: SessionId::new(41).unwrap(),
        name: Some("Archived".to_owned()),
        created_at_ms: 1,
        updated_at_ms: 1,
        last_opened_at_ms: 1,
        captured_duration_ns: 0,
        application_version: env!("CARGO_PKG_VERSION").to_owned(),
        keyboard: KeyboardContext::default(),
    };
    let session_id = session.id;

    {
        let app = harness.state_mut();
        app.open_rename_dialog(
            RenameTarget::Saved(session_id),
            session.name.as_deref(),
            egui::Id::new("stale-rename-opener"),
        );
        let (rename, _) = app.rename_prompt_mut().unwrap();
        rename.request_id = Some(41);
        rename.submitting = true;
        app.close_rename_dialog();
        app.focus_after_prompt = None;
        app.open_prompt(ActivePromptKind::Reset, None);
        app.handle_storage_event(StorageEvent::SessionRenamed {
            request_id: 41,
            session: Some(session),
        });
    }

    assert_eq!(
        harness.state().active_prompt_tag(),
        Some(ActivePromptTag::Reset)
    );
    harness.step();
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::Dialog, "Reset statistics?")
            .is_some()
    );
    harness.get_by_label("Cancel").click();
    harness.run();
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
    assert_ne!(
        harness.state().active_prompt_tag(),
        Some(ActivePromptTag::Reset)
    );
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
    assert_ne!(
        harness.state().active_prompt_tag(),
        Some(ActivePromptTag::Reset)
    );
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
    assert_ne!(
        harness.state().active_prompt_tag(),
        Some(ActivePromptTag::Disclosure)
    );

    shutdown_harness(&mut harness);
}
#[test]
fn dirty_boundaries_follow_editor_style_save_policy() {
    assert_eq!(boundary_policy(false, false), BoundaryPolicy::Proceed);
    assert_eq!(boundary_policy(false, true), BoundaryPolicy::Proceed);
    assert_eq!(boundary_policy(true, false), BoundaryPolicy::Prompt);
    assert_eq!(boundary_policy(true, true), BoundaryPolicy::Save);
}
