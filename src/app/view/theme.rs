use std::sync::Arc;

#[cfg(test)]
use eframe::egui::epaint::text::Tag;
use eframe::egui::{
    self, epaint::text::VariationCoords, Color32, CornerRadius, FontData, FontFamily, FontId,
    FontTweak, Stroke, TextStyle, Theme,
};
use egui_phosphor::Variant;

use crate::settings::AppearancePreference;

pub(super) const SPACE_XS: f32 = 4.0;
pub(super) const SPACE_SM: f32 = 8.0;
pub(super) const SPACE_MD: f32 = 12.0;
pub(super) const SPACE_LG: f32 = 16.0;
pub(super) const SPACE_XL: f32 = 24.0;
pub(super) const CARD_PADDING: i8 = 16;
pub(super) const PAGE_PADDING: i8 = 20;

pub(crate) const HACK_FONT_NAME: &str = "Hack";
const INTER_REGULAR_FONT_NAME: &str = "Inter Variable 400";
const INTER_MEDIUM_FONT_NAME: &str = "Inter Variable 500";
const INTER_SEMIBOLD_FONT_NAME: &str = "Inter Variable 600";
// Inter 4.1, SIL OFL 1.1. SHA-256:
// 4989b125924991b90d05b2d16e0e388c48f7d5bb8b30539bbf9c755278d0ccaf
const INTER_VARIABLE_BYTES: &[u8] = include_bytes!("../../../assets/fonts/InterVariable.ttf");
const PHOSPHOR_FONT_NAME: &str = "phosphor";

#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub background: Color32,
    pub surface: Color32,
    pub surface_subtle: Color32,
    pub border: Color32,
    pub text: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub recording: Color32,
    pub success: Color32,
    pub info: Color32,
    pub warning: Color32,
    pub error: Color32,
}

pub(crate) fn install(ctx: &egui::Context, preference: AppearancePreference) {
    ctx.set_fonts(font_definitions());
    ctx.style_mut_of(Theme::Light, |style| configure_style(style, Theme::Light));
    ctx.style_mut_of(Theme::Dark, |style| configure_style(style, Theme::Dark));
    ctx.set_theme(theme_preference(preference));
}

pub(crate) fn font_definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, Variant::Regular);

    let mut proportional_fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    proportional_fallbacks.retain(|font| font != PHOSPHOR_FONT_NAME);
    if !proportional_fallbacks
        .iter()
        .any(|font| font == HACK_FONT_NAME)
    {
        proportional_fallbacks.push(HACK_FONT_NAME.to_owned());
    }

    for (font_name, weight) in [
        (INTER_REGULAR_FONT_NAME, 400.0),
        (INTER_MEDIUM_FONT_NAME, 500.0),
        (INTER_SEMIBOLD_FONT_NAME, 600.0),
    ] {
        fonts.font_data.insert(
            font_name.to_owned(),
            Arc::new(
                FontData::from_static(INTER_VARIABLE_BYTES).tweak(FontTweak {
                    coords: VariationCoords::new([(b"wght", weight)]),
                    ..Default::default()
                }),
            ),
        );

        let mut family = vec![font_name.to_owned()];
        family.extend(proportional_fallbacks.iter().cloned());
        fonts.families.insert(named_family(font_name), family);
    }

    fonts.families.insert(
        named_family(PHOSPHOR_FONT_NAME),
        vec![PHOSPHOR_FONT_NAME.to_owned()],
    );

    let mut proportional = vec![INTER_REGULAR_FONT_NAME.to_owned()];
    proportional.extend(proportional_fallbacks);
    fonts
        .families
        .insert(FontFamily::Proportional, proportional);
    fonts
}

fn named_family(name: &'static str) -> FontFamily {
    FontFamily::Name(Arc::from(name))
}

pub(super) fn regular_font(size: f32) -> FontId {
    FontId::new(size, named_family(INTER_REGULAR_FONT_NAME))
}

pub(super) fn medium_font(size: f32) -> FontId {
    FontId::new(size, named_family(INTER_MEDIUM_FONT_NAME))
}

pub(super) fn semibold_font(size: f32) -> FontId {
    FontId::new(size, named_family(INTER_SEMIBOLD_FONT_NAME))
}

pub(super) fn icon_font(size: f32) -> FontId {
    FontId::new(size, named_family(PHOSPHOR_FONT_NAME))
}

pub(super) fn icon_font_for_ui(ui: &egui::Ui, size: f32) -> FontId {
    if inter_is_installed(ui) {
        icon_font(size)
    } else {
        FontId::proportional(size)
    }
}

pub(super) fn medium_font_for_ui(ui: &egui::Ui, size: f32) -> FontId {
    role_font_for_ui(ui, size, INTER_MEDIUM_FONT_NAME)
}

pub(super) fn semibold_font_for_ui(ui: &egui::Ui, size: f32) -> FontId {
    role_font_for_ui(ui, size, INTER_SEMIBOLD_FONT_NAME)
}

fn role_font_for_ui(ui: &egui::Ui, size: f32, role: &'static str) -> FontId {
    if inter_is_installed(ui) {
        FontId::new(size, named_family(role))
    } else {
        FontId::proportional(size)
    }
}

fn inter_is_installed(ui: &egui::Ui) -> bool {
    ui.style()
        .text_styles
        .get(&TextStyle::Body)
        .is_some_and(|font| font.family == named_family(INTER_REGULAR_FONT_NAME))
}

pub(super) fn theme_preference(preference: AppearancePreference) -> egui::ThemePreference {
    match preference {
        AppearancePreference::System => egui::ThemePreference::System,
        AppearancePreference::Light => egui::ThemePreference::Light,
        AppearancePreference::Dark => egui::ThemePreference::Dark,
    }
}

pub(super) fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Light => Palette {
            background: Color32::from_rgb(247, 248, 250),
            surface: Color32::WHITE,
            surface_subtle: Color32::from_rgb(242, 244, 248),
            border: Color32::from_rgb(229, 231, 235),
            text: Color32::from_rgb(31, 41, 55),
            accent: Color32::from_rgb(79, 70, 229),
            accent_hover: Color32::from_rgb(67, 56, 202),
            recording: Color32::from_rgb(224, 93, 93),
            success: Color32::from_rgb(22, 163, 74),
            info: Color32::from_rgb(79, 70, 229),
            warning: Color32::from_rgb(180, 101, 10),
            error: Color32::from_rgb(190, 45, 55),
        },
        Theme::Dark => Palette {
            background: Color32::from_rgb(15, 23, 32),
            surface: Color32::from_rgb(30, 41, 53),
            surface_subtle: Color32::from_rgb(37, 49, 63),
            border: Color32::from_rgb(57, 69, 82),
            text: Color32::from_rgb(235, 240, 246),
            accent: Color32::from_rgb(129, 119, 255),
            accent_hover: Color32::from_rgb(151, 143, 255),
            recording: Color32::from_rgb(240, 116, 116),
            success: Color32::from_rgb(74, 222, 128),
            info: Color32::from_rgb(129, 119, 255),
            warning: Color32::from_rgb(224, 159, 54),
            error: Color32::from_rgb(232, 92, 101),
        },
    }
}

fn configure_style(style: &mut egui::Style, theme: Theme) {
    let palette = palette(theme);
    style.spacing.item_spacing = egui::vec2(SPACE_SM, SPACE_SM);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 32.0;
    style.spacing.window_margin = egui::Margin::same(CARD_PADDING);
    style.spacing.menu_margin = egui::Margin::same(SPACE_SM as i8);
    style
        .text_styles
        .insert(TextStyle::Heading, semibold_font(24.0));
    style
        .text_styles
        .insert(TextStyle::Body, regular_font(14.0));
    style
        .text_styles
        .insert(TextStyle::Button, medium_font(14.0));
    style
        .text_styles
        .insert(TextStyle::Small, regular_font(12.0));
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(13.0, FontFamily::Monospace),
    );

    let visuals = &mut style.visuals;
    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.surface;
    visuals.window_stroke = Stroke::new(1.0, palette.border);
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.extreme_bg_color = palette.surface_subtle;
    visuals.code_bg_color = palette.surface_subtle;
    visuals.faint_bg_color = palette.surface_subtle;
    visuals.hyperlink_color = palette.accent;
    visuals.warn_fg_color = palette.warning;
    visuals.error_fg_color = palette.error;
    visuals.selection.bg_fill = palette.accent.gamma_multiply(0.30);
    visuals.selection.stroke = Stroke::new(1.5, palette.accent);
    visuals.weak_text_alpha = 0.78;

    visuals.widgets.noninteractive.bg_fill = palette.surface;
    visuals.widgets.noninteractive.weak_bg_fill = palette.surface_subtle;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(8);
    visuals.widgets.inactive.weak_bg_fill = palette.surface;
    visuals.widgets.inactive.bg_fill = palette.surface;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, palette.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(6);
    visuals.widgets.hovered.weak_bg_fill = palette.surface_subtle;
    visuals.widgets.hovered.bg_fill = palette.surface_subtle;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, palette.accent_hover);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(6);
    visuals.widgets.active.weak_bg_fill = palette.accent.gamma_multiply(0.20);
    visuals.widgets.active.bg_fill = palette.accent.gamma_multiply(0.20);
    visuals.widgets.active.bg_stroke = Stroke::new(1.5, palette.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.active.corner_radius = CornerRadius::same(6);
    visuals.widgets.open = visuals.widgets.hovered;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_stack_uses_fixed_inter_roles_and_preserves_fallbacks() {
        let fonts = font_definitions();
        assert!(fonts.font_data.contains_key(HACK_FONT_NAME));
        assert!(fonts.font_data.contains_key(PHOSPHOR_FONT_NAME));
        assert_eq!(
            fonts
                .font_data
                .keys()
                .filter(|name| name.starts_with("phosphor"))
                .count(),
            1
        );
        let proportional = fonts.families.get(&FontFamily::Proportional).unwrap();
        assert_eq!(
            proportional.first().map(String::as_str),
            Some(INTER_REGULAR_FONT_NAME)
        );
        assert!(!proportional.iter().any(|font| font == PHOSPHOR_FONT_NAME));
        assert_eq!(
            fonts.families.get(&named_family(PHOSPHOR_FONT_NAME)),
            Some(&vec![PHOSPHOR_FONT_NAME.to_owned()])
        );

        for (font_name, weight) in [
            (INTER_REGULAR_FONT_NAME, 400.0),
            (INTER_MEDIUM_FONT_NAME, 500.0),
            (INTER_SEMIBOLD_FONT_NAME, 600.0),
        ] {
            let data = fonts.font_data.get(font_name).unwrap();
            assert_eq!(data.tweak.coords.as_ref(), &[(Tag::new(b"wght"), weight)]);
            let family = fonts.families.get(&named_family(font_name)).unwrap();
            assert_eq!(family.first().map(String::as_str), Some(font_name));
            assert!(!family.iter().any(|font| font == PHOSPHOR_FONT_NAME));
            assert!(family.iter().any(|fallback| fallback == HACK_FONT_NAME));
        }

        let axes = fonts
            .font_data
            .get(INTER_REGULAR_FONT_NAME)
            .unwrap()
            .variation_axes();
        let weight = axes
            .iter()
            .find(|axis| axis.tag == Tag::new(b"wght"))
            .unwrap();
        assert_eq!(weight.range.min, 100.0);
        assert_eq!(weight.default, 400.0);
        assert_eq!(weight.range.max, 900.0);
    }

    #[test]
    fn preferences_map_to_egui_theme_preferences() {
        assert_eq!(
            theme_preference(AppearancePreference::System),
            egui::ThemePreference::System
        );
        assert_eq!(
            theme_preference(AppearancePreference::Light),
            egui::ThemePreference::Light
        );
        assert_eq!(
            theme_preference(AppearancePreference::Dark),
            egui::ThemePreference::Dark
        );
    }
}
