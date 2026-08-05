use super::{fixtures::*, *};

fn save_current_session(
    harness: &mut Harness<'static, App>,
    name: Option<&str>,
    first_save: bool,
) -> SessionId {
    if let Some(name) = name {
        rename_session(harness, name);
    }
    if first_save {
        allow_first_save(harness);
    } else {
        harness.get_by_label("Save").click();
        harness.step();
        wait_for_app(harness, |app| {
            app.working_session.id.is_some() && app.storage_tracker.status() == StorageStatus::Saved
        });
    }
    harness.state().working_session.id.unwrap()
}

fn start_with_saved_session(
    workspace: &TestWorkspace,
    name: Option<&str>,
) -> (Harness<'static, App>, SessionId) {
    let mut harness = workspace.start();
    let session_id = save_current_session(&mut harness, name, true);
    (harness, session_id)
}

fn start_with_two_saved_sessions(
    workspace: &TestWorkspace,
    first_name: Option<&str>,
    second_name: Option<&str>,
) -> (Harness<'static, App>, SessionId, SessionId) {
    let (mut harness, first_id) = start_with_saved_session(workspace, first_name);
    open_session_switcher(&mut harness);
    harness.get_by_label("New session").click();
    harness.run();
    assert_eq!(harness.state().working_session.id, None);
    let second_id = save_current_session(&mut harness, second_name, false);
    (harness, first_id, second_id)
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_dirty_session_switch_supports_save_discard_cancel_and_duplicate_validation() {
    let workspace = TestWorkspace::new();
    let (mut harness, home_id, work_id) =
        start_with_two_saved_sessions(&workspace, Some("Home"), None);

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
            .rename_prompt()
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
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_active_session_reset_and_delete_leave_a_fresh_session() {
    let workspace = TestWorkspace::new();
    let (mut harness, work_id, _) =
        start_with_two_saved_sessions(&workspace, Some("Work"), Some("Home"));

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
    shutdown_harness(&mut restarted);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_delete_all_saved_sessions_preserves_an_unsaved_draft() {
    let workspace = TestWorkspace::new();
    let database = workspace.paths.database_file();
    let (mut harness, _) = start_with_saved_session(&workspace, Some("Work"));

    open_session_switcher(&mut harness);
    harness.get_by_label("New session").click();
    harness.run();
    rename_session(&mut harness, "Unsaved draft");
    open_manage_sessions(&mut harness);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Delete all saved sessions")
        .click();
    harness.run();
    harness.get_by_label("Delete all permanently").click();
    harness.run();
    wait_for_app(&mut harness, |app| {
        !app.deleting_all && app.sessions.is_empty()
    });
    assert!(!database.exists());
    assert_eq!(
        harness.state().working_session.name.as_deref(),
        Some("Unsaved draft"),
        "deleting saved sessions must preserve an unsaved in-memory session"
    );
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_manage_sessions_renames_active_session_and_preserves_order() {
    let workspace = TestWorkspace::new();
    let (mut harness, alpha_id, beta_id) =
        start_with_two_saved_sessions(&workspace, Some("Alpha"), Some("Beta"));

    open_manage_sessions(&mut harness);
    harness.get_by_label_contains("Rename Beta").click();
    harness.run();
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .type_text("Beta active");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert!(harness.state().rename_prompt().is_none());
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
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_manage_sessions_validates_unloaded_rename_focus_and_ordering() {
    let workspace = TestWorkspace::new();
    let (mut harness, alpha_id, beta_id) =
        start_with_two_saved_sessions(&workspace, Some("Alpha"), Some("Beta"));
    rename_session(&mut harness, "Beta active");
    assert!(harness.state().working_dirty());

    open_manage_sessions(&mut harness);
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
        app.rename_prompt().is_none()
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
            .rename_prompt()
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
            .rename_prompt()
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
    assert!(harness.state().rename_prompt().is_none());
    assert!(harness.get_by_label_contains("Rename Archive").is_focused());

    harness.get_by_label_contains("Rename Archive").click();
    harness.run();
    harness.key_press(egui::Key::Backspace);
    harness.key_press(egui::Key::Enter);
    harness.step();
    wait_for_app(&mut harness, |app| {
        app.rename_prompt().is_none()
            && app
                .managed_sessions
                .iter()
                .any(|session| session.id == alpha_id && session.name.is_none())
    });
    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_manage_sessions_deletes_unloaded_session_and_preserves_active() {
    let workspace = TestWorkspace::new();
    let (mut harness, unloaded_id, active_id) =
        start_with_two_saved_sessions(&workspace, None, Some("Beta"));

    open_manage_sessions(&mut harness);
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
                .any(|session| session.id == unloaded_id)
    });
    assert_eq!(harness.state().working_session.id, Some(active_id));
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
            .rename_prompt()
            .map(|dialog| dialog.target),
        Some(super::super::RenameTarget::Saved(session_id)) if session_id.get() == 5
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
