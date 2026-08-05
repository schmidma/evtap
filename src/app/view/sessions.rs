use std::time::Duration;

use eframe::egui::{self, WidgetInfo, WidgetType};

use crate::session::{SessionId, SessionMetadata};

use super::super::{App, BoundaryTarget, RenameTarget};
use super::{components, format_duration, format_local_timestamp, theme};

enum ManageAction {
    New {
        opener: egui::Id,
    },
    Open {
        session_id: SessionId,
        opener: egui::Id,
    },
    Rename {
        session_id: SessionId,
        name: Option<String>,
        current: bool,
        opener: egui::Id,
    },
    Delete {
        session_id: SessionId,
        display_name: String,
        current: bool,
        opener: egui::Id,
    },
}

impl App {
    pub(super) fn render_manage_sessions(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        let mut restored_rename_focus = false;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(egui::RichText::new("Manage Sessions").strong());
                ui.label("Open, rename, or delete saved aggregate sessions.");
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                let new_session = ui.add_enabled(
                    !self.session_controls_busy(),
                    egui::Button::new("New session"),
                );
                if new_session.clicked() {
                    action = Some(ManageAction::New {
                        opener: new_session.id,
                    });
                }
            });
        });
        components::vertical_gap(ui, theme::SPACE_LG);

        if self.manage_list_loading && self.managed_sessions.is_empty() {
            components::loading_state(
                ui,
                "Loading saved sessions…",
                "Reading aggregate session metadata.",
            );
        } else if self.managed_sessions.is_empty() {
            components::empty_state(
                ui,
                egui_phosphor::regular::FOLDER_OPEN,
                "No saved sessions",
                "Save the active session to keep its aggregate statistics between launches.",
            );
        } else {
            let row_height = 160.0;
            let list_height = (ui.available_height() - 112.0).max(180.0);
            egui::ScrollArea::vertical()
                .id_salt("manage-sessions-list")
                .max_height(list_height)
                .show_rows(ui, row_height, self.managed_sessions.len(), |ui, rows| {
                    for index in rows {
                        let session = &self.managed_sessions[index];
                        let current = self.working_session.id == Some(session.id);
                        let busy = self.session_controls_busy();
                        let destructive_enabled =
                            self.listener.is_none() && !busy && !self.deleting_all;
                        let visible_name = if current {
                            self.working_session.display_name().to_owned()
                        } else {
                            session.display_name().to_owned()
                        };
                        let selector_label = if current {
                            format!(
                                "{} — {}",
                                visible_name,
                                format_local_timestamp(session.updated_at_ms)
                            )
                        } else {
                            manage_session_label(session)
                        };

                        ui.push_id(("managed-session", session.id.get()), |ui| {
                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), row_height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    components::card(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(&visible_name)
                                                    .font(theme::semibold_font_for_ui(ui, 15.0)),
                                            );
                                            if current {
                                                ui.label(
                                                    egui::RichText::new(if self.working_dirty() {
                                                        "Current · Unsaved changes"
                                                    } else {
                                                        "Current"
                                                    })
                                                    .color(theme::palette(ui.ctx().theme()).accent),
                                                );
                                            }
                                        });
                                        ui.small(format!(
                                            "Updated {} · {} captured",
                                            format_local_timestamp(session.updated_at_ms),
                                            format_duration(Duration::from_nanos(
                                                session.captured_duration_ns as u64,
                                            ))
                                        ));
                                        ui.small(session_keyboard_label(session));
                                        components::vertical_gap(ui, theme::SPACE_MD);
                                        ui.horizontal(|ui| {
                                            let open = ui.add_enabled(
                                                !busy && !current,
                                                egui::Button::new(if current {
                                                    "Current"
                                                } else {
                                                    "Open"
                                                }),
                                            );
                                            open.widget_info(|| {
                                                WidgetInfo::labeled(
                                                    WidgetType::Button,
                                                    open.enabled(),
                                                    format!("Open {selector_label}"),
                                                )
                                            });
                                            if open.clicked() {
                                                action = Some(ManageAction::Open {
                                                    session_id: session.id,
                                                    opener: open.id,
                                                });
                                            }

                                            let rename =
                                                ui.add_enabled(!busy, egui::Button::new("Rename"));
                                            rename.widget_info(|| {
                                                WidgetInfo::labeled(
                                                    WidgetType::Button,
                                                    rename.enabled(),
                                                    format!("Rename {selector_label}"),
                                                )
                                            });
                                            if self.focus_renamed_session == Some(session.id)
                                                && !self.manage_list_loading
                                            {
                                                rename.request_focus();
                                                restored_rename_focus = rename.has_focus();
                                            }
                                            if rename.clicked() {
                                                action = Some(ManageAction::Rename {
                                                    session_id: session.id,
                                                    name: if current {
                                                        self.working_session.name.clone()
                                                    } else {
                                                        session.name.clone()
                                                    },
                                                    current,
                                                    opener: rename.id,
                                                });
                                            }

                                            let delete = ui.add_enabled(
                                                destructive_enabled,
                                                egui::Button::new("Delete"),
                                            );
                                            delete.widget_info(|| {
                                                WidgetInfo::labeled(
                                                    WidgetType::Button,
                                                    delete.enabled(),
                                                    format!("Delete {selector_label}"),
                                                )
                                            });
                                            if delete.clicked() {
                                                action = Some(ManageAction::Delete {
                                                    session_id: session.id,
                                                    display_name: visible_name.clone(),
                                                    current,
                                                    opener: delete.id,
                                                });
                                            }
                                        });
                                    });
                                },
                            );
                        });
                    }
                });
        }

        components::vertical_gap(ui, theme::SPACE_LG);
        let palette = theme::palette(ui.ctx().theme());
        egui::Frame::new()
            .fill(palette.error.gamma_multiply(0.06))
            .stroke(egui::Stroke::new(1.0, palette.error.gamma_multiply(0.5)))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::same(theme::CARD_PADDING))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("Delete all saved sessions")
                            .font(theme::semibold_font_for_ui(ui, 14.0)),
                    );
                    ui.weak("Removes every saved aggregate session from local storage.");
                    let delete_all = ui.add_enabled(
                        !self.managed_sessions.is_empty()
                            && self.listener.is_none()
                            && !self.session_controls_busy(),
                        egui::Button::new("Delete all saved sessions"),
                    );
                    if delete_all.clicked() {
                        self.open_prompt(
                            super::super::ActivePromptKind::DeleteAll,
                            Some(delete_all.id),
                        );
                    }
                });
            });

        if restored_rename_focus {
            self.focus_renamed_session = None;
        }

        if let Some(action) = action {
            match action {
                ManageAction::New { opener } => {
                    self.request_boundary_from(BoundaryTarget::New, Some(opener));
                }
                ManageAction::Open { session_id, opener } => {
                    self.request_boundary_from(BoundaryTarget::Load(session_id), Some(opener));
                }
                ManageAction::Rename {
                    session_id,
                    name,
                    current,
                    opener,
                } => self.open_rename_dialog(
                    if current {
                        RenameTarget::Current
                    } else {
                        RenameTarget::Saved(session_id)
                    },
                    name.as_deref(),
                    opener,
                ),
                ManageAction::Delete {
                    session_id,
                    display_name,
                    current,
                    opener,
                } => self.prompt_delete_session(
                    Some(session_id),
                    display_name,
                    current,
                    Some(opener),
                ),
            }
        }
    }
}

fn manage_session_label(session: &SessionMetadata) -> String {
    format!(
        "{} — {}",
        session.display_name(),
        format_local_timestamp(session.updated_at_ms)
    )
}

fn session_keyboard_label(session: &SessionMetadata) -> String {
    let keyboard = session
        .keyboard
        .display_name
        .as_deref()
        .unwrap_or("No remembered keyboard");
    format!(
        "Keyboard: {keyboard} · XKB: {} / {}{}",
        session.keyboard.model,
        session.keyboard.layout,
        if session.keyboard.variant.is_empty() {
            String::new()
        } else {
            format!(" / {}", session.keyboard.variant)
        }
    )
}
