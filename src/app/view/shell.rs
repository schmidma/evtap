use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, WidgetInfo, WidgetType};

use crate::{metric::Metric, storage::StorageStatus};

use super::super::{
    App, AppView, BoundaryTarget, ListenerState, ScanWarning, SettingsSection, TimingView,
};
use super::{
    components::{self, BannerSeverity},
    format_compact_local_timestamp, format_duration, storage_status_label_for_operation, theme,
};

const TOP_BAR_HEIGHT: f32 = 64.0;
const NAVIGATION_WIDTH: f32 = 168.0;
const TOP_BAR_DEVICE_LABEL_CHARS: usize = 18;
const COMPACT_TOP_BAR_WINDOW_WIDTH: f32 = 1_000.0;
const SUMMARY_TILE_MIN_HEIGHT: f32 = 168.0;

fn compact_top_bar_label(value: &str) -> String {
    compact_label(value, TOP_BAR_DEVICE_LABEL_CHARS)
}

fn compact_label(value: &str, maximum_characters: usize) -> String {
    let mut characters = value.chars();
    let visible: String = characters.by_ref().take(maximum_characters).collect();
    if characters.next().is_some() {
        format!("{visible}…")
    } else {
        visible
    }
}

fn metric_analysis_card(
    ui: &mut egui::Ui,
    icon: &str,
    heading: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    components::card(ui, |ui| {
        ui.set_width(ui.available_width());
        components::card_header(ui, icon, heading);
        components::vertical_gap(ui, theme::SPACE_LG);
        add_contents(ui);
    });
}

#[derive(Clone, Copy)]
enum SwitcherAction {
    New,
    Load(crate::session::SessionId),
    RenameCurrent,
    ResetCurrent,
    DeleteCurrent,
    Manage,
}

impl App {
    pub(crate) fn render_shell(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette(ui.ctx().theme());
        egui::Panel::left("primary-navigation")
            .exact_size(NAVIGATION_WIDTH)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .stroke(egui::Stroke::new(1.0, palette.border))
                    .inner_margin(egui::Margin::symmetric(
                        theme::SPACE_MD as i8,
                        theme::SPACE_LG as i8,
                    )),
            )
            .show(ui, |ui| {
                ui.add_sized(
                    [ui.available_width(), 34.0],
                    egui::Label::new(
                        egui::RichText::new("evtap")
                            .font(theme::semibold_font(20.0))
                            .color(palette.text),
                    ),
                );
                ui.separator();
                components::vertical_gap(ui, theme::SPACE_LG);
                self.render_navigation(ui);
            });

        egui::Panel::top("global-top-bar")
            .exact_size(TOP_BAR_HEIGHT)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .stroke(egui::Stroke::new(1.0, palette.border))
                    .inner_margin(egui::Margin::symmetric(theme::SPACE_SM as i8, 0)),
            )
            .show(ui, |ui| self.render_top_bar(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(palette.background))
            .show(ui, |ui| {
                if matches!(self.view, AppView::Sessions) {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::same(theme::PAGE_PADDING))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            self.render_status_banners(ui);
                            self.render_content_page(ui);
                        });
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt(("application-content", self.view))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            egui::Frame::new()
                                .inner_margin(egui::Margin::same(theme::PAGE_PADDING))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    self.render_status_banners(ui);
                                    self.render_content_page(ui);
                                });
                        });
                }
            });
    }

    fn render_top_bar(&mut self, ui: &mut egui::Ui) {
        let compact = ui.ctx().content_rect().width() < COMPACT_TOP_BAR_WINDOW_WIDTH;
        ui.spacing_mut().item_spacing.x = if compact { 6.0 } else { 8.0 };
        ui.horizontal_centered(|ui| {
            let session_name = self.working_session.display_name().to_owned();
            let visible_session_name = if compact {
                compact_label(&session_name, 12)
            } else {
                compact_top_bar_label(&session_name)
            };
            let session = ui.add_sized(
                [if compact { 120.0 } else { 154.0 }, 36.0],
                egui::Button::new(format!("{visible_session_name}  ▾")),
            );
            session.widget_info(|| {
                let mut info = WidgetInfo::labeled(
                    WidgetType::Button,
                    session.enabled(),
                    "Switch active session",
                );
                info.current_text_value = Some(session_name.clone());
                info
            });
            if !self.session_switcher_open {
                components::tooltip_on_hover_or_focus(&session, |ui| {
                    ui.label(egui::RichText::new(&session_name).font(theme::semibold_font(13.0)));
                    ui.small(format!(
                        "{} captured · {}",
                        format_duration(self.working_session.duration()),
                        storage_status_label_for_operation(
                            self.storage_tracker.status(),
                            self.working_session.id.is_some(),
                            self.last_failed_operation,
                        )
                    ));
                });
            }
            if session.clicked() {
                self.session_switcher_open = !self.session_switcher_open;
            }
            self.render_session_switcher(&session);
            self.render_save_slot(ui);

            if compact {
                ui.add_space(2.0);
                self.render_top_keyboard_slot(ui);
                ui.add_space(2.0);
                self.render_capture_status_slot(ui);
                self.render_capture_action(ui);
            } else {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.render_capture_action(ui);
                    self.render_capture_status_slot(ui);
                    components::horizontal_gap(ui, theme::SPACE_XL);
                    self.render_top_keyboard_slot(ui);
                });
            }
        });
    }

    fn render_save_slot(&mut self, ui: &mut egui::Ui) {
        let status = self.storage_tracker.status();
        let label = storage_status_label_for_operation(
            status,
            self.working_session.id.is_some(),
            self.last_failed_operation,
        );
        let compact = ui.ctx().content_rect().width() < COMPACT_TOP_BAR_WINDOW_WIDTH;
        let visible_label = if compact {
            match status {
                StorageStatus::Dirty => "Unsaved",
                StorageStatus::Failed => label,
                _ => label,
            }
        } else {
            label
        };
        let enabled = self.save_action_enabled();
        let slot_width = if compact { 120.0 } else { 172.0 };
        ui.allocate_ui_with_layout(
            egui::vec2(slot_width, 36.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                let status_response = ui.label(egui::RichText::new(visible_label).small().strong());
                status_response.widget_info(|| {
                    WidgetInfo::labeled(
                        WidgetType::Label,
                        status_response.enabled(),
                        format!("Save status: {label}"),
                    )
                });
                let save = ui
                    .add_enabled(enabled, egui::Button::new("Save"))
                    .on_disabled_hover_text(if self.save_controls_busy() {
                        "Wait for the current storage operation to finish."
                    } else {
                        "The current session is already saved."
                    });
                if save.clicked() {
                    self.request_save_from(None, Some(save.id));
                }
            },
        );
    }

    fn save_controls_busy(&self) -> bool {
        self.loading_session
            || self.deleting_session
            || self.deleting_all
            || self.storage_tracker.in_flight().is_some()
    }

    fn save_action_enabled(&self) -> bool {
        let already_saved = self.storage_tracker.status() == StorageStatus::Saved
            && self.working_session.id.is_some()
            && !self.working_dirty();
        !self.save_controls_busy() && !already_saved
    }

    fn render_top_keyboard_slot(&mut self, ui: &mut egui::Ui) {
        let controls_enabled = self.input_controls_enabled();
        let (selected_text, accessible_selected_text) = match &self.devices {
            None => ("Scanning…".to_owned(), "Scanning for keyboards".to_owned()),
            Some(devices) if devices.is_empty() => {
                ("No keyboard".to_owned(), "No readable keyboard".to_owned())
            }
            Some(devices) => self
                .selected_device
                .and_then(|index| devices.get(index))
                .map_or_else(
                    || ("Select keyboard".to_owned(), "Select keyboard".to_owned()),
                    |device| (compact_top_bar_label(&device.name), device.name.clone()),
                ),
        };
        let mut request_scan = false;

        let compact = ui.ctx().content_rect().width() < COMPACT_TOP_BAR_WINDOW_WIDTH;
        ui.allocate_ui_with_layout(
            egui::vec2(if compact { 150.0 } else { 180.0 }, 44.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_enabled_ui(controls_enabled, |ui| {
                    egui::ComboBox::from_id_salt("top-keyboard-selector")
                        .width(if compact { 92.0 } else { 122.0 })
                        .selected_text(&selected_text)
                        .show_ui(ui, |ui| {
                            if let Some(devices) = &self.devices {
                                for (index, device) in devices.iter().enumerate() {
                                    let response = ui.selectable_value(
                                        &mut self.selected_device,
                                        Some(index),
                                        &device.name,
                                    );
                                    components::tooltip_on_hover_or_focus(&response, |ui| {
                                        ui.add(
                                            egui::Label::new(format!(
                                                "{} ({})",
                                                device.physical_path, device.path
                                            ))
                                            .selectable(true),
                                        );
                                    });
                                }
                            }
                        })
                        .response
                        .widget_info(|| {
                            let mut info = WidgetInfo::labeled(
                                WidgetType::ComboBox,
                                controls_enabled,
                                "Keyboard",
                            );
                            info.current_text_value = Some(accessible_selected_text.clone());
                            info
                        });
                })
                .response
                .on_disabled_hover_text("Stop capture to change input settings.");

                let scan_enabled = controls_enabled && self.devices.is_some();
                if components::accessible_icon_button_enabled(
                    ui,
                    scan_enabled,
                    egui_phosphor::regular::ARROWS_CLOCKWISE,
                    "Rescan keyboards",
                )
                .on_disabled_hover_text(if controls_enabled {
                    "Wait for the current keyboard scan to finish."
                } else {
                    "Stop capture to rescan keyboards."
                })
                .clicked()
                {
                    request_scan = true;
                }
            },
        );

        if request_scan {
            self.request_scan();
        }
    }

    fn render_capture_status_slot(&self, ui: &mut egui::Ui) {
        let palette = theme::palette(ui.ctx().theme());
        let (label, color) = match self.listener_state {
            ListenerState::Connecting => ("Connecting…", palette.warning),
            ListenerState::Listening => ("Listening", palette.recording),
            ListenerState::Stopping => ("Stopping…", palette.warning),
            ListenerState::Failed => ("Unavailable", palette.error),
            ListenerState::Idle if self.scan_error.is_some() => ("Unavailable", palette.error),
            ListenerState::Idle if self.devices.is_none() => ("Scanning…", palette.info),
            ListenerState::Idle if self.devices.as_ref().is_some_and(Vec::is_empty) => {
                ("Unavailable", palette.warning)
            }
            ListenerState::Idle if self.selected_device.is_none() => {
                ("Select keyboard", palette.info)
            }
            ListenerState::Idle => ("Ready", palette.success),
        };
        let compact = ui.ctx().content_rect().width() < COMPACT_TOP_BAR_WINDOW_WIDTH;
        let visible_label = if compact {
            match label {
                "Select keyboard" => "Select",
                "Connecting…" | "Stopping…" => "Busy…",
                _ => label,
            }
        } else {
            label
        };
        let mut status = egui::text::LayoutJob::default();
        status.append(
            "●  ",
            0.0,
            egui::TextFormat {
                color,
                ..Default::default()
            },
        );
        status.append(
            visible_label,
            0.0,
            egui::TextFormat {
                color: ui.visuals().text_color(),
                ..Default::default()
            },
        );
        let response = ui.add_sized(
            [if compact { 84.0 } else { 116.0 }, 36.0],
            egui::Label::new(status).sense(egui::Sense::focusable_noninteractive()),
        );
        response.widget_info(|| {
            WidgetInfo::labeled(
                WidgetType::Label,
                true,
                format!("Capture status: {label}. Aggregate measurements only"),
            )
        });
        components::tooltip_on_hover_or_focus(&response, |ui| {
            ui.label("Capture stores aggregate measurements only.");
            ui.small("Raw events and reconstructed typed text are never saved.");
        });
    }

    fn render_capture_action(&mut self, ui: &mut egui::Ui) {
        let busy = self.loading_session
            || self.deleting_session
            || self.deleting_all
            || matches!(
                self.listener_state,
                ListenerState::Connecting | ListenerState::Stopping
            );
        let action_width = if ui.ctx().content_rect().width() < COMPACT_TOP_BAR_WINDOW_WIDTH {
            84.0
        } else {
            90.0
        };
        if self.listener.is_some() {
            let response = ui.add_enabled(
                !busy,
                egui::Button::new("Stop").min_size(egui::vec2(action_width, 36.0)),
            );
            response.widget_info(|| {
                WidgetInfo::labeled(
                    WidgetType::Button,
                    response.enabled(),
                    "Stop keyboard capture",
                )
            });
            if response.clicked() {
                self.stop_listener();
            }
        } else {
            let can_start = self.selected_device.is_some() && !busy;
            let response = ui
                .add_enabled(
                    can_start,
                    egui::Button::new("Start").min_size(egui::vec2(action_width, 36.0)),
                )
                .on_disabled_hover_text(if self.selected_device.is_none() {
                    if self.devices.as_ref().is_some_and(Vec::is_empty) {
                        "No readable keyboard is available. Check permissions, then rescan."
                    } else {
                        "Select a keyboard before starting capture."
                    }
                } else {
                    "Wait for the current operation to finish."
                });
            response.widget_info(|| {
                WidgetInfo::labeled(
                    WidgetType::Button,
                    response.enabled(),
                    "Start keyboard capture",
                )
            });
            if response.clicked()
                && let Some(index) = self.selected_device
            {
                self.begin_listening(index);
            }
        }
    }

    fn render_navigation(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = theme::SPACE_SM;
        self.navigation_button(
            ui,
            egui_phosphor::regular::HOUSE,
            "Overview",
            matches!(self.view, AppView::Overview),
            AppView::Overview,
        );
        self.navigation_button(
            ui,
            egui_phosphor::regular::CHART_BAR_HORIZONTAL,
            "Key Usage",
            matches!(self.view, AppView::KeyUsage),
            AppView::KeyUsage,
        );
        self.navigation_button(
            ui,
            egui_phosphor::regular::CLOCK,
            "Timing",
            matches!(self.view, AppView::Timing(_)),
            AppView::Timing(self.timing_view),
        );
        self.navigation_button(
            ui,
            egui_phosphor::regular::ARROW_U_DOWN_LEFT,
            "Corrections",
            matches!(self.view, AppView::Corrections),
            AppView::Corrections,
        );
        self.navigation_button(
            ui,
            egui_phosphor::regular::GEAR,
            "Settings",
            matches!(self.view, AppView::Settings(_)),
            AppView::Settings(SettingsSection::Input),
        );
    }

    fn navigation_button(
        &mut self,
        ui: &mut egui::Ui,
        icon: &str,
        label: &str,
        selected: bool,
        target: AppView,
    ) {
        let response = ui.add_sized(
            [ui.available_width(), 44.0],
            egui::Button::selectable(selected, ""),
        );
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::Button, response.enabled(), selected, label)
        });

        let visuals = ui.style().interact_selectable(&response, selected);
        let icon_center = egui::pos2(response.rect.left() + 34.0, response.rect.center().y);
        let label_start = egui::pos2(response.rect.left() + 58.0, response.rect.center().y);
        ui.painter().text(
            icon_center,
            egui::Align2::CENTER_CENTER,
            icon,
            theme::icon_font(18.0),
            visuals.text_color(),
        );
        ui.painter().text(
            label_start,
            egui::Align2::LEFT_CENTER,
            label,
            theme::medium_font(15.0),
            visuals.text_color(),
        );

        if response.clicked() {
            self.view = target;
        }
    }

    fn render_status_banners(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = self.storage_error.clone() {
            use crate::storage::StorageOperation;
            let operation = self.last_failed_operation;
            let title = match operation {
                Some(StorageOperation::Open) => "Local storage unavailable",
                Some(StorageOperation::Save | StorageOperation::ShutdownSave) => {
                    "Session could not be saved"
                }
                Some(StorageOperation::List) => "Session list could not be loaded",
                Some(StorageOperation::Load) => "Session could not be loaded",
                Some(StorageOperation::Rename) => "Session could not be renamed",
                Some(StorageOperation::Delete) => "Session could not be deleted",
                Some(StorageOperation::DeleteAll) => "Saved sessions could not be deleted",
                Some(StorageOperation::Maintenance) | None => "Storage needs attention",
            };
            components::contextual_banner(ui, BannerSeverity::Error, title, &error);
            if let Some(details) = &self.storage_error_details {
                egui::CollapsingHeader::new("Technical details")
                    .id_salt("storage-error-details")
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(details).selectable(true));
                    });
            }
            match operation {
                Some(StorageOperation::Open) => {
                    if ui.button("Retry opening local storage").clicked()
                        && let Some(worker) = &self.storage
                    {
                        let _ = worker.send(crate::storage::StorageCommand::RetryOpen {
                            last_session_id: self.settings.last_session_id(),
                        });
                    }
                }
                Some(StorageOperation::Save | StorageOperation::ShutdownSave) => {
                    if ui.button("Retry save").clicked() {
                        self.request_save(None);
                    }
                }
                Some(StorageOperation::List) => {
                    if ui.button("Retry session list").clicked() {
                        match self.failed_list_order {
                            Some(crate::storage::SessionListOrder::LastOpened) => {
                                self.request_session_list();
                            }
                            Some(crate::storage::SessionListOrder::LastUpdated) => {
                                self.request_manage_session_list();
                            }
                            None => self.refresh_session_lists(),
                        }
                    }
                }
                Some(StorageOperation::Load) => {
                    if ui.button("Choose a session").clicked() {
                        self.session_switcher_open = true;
                    }
                }
                Some(StorageOperation::Delete | StorageOperation::DeleteAll) => {
                    if ui.button("Open Manage Sessions").clicked() {
                        self.open_manage_sessions();
                    }
                }
                Some(StorageOperation::Rename | StorageOperation::Maintenance) | None => {}
            }
            components::vertical_gap(ui, theme::SPACE_LG);
        }
        if let Some(error) = self.capture_error.clone() {
            components::contextual_banner(
                ui,
                BannerSeverity::Error,
                "Capture stopped",
                &format!(
                    "{error}. The previous keyboard is unavailable. Select a keyboard and choose Start to resume; capture will not restart automatically."
                ),
            );
            if ui.button("Open Input settings").clicked() {
                self.view = AppView::Settings(SettingsSection::Input);
            }
            components::vertical_gap(ui, theme::SPACE_LG);
        }
        if let Some(error) = self.scan_error.clone() {
            components::contextual_banner(
                ui,
                BannerSeverity::Error,
                "Keyboard scan failed",
                &error,
            );
            if ui
                .add_enabled(
                    self.devices.is_some(),
                    egui::Button::new("Rescan keyboards"),
                )
                .clicked()
            {
                self.request_scan();
            }
            components::vertical_gap(ui, theme::SPACE_LG);
        }
        let showing_settings = matches!(self.view, AppView::Settings(_));
        let showing_keyboard_settings = matches!(
            self.view,
            AppView::Settings(SettingsSection::KeyboardInterpretation)
        );
        if !showing_keyboard_settings && let Some(error) = self.keyboard_error.clone() {
            components::contextual_banner(
                ui,
                BannerSeverity::Error,
                "Keyboard interpretation unchanged",
                &error,
            );
            components::vertical_gap(ui, theme::SPACE_LG);
        }
        if !showing_settings && let Some(error) = self.settings_error.clone() {
            components::contextual_banner(
                ui,
                BannerSeverity::Error,
                "Settings could not be saved",
                &error,
            );
            components::vertical_gap(ui, theme::SPACE_LG);
        }
        if let Some(warning) = self.scan_warning {
            let (title, detail, permission_help) = match warning {
                ScanWarning::NoKeyboardDetected => (
                    "No keyboard detected",
                    "No keyboard-like evdev device was found. Connect a keyboard, then rescan.",
                    false,
                ),
                ScanWarning::PermissionDenied { count } => (
                    "Keyboard permission needed",
                    if count == 1 {
                        "One keyboard was detected but cannot be read. Check Linux /dev/input permissions, then rescan."
                    } else {
                        "Multiple keyboards were detected but cannot be read. Check Linux /dev/input permissions, then rescan."
                    },
                    true,
                ),
                ScanWarning::Unavailable { count } => (
                    "No readable keyboard",
                    if count == 1 {
                        "One possible keyboard could not be inspected. Reconnect it, then rescan."
                    } else {
                        "Possible keyboards could not be inspected. Reconnect them, then rescan."
                    },
                    false,
                ),
                ScanWarning::Incomplete {
                    issue_count,
                    permission_denied,
                } => (
                    "Keyboard list may be incomplete",
                    if issue_count == 1 {
                        "One additional possible keyboard could not be inspected."
                    } else {
                        "Some additional possible keyboards could not be inspected."
                    },
                    permission_denied > 0,
                ),
            };
            components::contextual_banner(ui, BannerSeverity::Warning, title, detail);
            ui.horizontal_wrapped(|ui| {
                if !matches!(self.view, AppView::Settings(SettingsSection::Input))
                    && ui
                        .add_enabled(
                            self.devices.is_some(),
                            egui::Button::new("Rescan keyboards"),
                        )
                        .clicked()
                {
                    self.request_scan();
                }
                if permission_help {
                    ui.hyperlink_to(
                        "Permission help",
                        format!(
                            "{}/blob/main/docs/troubleshooting.md",
                            env!("CARGO_PKG_REPOSITORY")
                        ),
                    )
                    .on_hover_text("Linux evdev permission guidance");
                }
            });
            components::vertical_gap(ui, theme::SPACE_LG);
        }
        if let Some(notice) = self.session_notice.clone() {
            if components::dismissible_warning_banner(
                ui,
                "Selected session no longer exists",
                &notice,
                &[],
            ) {
                self.session_notice = None;
            }
            components::vertical_gap(ui, theme::SPACE_LG);
        }
        if !self.recovery_messages.is_empty() {
            let detail = "Affected analytics restarted empty; unaffected analytics remain usable. The saved database was not modified.";
            if components::dismissible_warning_banner(
                ui,
                "Some analytics could not be restored",
                detail,
                &self.recovery_messages,
            ) {
                self.recovery_messages.clear();
            }
            components::vertical_gap(ui, theme::SPACE_LG);
        }
    }

    fn render_content_page(&mut self, ui: &mut egui::Ui) {
        if self.loading_session {
            ui.heading(egui::RichText::new("Loading session").strong());
            components::vertical_gap(ui, theme::SPACE_LG);
            components::loading_state(ui, "Loading session…", "Preparing aggregate analytics.");
            return;
        }

        match self.view {
            AppView::Overview => self.render_overview_page(ui),
            AppView::KeyUsage => self.render_key_usage_page(ui),
            AppView::Timing(view) => self.render_timing_page(ui, view),
            AppView::Corrections => self.render_corrections_page(ui),
            AppView::Sessions => self.render_sessions_page(ui),
            AppView::Settings(section) => self.render_settings_page(ui, section),
        }
    }

    fn page_heading(&self, ui: &mut egui::Ui, title: &str, description: &str) {
        ui.heading(title);
        ui.label(description);
        components::vertical_gap(ui, theme::SPACE_XL);
    }

    fn render_overview_page(&mut self, ui: &mut egui::Ui) {
        self.page_heading(
            ui,
            "Overview",
            "Aggregate keyboard mechanics for the active session.",
        );
        if !self.working_session.metrics.has_data() {
            components::contextual_banner(
                ui,
                BannerSeverity::Info,
                "Start with a short typing session",
                "Choose a keyboard, select Start, and type normally. evtap builds aggregate analytics without keeping raw input events.",
            );
            components::vertical_gap(ui, theme::SPACE_LG);
        }

        let render_summary = |ui: &mut egui::Ui, index: usize| -> Option<AppView> {
            match index {
                0 => components::open_card(
                    ui,
                    "overview-total",
                    "Open Key Usage",
                    SUMMARY_TILE_MIN_HEIGHT,
                    None,
                    |ui| {
                        self.working_session.metrics.total_presses().summary_ui(ui);
                    },
                )
                .then_some(AppView::KeyUsage),
                1 => components::open_card(
                    ui,
                    "overview-most-used",
                    "Open Key Usage",
                    SUMMARY_TILE_MIN_HEIGHT,
                    None,
                    |ui| self.working_session.metrics.key_usage().most_used_ui(ui),
                )
                .then_some(AppView::KeyUsage),
                2 => components::open_card(
                    ui,
                    "overview-dwell",
                    "Open Dwell timing",
                    SUMMARY_TILE_MIN_HEIGHT,
                    None,
                    |ui| {
                        self.working_session.metrics.dwell_time().summary_ui(ui);
                    },
                )
                .then_some(AppView::Timing(TimingView::Dwell)),
                3 => components::open_card(
                    ui,
                    "overview-flight",
                    "Open Flight timing",
                    SUMMARY_TILE_MIN_HEIGHT,
                    None,
                    |ui| self.working_session.metrics.flight_time().summary_ui(ui),
                )
                .then_some(AppView::Timing(TimingView::Flight)),
                _ => None,
            }
        };
        let mut requested_view = None;
        if ui.available_width() >= 760.0 {
            ui.columns(4, |columns| {
                for (index, column) in columns.iter_mut().enumerate() {
                    if let Some(view) = render_summary(column, index) {
                        requested_view = Some(view);
                    }
                }
            });
        } else {
            for first in [0, 2] {
                ui.columns(2, |columns| {
                    for (offset, column) in columns.iter_mut().enumerate() {
                        if let Some(view) = render_summary(column, first + offset) {
                            requested_view = Some(view);
                        }
                    }
                });
            }
        }
        if let Some(view) = requested_view {
            if let AppView::Timing(timing_view) = view {
                self.timing_view = timing_view;
            }
            self.view = view;
        }

        components::vertical_gap(ui, theme::SPACE_MD);
        let key_usage_card = |ui: &mut egui::Ui| {
            components::open_card(
                ui,
                "overview-key-ranking",
                "Open Key Usage",
                0.0,
                Some("Open Key Usage"),
                |ui| {
                    components::card_header(
                        ui,
                        egui_phosphor::regular::CHART_BAR_HORIZONTAL,
                        "Key Usage",
                    );
                    ui.weak("Most-used physical keys in this session.");
                    components::vertical_gap(ui, theme::SPACE_LG);
                    self.working_session.metrics.key_usage().summary_ui(ui);
                },
            )
        };
        let corrections_card = |ui: &mut egui::Ui| {
            components::open_card(
                ui,
                "overview-corrections",
                "Open Corrections",
                0.0,
                Some("Open Corrections"),
                |ui| {
                    components::card_header(
                        ui,
                        egui_phosphor::regular::ARROW_U_DOWN_LEFT,
                        "Correction Signals",
                    );
                    ui.weak("Observed deletions and inferred replacements.");
                    components::vertical_gap(ui, theme::SPACE_LG);
                    self.working_session.metrics.corrections().summary_ui(ui);
                },
            )
        };
        let (open_key_usage, open_corrections) = if ui.available_width() >= 700.0 {
            let mut open_key_usage = false;
            let mut open_corrections = false;
            let available = ui.available_width();
            let gap = ui.spacing().item_spacing.x;
            let primary_width = (available - gap) * 0.62;
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(primary_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        open_key_usage = key_usage_card(ui);
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(available - primary_width - gap, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        open_corrections = corrections_card(ui);
                    },
                );
            });
            (open_key_usage, open_corrections)
        } else {
            let open_key_usage = key_usage_card(ui);
            components::vertical_gap(ui, theme::SPACE_LG);
            let open_corrections = corrections_card(ui);
            (open_key_usage, open_corrections)
        };
        if open_key_usage {
            self.view = AppView::KeyUsage;
        } else if open_corrections {
            self.view = AppView::Corrections;
        }
    }

    fn render_key_usage_page(&self, ui: &mut egui::Ui) {
        self.page_heading(
            ui,
            "Key Usage",
            "Physical key counts identified by Linux key code, with each key's share of all physical presses in this session.",
        );
        metric_analysis_card(
            ui,
            egui_phosphor::regular::CHART_BAR_HORIZONTAL,
            "Physical key ranking",
            |ui| self.working_session.metrics.key_usage().analysis_ui(ui),
        );
    }

    fn render_timing_page(&mut self, ui: &mut egui::Ui, timing_view: TimingView) {
        let description = match timing_view {
            TimingView::Dwell => {
                "How long produced-text keys are held, weighted by completed press-to-release samples."
            }
            TimingView::Flight => {
                "Release-to-next-press timing grouped by the destination produced text."
            }
            TimingView::Bigrams => {
                "Press-to-press timing for consecutive produced text; pairs appear after at least three samples."
            }
        };
        self.page_heading(ui, "Timing", description);
        ui.horizontal(|ui| {
            for (label, target) in [
                ("Dwell", TimingView::Dwell),
                ("Flight", TimingView::Flight),
                ("Bigrams", TimingView::Bigrams),
            ] {
                if ui.selectable_label(timing_view == target, label).clicked() {
                    self.timing_view = target;
                    self.view = AppView::Timing(target);
                }
            }
        });
        components::vertical_gap(ui, theme::SPACE_LG);
        match timing_view {
            TimingView::Dwell => {
                metric_analysis_card(ui, egui_phosphor::regular::TIMER, "Dwell time", |ui| {
                    self.working_session.metrics.dwell_time().analysis_ui(ui)
                })
            }
            TimingView::Flight => metric_analysis_card(
                ui,
                egui_phosphor::regular::PAPER_PLANE_TILT,
                "Flight time",
                |ui| self.working_session.metrics.flight_time().analysis_ui(ui),
            ),
            TimingView::Bigrams => {
                metric_analysis_card(ui, egui_phosphor::regular::CLOCK, "Bigram speed", |ui| {
                    self.working_session.metrics.bigram_speed().analysis_ui(ui)
                })
            }
        }
    }

    fn render_corrections_page(&self, ui: &mut egui::Ui) {
        self.page_heading(
            ui,
            "Corrections",
            "Backspace-based estimates: deletions are observed before backspace and replacements inferred from the next produced text—not an accuracy score.",
        );
        metric_analysis_card(
            ui,
            egui_phosphor::regular::ARROW_U_DOWN_LEFT,
            "Correction signals",
            |ui| self.working_session.metrics.corrections().analysis_ui(ui),
        );
    }

    fn render_sessions_page(&mut self, ui: &mut egui::Ui) {
        self.render_manage_sessions(ui);
    }

    fn render_session_switcher(&mut self, anchor: &egui::Response) {
        let requested_open = self.session_switcher_open;
        let mut open = requested_open;
        let mut action = None;
        let popup = egui::Popup::menu(anchor)
            .id(egui::Id::new("session-switcher"))
            .open(open)
            .width(360.0)
            .show(|ui| {
                let busy = self.session_controls_busy();
                if self.sessions.is_empty() {
                    ui.weak("No saved sessions yet");
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("session-switcher-list")
                        .max_height(280.0)
                        .show_rows(ui, 48.0, self.sessions.len(), |ui, rows| {
                            for index in rows {
                                let session = &self.sessions[index];
                                let selected = self.working_session.id == Some(session.id);
                                let opened =
                                    format_compact_local_timestamp(session.last_opened_at_ms);
                                let keyboard = session
                                    .keyboard
                                    .display_name
                                    .as_deref()
                                    .unwrap_or("No remembered keyboard");
                                let mut label = if session.name.is_none() {
                                    format!("Untitled session · {opened}")
                                } else {
                                    session.display_name().to_owned()
                                };
                                if selected {
                                    label.push_str(if self.working_dirty() {
                                        "  ·  Current · Unsaved changes"
                                    } else {
                                        "  ·  Current"
                                    });
                                }
                                label.push_str(&format!("\nOpened {opened} · {keyboard}"));
                                if ui
                                    .add_enabled(
                                        !busy,
                                        egui::Button::selectable(selected, label)
                                            .min_size(egui::vec2(ui.available_width(), 48.0)),
                                    )
                                    .clicked()
                                    && !selected
                                {
                                    action = Some(SwitcherAction::Load(session.id));
                                }
                            }
                        });
                }
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(!busy, egui::Button::new("New session"))
                        .clicked()
                    {
                        action = Some(SwitcherAction::New);
                    }
                    if ui.add_enabled(!busy, egui::Button::new("Rename")).clicked() {
                        action = Some(SwitcherAction::RenameCurrent);
                    }
                    if ui.button("Manage sessions").clicked() {
                        action = Some(SwitcherAction::Manage);
                    }
                });
                ui.menu_button("Session actions", |ui| {
                    let destructive_enabled = self.listener.is_none() && !busy;
                    if ui
                        .add_enabled(destructive_enabled, egui::Button::new("Reset statistics"))
                        .clicked()
                    {
                        action = Some(SwitcherAction::ResetCurrent);
                        ui.close();
                    }
                    if ui
                        .add_enabled(destructive_enabled, egui::Button::new("Delete session"))
                        .clicked()
                    {
                        action = Some(SwitcherAction::DeleteCurrent);
                        ui.close();
                    }
                });
            });
        if popup
            .as_ref()
            .is_some_and(|response| response.response.should_close())
            && !anchor.clicked()
        {
            open = false;
        } else if anchor.clicked() {
            open = requested_open;
        }
        self.session_switcher_open = open;

        if let Some(action) = action {
            self.session_switcher_open = false;
            match action {
                SwitcherAction::New => {
                    self.request_boundary_from(BoundaryTarget::New, Some(anchor.id));
                }
                SwitcherAction::Load(session_id) => {
                    self.request_boundary_from(BoundaryTarget::Load(session_id), Some(anchor.id));
                }
                SwitcherAction::RenameCurrent => {
                    let name = self.working_session.name.clone();
                    self.open_rename_dialog(
                        super::super::RenameTarget::Current,
                        name.as_deref(),
                        anchor.id,
                    );
                }
                SwitcherAction::ResetCurrent => {
                    self.begin_prompt(Some(anchor.id));
                    self.confirm_reset = true;
                }
                SwitcherAction::DeleteCurrent => {
                    let session_id = self.working_session.id;
                    let display_name = self.working_session.display_name().to_owned();
                    self.prompt_delete_session(session_id, display_name, true, Some(anchor.id));
                }
                SwitcherAction::Manage => self.open_manage_sessions(),
            }
        }
    }

    pub(super) fn session_controls_busy(&self) -> bool {
        self.loading_session
            || self.storage_tracker.in_flight().is_some()
            || self.deleting_session
            || self.deleting_all
            || self.listener_state == ListenerState::Stopping
    }

    pub(super) fn input_controls_enabled(&self) -> bool {
        self.listener.is_none()
            && !matches!(
                self.listener_state,
                ListenerState::Connecting | ListenerState::Stopping
            )
            && !self.loading_session
            && !self.deleting_session
            && !self.deleting_all
    }

    pub(crate) fn handle_global_shortcuts(&mut self, ctx: &egui::Context, text_edit_focused: bool) {
        if text_edit_focused {
            return;
        }

        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            self.close_foremost_safe_overlay();
            return;
        }

        let overlay_open = self.disclosure_prompt.is_some()
            || self.boundary_prompt.is_some()
            || self.rename_dialog.is_some()
            || self.confirm_reset
            || self.confirm_delete.is_some()
            || self.confirm_delete_all;
        if overlay_open {
            return;
        }

        let ctrl = |key| KeyboardShortcut::new(Modifiers::CTRL, key);
        if ctx.input_mut(|input| input.consume_shortcut(&ctrl(Key::S))) {
            if self.save_action_enabled() {
                self.request_save(None);
            }
        } else if ctx.input_mut(|input| input.consume_shortcut(&ctrl(Key::N))) {
            if !self.session_controls_busy() {
                self.session_switcher_open = false;
                self.request_boundary(BoundaryTarget::New);
            }
        } else if ctx.input_mut(|input| input.consume_shortcut(&ctrl(Key::K))) {
            self.session_switcher_open = true;
        } else if ctx.input_mut(|input| input.consume_shortcut(&ctrl(Key::Comma))) {
            self.session_switcher_open = false;
            self.view = AppView::Settings(SettingsSection::Input);
        }
    }

    fn close_foremost_safe_overlay(&mut self) {
        if self.disclosure_prompt.is_some() {
            self.disclosure_prompt = None;
            self.finish_prompt();
        } else if self.boundary_prompt.is_some() {
            self.boundary_prompt = None;
            self.finish_prompt();
        } else if self.rename_dialog.is_some() {
            self.close_rename_dialog();
        } else if self.confirm_reset {
            self.confirm_reset = false;
            self.finish_prompt();
        } else if self.confirm_delete.is_some() {
            self.confirm_delete = None;
            self.finish_prompt();
        } else if self.confirm_delete_all {
            self.confirm_delete_all = false;
            self.finish_prompt();
        } else {
            self.session_switcher_open = false;
        }
    }
}
