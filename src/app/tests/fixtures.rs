use super::*;

pub(super) struct TestWorkspace {
    _temporary: TempDir,
    pub(super) paths: AppPaths,
}

impl TestWorkspace {
    pub(super) fn new() -> Self {
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

    pub(super) fn start(&self) -> Harness<'static, App> {
        self.start_with_size(egui::vec2(900.0, 1_200.0))
    }

    pub(super) fn start_with_size(&self, size: egui::Vec2) -> Harness<'static, App> {
        self.start_with_size_and_scale(size, 1.0)
    }

    pub(super) fn start_with_size_and_scale(
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

pub(super) fn wait_for_app(harness: &mut Harness<'_, App>, predicate: impl Fn(&App) -> bool) {
    wait_for_app_attempts(harness, 200, predicate);
}

pub(super) fn storage_failure_operation(app: &App) -> Option<StorageOperation> {
    app.storage_failure
        .as_ref()
        .map(|failure| failure.operation)
}

pub(super) fn storage_failure_order(app: &App) -> Option<SessionListOrder> {
    app.storage_failure
        .as_ref()
        .and_then(|failure| failure.list_order)
}

pub(super) fn wait_for_app_attempts(
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
        harness.state().active_prompt_tag(),
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
        harness.state().storage_failure,
    );
}

pub(super) fn request_headless_close(harness: &mut Harness<'_, App>) {
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

pub(super) fn shutdown_harness(harness: &mut Harness<'_, App>) {
    eframe::App::on_exit(harness.state_mut(), None);
}

pub(super) fn open_session_switcher(harness: &mut Harness<'_, App>) {
    assert!(
        !harness.state().session_switcher_open,
        "session switcher unexpectedly remained open before its opener was activated"
    );
    harness
        .get_by_label("Switch active session")
        .click_accesskit();
    harness.run();
}

pub(super) fn open_manage_sessions(harness: &mut Harness<'_, App>) {
    if !matches!(harness.state().view, AppView::Sessions) {
        open_session_switcher(harness);
        harness.get_by_label("Manage sessions").click();
        harness.run();
        wait_for_app(harness, |app| !app.manage_list_loading);
    }
}

pub(super) fn open_storage_settings(harness: &mut Harness<'_, App>) {
    harness.get_by_label_contains("Settings").click();
    harness.run();
    harness.get_by_label("Storage & privacy").click();
    harness.run();
}

pub(super) fn rename_session(harness: &mut Harness<'_, App>, name: &str) {
    open_session_switcher(harness);
    assert!(
        harness.query_by_label("Rename").is_some(),
        "rename action unavailable for {name:?}: switcher_open={}, switch_focused={}, current_id={:?}, current_name={:?}, dirty={}, prompt={:?}",
        harness.state().session_switcher_open,
        harness.get_by_label("Switch active session").is_focused(),
        harness.state().working_session.id,
        harness.state().working_session.name,
        harness.state().working_dirty(),
        harness.state().active_prompt_tag(),
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

pub(super) fn allow_first_save(harness: &mut Harness<'_, App>) {
    harness.get_by_label("Save").click();
    harness.run();
    harness.get_by_label("Allow local saves").click();
    harness.step();
    wait_for_app(harness, |app| {
        app.working_session.id.is_some() && app.storage_tracker.status() == StorageStatus::Saved
    });
}

pub(super) fn install_analytics_fixture(app: &mut App, key_rows: u16) {
    app.working_session.metrics = SessionMetrics::default();
    let event = |index: u16, at_ms: u64, kind, role, text: Option<String>| {
        KeyEvent::new(
            PhysicalKey::new(30 + index, format!("KEY_{index}")),
            text,
            SystemTime::UNIX_EPOCH + Duration::from_millis(at_ms),
            kind,
            role,
        )
    };

    for cycle in 0..3_u16 {
        for index in 0..key_rows {
            let text = char::from_u32(u32::from(b'a') + u32::from(index))
                .unwrap_or('?')
                .to_string();
            let press_at = u64::from(cycle * key_rows + index) * 100;
            app.working_session.metrics.process(&event(
                index,
                press_at,
                KeyEventKind::Press,
                KeyRole::Other,
                Some(text.clone()),
            ));
            app.working_session.metrics.process(&event(
                index,
                press_at + 50,
                KeyEventKind::Release,
                KeyRole::Other,
                Some(text),
            ));
        }
    }

    let correction_at = u64::from(key_rows) * 300;
    app.working_session.metrics.process(&event(
        0,
        correction_at,
        KeyEventKind::Press,
        KeyRole::Backspace,
        None,
    ));
    app.working_session.metrics.process(&event(
        key_rows.saturating_sub(1),
        correction_at + 50,
        KeyEventKind::Press,
        KeyRole::Other,
        Some("z".to_owned()),
    ));
    app.working_session.metrics.clear_in_flight();
}
