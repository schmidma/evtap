use eframe::egui::{self, Response, WidgetInfo, WidgetType};

use super::{super::theme, core::card};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app::view) enum BannerSeverity {
    Info,
    Warning,
    Error,
}

pub(in crate::app::view) fn empty_state(
    ui: &mut egui::Ui,
    icon: &str,
    heading: &str,
    detail: &str,
) -> Response {
    card(ui, |ui| inline_empty_state(ui, icon, heading, detail)).inner
}

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

pub(in crate::app::view) fn contextual_banner(
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

pub(in crate::app::view) fn loading_state(
    ui: &mut egui::Ui,
    heading: &str,
    detail: &str,
) -> Response {
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
