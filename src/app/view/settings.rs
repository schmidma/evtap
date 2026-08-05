use std::time::Duration;

use eframe::egui::{self, WidgetInfo, WidgetType};

use crate::{settings::AppearancePreference, storage::database_disk_usage};

use super::super::{App, AppView, DisclosureIntent, SettingsSection};
use super::{components, format_byte_size, theme};

const SETTINGS_CATEGORY_WIDTH: f32 = 176.0;
const COPY_CONFIRMATION_DURATION: Duration = Duration::from_secs(2);

impl App {
    pub(super) fn render_settings_page(&mut self, ui: &mut egui::Ui, section: SettingsSection) {
        ui.heading(egui::RichText::new("Settings").font(theme::semibold_font_for_ui(ui, 24.0)));
        ui.label("Configure capture, keyboard interpretation, local storage, and appearance.");
        self.render_inline_settings_error(ui);
        components::vertical_gap(ui, theme::SPACE_XL);

        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(SETTINGS_CATEGORY_WIDTH, ui.available_height()),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.spacing_mut().item_spacing.y = theme::SPACE_SM;
                    for (label, target) in [
                        ("Input", SettingsSection::Input),
                        (
                            "Keyboard interpretation",
                            SettingsSection::KeyboardInterpretation,
                        ),
                        ("Storage & privacy", SettingsSection::StoragePrivacy),
                        ("Appearance", SettingsSection::Appearance),
                        ("About", SettingsSection::About),
                    ] {
                        let response = ui.add_sized(
                            [ui.available_width(), 40.0],
                            egui::Button::selectable(section == target, label),
                        );
                        if response.clicked() {
                            self.view = AppView::Settings(target);
                        }
                    }
                },
            );
            ui.separator();
            components::horizontal_gap(ui, theme::SPACE_LG);
            ui.vertical(|ui| {
                ui.set_min_width(300.0);
                match section {
                    SettingsSection::Input => self.render_input_settings(ui),
                    SettingsSection::KeyboardInterpretation => {
                        self.render_keyboard_interpretation_settings(ui);
                    }
                    SettingsSection::StoragePrivacy => self.render_storage_settings(ui),
                    SettingsSection::Appearance => self.render_appearance_settings(ui),
                    SettingsSection::About => self.render_about_settings(ui),
                }
            });
        });
    }

    fn render_input_settings(&mut self, ui: &mut egui::Ui) {
        settings_heading(
            ui,
            "Input",
            "Choose the readable Linux evdev keyboard used while capture is active.",
        );
        components::card(ui, |ui| {
            ui.set_width(ui.available_width());
            components::card_header(ui, egui_phosphor::regular::KEYBOARD, "Capture keyboard");
            components::vertical_gap(ui, theme::SPACE_LG);
            self.render_device_picker(ui);
            components::vertical_gap(ui, theme::SPACE_LG);
            ui.weak("Device selection controls capture only. Keyboard interpretation is configured separately.");
        });
    }

    fn render_device_picker(&mut self, ui: &mut egui::Ui) {
        let picker_enabled = self.input_controls_enabled();
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
                            });
                    })
                    .response
                    .on_disabled_hover_text("Stop capture to change the capture keyboard.");
                }
            }
            if ui
                .add_enabled(
                    picker_enabled && self.devices.is_some(),
                    egui::Button::new("Rescan keyboards"),
                )
                .on_disabled_hover_text(if picker_enabled {
                    "Wait for the current keyboard scan to finish."
                } else {
                    "Stop capture to rescan keyboards."
                })
                .clicked()
            {
                request_scan = true;
            }
        });
        if self.working_session.keyboard.display_name.is_some() {
            ui.weak("The session's remembered keyboard is a suggestion; any readable keyboard may be used.");
        }
        if !picker_enabled {
            ui.weak("Stop capture before changing or rescanning input devices.");
        }
        if request_scan {
            self.request_scan();
        }
    }

    fn render_keyboard_interpretation_settings(&mut self, ui: &mut egui::Ui) {
        settings_heading(
            ui,
            "Keyboard interpretation",
            "Choose how XKB translates physical keys for text-based metrics.",
        );
        components::card(ui, |ui| {
            ui.set_width(ui.available_width());
            components::card_header(ui, egui_phosphor::regular::KEYBOARD, "XKB configuration");
            components::vertical_gap(ui, theme::SPACE_LG);
            ui.label("These choices affect produced-text metrics. They never restrict which keyboard evtap can capture.");
            components::vertical_gap(ui, theme::SPACE_LG);
            self.render_keyboard_configuration(ui);
        });

        if let Some(error) = self.keyboard_error.clone() {
            components::vertical_gap(ui, theme::SPACE_LG);
            components::contextual_banner(
                ui,
                components::BannerSeverity::Error,
                "Keyboard configuration could not be applied",
                &error,
            );
        }
    }

    fn render_keyboard_configuration(&mut self, ui: &mut egui::Ui) {
        let enabled = self.input_controls_enabled();
        let previous_model = self.model.clone();
        let previous_layout = self.layout.clone();
        let previous_variant = self.variant.clone();
        let previous_available_variants = self.available_variants.clone();
        let controls = ui.add_enabled_ui(enabled, |ui| {
            let model = searchable_selector(
                ui,
                "xkb-model-selector",
                "Model",
                &self.model,
                &self.available_models,
            );
            let layout = searchable_selector(
                ui,
                "xkb-layout-selector",
                "Layout",
                &self.layout,
                &self.available_layouts,
            );
            let variant = searchable_selector(
                ui,
                "xkb-variant-selector",
                "Variant",
                &self.variant,
                &self.available_variants,
            );
            components::vertical_gap(ui, theme::SPACE_LG);
            let using_defaults =
                self.model.is_empty() && self.layout.is_empty() && self.variant.is_empty();
            let reset = ui
                .add_enabled(!using_defaults, egui::Button::new("Reset to defaults"))
                .on_disabled_hover_text("The XKB configuration already uses defaults.")
                .clicked();
            (model, layout, variant, reset)
        });
        controls
            .response
            .on_disabled_hover_text("Stop capture to change keyboard interpretation.");

        let (model, layout, variant, reset) = controls.inner;
        let mut changed = false;
        if let Some(model) = model {
            self.model = model;
            changed = true;
        }
        if let Some(layout) = layout {
            self.layout = layout;
            self.update_variants();
            changed = true;
        }
        if let Some(variant) = variant {
            self.variant = variant;
            changed = true;
        }
        if reset {
            self.model.clear();
            self.layout.clear();
            self.variant.clear();
            self.available_variants.clear();
            changed = true;
        }
        if changed && !self.apply_keyboard_settings() {
            self.model = previous_model;
            self.layout = previous_layout;
            self.variant = previous_variant;
            self.available_variants = previous_available_variants;
        }
        if !enabled {
            ui.weak("Stop capture before changing keyboard interpretation.");
        }
    }

    fn render_storage_settings(&mut self, ui: &mut egui::Ui) {
        settings_heading(
            ui,
            "Storage & privacy",
            "Control optional local persistence of aggregate session analytics.",
        );

        components::card(ui, |ui| {
            ui.set_width(ui.available_width());
            components::card_header(ui, egui_phosphor::regular::DATABASE, "Local database");
            components::vertical_gap(ui, theme::SPACE_LG);
            ui.label("Saved data contains durable aggregates only. It is unencrypted, kept in a user-only location, and never synchronized by evtap.");
            components::vertical_gap(ui, theme::SPACE_MD);
            ui.weak("Database path");
            let database_path = self.paths.database_file().display().to_string();
            ui.add(egui::Label::new(&database_path).wrap());
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "Disk usage: {}",
                    format_byte_size(database_disk_usage(&self.paths.database_file()))
                ));
                copy_path_button(ui, &database_path);
            });
            components::vertical_gap(ui, theme::SPACE_MD);
            ui.small("Directories use user-only permissions; database and settings files are restricted to the current user. Filesystem backups and privileged processes may still retain or read copies.");
            if self.settings.storage_disclosure_acknowledged() {
                components::vertical_gap(ui, theme::SPACE_MD);
                let review = ui.button("Review local storage disclosure");
                if review.clicked() {
                    self.open_disclosure_prompt(DisclosureIntent::Review, Some(review.id));
                }
            } else {
                components::vertical_gap(ui, theme::SPACE_MD);
                ui.weak("The complete disclosure appears before the first manual save or when autosave is enabled.");
            }
        });

        components::vertical_gap(ui, theme::SPACE_LG);
        components::card(ui, |ui| {
            ui.set_width(ui.available_width());
            components::card_header(ui, egui_phosphor::regular::ARROWS_CLOCKWISE, "Autosave");
            components::vertical_gap(ui, theme::SPACE_LG);
            let mut autosave = self.settings.autosave_enabled();
            let autosave_toggle = ui.checkbox(&mut autosave, "Autosave sessions");
            if autosave_toggle.changed() {
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
                        self.open_disclosure_prompt(
                            DisclosureIntent::EnableAutosave,
                            Some(autosave_toggle.id),
                        );
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
            ui.small("With unsaved changes, autosave writes immediately when enabled, every 30 seconds during capture, after Stop or listener failure, before switching or creating a new session, and during a normal close.");
            ui.small("Save now remains available independently in the top bar.");
        });

        components::vertical_gap(ui, theme::SPACE_LG);
        let palette = theme::palette(ui.ctx().theme());
        egui::Frame::new()
            .fill(palette.surface_subtle)
            .stroke(egui::Stroke::new(1.0, palette.error))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::same(theme::CARD_PADDING))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                components::section_title(ui, "Saved data actions");
                ui.label("Review or permanently delete locally saved aggregate sessions.");
                components::vertical_gap(ui, theme::SPACE_LG);
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Manage saved sessions").clicked() {
                        self.open_manage_sessions();
                    }
                    let can_delete = !self.session_controls_busy()
                        && (!self.sessions.is_empty() || self.working_session.id.is_some());
                    let delete_all = ui
                        .add_enabled(can_delete, egui::Button::new("Delete all saved sessions"))
                        .on_disabled_hover_text(if self.session_controls_busy() {
                            "Wait for the current session operation to finish."
                        } else {
                            "There are no saved sessions to delete."
                        });
                    if delete_all.clicked() {
                        self.open_prompt(
                            super::super::ActivePromptKind::DeleteAll,
                            Some(delete_all.id),
                        );
                    }
                });
            });
    }

    fn render_appearance_settings(&mut self, ui: &mut egui::Ui) {
        settings_heading(
            ui,
            "Appearance",
            "Choose whether evtap follows the desktop theme or uses a fixed appearance.",
        );
        components::card(ui, |ui| {
            ui.set_width(ui.available_width());
            components::card_header(ui, egui_phosphor::regular::PALETTE, "Color theme");
            components::vertical_gap(ui, theme::SPACE_LG);
            let current = self.settings.appearance_preference();
            let mut selected = current;
            for (label, detail, preference) in [
                (
                    "System",
                    "Follow the desktop light or dark preference.",
                    AppearancePreference::System,
                ),
                (
                    "Light",
                    "Always use the light theme.",
                    AppearancePreference::Light,
                ),
                (
                    "Dark",
                    "Always use the dark theme.",
                    AppearancePreference::Dark,
                ),
            ] {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut selected, preference, label);
                    ui.weak(detail);
                });
            }
            if selected != current {
                self.settings.set_appearance_preference(selected);
                if self.save_settings() {
                    theme::install(ui.ctx(), selected);
                } else {
                    self.settings.set_appearance_preference(current);
                }
            }
        });
    }

    fn render_about_settings(&mut self, ui: &mut egui::Ui) {
        settings_heading(
            ui,
            "About",
            "Version, project resources, and software licenses.",
        );
        components::card(ui, |ui| {
            ui.set_width(ui.available_width());
            components::card_header(ui, egui_phosphor::regular::INFO, "evtap");
            components::vertical_gap(ui, theme::SPACE_LG);
            ui.label(
                egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                    .font(theme::semibold_font_for_ui(ui, 16.0)),
            );
            ui.label(env!("CARGO_PKG_DESCRIPTION"));
            components::vertical_gap(ui, theme::SPACE_LG);
            let repository = env!("CARGO_PKG_REPOSITORY");
            ui.horizontal_wrapped(|ui| {
                ui.hyperlink_to("Repository", repository);
                ui.hyperlink_to("Releases", format!("{repository}/releases"));
                ui.hyperlink_to("Documentation", format!("{repository}/blob/main/README.md"));
            });
            components::vertical_gap(ui, theme::SPACE_LG);
            components::section_title(ui, "Licenses");
            ui.horizontal_wrapped(|ui| {
                ui.hyperlink_to("MIT", format!("{repository}/blob/main/LICENSE-MIT"));
                ui.hyperlink_to(
                    "Apache-2.0",
                    format!("{repository}/blob/main/LICENSE-APACHE"),
                );
                ui.hyperlink_to(
                    "Inter font (SIL OFL 1.1)",
                    format!("{repository}/blob/main/LICENSE-INTER"),
                );
            });
            components::vertical_gap(ui, theme::SPACE_LG);
            ui.weak("Links open only when selected. evtap performs no automatic update checks or background network requests.");
        });
    }

    fn render_inline_settings_error(&self, ui: &mut egui::Ui) {
        if let Some(error) = &self.settings_error {
            components::vertical_gap(ui, theme::SPACE_LG);
            components::contextual_banner(
                ui,
                components::BannerSeverity::Error,
                "Settings could not be saved",
                error,
            );
        }
    }
}

fn settings_heading(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.label(egui::RichText::new(title).font(theme::semibold_font_for_ui(ui, 18.0)));
    ui.label(description);
    components::vertical_gap(ui, theme::SPACE_LG);
}

fn searchable_selector(
    ui: &mut egui::Ui,
    id_salt: &'static str,
    label: &str,
    current: &str,
    options: &[String],
) -> Option<String> {
    let mut selected = None;
    let query_id = ui.make_persistent_id((id_salt, "query"));
    let selected_text = if current.is_empty() {
        "Default"
    } else {
        current
    };

    egui::ComboBox::new(id_salt, label)
        .width(ui.available_width().min(360.0))
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            ui.set_min_width(300.0);
            let mut query = ui
                .ctx()
                .data_mut(|data| data.get_temp::<String>(query_id).unwrap_or_default());
            let search_label = ui.label(format!("Search {}", label.to_lowercase()));
            let search = ui
                .add(
                    egui::TextEdit::singleline(&mut query)
                        .id_salt((id_salt, "search"))
                        .hint_text("Type to filter…")
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(search_label.id);
            if search.changed() {
                ui.ctx()
                    .data_mut(|data| data.insert_temp(query_id, query.clone()));
            }
            ui.separator();

            let normalized = query.trim().to_lowercase();
            let default_matches = normalized.is_empty() || "default".contains(&normalized);
            if default_matches && ui.selectable_label(current.is_empty(), "Default").clicked() {
                selected = Some(String::new());
                ui.ctx()
                    .data_mut(|data| data.insert_temp(query_id, String::new()));
                ui.close();
            }

            let matches: Vec<&str> = options
                .iter()
                .map(String::as_str)
                .filter(|option| {
                    !option.is_empty()
                        && (normalized.is_empty()
                            || option.to_lowercase().contains(normalized.as_str()))
                })
                .collect();
            let row_height = ui.spacing().interact_size.y;
            egui::ScrollArea::vertical()
                .id_salt((id_salt, "results"))
                .max_height(220.0)
                .show_rows(ui, row_height, matches.len(), |ui, rows| {
                    for option in &matches[rows] {
                        if ui.selectable_label(current == *option, *option).clicked() {
                            selected = Some((*option).to_owned());
                            ui.ctx()
                                .data_mut(|data| data.insert_temp(query_id, String::new()));
                            ui.close();
                        }
                    }
                });
            if !default_matches && matches.is_empty() {
                ui.weak("No matching options");
            }
        });

    selected
}

fn copy_path_button(ui: &mut egui::Ui, path: &str) {
    let copied_id = ui.make_persistent_id("database-path-copied-until");
    let now = ui.input(|input| input.time);
    let copied_until = ui
        .ctx()
        .data_mut(|data| data.get_temp::<f64>(copied_id).unwrap_or(f64::NEG_INFINITY));
    let copied = now < copied_until;
    let response = ui.button(if copied { "Copied" } else { "Copy path" });
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::Button,
            ui.is_enabled(),
            if copied {
                "Database path copied"
            } else {
                "Copy database path"
            },
        )
    });
    if response.clicked() {
        ui.ctx().copy_text(path.to_owned());
        ui.ctx().data_mut(|data| {
            data.insert_temp(copied_id, now + COPY_CONFIRMATION_DURATION.as_secs_f64());
        });
        ui.ctx().request_repaint();
    } else if copied {
        ui.ctx()
            .request_repaint_after(Duration::from_secs_f64((copied_until - now).max(0.0)));
    }
}
