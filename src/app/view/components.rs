mod core;
mod feedback;
mod metrics;
mod tokens;

pub(super) use core::{
    accessible_icon_button_enabled, card, destructive_button, horizontal_gap, modal, modal_actions,
    open_card, primary_button, vertical_gap,
};
pub(crate) use core::{card_header, disclosure_list, section_title, tooltip_on_hover_or_focus};
pub(super) use feedback::{BannerSeverity, contextual_banner, empty_state, loading_state};
pub(crate) use feedback::{dismissible_warning_banner, inline_empty_state};
pub(crate) use metrics::{
    format_compact_count, format_duration_ms, format_exact_count, format_exact_count_u128,
    metric_summary_value, ranked_bar_with_label, summary_value,
};
pub(crate) use tokens::{TextTokenContext, describe_text_token, physical_key_token, text_token};

#[cfg(test)]
mod tests {
    use eframe::egui;
    use egui_kittest::{Harness, kittest::Queryable};

    use super::*;

    #[test]
    fn custom_controls_expose_accessible_names_and_focused_token_details() {
        let mut harness = Harness::new_ui(|ui| {
            accessible_icon_button_enabled(
                ui,
                true,
                egui_phosphor::regular::ARROWS_CLOCKWISE,
                "Rescan keyboards",
            );
            ranked_bar_with_label(ui, "Space: 42 presses", 42.0, 100.0, "42 presses", |ui| {
                ui.label("Space");
            });
            text_token(ui, "\t", TextTokenContext::ProducedText);
            physical_key_token(ui, "KEY_A", 30);
        });

        let button =
            harness.get_by_role_and_label(egui::accesskit::Role::Button, "Rescan keyboards");
        button.focus();
        harness.run();
        assert!(
            harness
                .query_by_role_and_label(egui::accesskit::Role::Label, "Rescan keyboards")
                .is_some()
        );
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
