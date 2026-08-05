use super::{fixtures::*, *};

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
        AppView::Settings(super::super::SettingsSection::Input)
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
