//! Toolbar icons, drawn from lines rather than typed.
//!
//! `⯇`, `↶`, `≡` and the rest live in Unicode blocks that Arial, Segoe UI and
//! the faces egui ships all decline to cover, so they render as hollow boxes —
//! silently, with everything compiling and every test green. Calx found this and
//! wrote it down (`LEARNINGS.md` §7); this file is what that costs, and it is
//! cheaper than a toolbar of empty rectangles.
//!
//! The three that genuinely are letters — B, I, U — are drawn as letters, in the
//! real bold, italic and underlined faces, so the button is its own preview.

use ui_kit::egui;

/// The side of a square icon button, in points.
pub const SIZE: f32 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Undo,
    Redo,
    AlignLeft,
    AlignCenter,
    AlignRight,
    Justify,
    Grow,
    Shrink,
    /// The find bar's previous-match button.
    ChevronUp,
    /// And its next-match button.
    ChevronDown,
}

/// Draws one icon button and reports whether it was pressed.
pub fn button(ui: &mut egui::Ui, icon: Icon, on: bool, tip: &str) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(SIZE + 6.0, SIZE + 4.0), egui::Sense::click());
    let visuals = ui.style().interact(&response);
    if on || response.hovered() {
        let look = if on {
            ui.visuals().widgets.open
        } else {
            *visuals
        };
        ui.painter().rect(
            rect,
            4.0,
            look.weak_bg_fill,
            look.bg_stroke,
            egui::StrokeKind::Inside,
        );
    }
    let ink = if ui.is_enabled() {
        egui::Color32::from_gray(0x33)
    } else {
        egui::Color32::from_gray(0xAA)
    };
    draw(ui.painter(), icon, rect.center(), ink);
    response.on_hover_text(tip).clicked()
}

fn draw(painter: &egui::Painter, icon: Icon, at: egui::Pos2, ink: egui::Color32) {
    let stroke = egui::Stroke::new(1.4, ink);
    let half = SIZE / 2.0;
    match icon {
        Icon::Undo | Icon::Redo => {
            // An arrow curving back on itself: five segments of a half circle,
            // and a head on the end that says which way it goes.
            let back = icon == Icon::Undo;
            let radius = half * 0.62;
            let centre = at + egui::vec2(0.0, radius * 0.35);
            let mut points = Vec::new();
            for step in 0..=10 {
                let t = std::f32::consts::PI * (step as f32 / 10.0);
                let angle = if back { std::f32::consts::PI + t } else { -t };
                points.push(centre + egui::vec2(radius * angle.cos(), -radius * angle.sin()));
            }
            painter.add(egui::Shape::line(points.clone(), stroke));
            let tip = *points.last().expect("eleven points");
            let sign = if back { 1.0 } else { -1.0 };
            painter.add(egui::Shape::line(
                vec![
                    tip + egui::vec2(-3.0 * sign, -3.0),
                    tip,
                    tip + egui::vec2(-3.0 * sign, 3.0),
                ],
                stroke,
            ));
        }
        Icon::AlignLeft | Icon::AlignCenter | Icon::AlignRight | Icon::Justify => {
            // Four lines, the short ones placed by the alignment.
            let full = SIZE * 0.72;
            let short = full * 0.62;
            for row in 0..4 {
                let y = at.y - 6.0 + row as f32 * 4.0;
                let long = matches!(icon, Icon::Justify) || row % 2 == 0;
                let width = if long { full } else { short };
                let x = match icon {
                    Icon::AlignRight => at.x + full / 2.0 - width,
                    Icon::AlignCenter => at.x - width / 2.0,
                    _ => at.x - full / 2.0,
                };
                painter.line_segment(
                    [egui::pos2(x, y), egui::pos2(x + width, y)],
                    egui::Stroke::new(1.6, ink),
                );
            }
        }
        Icon::ChevronUp | Icon::ChevronDown => {
            let sign = if icon == Icon::ChevronUp { -1.0 } else { 1.0 };
            let tip = at + egui::vec2(0.0, 2.5 * sign);
            painter.add(egui::Shape::line(
                vec![
                    tip + egui::vec2(-4.5, -3.0 * sign),
                    tip,
                    tip + egui::vec2(4.5, -3.0 * sign),
                ],
                stroke,
            ));
        }
        Icon::Grow | Icon::Shrink => {
            // A big A and a small one, with an arrow saying which way.
            let up = icon == Icon::Grow;
            let letter = |painter: &egui::Painter, at: egui::Pos2, size: f32| {
                painter.add(egui::Shape::line(
                    vec![
                        at + egui::vec2(-size / 2.0, size / 2.0),
                        at + egui::vec2(0.0, -size / 2.0),
                        at + egui::vec2(size / 2.0, size / 2.0),
                    ],
                    stroke,
                ));
                painter.line_segment(
                    [
                        at + egui::vec2(-size / 4.0, size / 6.0),
                        at + egui::vec2(size / 4.0, size / 6.0),
                    ],
                    stroke,
                );
            };
            letter(painter, at + egui::vec2(-5.0, 1.0), 11.0);
            letter(painter, at + egui::vec2(4.0, 3.0), 7.0);
            let tip = at + egui::vec2(8.0, if up { -6.0 } else { -1.0 });
            let sign = if up { 1.0 } else { -1.0 };
            painter.add(egui::Shape::line(
                vec![
                    tip + egui::vec2(-2.5, 2.5 * sign),
                    tip,
                    tip + egui::vec2(2.5, 2.5 * sign),
                ],
                stroke,
            ));
        }
    }
}

/// A B, an I or a U, drawn in the face it turns on.
pub fn emphasis(ui: &mut egui::Ui, letter: &str, on: bool, tip: &str) -> bool {
    let bold = letter == "B";
    let italic = letter == "I";
    let font = egui::FontId::new(
        13.0,
        ui_kit::fonts::face(ui_kit::Family::Serif, bold, italic),
    );
    let mut text = egui::RichText::new(letter).font(font);
    if letter == "U" {
        text = text.underline();
    }
    let mut button = egui::Button::new(text).min_size(egui::vec2(SIZE + 6.0, SIZE + 4.0));
    if on {
        let held = ui.visuals().widgets.open;
        button = button.fill(held.weak_bg_fill).stroke(held.bg_stroke);
    }
    ui.add(button).on_hover_text(tip).clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon draws *something*, which is the failure a hollow box is.
    #[test]
    fn every_icon_puts_ink_on_the_screen() {
        let ctx = egui::Context::default();
        ui_kit::fonts::register(&ctx, &[]);
        for icon in [
            Icon::Undo,
            Icon::Redo,
            Icon::AlignLeft,
            Icon::AlignCenter,
            Icon::AlignRight,
            Icon::Justify,
            Icon::Grow,
            Icon::Shrink,
            Icon::ChevronUp,
            Icon::ChevronDown,
        ] {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(200.0, 60.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    button(ui, icon, false, "");
                },
            );
            out.textures_delta.clear();
            let shapes: Vec<_> = out
                .shapes
                .into_iter()
                .map(|s| s.shape)
                .filter(|shape| {
                    matches!(
                        shape,
                        egui::Shape::LineSegment { .. } | egui::Shape::Path(_)
                    )
                })
                .collect();
            assert!(
                !shapes.is_empty(),
                "{icon:?} drew nothing — a toolbar of hollow boxes is what this file exists to prevent"
            );
        }
    }
}
