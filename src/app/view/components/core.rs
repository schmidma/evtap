use eframe::egui::{self, Response, WidgetInfo, WidgetType};

use super::super::theme;

pub(in crate::app::view) fn modal<R>(
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

pub(in crate::app::view) fn vertical_gap(ui: &mut egui::Ui, gap: f32) {
    let extra = (gap - ui.spacing().item_spacing.y).max(0.0);
    ui.add_space(extra);
}

pub(in crate::app::view) fn horizontal_gap(ui: &mut egui::Ui, gap: f32) {
    let extra = (gap - ui.spacing().item_spacing.x).max(0.0);
    ui.add_space(extra);
}

pub(in crate::app::view) fn modal_actions<R>(
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

pub(in crate::app::view) fn primary_button(ui: &mut egui::Ui, label: &str) -> Response {
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

pub(in crate::app::view) fn destructive_button(ui: &mut egui::Ui, label: &str) -> Response {
    let palette = theme::palette(ui.ctx().theme());
    let text = if ui.visuals().dark_mode {
        palette.background
    } else {
        egui::Color32::WHITE
    };
    ui.add(egui::Button::new(egui::RichText::new(label).color(text)).fill(palette.error))
}

pub(in crate::app::view) fn card<R>(
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

pub(in crate::app::view) fn open_card(
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

pub(in crate::app::view) fn accessible_icon_button_enabled(
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
