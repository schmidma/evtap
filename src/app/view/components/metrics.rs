use eframe::egui::{self, Response, WidgetInfo, WidgetType};

use super::{
    super::theme,
    core::{card_header, tooltip_on_hover_or_focus, vertical_gap},
};

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

fn ranked_bar_ratio(value: f64, maximum: f64) -> f32 {
    if !value.is_finite() || !maximum.is_finite() || value <= 0.0 || maximum <= 0.0 {
        return 0.0;
    }
    (value / maximum).clamp(0.0, 1.0) as f32
}

fn ranked_bar_fill_rect(bar_rect: egui::Rect, ratio: f32) -> Option<egui::Rect> {
    (ratio > 0.0).then(|| {
        egui::Rect::from_min_size(
            bar_rect.min,
            egui::vec2(bar_rect.width() * ratio, bar_rect.height()),
        )
    })
}

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

pub(crate) fn format_exact_count(value: u64) -> String {
    format_exact_count_u128(u128::from(value))
}

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

#[cfg(test)]
mod tests {
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
        assert_eq!(format_duration_ms(0.000_001), "0.000001 ms");
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
}
