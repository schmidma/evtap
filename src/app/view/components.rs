use std::time::Duration;

use eframe::egui::{self, Response, WidgetInfo, WidgetType};

use super::theme;

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BannerSeverity {
    Info,
    Warning,
    Error,
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TextTokenContext {
    ProducedText,
    PhysicalKey,
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TextTokenDescription {
    pub visible: String,
    pub escaped: String,
    pub code_points: String,
    pub unicode_names: String,
    pub accessible_label: String,
    context_label: &'static str,
    printable: bool,
}

pub(super) fn modal<R>(
    ctx: &egui::Context,
    id_salt: impl egui::AsIdSalt,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> (R, bool) {
    let available = ctx.content_rect().size();
    let width = (available.x - 64.0).clamp(280.0, 520.0);
    let response = egui::Modal::new(egui::Id::new(id_salt)).show(ctx, |ui| {
        ui.set_width(width);
        ui.ctx().accesskit_node_builder(ui.id(), |node| {
            node.set_role(egui::accesskit::Role::Dialog);
            node.set_label(title);
            node.set_modal();
        });
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("dialog-content")
                .accessibility_parent(ui.id()),
            |ui| {
                ui.set_width(width);
                ui.label(egui::RichText::new(title).font(theme::semibold_font_for_ui(ui, 20.0)));
                vertical_gap(ui, theme::SPACE_LG);
                add_contents(ui)
            },
        )
        .inner
    });
    let should_close = response.should_close();
    (response.inner, should_close)
}

pub(super) fn vertical_gap(ui: &mut egui::Ui, gap: f32) {
    let extra = (gap - ui.spacing().item_spacing.y).max(0.0);
    ui.add_space(extra);
}

pub(super) fn horizontal_gap(ui: &mut egui::Ui, gap: f32) {
    let extra = (gap - ui.spacing().item_spacing.x).max(0.0);
    ui.add_space(extra);
}

pub(super) fn modal_actions<R>(
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    vertical_gap(ui, theme::SPACE_LG);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
        ui.horizontal_wrapped(add_contents)
    })
    .inner
}

pub(super) fn primary_button(ui: &mut egui::Ui, label: &str) -> Response {
    let palette = theme::palette(ui.ctx().theme());
    let text = if ui.visuals().dark_mode {
        palette.background
    } else {
        egui::Color32::WHITE
    };
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(text))
            .fill(palette.accent)
            .stroke(egui::Stroke::new(1.0, palette.accent_hover)),
    )
}

pub(super) fn destructive_button(ui: &mut egui::Ui, label: &str) -> Response {
    let palette = theme::palette(ui.ctx().theme());
    let text = if ui.visuals().dark_mode {
        palette.background
    } else {
        egui::Color32::WHITE
    };
    ui.add(egui::Button::new(egui::RichText::new(label).color(text)).fill(palette.error))
}

pub(super) fn card<R>(
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let palette = theme::palette(ui.ctx().theme());
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(egui::Stroke::new(1.0, palette.border))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(theme::CARD_PADDING))
        .show(ui, |ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), add_contents)
                .inner
        })
}

pub(super) fn open_card(
    ui: &mut egui::Ui,
    id_salt: impl egui::AsIdSalt,
    accessible_label: &str,
    minimum_height: f32,
    footer_label: Option<&str>,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> bool {
    ui.push_id(id_salt, |ui| {
        let card = card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(minimum_height);
            add_contents(ui);
            if let Some(footer_label) = footer_label {
                vertical_gap(ui, theme::SPACE_LG);
                ui.separator();
                vertical_gap(ui, theme::SPACE_MD);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(footer_label)
                            .font(theme::medium_font_for_ui(ui, 13.0))
                            .color(theme::palette(ui.ctx().theme()).accent),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(egui_phosphor::regular::ARROW_RIGHT)
                                .font(theme::icon_font_for_ui(ui, 14.0))
                                .color(theme::palette(ui.ctx().theme()).accent),
                        );
                    });
                });
            }
        });
        let response = card
            .response
            .interact(egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), accessible_label)
        });

        let palette = theme::palette(ui.ctx().theme());
        if response.hovered() || response.has_focus() {
            ui.painter().rect_stroke(
                response.rect,
                egui::CornerRadius::same(10),
                egui::Stroke::new(
                    if response.has_focus() { 2.0 } else { 1.5 },
                    palette.accent_hover,
                ),
                egui::StrokeKind::Inside,
            );
        }
        if footer_label.is_none() {
            ui.painter().text(
                response.rect.right_top() + egui::vec2(-theme::SPACE_LG, theme::SPACE_LG),
                egui::Align2::RIGHT_TOP,
                egui_phosphor::regular::ARROW_UP_RIGHT,
                theme::icon_font_for_ui(ui, 14.0),
                if response.hovered() || response.has_focus() {
                    palette.accent_hover
                } else {
                    ui.visuals().weak_text_color()
                },
            );
        }
        response.clicked()
    })
    .inner
}

pub(crate) fn card_header(ui: &mut egui::Ui, icon: &str, title: &str) -> Response {
    let palette = theme::palette(ui.ctx().theme());

    ui.horizontal(|ui| {
        egui::Frame::new()
            .fill(palette.accent.gamma_multiply(0.12))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::same(theme::SPACE_XS as i8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(icon)
                        .font(theme::icon_font_for_ui(ui, 16.0))
                        .color(palette.accent),
                );
            });
        ui.add(
            egui::Label::new(
                egui::RichText::new(title).font(theme::semibold_font_for_ui(ui, 14.0)),
            )
            .wrap(),
        );
    })
    .response
}

pub(crate) fn section_title(ui: &mut egui::Ui, title: &str) -> Response {
    let response = ui.label(egui::RichText::new(title).font(theme::semibold_font_for_ui(ui, 16.0)));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, ui.is_enabled(), title));
    response
}

pub(crate) fn disclosure_list(
    ui: &mut egui::Ui,
    state_id: egui::Id,
    total_rows: usize,
    collapsed_rows: usize,
    add_rows: impl FnOnce(&mut egui::Ui, usize),
) {
    let expanded = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(state_id))
        .unwrap_or(false);
    let shown_rows = if expanded {
        total_rows
    } else {
        total_rows.min(collapsed_rows)
    };
    ui.weak(format!("Showing {shown_rows} of {total_rows}"));
    add_rows(ui, shown_rows);

    if total_rows <= collapsed_rows {
        return;
    }

    vertical_gap(ui, theme::SPACE_LG);
    ui.separator();
    let remaining = total_rows.saturating_sub(shown_rows);
    let (visible_label, accessible_label) = if expanded {
        ("Show fewer  ↑".to_owned(), "Show fewer".to_owned())
    } else {
        (
            format!("Show {remaining} more  ↓"),
            format!("Show {remaining} more"),
        )
    };
    let response = ui.add_sized(
        [ui.available_width(), 36.0],
        egui::Button::new(
            egui::RichText::new(visible_label).font(theme::medium_font_for_ui(ui, 13.0)),
        )
        .frame(false),
    );
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::Button,
            response.enabled(),
            accessible_label.clone(),
        )
    });
    if response.clicked() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(state_id, !expanded));
    }
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
pub(crate) fn ranked_bar(
    ui: &mut egui::Ui,
    label: &str,
    value: f64,
    maximum: f64,
    visible_value: &str,
) -> Response {
    ranked_bar_with_label(
        ui,
        &format!("{label}: {visible_value}"),
        value,
        maximum,
        visible_value,
        |ui| {
            ui.label(label);
        },
    )
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
pub(crate) fn ranked_bar_with_label(
    ui: &mut egui::Ui,
    accessible_label: &str,
    value: f64,
    maximum: f64,
    visible_value: &str,
    add_label: impl FnOnce(&mut egui::Ui),
) -> Response {
    let ratio = ranked_bar_ratio(value, maximum);
    let palette = theme::palette(ui.ctx().theme());
    let response = ui
        .vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                add_label(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(visible_value);
                });
            });
            let (bar_rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.0), egui::Sense::hover());
            ui.painter().rect_filled(
                bar_rect,
                egui::CornerRadius::same(3),
                palette.surface_subtle,
            );
            ui.painter().rect_stroke(
                bar_rect,
                egui::CornerRadius::same(3),
                egui::Stroke::new(1.0, palette.border),
                egui::StrokeKind::Inside,
            );
            if let Some(fill_rect) = ranked_bar_fill_rect(bar_rect, ratio) {
                ui.painter()
                    .rect_filled(fill_rect, egui::CornerRadius::same(3), palette.accent);
            }
        })
        .response;
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::ProgressIndicator,
            ui.is_enabled(),
            accessible_label,
        )
    });
    response
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
pub(super) fn ranked_bar_ratio(value: f64, maximum: f64) -> f32 {
    if !value.is_finite() || !maximum.is_finite() || value <= 0.0 || maximum <= 0.0 {
        return 0.0;
    }
    (value / maximum).clamp(0.0, 1.0) as f32
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
fn ranked_bar_fill_rect(bar_rect: egui::Rect, ratio: f32) -> Option<egui::Rect> {
    (ratio > 0.0).then(|| {
        egui::Rect::from_min_size(
            bar_rect.min,
            egui::vec2(bar_rect.width() * ratio, bar_rect.height()),
        )
    })
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
pub(super) fn empty_state(ui: &mut egui::Ui, icon: &str, heading: &str, detail: &str) -> Response {
    card(ui, |ui| inline_empty_state(ui, icon, heading, detail)).inner
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
pub(crate) fn inline_empty_state(
    ui: &mut egui::Ui,
    icon: &str,
    heading: &str,
    detail: &str,
) -> Response {
    let response = ui
        .vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(icon)
                    .font(theme::icon_font_for_ui(ui, 22.0))
                    .weak(),
            );
            ui.label(egui::RichText::new(heading).font(theme::semibold_font_for_ui(ui, 14.0)));
            ui.weak(detail);
        })
        .response;
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::Other,
            ui.is_enabled(),
            format!("{heading}. {detail}"),
        )
    });
    response
}

pub(super) fn contextual_banner(
    ui: &mut egui::Ui,
    severity: BannerSeverity,
    title: &str,
    detail: &str,
) -> Response {
    let palette = theme::palette(ui.ctx().theme());
    let (icon, color, severity_label) = match severity {
        BannerSeverity::Info => (egui_phosphor::regular::INFO, palette.info, "Information"),
        BannerSeverity::Warning => (egui_phosphor::regular::WARNING, palette.warning, "Warning"),
        BannerSeverity::Error => (
            egui_phosphor::regular::WARNING_CIRCLE,
            palette.error,
            "Error",
        ),
    };
    let response = egui::Frame::new()
        .fill(palette.surface_subtle)
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(
            theme::SPACE_MD as i8,
            theme::SPACE_SM as i8,
        ))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(icon)
                        .font(theme::icon_font_for_ui(ui, 18.0))
                        .color(color),
                );
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(title).font(theme::semibold_font_for_ui(ui, 14.0)),
                    );
                    ui.label(detail);
                });
            });
        })
        .response;
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::Other,
            ui.is_enabled(),
            format!("{severity_label}: {title}. {detail}"),
        )
    });
    response
}

pub(crate) fn dismissible_warning_banner(
    ui: &mut egui::Ui,
    title: &str,
    detail: &str,
    details: &[String],
) -> bool {
    let palette = theme::palette(ui.ctx().theme());
    let mut dismissed = false;
    let response = egui::Frame::new()
        .fill(palette.surface_subtle)
        .stroke(egui::Stroke::new(1.0, palette.warning))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(
            theme::SPACE_MD as i8,
            theme::SPACE_SM as i8,
        ))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.label(
                    egui::RichText::new(egui_phosphor::regular::WARNING)
                        .font(theme::icon_font_for_ui(ui, 18.0))
                        .color(palette.warning),
                );
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(title).font(theme::semibold_font_for_ui(ui, 14.0)),
                    );
                    ui.label(detail);
                    if !details.is_empty() {
                        egui::CollapsingHeader::new("Affected analytics")
                            .id_salt("recovery-details")
                            .show(ui, |ui| {
                                for item in details {
                                    ui.add(egui::Label::new(item).selectable(true));
                                }
                            });
                    }
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Dismiss").clicked() {
                    dismissed = true;
                }
            });
        })
        .response;
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::Other,
            ui.is_enabled(),
            format!("Warning: {title}. {detail} {}", details.join(" ")),
        )
    });
    dismissed
}

pub(super) fn loading_state(ui: &mut egui::Ui, heading: &str, detail: &str) -> Response {
    let response = card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(heading).font(theme::semibold_font_for_ui(ui, 14.0)));
                ui.weak(detail);
            });
        });
    })
    .response;
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::ProgressIndicator,
            ui.is_enabled(),
            format!("{heading}. {detail}"),
        )
    });
    response
}

pub(super) fn accessible_icon_button_enabled(
    ui: &mut egui::Ui,
    enabled: bool,
    icon: &str,
    accessible_label: &str,
) -> Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(icon).font(theme::icon_font_for_ui(ui, 17.0))),
    );
    response
        .widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), accessible_label));

    tooltip_on_hover_or_focus(&response, |ui| {
        ui.label(accessible_label);
    });
    response
}

pub(crate) fn tooltip_on_hover_or_focus(
    response: &Response,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let mut tooltip = egui::Tooltip::for_widget(response);
    tooltip.popup = tooltip
        .popup
        .open(response.hovered() || response.has_focus());
    tooltip.show(add_contents);
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
pub(crate) fn summary_value(
    ui: &mut egui::Ui,
    label: &str,
    visible_value: &str,
    accessible_value: &str,
    detail: &str,
) -> Response {
    ui.label(egui::RichText::new(label).font(theme::medium_font_for_ui(ui, 13.0)));
    summary_value_body(ui, label, visible_value, accessible_value, detail)
}

pub(crate) fn metric_summary_value(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    visible_value: &str,
    accessible_value: &str,
    detail: &str,
) -> Response {
    card_header(ui, icon, label);
    vertical_gap(ui, theme::SPACE_MD);
    summary_value_body(ui, label, visible_value, accessible_value, detail)
}

fn summary_value_body(
    ui: &mut egui::Ui,
    label: &str,
    visible_value: &str,
    accessible_value: &str,
    detail: &str,
) -> Response {
    let response = ui.add(
        egui::Label::new(
            egui::RichText::new(visible_value).font(theme::semibold_font_for_ui(ui, 26.0)),
        )
        .sense(egui::Sense::focusable_noninteractive()),
    );
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::Label,
            ui.is_enabled(),
            format!("{label}: {accessible_value}"),
        )
    });
    if visible_value != accessible_value {
        tooltip_on_hover_or_focus(&response, |ui| {
            ui.label(format!("Exact value: {accessible_value}"));
        });
    }
    ui.add(egui::Label::new(egui::RichText::new(detail).weak()).wrap());
    response
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
pub(crate) fn format_exact_count(value: u64) -> String {
    format_exact_count_u128(u128::from(value))
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
pub(crate) fn format_exact_count_u128(value: u128) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    let first_group = digits.len() % 3;
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && index % 3 == first_group {
            grouped.push(',');
        }
        grouped.push(character);
    }
    grouped
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
pub(crate) fn format_compact_count(value: u64) -> String {
    const SUFFIXES: [&str; 5] = ["k", "M", "B", "T", "Q"];

    if value < 1_000 {
        return value.to_string();
    }

    let mut compact = value as f64 / 1_000.0;
    let mut suffix = 0;
    while compact >= 999.95 && suffix + 1 < SUFFIXES.len() {
        compact /= 1_000.0;
        suffix += 1;
    }
    format!("{compact:.1}{}", SUFFIXES[suffix])
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
pub(crate) fn format_duration_ms(milliseconds: f64) -> String {
    if !milliseconds.is_finite() || milliseconds < 0.0 {
        return "—".to_owned();
    }
    if milliseconds == 0.0 {
        return "0.0 ms".to_owned();
    }
    if milliseconds >= 1.0 {
        format!("{milliseconds:.1} ms")
    } else if milliseconds >= 0.1 {
        format!("{milliseconds:.2} ms")
    } else if milliseconds >= 0.01 {
        format!("{milliseconds:.3} ms")
    } else {
        let value = format!("{milliseconds:.6}");
        format!("{} ms", value.trim_end_matches('0').trim_end_matches('.'))
    }
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
pub(super) fn format_metric_duration(duration: Duration) -> String {
    format_duration_ms(duration.as_secs_f64() * 1_000.0)
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
pub(crate) fn describe_text_token(value: &str, context: TextTokenContext) -> TextTokenDescription {
    let escaped = format!(
        "\"{}\"",
        value
            .chars()
            .flat_map(char::escape_default)
            .collect::<String>()
    );
    let code_points = if value.is_empty() {
        "(empty)".to_owned()
    } else {
        value
            .chars()
            .map(|character| format!("U+{:04X}", u32::from(character)))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let unicode_names = if value.is_empty() {
        "(none)".to_owned()
    } else {
        value
            .chars()
            .map(|character| {
                unicode_names2::name(character)
                    .map(|name| name.to_string())
                    .unwrap_or_else(|| "(name unavailable)".to_owned())
            })
            .collect::<Vec<_>>()
            .join(" + ")
    };
    let special_label = whitespace_label(value);
    let printable = !value.is_empty()
        && value.chars().all(|character| {
            !character.is_control() && !character.is_whitespace() && character != '\u{200B}'
        });
    let visible = special_label.map_or_else(
        || {
            if printable {
                value.to_owned()
            } else {
                code_points.clone()
            }
        },
        str::to_owned,
    );
    let context_label = match context {
        TextTokenContext::ProducedText => "Produced text",
        TextTokenContext::PhysicalKey => "Physical key identity",
    };
    let readable = special_label.unwrap_or(&visible);
    let accessible_label = if printable && unicode_names != "(none)" {
        format!("{context_label}: {readable}. {unicode_names}")
    } else {
        format!("{context_label}: {readable}")
    };

    TextTokenDescription {
        visible,
        escaped,
        code_points,
        unicode_names,
        accessible_label,
        context_label,
        printable,
    }
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
pub(crate) fn text_token(ui: &mut egui::Ui, value: &str, context: TextTokenContext) -> Response {
    text_token_with_key_code(ui, value, context, None)
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
pub(crate) fn physical_key_token(ui: &mut egui::Ui, label: &str, linux_key_code: u16) -> Response {
    text_token_with_key_code(
        ui,
        label,
        TextTokenContext::PhysicalKey,
        Some(linux_key_code),
    )
}

fn text_token_with_key_code(
    ui: &mut egui::Ui,
    value: &str,
    context: TextTokenContext,
    linux_key_code: Option<u16>,
) -> Response {
    let description = describe_text_token(value, context);
    let font_id = token_font_id(ui);
    let glyphs_available =
        token_glyphs_available(ui, value, context, description.printable, &font_id);
    let rendered = rendered_text_token(&description, glyphs_available);
    let accessible_label = linux_key_code.map_or_else(
        || description.accessible_label.clone(),
        |code| format!("{}. Linux key code {code}", description.accessible_label),
    );
    let response = ui.add(
        egui::Label::new(
            egui::RichText::new(rendered)
                .font(font_id)
                .background_color(ui.visuals().code_bg_color),
        )
        .sense(egui::Sense::focusable_noninteractive()),
    );
    response
        .widget_info(|| WidgetInfo::labeled(WidgetType::Label, ui.is_enabled(), &accessible_label));

    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            egui::CornerRadius::same(4),
            egui::Stroke::new(1.5, theme::palette(ui.ctx().theme()).accent),
            egui::StrokeKind::Outside,
        );
    }

    tooltip_on_hover_or_focus(&response, |ui| {
        ui.label(description.context_label);
        if let Some(code) = linux_key_code {
            ui.label(format!("Linux key code: {code}"));
        }
        ui.label(format!("Escaped: {}", description.escaped));
        ui.label(format!("Unicode: {}", description.code_points));
        ui.label(format!("Names: {}", description.unicode_names));
    });
    response
}

fn token_font_id(ui: &egui::Ui) -> egui::FontId {
    let body = egui::TextStyle::Body.resolve(ui.style());
    egui::FontId::new(body.size, egui::FontFamily::Proportional)
}

fn token_glyphs_available(
    ui: &egui::Ui,
    value: &str,
    context: TextTokenContext,
    printable: bool,
    font_id: &egui::FontId,
) -> bool {
    if context == TextTokenContext::PhysicalKey {
        return true;
    }
    printable && ui.ctx().fonts_mut(|fonts| fonts.has_glyphs(font_id, value))
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
pub(super) fn rendered_text_token(
    description: &TextTokenDescription,
    glyphs_available: bool,
) -> String {
    if description.printable && !glyphs_available {
        description.code_points.clone()
    } else {
        description.visible.clone()
    }
}

#[allow(dead_code, reason = "consumed incrementally by the redesign views")]
fn whitespace_label(value: &str) -> Option<&'static str> {
    match value {
        "" => Some("Empty text"),
        " " => Some("Space"),
        "\t" => Some("Tab"),
        "\n" => Some("Newline"),
        "\r" => Some("Carriage return"),
        "\r\n" => Some("Carriage return + Newline"),
        "\u{00A0}" => Some("Non-breaking space"),
        "\u{200B}" => Some("Zero-width space"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::{
        kittest::{NodeT, Queryable},
        Harness,
    };

    use super::*;

    #[test]
    fn count_formatting_has_stable_thresholds_and_grouping() {
        assert_eq!(format_exact_count(0), "0");
        assert_eq!(format_exact_count(999), "999");
        assert_eq!(format_exact_count(1_000), "1,000");
        assert_eq!(format_exact_count(12_345_678), "12,345,678");
        assert_eq!(
            format_exact_count_u128(u128::from(u64::MAX) + 1),
            "18,446,744,073,709,551,616"
        );

        assert_eq!(format_compact_count(999), "999");
        assert_eq!(format_compact_count(1_000), "1.0k");
        assert_eq!(format_compact_count(12_400), "12.4k");
        assert_eq!(format_compact_count(999_949), "999.9k");
        assert_eq!(format_compact_count(999_950), "1.0M");
        assert_eq!(format_compact_count(1_000_000), "1.0M");
        assert_eq!(format_compact_count(1_000_000_000), "1.0B");
    }

    #[test]
    fn duration_formatting_preserves_small_values() {
        assert_eq!(format_duration_ms(93.44), "93.4 ms");
        assert_eq!(format_duration_ms(0.125), "0.12 ms");
        assert_eq!(format_duration_ms(0.0125), "0.013 ms");
        assert_eq!(format_duration_ms(0.000_5), "0.0005 ms");
        assert_eq!(
            format_metric_duration(Duration::from_nanos(1)),
            "0.000001 ms"
        );
        assert_eq!(format_duration_ms(f64::NAN), "—");
    }

    #[test]
    fn ranked_bar_ratios_are_clamped_and_zero_has_no_fill() {
        assert_eq!(ranked_bar_ratio(5.0, 10.0), 0.5);
        assert_eq!(ranked_bar_ratio(12.0, 10.0), 1.0);
        assert_eq!(ranked_bar_ratio(-1.0, 10.0), 0.0);
        assert_eq!(ranked_bar_ratio(1.0, 0.0), 0.0);
        assert_eq!(ranked_bar_ratio(f64::NAN, 10.0), 0.0);

        let bar_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 6.0));
        assert!(ranked_bar_fill_rect(bar_rect, 0.0).is_none());
        assert_eq!(ranked_bar_fill_rect(bar_rect, 0.5).unwrap().width(), 50.0);
    }

    #[test]
    fn text_descriptions_cover_whitespace_unicode_and_sequences() {
        let tab = describe_text_token("\t", TextTokenContext::ProducedText);
        assert_eq!(tab.visible, "Tab");
        assert_eq!(tab.escaped, "\"\\t\"");
        assert_eq!(tab.code_points, "U+0009");
        assert!(tab.accessible_label.contains("Produced text: Tab"));

        let nbsp = describe_text_token("\u{00A0}", TextTokenContext::ProducedText);
        assert_eq!(nbsp.visible, "Non-breaking space");
        assert_eq!(nbsp.code_points, "U+00A0");
        assert_eq!(nbsp.unicode_names, "NO-BREAK SPACE");

        let sequence = describe_text_token("e\u{301}", TextTokenContext::PhysicalKey);
        assert_eq!(sequence.visible, "e\u{301}");
        assert_eq!(sequence.code_points, "U+0065 U+0301");
        assert_eq!(
            sequence.unicode_names,
            "LATIN SMALL LETTER E + COMBINING ACUTE ACCENT"
        );
        assert!(sequence
            .accessible_label
            .starts_with("Physical key identity:"));
    }

    #[test]
    fn unsupported_printable_glyphs_use_code_point_fallback() {
        let description = describe_text_token("🜁", TextTokenContext::ProducedText);
        assert_eq!(rendered_text_token(&description, true), "🜁");
        assert_eq!(rendered_text_token(&description, false), "U+1F701");

        let whitespace = describe_text_token(" ", TextTokenContext::ProducedText);
        assert_eq!(rendered_text_token(&whitespace, false), "Space");
    }

    #[test]
    fn active_token_font_stack_renders_glyphs_or_deterministic_fallbacks() {
        let mut installed = false;
        let mut harness = Harness::new_ui(move |ui| {
            if !installed {
                theme::install(ui.ctx(), crate::settings::AppearancePreference::Light);
                installed = true;
            }
            text_token(ui, "e", TextTokenContext::ProducedText);
            text_token(ui, "🜁", TextTokenContext::ProducedText);
            physical_key_token(ui, "KEY_A", 30);
        });
        harness.run_steps(2);

        let rendered_text = |node: egui_kittest::Node<'_>| {
            let accessible = node.accesskit_node();
            accessible
                .children()
                .filter_map(|child| child.value())
                .collect::<String>()
        };
        assert_eq!(
            rendered_text(harness.get_by_label_contains("Produced text: e.")),
            "e"
        );
        assert_eq!(
            rendered_text(harness.get_by_label_contains("Produced text: 🜁.")),
            "U+1F701"
        );
        assert_eq!(
            rendered_text(harness.get_by_label_contains("Linux key code 30")),
            "KEY_A"
        );
    }

    #[test]
    fn custom_controls_expose_accessible_names_and_focused_token_details() {
        let mut harness = Harness::new_ui(|ui| {
            accessible_icon_button_enabled(
                ui,
                true,
                egui_phosphor::regular::ARROWS_CLOCKWISE,
                "Rescan keyboards",
            );
            ranked_bar(ui, "Space", 42.0, 100.0, "42 presses");
            text_token(ui, "\t", TextTokenContext::ProducedText);
            physical_key_token(ui, "KEY_A", 30);
        });

        let button =
            harness.get_by_role_and_label(egui::accesskit::Role::Button, "Rescan keyboards");
        button.focus();
        harness.run();
        assert!(harness
            .query_by_role_and_label(egui::accesskit::Role::Label, "Rescan keyboards")
            .is_some());
        assert!(harness.query_by_label("Space: 42 presses").is_some());
        let token = harness.get_by_label_contains("Produced text: Tab");
        token.focus();
        harness.run();
        assert!(harness.query_by_label("Escaped: \"\\t\"").is_some());

        let physical = harness.get_by_label_contains("Linux key code 30");
        physical.focus();
        harness.run();
        assert!(harness.query_by_label("Linux key code: 30").is_some());
    }
}
