use super::{fixtures::*, *};

fn start_with_analytics_fixture(workspace: &TestWorkspace, key_rows: u16) -> Harness<'static, App> {
    let mut harness = workspace.start_with_size(egui::vec2(1_000.0, 900.0));
    install_analytics_fixture(harness.state_mut(), key_rows);
    harness.step();
    harness
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_analytics_overview_composes_metrics_and_recovery_card() {
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

    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_timing_and_corrections_navigation_preserves_selected_timing_page() {
    let workspace = TestWorkspace::new();
    let mut harness = start_with_analytics_fixture(&workspace, 9);

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

    shutdown_harness(&mut harness);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_key_usage_expansion_temporary_state_persists() {
    let workspace = TestWorkspace::new();
    let mut harness = start_with_analytics_fixture(&workspace, 9);

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
