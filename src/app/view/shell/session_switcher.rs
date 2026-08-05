use eframe::egui;

use super::super::format_compact_local_timestamp;
use crate::app::{ActivePromptKind, App, BoundaryTarget, ListenerState, RenameTarget};

enum SwitcherAction {
    New,
    Load(crate::session::SessionId),
    RenameCurrent,
    ResetCurrent,
    DeleteCurrent,
    Manage,
}

impl App {
    pub(in crate::app::view::shell) fn render_session_switcher(&mut self, anchor: &egui::Response) {
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
                    self.open_rename_dialog(RenameTarget::Current, name.as_deref(), anchor.id);
                }
                SwitcherAction::ResetCurrent => {
                    self.open_prompt(ActivePromptKind::Reset, Some(anchor.id));
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

    pub(in crate::app::view) fn session_controls_busy(&self) -> bool {
        self.loading_session
            || self.storage_tracker.in_flight().is_some()
            || self.deleting_session
            || self.deleting_all
            || self.listener_state == ListenerState::Stopping
    }
}
