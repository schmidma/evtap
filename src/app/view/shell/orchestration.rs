use eframe::egui::{self, WidgetInfo, WidgetType};

use super::super::{components, format_duration, storage_status_label_for_operation, theme};
use crate::app::{App, AppView, SettingsSection};

const TOP_BAR_DEVICE_LABEL_CHARS: usize = 18;
pub(in crate::app::view::shell) const COMPACT_TOP_BAR_WINDOW_WIDTH: f32 = 1_000.0;
pub(in crate::app::view::shell) fn compact_top_bar_label(value: &str) -> String {
    compact_label(value, TOP_BAR_DEVICE_LABEL_CHARS)
}

pub(in crate::app::view::shell) fn compact_label(value: &str, maximum_characters: usize) -> String {
    let mut characters = value.chars();
    let visible: String = characters.by_ref().take(maximum_characters).collect();
    if characters.next().is_some() {
        format!("{visible}…")
    } else {
        visible
    }
}

impl App {
    pub(in crate::app::view::shell) fn render_top_bar(&mut self, ui: &mut egui::Ui) {
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
                            self.storage_failure
                                .as_ref()
                                .map(|failure| failure.operation),
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

    pub(in crate::app::view::shell) fn render_navigation(&mut self, ui: &mut egui::Ui) {
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

    pub(in crate::app::view::shell) fn render_content_page(&mut self, ui: &mut egui::Ui) {
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

    fn render_sessions_page(&mut self, ui: &mut egui::Ui) {
        self.render_manage_sessions(ui);
    }
}
