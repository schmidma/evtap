mod analytics;
mod orchestration;
mod session_switcher;
mod status;

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};

use super::{components, theme};
use crate::app::{App, AppView, BoundaryTarget, SettingsSection};

const TOP_BAR_HEIGHT: f32 = 64.0;
const NAVIGATION_WIDTH: f32 = 168.0;

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

    pub(crate) fn handle_global_shortcuts(&mut self, ctx: &egui::Context, text_edit_focused: bool) {
        if text_edit_focused {
            return;
        }

        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            self.close_foremost_safe_overlay();
            return;
        }

        if self.active_prompt.is_some() {
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
        if self.active_prompt.is_some() {
            let _ = self.finish_prompt();
        } else {
            self.session_switcher_open = false;
        }
    }
}
