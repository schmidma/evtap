use std::time::Duration;

use chrono::{Local, TimeZone};
use eframe::egui;

use crate::{
    session::SessionMetadata,
    storage::{StorageCommand, StorageOperation, StorageStatus, database_disk_usage},
};

use super::{App, BoundaryTarget, DisclosureIntent, ListenerState};

impl App {
    pub(super) fn render_session_management(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.heading("Session");
            let busy = self.loading_session
                || self.storage_tracker.in_flight().is_some()
                || self.deleting_session
                || self.deleting_all
                || self.listener_state == ListenerState::Stopping;
            let mut requested_target = None;
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    egui::ComboBox::from_label("Current")
                        .selected_text(self.working_session.display_name())
                        .show_ui(ui, |ui| {
                            for saved in &self.sessions {
                                let selected = self.working_session.id == Some(saved.id);
                                let label = session_selector_label(saved);
                                if ui.selectable_label(selected, label).clicked() && !selected {
                                    requested_target = Some(BoundaryTarget::Load(saved.id));
                                }
                            }
                        });
                });
                if ui
                    .add_enabled(!busy, egui::Button::new("New session"))
                    .clicked()
                {
                    requested_target = Some(BoundaryTarget::New);
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Save now"))
                    .clicked()
                {
                    self.request_save(None);
                }
                if ui.add_enabled(!busy, egui::Button::new("Rename")).clicked() {
                    self.rename_buffer = self.working_session.name.clone().unwrap_or_default();
                    self.rename_open = true;
                    self.rename_error = None;
                }
                if ui
                    .add_enabled(
                        self.listener.is_none() && !busy,
                        egui::Button::new("Reset statistics"),
                    )
                    .clicked()
                {
                    self.confirm_reset = true;
                }
                if ui
                    .add_enabled(
                        self.listener.is_none() && !busy,
                        egui::Button::new("Delete session"),
                    )
                    .clicked()
                {
                    self.confirm_delete = true;
                }
            });
            if let Some(target) = requested_target {
                self.request_boundary(target);
            }

            ui.horizontal_wrapped(|ui| {
                let mut autosave = self.settings.autosave_enabled();
                if ui.checkbox(&mut autosave, "Autosave sessions").changed() {
                    if autosave {
                        if self.settings.storage_disclosure_acknowledged() {
                            self.settings.set_autosave_enabled(true);
                            if self.save_settings() {
                                if self.working_dirty() {
                                    self.request_save(None);
                                }
                            } else {
                                self.settings.set_autosave_enabled(false);
                            }
                        } else {
                            self.disclosure_prompt = Some(DisclosureIntent::EnableAutosave);
                        }
                    } else {
                        self.settings.set_autosave_enabled(false);
                        if self.save_settings() {
                            self.checkpoint_schedule.clear();
                        } else {
                            self.settings.set_autosave_enabled(true);
                        }
                    }
                }
                ui.label(storage_status_label(
                    self.storage_tracker.status(),
                    self.working_session.id.is_some(),
                ));
                let retryable = matches!(
                    self.last_failed_operation,
                    Some(StorageOperation::Open | StorageOperation::Save)
                );
                if retryable && ui.button("Retry storage operation").clicked() {
                    if self.last_failed_operation == Some(StorageOperation::Open) {
                        if let Some(worker) = &self.storage {
                            let _ = worker.send(StorageCommand::RetryOpen {
                                last_session_id: self.settings.last_session_id(),
                            });
                        }
                    } else {
                        self.request_save(None);
                    }
                }
            });
            ui.small(format!(
                "Capture duration: {} · Created: {}",
                format_duration(self.working_session.duration()),
                format_local_timestamp(self.working_session.created_at_ms)
            ));
            if self.working_session.restored {
                ui.weak("Restored from disk; capture is paused until you start listening.");
            }
            if let Some(error) = &self.storage_error {
                ui.colored_label(egui::Color32::RED, error);
            }
            ui.horizontal_wrapped(|ui| {
                ui.small(format!(
                    "Storage: {} ({})",
                    self.paths.database_file().display(),
                    format_byte_size(database_disk_usage(&self.paths.database_file()))
                ));
                if ui
                    .add_enabled(
                        !self.sessions.is_empty() && !busy,
                        egui::Button::new("Delete all saved sessions"),
                    )
                    .clicked()
                {
                    self.confirm_delete_all = true;
                }
            });
        });
    }

    pub(super) fn render_capture_setup(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.heading("Capture setup");
            self.render_device_picker(ui);
            ui.add_space(4.0);
            self.render_keyboard_configuration(ui);
            ui.add_space(4.0);
            self.render_capture_controls(ui);
            self.render_capture_status(ui);
        });
    }

    fn render_device_picker(&mut self, ui: &mut egui::Ui) {
        let picker_enabled = self.listener.is_none()
            && !matches!(self.listener_state, ListenerState::Stopping)
            && !self.loading_session;
        let mut request_scan = false;
        ui.horizontal_wrapped(|ui| {
            match &self.devices {
                None => {
                    ui.spinner();
                    ui.label("Scanning for keyboards…");
                }
                Some(devices) if devices.is_empty() => {
                    ui.label("No readable keyboards");
                }
                Some(devices) => {
                    let text = self
                        .selected_device
                        .and_then(|index| devices.get(index))
                        .map_or("Select a keyboard", |device| device.name.as_str());
                    ui.add_enabled_ui(picker_enabled, |ui| {
                        egui::ComboBox::from_label("Keyboard")
                            .selected_text(text)
                            .show_ui(ui, |ui| {
                                for (index, device) in devices.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut self.selected_device,
                                        Some(index),
                                        &device.name,
                                    )
                                    .on_hover_ui(|ui| {
                                        ui.label(format!(
                                            "{} ({})",
                                            device.physical_path, device.path
                                        ));
                                    });
                                }
                            });
                    });
                }
            }
            if ui
                .add_enabled(
                    picker_enabled && self.devices.is_some(),
                    egui::Button::new("Rescan"),
                )
                .clicked()
            {
                request_scan = true;
            }
        });
        if self.working_session.keyboard.display_name.is_some() {
            ui.weak("The session's remembered keyboard is a suggestion; any readable keyboard may be used.");
        }
        if request_scan {
            self.request_scan();
        }
    }

    fn render_keyboard_configuration(&mut self, ui: &mut egui::Ui) {
        let enabled = self.listener.is_none() && !self.loading_session;
        let mut changed = false;
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_label("Model")
                    .width(80.0)
                    .selected_text(&self.model)
                    .show_ui(ui, |ui| {
                        for model in &self.available_models {
                            changed |= ui
                                .selectable_value(&mut self.model, model.clone(), model)
                                .clicked();
                        }
                    });
                egui::ComboBox::from_label("Layout")
                    .selected_text(&self.layout)
                    .show_ui(ui, |ui| {
                        let mut update_variants = false;
                        for layout in &self.available_layouts {
                            if ui
                                .selectable_value(&mut self.layout, layout.clone(), layout)
                                .clicked()
                            {
                                changed = true;
                                update_variants = true;
                            }
                        }
                        if update_variants {
                            self.update_variants();
                        }
                    });
                let variant_text = if self.variant.is_empty() {
                    "Default"
                } else {
                    &self.variant
                };
                egui::ComboBox::from_label("Variant")
                    .selected_text(variant_text)
                    .show_ui(ui, |ui| {
                        changed |= ui
                            .selectable_value(&mut self.variant, String::new(), "Default")
                            .clicked();
                        for variant in &self.available_variants {
                            if !variant.is_empty() {
                                changed |= ui
                                    .selectable_value(&mut self.variant, variant.clone(), variant)
                                    .clicked();
                            }
                        }
                    });
            });
        });
        if changed {
            self.reinit_xkb();
            self.save_keyboard_settings();
            self.working_session.keyboard.model.clone_from(&self.model);
            self.working_session
                .keyboard
                .layout
                .clone_from(&self.layout);
            self.working_session
                .keyboard
                .variant
                .clone_from(&self.variant);
            if self.working_session.id.is_some() || self.session_has_content() {
                self.note_session_dirty();
            }
        }
    }

    fn render_capture_controls(&mut self, ui: &mut egui::Ui) {
        let busy = self.loading_session
            || self.deleting_session
            || self.deleting_all
            || matches!(self.listener_state, ListenerState::Stopping);
        ui.horizontal_wrapped(|ui| {
            if self.listener.is_some() {
                if ui
                    .add_enabled(!busy, egui::Button::new("Stop listening"))
                    .clicked()
                {
                    self.stop_listener();
                }
            } else {
                let selected_index = self.selected_device;
                if ui
                    .add_enabled(
                        selected_index.is_some() && !busy,
                        egui::Button::new("Start listening"),
                    )
                    .clicked()
                    && let Some(index) = selected_index
                {
                    self.begin_listening(index);
                }
            }
        });
    }

    fn render_capture_status(&self, ui: &mut egui::Ui) {
        for error in [
            self.scan_error.as_ref(),
            self.keyboard_error.as_ref(),
            self.settings_error.as_ref(),
            self.capture_error.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            ui.colored_label(egui::Color32::RED, error);
        }
        if let Some(warning) = &self.scan_warning {
            ui.colored_label(egui::Color32::YELLOW, warning);
        }
        match self.listener_state {
            ListenerState::Idle => ui.weak("Not listening"),
            ListenerState::Connecting => ui.label("Connecting to keyboard…"),
            ListenerState::Listening => ui.colored_label(egui::Color32::GREEN, "Listening"),
            ListenerState::Stopping => ui.label("Stopping listener…"),
            ListenerState::Failed => {
                ui.colored_label(egui::Color32::RED, "Capture stopped because of an error")
            }
        };
    }

    pub(super) fn render_prompts(&mut self, ctx: &egui::Context) {
        if let Some(intent) = self.disclosure_prompt {
            egui::Window::new("Save sensitive aggregate statistics?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("evtap reads global keyboard input while listening. Saving writes sensitive character, bigram, correction, key-usage, count, and timing aggregates to a local, unencrypted SQLite database.");
                    ui.label("Raw event sequences, device paths, pressed-key state, and unfinished timing or correction context are not stored. No telemetry or synchronization is performed.");
                    ui.colored_label(egui::Color32::YELLOW, "Anyone who can read your files, including backups or privileged processes, may read the saved aggregates.");
                    ui.horizontal(|ui| {
                        if ui.button("Allow local saves").clicked() {
                            self.settings.acknowledge_storage_disclosure();
                            if self.save_settings() {
                                self.disclosure_prompt = None;
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
                                }
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.disclosure_prompt = None;
                        }
                    });
                });
        }

        if let Some(target) = self.boundary_prompt {
            let exiting = target == BoundaryTarget::Exit;
            egui::Window::new(if exiting {
                "Save changes before exiting?"
            } else {
                "Save changes before switching sessions?"
            })
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} has unsaved changes.",
                    self.working_session.display_name()
                ));
                ui.horizontal(|ui| {
                    if ui
                        .button(if exiting {
                            "Save and exit"
                        } else {
                            "Save and switch"
                        })
                        .clicked()
                    {
                        self.boundary_prompt = None;
                        self.request_save(Some(target));
                    }
                    if ui
                        .button(if self.working_session.id.is_some() {
                            "Discard changes"
                        } else {
                            "Discard session"
                        })
                        .clicked()
                    {
                        self.boundary_prompt = None;
                        self.execute_boundary(target);
                    }
                    if ui.button("Cancel").clicked() {
                        self.boundary_prompt = None;
                    }
                });
            });
        }

        if self.rename_open {
            egui::Window::new("Rename session")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Leave the name empty to keep this session untitled.");
                    ui.text_edit_singleline(&mut self.rename_buffer);
                    if let Some(error) = &self.rename_error {
                        ui.colored_label(egui::Color32::RED, error);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Apply").clicked() {
                            self.apply_rename();
                        }
                        if ui.button("Cancel").clicked() {
                            self.rename_open = false;
                            self.rename_error = None;
                        }
                    });
                });
        }

        if self.confirm_reset {
            egui::Window::new("Reset statistics?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("All aggregate statistics and capture duration in the current session will be reset.");
                    ui.horizontal(|ui| {
                        if ui.button("Reset statistics").clicked() {
                            self.reset_statistics();
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_reset = false;
                        }
                    });
                });
        }

        if self.confirm_delete {
            egui::Window::new("Delete session?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    if self.working_session.id.is_some() {
                        ui.label("The saved copy and all current unsaved changes will be deleted. This cannot be undone.");
                    } else {
                        ui.label("The current in-memory session will be discarded. This cannot be undone.");
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Delete permanently").clicked() {
                            self.delete_current_session();
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_delete = false;
                        }
                    });
                });
        }

        if self.confirm_delete_all {
            egui::Window::new("Delete all saved sessions?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Every saved session and the current working state will be removed. Filesystem backups and snapshots may retain copies.");
                    ui.horizontal(|ui| {
                        if ui.button("Delete all permanently").clicked() {
                            self.delete_all_sessions();
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_delete_all = false;
                        }
                    });
                });
        }
    }
}

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

fn format_local_timestamp(timestamp_ms: i64) -> String {
    Local
        .timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|timestamp| timestamp.format("%Y-%m-%d %H:%M:%S %:z").to_string())
        .unwrap_or_else(|| format!("{timestamp_ms} ms since Unix epoch"))
}

pub(super) fn storage_status_label(status: StorageStatus, has_id: bool) -> &'static str {
    match status {
        StorageStatus::Loading => "Loading saved sessions…",
        StorageStatus::Unsaved => "Unsaved session",
        StorageStatus::Saved if has_id => "Saved",
        StorageStatus::Saved => "Unsaved session",
        StorageStatus::Dirty => "Unsaved changes",
        StorageStatus::Saving => "Saving…",
        StorageStatus::Failed => "Storage operation failed",
    }
}
