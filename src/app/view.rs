use std::time::Duration;

use chrono::{Local, TimeZone};
use eframe::egui;

#[cfg(test)]
use crate::session::SessionMetadata;
use crate::storage::{StorageOperation, StorageStatus};

use super::{App, BoundaryTarget, DisclosureIntent, MAX_SESSION_NAME_BYTES};

pub(crate) mod components;
mod sessions;
mod settings;
mod shell;
pub(super) mod theme;

impl App {
    pub(super) fn render_prompts(&mut self, ctx: &egui::Context) -> bool {
        let focus_to_restore = self.focus_after_prompt.take();
        let mut text_edit_focused = false;
        if let Some(intent) = self.disclosure_prompt {
            let reviewing = intent == DisclosureIntent::Review;
            let title = if reviewing {
                "Local storage disclosure"
            } else {
                "Save sensitive aggregate statistics?"
            };
            let focus_primary = std::mem::take(&mut self.prompt_needs_focus);
            let (_, should_close) = components::modal(ctx, "storage-disclosure", title, |ui| {
                ui.label("evtap reads global keyboard input while listening. Saving writes sensitive character, bigram, correction, key-usage, count, and timing aggregates to a local, unencrypted SQLite database.");
                ui.label("Raw event sequences, device paths, pressed-key state, and unfinished timing or correction context are not stored. No telemetry or synchronization is performed.");
                ui.colored_label(
                        theme::palette(ui.ctx().theme()).warning,
                        "Anyone who can read your files, including backups or privileged processes, may read the saved aggregates.",
                    );
                components::modal_actions(ui, |ui| {
                    if reviewing {
                        let close = components::primary_button(ui, "Close");
                        if focus_primary {
                            close.request_focus();
                        }
                        if close.clicked() {
                            self.disclosure_prompt = None;
                            self.finish_prompt();
                        }
                    } else {
                        let allow = components::primary_button(ui, "Allow local saves");
                        if focus_primary {
                            allow.request_focus();
                        }
                        if allow.clicked() {
                            self.settings.set_storage_disclosure_acknowledged(true);
                            if self.save_settings() {
                                self.disclosure_prompt = None;
                                self.finish_prompt();
                                match intent {
                                    DisclosureIntent::Save(after) => self.begin_save(after),
                                    DisclosureIntent::EnableAutosave => {
                                        self.settings.set_autosave_enabled(true);
                                        if self.save_settings() {
                                            if self.working_dirty() {
                                                self.begin_save(None);
                                            }
                                        } else {
                                            self.settings.set_autosave_enabled(false);
                                        }
                                    }
                                    DisclosureIntent::Review => {}
                                }
                            } else {
                                self.settings.set_storage_disclosure_acknowledged(false);
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.disclosure_prompt = None;
                            self.finish_prompt();
                        }
                    }
                });
            });
            if should_close && self.disclosure_prompt.is_some() {
                self.disclosure_prompt = None;
                self.finish_prompt();
            }
        }

        if let Some(target) = self.boundary_prompt {
            let exiting = target == BoundaryTarget::Exit;
            let title = if exiting {
                "Save changes before exiting?"
            } else {
                "Save changes before switching sessions?"
            };
            let focus_primary = std::mem::take(&mut self.prompt_needs_focus);
            let (_, should_close) = components::modal(ctx, "dirty-boundary", title, |ui| {
                ui.label(format!(
                    "{} has unsaved changes.",
                    self.working_session.display_name()
                ));
                components::modal_actions(ui, |ui| {
                    let save = components::primary_button(
                        ui,
                        if exiting {
                            "Save and exit"
                        } else {
                            "Save and switch"
                        },
                    );
                    if focus_primary {
                        save.request_focus();
                    }
                    if save.clicked() {
                        self.boundary_prompt = None;
                        self.finish_prompt();
                        self.request_save(Some(target));
                    }
                    if components::destructive_button(
                        ui,
                        if self.working_session.id.is_some() {
                            "Discard changes"
                        } else {
                            "Discard session"
                        },
                    )
                    .clicked()
                    {
                        self.boundary_prompt = None;
                        self.finish_prompt();
                        self.execute_boundary(target);
                    }
                    if ui.button("Cancel").clicked() {
                        self.boundary_prompt = None;
                        self.finish_prompt();
                    }
                });
            });
            if should_close && self.boundary_prompt.is_some() {
                self.boundary_prompt = None;
                self.finish_prompt();
            }
        }

        let mut submit_rename = false;
        let mut cancel_rename = false;
        if let Some(dialog) = &mut self.rename_dialog {
            let (_, should_close) =
                components::modal(ctx, "rename-session", "Rename session", |ui| {
                    ui.label("Leave the name empty to keep this session untitled.");
                    ui.label("Session name");
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut dialog.buffer)
                            .id_salt("rename-session-name")
                            .desired_width(f32::INFINITY),
                    );
                    edit.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::TextEdit,
                            edit.enabled(),
                            "Session name",
                        )
                    });
                    if dialog.focus_text {
                        edit.request_focus();
                        let mut state =
                            egui::TextEdit::load_state(ctx, edit.id).unwrap_or_default();
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(dialog.buffer.chars().count()),
                            )));
                        state.store(ctx, edit.id);
                        dialog.focus_text = false;
                    }
                    text_edit_focused = edit.has_focus();
                    let used_bytes = dialog.buffer.trim().len();
                    if used_bytes >= 64 {
                        let remaining = MAX_SESSION_NAME_BYTES.saturating_sub(used_bytes);
                        ui.small(format!("{remaining} of 80 UTF-8 bytes remaining"));
                    }
                    if let Some(error) = &dialog.error {
                        ui.colored_label(egui::Color32::RED, error);
                    }
                    if edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                        submit_rename = !dialog.submitting;
                    }
                    components::modal_actions(ui, |ui| {
                        if ui
                            .add_enabled_ui(!dialog.submitting, |ui| {
                                components::primary_button(ui, "Apply")
                            })
                            .inner
                            .clicked()
                        {
                            submit_rename = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel_rename = true;
                        }
                        if dialog.submitting {
                            ui.weak("Renaming…");
                        }
                    });
                });
            cancel_rename |= should_close;
        }
        if submit_rename {
            self.submit_rename();
        } else if cancel_rename {
            self.close_rename_dialog();
        }

        if self.confirm_reset {
            let mut reset = false;
            let mut cancel = false;
            let focus_primary = std::mem::take(&mut self.prompt_needs_focus);
            let (_, should_close) = components::modal(
                ctx,
                "reset-statistics",
                "Reset statistics?",
                |ui| {
                    ui.label(format!(
                        "All aggregate statistics and capture duration in {} will be reset. This cannot be undone.",
                        self.working_session.display_name()
                    ));
                    components::modal_actions(ui, |ui| {
                        let cancel_button = components::primary_button(ui, "Cancel");
                        if focus_primary {
                            cancel_button.request_focus();
                        }
                        cancel = cancel_button.clicked();
                        reset = components::destructive_button(ui, "Reset statistics").clicked();
                    });
                },
            );
            if reset {
                self.reset_statistics();
            } else if cancel || should_close {
                self.confirm_reset = false;
                self.finish_prompt();
            }
        }

        let mut delete_confirmed = false;
        let mut delete_cancelled = false;
        if let Some(prompt) = &self.confirm_delete {
            let target = prompt.display_name.clone();
            let current = prompt.current;
            let saved = prompt.session_id.is_some();
            let focus_primary = std::mem::take(&mut self.prompt_needs_focus);
            let (_, should_close) = components::modal(
                ctx,
                "delete-session",
                "Delete session?",
                |ui| {
                    ui.label(format!("Delete {target}?"));
                    if current && saved {
                        ui.label("The saved copy and all current unsaved changes will be deleted. This cannot be undone.");
                    } else if current {
                        ui.label("The current in-memory session will be discarded. This cannot be undone.");
                    } else {
                        ui.label("Its saved aggregate statistics will be deleted. This cannot be undone.");
                    }
                    components::modal_actions(ui, |ui| {
                        let cancel = components::primary_button(ui, "Cancel");
                        if focus_primary {
                            cancel.request_focus();
                        }
                        delete_cancelled = cancel.clicked();
                        delete_confirmed =
                            components::destructive_button(ui, "Delete permanently").clicked();
                    });
                },
            );
            delete_cancelled |= should_close;
        }
        if delete_confirmed {
            self.delete_prompted_session();
        } else if delete_cancelled {
            self.confirm_delete = None;
            self.finish_prompt();
        }

        if self.confirm_delete_all {
            let delete_all_detail = if self.working_session.id.is_some() {
                "Every saved session, including the active session and its unsaved changes, will be removed. Filesystem backups and snapshots may retain copies."
            } else {
                "Every saved session will be removed. The active unsaved session will remain in memory. Filesystem backups and snapshots may retain copies."
            };
            let mut delete_all = false;
            let mut cancel = false;
            let focus_primary = std::mem::take(&mut self.prompt_needs_focus);
            let (_, should_close) = components::modal(
                ctx,
                "delete-all-sessions",
                "Delete all saved sessions?",
                |ui| {
                    ui.label(delete_all_detail);
                    components::modal_actions(ui, |ui| {
                        let cancel_button = components::primary_button(ui, "Cancel");
                        if focus_primary {
                            cancel_button.request_focus();
                        }
                        cancel = cancel_button.clicked();
                        delete_all =
                            components::destructive_button(ui, "Delete all permanently").clicked();
                    });
                },
            );
            if delete_all {
                self.delete_all_sessions();
            } else if cancel || should_close {
                self.confirm_delete_all = false;
                self.finish_prompt();
            }
        }

        if let Some(opener) = focus_to_restore {
            ctx.memory_mut(|memory| memory.request_focus(opener));
        }

        text_edit_focused
    }
}

#[cfg(test)]
pub(super) fn session_selector_label(metadata: &SessionMetadata) -> String {
    let keyboard = metadata
        .keyboard
        .display_name
        .as_deref()
        .unwrap_or("No keyboard");
    format!(
        "{} — {} — {}",
        metadata.display_name(),
        keyboard,
        format_local_timestamp(metadata.updated_at_ms)
    )
}

fn format_byte_size(bytes: u64) -> String {
    const KIBIBYTE: u64 = 1024;
    const MEBIBYTE: u64 = 1024 * KIBIBYTE;
    if bytes >= MEBIBYTE {
        format!("{:.1} MiB", bytes as f64 / MEBIBYTE as f64)
    } else if bytes >= KIBIBYTE {
        format!("{:.1} KiB", bytes as f64 / KIBIBYTE as f64)
    } else {
        format!("{bytes} B")
    }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

pub(super) fn format_compact_local_timestamp(timestamp_ms: i64) -> String {
    Local
        .timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|timestamp| timestamp.format("%b %-d, %H:%M").to_string())
        .unwrap_or_else(|| format!("{timestamp_ms} ms"))
}

fn format_local_timestamp(timestamp_ms: i64) -> String {
    Local
        .timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|timestamp| timestamp.format("%Y-%m-%d %H:%M:%S %:z").to_string())
        .unwrap_or_else(|| format!("{timestamp_ms} ms since Unix epoch"))
}

#[cfg(test)]
pub(super) fn storage_status_label(status: StorageStatus, has_id: bool) -> &'static str {
    storage_status_label_for_operation(status, has_id, None)
}

pub(super) fn storage_status_label_for_operation(
    status: StorageStatus,
    has_id: bool,
    failed_operation: Option<StorageOperation>,
) -> &'static str {
    match status {
        StorageStatus::Loading => "Loading…",
        StorageStatus::Unsaved => "Not saved",
        StorageStatus::Saved if has_id => "Saved",
        StorageStatus::Saved => "Not saved",
        StorageStatus::Dirty => "Unsaved changes",
        StorageStatus::Saving => "Saving…",
        StorageStatus::Failed => match failed_operation {
            Some(StorageOperation::Save | StorageOperation::ShutdownSave) => "Save failed",
            Some(StorageOperation::Open) => "Storage unavailable",
            Some(StorageOperation::Load) => "Load failed",
            Some(StorageOperation::Rename) => "Rename failed",
            Some(StorageOperation::Delete) => "Delete failed",
            Some(StorageOperation::DeleteAll) => "Delete all failed",
            Some(StorageOperation::List) => "Session list failed",
            Some(StorageOperation::Maintenance) | None => "Storage failed",
        },
    }
}
