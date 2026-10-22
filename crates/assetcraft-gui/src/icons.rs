//! Small vector icons.
//!
//! egui's bundled fonts do not cover the arrows and ticks the Bridge reference
//! uses (they render as tofu boxes), so the shell draws them with the painter.

use egui::{Painter, Pos2, Stroke};

/// A tick mark centred on `center`, roughly `size` wide.
pub fn check(painter: &Painter, center: Pos2, size: f32, stroke: Stroke) {
    let p = |x: f32, y: f32| center + egui::vec2(x * size, y * size);
    painter.line_segment([p(-0.38, 0.02), p(-0.10, 0.30)], stroke);
    painter.line_segment([p(-0.10, 0.30), p(0.38, -0.30)], stroke);
}

/// A downward arrow, as on the hover download button.
pub fn download(painter: &Painter, center: Pos2, size: f32, stroke: Stroke) {
    let p = |x: f32, y: f32| center + egui::vec2(x * size, y * size);
    painter.line_segment([p(0.0, -0.45), p(0.0, 0.18)], stroke);
    painter.line_segment([p(-0.30, -0.10), p(0.0, 0.22)], stroke);
    painter.line_segment([p(0.30, -0.10), p(0.0, 0.22)], stroke);
}

/// The magnifier in the search field.
pub fn search(painter: &Painter, center: Pos2, size: f32, stroke: Stroke) {
    let c = center + egui::vec2(-0.08 * size, -0.08 * size);
    painter.circle_stroke(c, 0.28 * size, stroke);
    painter.line_segment(
        [
            c + egui::vec2(0.20 * size, 0.20 * size),
            c + egui::vec2(0.46 * size, 0.46 * size),
        ],
        stroke,
    );
}
