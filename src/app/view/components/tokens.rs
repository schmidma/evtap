use eframe::egui::{self, Response, WidgetInfo, WidgetType};

use super::{super::theme, core::tooltip_on_hover_or_focus};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TextTokenContext {
    ProducedText,
    PhysicalKey,
}

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

pub(crate) fn text_token(ui: &mut egui::Ui, value: &str, context: TextTokenContext) -> Response {
    text_token_with_key_code(ui, value, context, None)
}

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

fn rendered_text_token(description: &TextTokenDescription, glyphs_available: bool) -> String {
    if description.printable && !glyphs_available {
        description.code_points.clone()
    } else {
        description.visible.clone()
    }
}

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
        Harness,
        kittest::{NodeT, Queryable},
    };

    use super::*;
    use crate::app::view::theme;

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
        assert!(
            sequence
                .accessible_label
                .starts_with("Physical key identity:")
        );
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
}
