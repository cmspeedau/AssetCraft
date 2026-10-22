//! The asset grid: square cards with generated previews, badges, hover
//! actions, and the favourite heart — the Bridge card anatomy from
//! `docs/reference/quixel-bridge/images/24-related-assets.jpg`.

use egui::{Align2, Color32, Painter, Rect, Response, Sense, Shape, Stroke, Ui, Vec2};

use crate::app::App;
use crate::{icons, theme};

/// Thumbnail width; the name strip sits inside the card, as in Bridge.
pub const CARD_W: f32 = 220.0;
pub const GAP: f32 = 12.0;

pub fn show(ui: &mut Ui, app: &mut App, hits: &[usize]) {
    let cols = ((ui.available_width() + GAP) / (CARD_W + GAP))
        .max(1.0)
        .floor() as usize;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Mock data is small; a real catalog grid virtualizes rows here.
            for row in hits.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for &i in row {
                        card(ui, app, i);
                    }
                });
                ui.add_space(GAP - ui.spacing().item_spacing.y);
            }
        });
}

fn card(ui: &mut Ui, app: &mut App, i: usize) {
    let (id, name, hue, free, is_new, favorite, downloaded) = {
        let asset = &app.assets[i];
        (
            asset.id,
            asset.name,
            asset.hue,
            asset.free,
            asset.is_new,
            asset.favorite,
            asset.downloaded,
        )
    };
    let size = Vec2::new(CARD_W, CARD_W);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter().clone();

    let hovered = resp.hovered();
    let is_open = app.selected == Some(i);
    let is_checked = app.checked.contains(&i);

    painter.rect_filled(
        rect,
        8.0,
        if hovered || is_open {
            Color32::from_rgb(32, 32, 32)
        } else {
            Color32::from_rgb(26, 26, 26)
        },
    );
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

    draw_sphere(&painter, rect, hue, downloaded);

    if free {
        badge(
            &painter,
            rect.min + Vec2::new(10.0, 10.0),
            "FREE",
            theme::GREEN,
        );
    }
    if is_new {
        let y = if free { 34.0 } else { 10.0 };
        badge(
            &painter,
            rect.min + Vec2::new(10.0, y),
            "NEW",
            theme::BLUE_SOFT,
        );
    }

    if hovered && !downloaded {
        let center = rect.right_top() + Vec2::new(-24.0, 24.0);
        painter.circle_filled(center, 13.0, theme::ELEVATED);
        painter.circle_stroke(center, 13.0, Stroke::new(1.0_f32, theme::GREEN));
        icons::download(&painter, center, 20.0, Stroke::new(1.6_f32, theme::GREEN));
    }

    {
        let center = rect.left_top() + Vec2::new(22.0, if free || is_new { 58.0 } else { 22.0 });
        if hovered || is_checked {
            painter.circle_filled(
                center,
                10.0,
                if is_checked {
                    theme::BLUE
                } else {
                    theme::PANEL
                },
            );
            painter.circle_stroke(center, 10.0, Stroke::new(1.0_f32, theme::BLUE));
            if is_checked {
                icons::check(&painter, center, 11.0, Stroke::new(1.5_f32, Color32::WHITE));
            }
        }
    }

    let strip = Rect::from_min_size(
        rect.min + Vec2::new(0.0, rect.height() - 34.0),
        Vec2::new(rect.width(), 34.0),
    );
    let heart = strip.right_center() + Vec2::new(-20.0, 0.0);
    painter.text(
        strip.min + Vec2::new(12.0, 17.0),
        Align2::LEFT_CENTER,
        name,
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

    let heart_hit = Rect::from_center_size(heart, Vec2::splat(28.0));
    if ui
        .interact(heart_hit, ui.id().with((id, "heart")), Sense::click())
        .clicked()
    {
        app.toggle_favorite(i);
    }
    if hovered {
        let check_hit = Rect::from_center_size(
            rect.left_top() + Vec2::new(22.0, if free || is_new { 58.0 } else { 22.0 }),
            Vec2::splat(28.0),
        );
        if ui
            .interact(check_hit, ui.id().with((id, "check")), Sense::click())
            .clicked()
        {
            app.toggle_checked(i);
        }
    }

    if clicked(&resp) {
        app.selected = Some(i);
    }
}

fn clicked(resp: &Response) -> bool {
    resp.clicked() || (resp.secondary_clicked())
}

/// A Bridge-style rendered preview: a lit sphere over a shadow ellipse.
pub fn draw_sphere(painter: &Painter, rect: Rect, hue: f32, downloaded: bool) {
    let r = rect.width().min(rect.height() * 0.82) * 0.34;
    let center = rect.center() + Vec2::new(0.0, -rect.height() * 0.03);
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

    if downloaded {
        let c = center + Vec2::new(r * 0.9, -r * 0.9);
        painter.circle_filled(c, 9.0, theme::GREEN);
        icons::check(painter, c, 10.0, Stroke::new(1.4_f32, Color32::WHITE));
    }
}

fn badge(painter: &Painter, min: egui::Pos2, text: &str, fill: Color32) {
    let size = Vec2::new(44.0, 18.0);
    let rect = Rect::from_min_size(min, size);
    painter.rect_filled(rect, 9.0, fill);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(11.0),
        Color32::WHITE,
    );
}
