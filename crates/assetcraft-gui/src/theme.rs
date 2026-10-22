//! The dark visual identity, sampled from the Quixel Bridge reference
//! (docs/reference/quixel-bridge/README.md, "Design tokens").

use egui::{Color32, Style, Visuals};

/// App and grid background.
pub const BG: Color32 = Color32::from_rgb(37, 37, 37);
/// Side panels and insets.
pub const PANEL: Color32 = Color32::from_rgb(23, 23, 23);
/// Search field, bars, cards, dialogs.
pub const ELEVATED: Color32 = Color32::from_rgb(46, 46, 46);
/// Card separators and input outlines.
pub const BORDER: Color32 = Color32::from_rgb(58, 58, 58);
/// Titles.
pub const FG: Color32 = Color32::from_rgb(230, 230, 230);
/// Body text and nav labels.
pub const FG_SECONDARY: Color32 = Color32::from_rgb(182, 180, 176);
/// Counts and secondary metadata.
pub const FG_MUTED: Color32 = Color32::from_rgb(110, 110, 110);
/// Download, progress, FREE badge.
pub const GREEN: Color32 = Color32::from_rgb(145, 183, 89);
/// Primary buttons and links.
pub const BLUE: Color32 = Color32::from_rgb(79, 164, 227);
/// The `NEW` pill and wizard highlight.
pub const BLUE_SOFT: Color32 = Color32::from_rgb(116, 164, 192);

/// Install the AssetCraft dark theme on a context.
pub fn apply(ctx: &egui::Context) {
    let mut style = Style {
        visuals: Visuals::dark(),
        ..Style::default()
    };

    style.visuals.panel_fill = PANEL;
    style.visuals.window_fill = ELEVATED;
    style.visuals.extreme_bg_color = Color32::from_rgb(17, 17, 17);
    style.visuals.override_text_color = Some(FG_SECONDARY);
    style.visuals.selection.bg_fill = GREEN.gamma_multiply(0.35);
    style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, GREEN);
    style.visuals.hyperlink_color = BLUE;

    let widgets = &mut style.visuals.widgets;
    widgets.noninteractive.bg_fill = PANEL;
    widgets.noninteractive.fg_stroke.color = FG_SECONDARY;
    widgets.inactive.bg_fill = ELEVATED;
    widgets.inactive.fg_stroke.color = FG_SECONDARY;
    widgets.hovered.bg_fill = Color32::from_rgb(60, 60, 60);
    widgets.hovered.fg_stroke.color = FG;
    widgets.hovered.weak_bg_fill = Color32::from_rgb(60, 60, 60);
    widgets.active.bg_fill = Color32::from_rgb(30, 30, 30);
    widgets.open.bg_fill = PANEL;

    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);

    ctx.set_style(style);
}

/// A saturated, preview-ish colour from a hue in degrees.
pub fn hue_color(hue: f32) -> Color32 {
    let c = (1.0_f32 - (2.0_f32 * 0.6 - 1.0).abs()) * 0.85;
    let x = c * (1.0_f32 - (((hue / 60.0).floor() % 2.0) - 1.0).abs());
    let m = 0.6 - c / 2.0;
    let (r, g, b) = match (hue / 60.0) as usize % 6 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    Color32::from_rgb(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}
