use eframe::egui::{self, WidgetInfo, WidgetType};

use super::super::{
    components::{self, BannerSeverity},
    storage_status_label_for_operation, theme,
};
use super::orchestration::{COMPACT_TOP_BAR_WINDOW_WIDTH, compact_top_bar_label};
use crate::{
    app::{App, AppView, ListenerState, ScanWarning, SettingsSection},
    storage::StorageStatus,
};

impl App {
    pub(in crate::app::view::shell) fn render_save_slot(&mut self, ui: &mut egui::Ui) {
        let status = self.storage_tracker.status();
        let label = storage_status_label_for_operation(
            status,
            self.working_session.id.is_some(),
            self.storage_failure
                .as_ref()
                .map(|failure| failure.operation),
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

    pub(in crate::app::view::shell) fn save_action_enabled(&self) -> bool {
        let already_saved = self.storage_tracker.status() == StorageStatus::Saved
            && self.working_session.id.is_some()
            && !self.working_dirty();
        !self.save_controls_busy() && !already_saved
    }

    pub(in crate::app::view::shell) fn render_top_keyboard_slot(&mut self, ui: &mut egui::Ui) {
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

    pub(in crate::app::view::shell) fn render_capture_status_slot(&self, ui: &mut egui::Ui) {
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

    pub(in crate::app::view::shell) fn render_capture_action(&mut self, ui: &mut egui::Ui) {
        let busy = self.loading_session
            || self.deleting_session
            || self.deleting_all
            || matches!(
                self.listener_state,
                ListenerState::Connecting | ListenerState::Stopping
            );
        let action_width = if ui.ctx().content_rect().width() < COMPACT_TOP_BAR_WINDOW_WIDTH {
            78.0
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

    pub(in crate::app::view::shell) fn render_status_banners(&mut self, ui: &mut egui::Ui) {
        if let Some(failure) = self.storage_failure.clone() {
            use crate::storage::StorageOperation;
            let title = match failure.operation {
                StorageOperation::Open => "Local storage unavailable",
                StorageOperation::Save | StorageOperation::ShutdownSave => {
                    "Session could not be saved"
                }
                StorageOperation::List => "Session list could not be loaded",
                StorageOperation::Load => "Session could not be loaded",
                StorageOperation::Rename => "Session could not be renamed",
                StorageOperation::Delete => "Session could not be deleted",
                StorageOperation::DeleteAll => "Saved sessions could not be deleted",
                StorageOperation::Maintenance => "Storage needs attention",
            };
            components::contextual_banner(ui, BannerSeverity::Error, title, &failure.message);
            egui::CollapsingHeader::new("Technical details")
                .id_salt("storage-error-details")
                .show(ui, |ui| {
                    ui.add(egui::Label::new(&failure.details).selectable(true));
                });
            match failure.operation {
                StorageOperation::Open => {
                    if ui.button("Retry opening local storage").clicked() {
                        let retry_error = self.storage.as_ref().map_or_else(
                            || Some("Storage worker is unavailable".to_owned()),
                            |worker| {
                                worker
                                    .send(crate::storage::StorageCommand::RetryOpen {
                                        last_session_id: self.settings.last_session_id(),
                                    })
                                    .err()
                                    .map(|error| {
                                        format!("Could not retry opening local storage: {error}")
                                    })
                            },
                        );
                        if let Some(error) = retry_error {
                            self.set_storage_failure(StorageOperation::Open, None, error);
                        }
                    }
                }
                StorageOperation::Save | StorageOperation::ShutdownSave => {
                    if ui.button("Retry save").clicked() {
                        self.request_save(None);
                    }
                }
                StorageOperation::List => {
                    if ui.button("Retry session list").clicked() {
                        match failure.list_order {
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
                StorageOperation::Load => {
                    if ui.button("Choose a session").clicked() {
                        self.session_switcher_open = true;
                    }
                }
                StorageOperation::Delete | StorageOperation::DeleteAll => {
                    if ui.button("Open Manage Sessions").clicked() {
                        self.open_manage_sessions();
                    }
                }
                StorageOperation::Rename | StorageOperation::Maintenance => {}
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

    pub(in crate::app::view) fn input_controls_enabled(&self) -> bool {
        self.listener.is_none()
            && !matches!(
                self.listener_state,
                ListenerState::Connecting | ListenerState::Stopping
            )
            && !self.loading_session
            && !self.deleting_session
            && !self.deleting_all
    }
}
