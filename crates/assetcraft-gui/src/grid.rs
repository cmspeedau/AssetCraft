//! The asset grid: cards with marketplace thumbnails (or a generated
//! placeholder), badges, hover actions, and the favourite heart.

use egui::{Align2, Color32, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};

use crate::app::App;
use crate::{icons, theme};

/// Card width; the title strip sits inside the card, as in Bridge.
pub const CARD_W: f32 = 220.0;
pub const GAP: f32 = 12.0;
/// Height of the title strip under the image.
const STRIP_H: f32 = 38.0;

pub fn show(ui: &mut Ui, ctx: &egui::Context, app: &mut App, hits: &[usize]) {
    let cols = ((ui.available_width() + GAP) / (CARD_W + GAP))
        .max(1.0)
        .floor() as usize;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for row in hits.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for &i in row {
                        card(ui, app, i);
                    }
                });
                ui.add_space(GAP - ui.spacing().item_spacing.y);
            }

            if app.can_load_more() {
                ui.add_space(6.0);
                ui.vertical_centered(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Load more").color(theme::FG_SECONDARY),
                            )
                            .fill(theme::ELEVATED)
                            .min_size(egui::vec2(160.0, 30.0)),
                        )
                        .clicked()
                    {
                        app.load_more(ctx);
                    }
                });
                ui.add_space(12.0);
            }
        });
}

fn card(ui: &mut Ui, app: &mut App, i: usize) {
    let (id, title, hue, free, rating, is_new, favorite) = {
        let card = &app.cards[i];
        (
            card.id.clone(),
            card.title.clone(),
            card.hue,
            card.free,
            card.rating,
            card.is_new,
            app.favorites.contains(&card.id),
        )
    };

    let (rect, response) = ui.allocate_exact_size(Vec2::splat(CARD_W), Sense::click());
    let image_area = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.max.y - STRIP_H));
    let hovered = response.hovered();
    let is_open = app.selected == Some(i);
    let is_checked = app.checked.contains(&id);
    let fill = if hovered || is_open {
        Color32::from_rgb(32, 32, 32)
    } else {
        Color32::from_rgb(26, 26, 26)
    };

    let painter = ui.painter().clone();
    painter.rect_filled(rect, 8.0, fill);
    if is_open || hovered {
        painter.rect_stroke(
            rect,
            8.0,
            if is_open {
                Stroke::new(1.5_f32, theme::BLUE)
            } else {
                Stroke::new(1.0_f32, Color32::from_rgb(110, 110, 110))
            },
            egui::StrokeKind::Inside,
        );
    }

    // Only ask for a thumbnail once the card is on screen and still missing.
    if ui.clip_rect().intersects(rect) {
        app.ensure_thumbnail(i);
    }

    let image = app.texture(&id);
    match image {
        Some(texture) => draw_thumbnail(&painter, image_area, texture),
        None => draw_placeholder(&painter, image_area, hue),
    }

    if free {
        badge(
            &painter,
            rect.min + Vec2::new(10.0, 10.0),
            "FREE",
            theme::GREEN,
        );
    }
    if is_new {
        let x = if free { 62.0 } else { 10.0 };
        badge(
            &painter,
            rect.min + Vec2::new(x, 10.0),
            "NEW",
            theme::BLUE_SOFT,
        );
    }

    if hovered && rating.is_some() && app.download_enabled() {
        // Rating yields the corner to the download button on hover.
    } else if let Some(rating) = rating {
        let anchor = Pos2::new(image_area.max.x - 10.0, image_area.min.y + 10.0);
        painter.text(
            anchor,
            Align2::RIGHT_TOP,
            format!("{rating:.1}"),
            egui::FontId::proportional(11.0),
            Color32::from_rgb(220, 220, 220),
        );
    }

    if hovered && app.download_enabled() && !app.local.contains(&id) {
        let center = Pos2::new(image_area.max.x - 24.0, image_area.min.y + 24.0);
        painter.circle_filled(center, 13.0, theme::ELEVATED);
        painter.circle_stroke(center, 13.0, Stroke::new(1.0_f32, theme::GREEN));
        icons::download(&painter, center, 20.0, Stroke::new(1.6_f32, theme::GREEN));
        let hit = Rect::from_center_size(center, Vec2::splat(28.0));
        if ui
            .interact(hit, ui.id().with((&id, "download")), Sense::click())
            .clicked()
        {
            app.start_download(i);
        }
    }

    let check_center = Pos2::new(
        rect.min.x + 22.0,
        image_area.min.y + if free || is_new { 58.0 } else { 22.0 },
    );
    if hovered || is_checked {
        painter.circle_filled(
            check_center,
            10.0,
            if is_checked {
                theme::BLUE
            } else {
                theme::PANEL
            },
        );
        painter.circle_stroke(check_center, 10.0, Stroke::new(1.0_f32, theme::BLUE));
        if is_checked {
            icons::check(
                &painter,
                check_center,
                11.0,
                Stroke::new(1.5_f32, Color32::WHITE),
            );
        }
    }
    if hovered
        && ui
            .interact(
                Rect::from_center_size(check_center, Vec2::splat(28.0)),
                ui.id().with((&id, "check")),
                Sense::click(),
            )
            .clicked()
    {
        app.toggle_checked(&id);
    }

    let strip = Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - STRIP_H), rect.max);
    painter.rect_filled(strip, 0.0, fill);
    let heart = Pos2::new(strip.max.x - 20.0, strip.center().y);
    painter.text(
        Pos2::new(strip.min.x + 12.0, strip.center().y),
        Align2::LEFT_CENTER,
        ellipsize(&title, 24),
        egui::FontId::proportional(13.0),
        theme::FG_SECONDARY,
    );
    painter.text(
        heart,
        Align2::CENTER_CENTER,
        "\u{2665}",
        egui::FontId::proportional(13.0),
        if favorite {
            Color32::from_rgb(224, 96, 96)
        } else {
            theme::FG_MUTED
        },
    );
    if ui
        .interact(
            Rect::from_center_size(heart, Vec2::splat(28.0)),
            ui.id().with((&id, "heart")),
            Sense::click(),
        )
        .clicked()
    {
        app.toggle_favorite(&id);
    }

    if response.clicked() || response.secondary_clicked() {
        app.select(i);
    }
}

/// Paint `texture` into `rect`, cropping to fill it without distortion.
pub fn draw_thumbnail(painter: &Painter, rect: Rect, texture: &egui::TextureHandle) {
    let source = texture.size_vec2();
    if source.x <= 0.0 || source.y <= 0.0 {
        return;
    }
    let scale = (rect.width() / source.x).max(rect.height() / source.y);
    let target = Rect::from_center_size(rect.center(), source * scale);
    painter.with_clip_rect(rect).image(
        texture.id(),
        target,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
}

/// A Bridge-style rendered preview: a lit sphere over a shadow ellipse, used
/// until (or instead of) a marketplace thumbnail.
pub fn draw_placeholder(painter: &Painter, rect: Rect, hue: f32) {
    let r = rect.width().min(rect.height()) * 0.28;
    let center = rect.center();
    let base = theme::hue_color(hue);

    painter.add(Shape::ellipse_filled(
        center + Vec2::new(0.0, r * 1.06),
        Vec2::new(r * 0.92, r * 0.18),
        Color32::from_black_alpha(90),
    ));
    painter.circle_filled(center, r, base);
    painter.circle_filled(
        center + Vec2::new(-r * 0.32, -r * 0.36),
        r * 0.42,
        base.linear_multiply(1.45),
    );
    painter.circle_filled(
        center + Vec2::new(r * 0.38, r * 0.42),
        r * 0.55,
        base.linear_multiply(0.45),
    );
}

/// Cut `text` to `max` characters on a character boundary, adding an ellipsis.
fn ellipsize(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(max.saturating_sub(3)).collect();
    short.push_str("...");
    short
}

fn badge(painter: &Painter, min: Pos2, text: &str, fill: Color32) {
    let rect = Rect::from_min_size(min, Vec2::new(44.0, 18.0));
    painter.rect_filled(rect, 9.0, fill);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(11.0),
        Color32::WHITE,
    );
}

#[cfg(test)]
mod tests {
    use super::ellipsize;

    #[test]
    fn long_titles_are_cut_on_a_char_boundary() {
        assert_eq!(ellipsize("short", 24), "short");
        assert_eq!(
            ellipsize("a very long marketplace listing", 12),
            "a very lo..."
        );
        assert_eq!(ellipsize("héllo wörld and more", 8), "héllo...");
    }
}
