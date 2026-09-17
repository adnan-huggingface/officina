//! Toolbar icons, drawn from lines rather than typed.
//!
//! `⯇`, `↶`, `≡` and the rest live in Unicode blocks that Arial, Segoe UI and
//! the faces egui ships all decline to cover, so they render as hollow boxes —
//! silently, with everything compiling and every test green. Calx found this and
//! wrote it down (`LEARNINGS.md` §7); this file is what that costs, and it is
//! cheaper than a toolbar of empty rectangles.
//!
//! The four that genuinely are letters — B, I, U, S — are drawn as letters, in
//! the real bold, italic, underlined and struck faces, so the button is its own
//! preview. Every glyph is [`ui_kit::theme::ICON`] points on a side at a
//! stroke of [`ui_kit::theme::ICON_STROKE`], inside a target of
//! [`ui_kit::theme::TARGET`], which is the least a pointer can reliably hit.

use ui_kit::{egui, theme};

/// The side of a glyph, in points.
pub const SIZE: f32 = theme::ICON;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Undo,
    Redo,
    AlignLeft,
    AlignCenter,
    AlignRight,
    Justify,
    /// The find bar's previous-match button.
    ChevronUp,
    /// And its next-match button, and every combo's opener.
    ChevronDown,
    Bullets,
    Numbering,
    IndentIn,
    IndentOut,
    LineSpacing,
    /// An A over a bar of the current colour; the bar is drawn by the caller.
    TextColour,
    /// A marker pen over a bar of the current colour.
    Highlight,
    Table,
    Picture,
    Find,
    Comment,
    TrackChanges,
    /// The Navigate pane: a list down the left of a page.
    Navigate,
    /// The Review pane: a card with a tick down the right.
    Review,
    /// The toolbar's overflow: two chevrons.
    Overflow,
}

/// Draws one icon button and reports whether it was pressed.
///
/// A button that is *on* — a toggle whose state is true — wears the lit tint
/// and a two-point bar of the accent along its bottom edge, so that its state
/// is not carried by a tint alone.
pub fn button(ui: &mut egui::Ui, icon: Icon, on: bool, tip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(theme::TARGET, egui::Sense::click());
    paint_state(ui, rect, &response, on);
    draw(ui.painter(), icon, rect.center(), ink(ui));
    response.on_hover_text(tip)
}

/// The fill a flat control wears for its state, and the bar when it is on.
pub fn paint_state(ui: &egui::Ui, rect: egui::Rect, response: &egui::Response, on: bool) {
    let fill = match (on, response.is_pointer_button_down_on(), response.hovered()) {
        (_, true, _) => Some(theme::TINT_DOWN),
        (true, _, _) => Some(theme::TINT_ON),
        (false, _, true) => Some(theme::TINT_HOVER),
        _ => None,
    };
    if let Some(fill) = fill {
        ui.painter()
            .rect_filled(rect, theme::RADIUS_CONTROL as f32, fill);
    }
    if on {
        let bar = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 3.0, rect.bottom() - 2.0),
            egui::pos2(rect.right() - 3.0, rect.bottom()),
        );
        ui.painter().rect_filled(bar, 1.0, theme::ACCENT);
    }
}

fn ink(ui: &egui::Ui) -> egui::Color32 {
    if ui.is_enabled() {
        theme::INK
    } else {
        theme::INK_FAINT
    }
}

/// Draws a glyph centred on `at`.
pub fn draw(painter: &egui::Painter, icon: Icon, at: egui::Pos2, ink: egui::Color32) {
    let stroke = egui::Stroke::new(theme::ICON_STROKE, ink);
    let thin = egui::Stroke::new(1.2, ink);
    let half = SIZE / 2.0;
    let line = |points: Vec<egui::Pos2>| painter.add(egui::Shape::line(points, stroke));
    let seg = |a: egui::Pos2, b: egui::Pos2| painter.line_segment([a, b], stroke);
    let chevron = |tip: egui::Pos2, dx: f32, dy: f32| {
        painter.add(egui::Shape::line(
            vec![tip + egui::vec2(-dx, dy), tip, tip + egui::vec2(dx, dy)],
            stroke,
        ));
    };
    match icon {
        Icon::Undo | Icon::Redo => {
            // An arrow curving back on itself: a half circle, and a head on
            // the end that says which way it goes.
            let back = icon == Icon::Undo;
            let radius = half * 0.6;
            let centre = at + egui::vec2(0.0, radius * 0.35);
            let mut points = Vec::new();
            for step in 0..=10 {
                let t = std::f32::consts::PI * (step as f32 / 10.0);
                let angle = if back { std::f32::consts::PI + t } else { -t };
                points.push(centre + egui::vec2(radius * angle.cos(), -radius * angle.sin()));
            }
            let tip = *points.last().expect("eleven points");
            line(points);
            let sign = if back { 1.0 } else { -1.0 };
            line(vec![
                tip + egui::vec2(-3.0 * sign, -3.0),
                tip,
                tip + egui::vec2(-3.0 * sign, 3.0),
            ]);
        }
        Icon::AlignLeft | Icon::AlignCenter | Icon::AlignRight | Icon::Justify => {
            // Four lines, the short ones placed by the alignment.
            let full = SIZE * 0.85;
            let short = full * 0.6;
            for row in 0..4u32 {
                let y = at.y - 5.5 + row as f32 * 3.7;
                let long = matches!(icon, Icon::Justify) || row.is_multiple_of(2);
                let width = if long { full } else { short };
                let x = match icon {
                    Icon::AlignRight => at.x + full / 2.0 - width,
                    Icon::AlignCenter => at.x - width / 2.0,
                    _ => at.x - full / 2.0,
                };
                seg(egui::pos2(x, y), egui::pos2(x + width, y));
            }
        }
        Icon::ChevronUp | Icon::ChevronDown => {
            let sign = if icon == Icon::ChevronUp { -1.0 } else { 1.0 };
            chevron(at + egui::vec2(0.0, 2.0 * sign), 4.0, -3.0 * sign);
        }
        Icon::Overflow => {
            // Two chevrons pointing right: the row goes on.
            for dx in [-3.0, 3.0] {
                line(vec![
                    at + egui::vec2(dx - 2.5, -4.0),
                    at + egui::vec2(dx + 0.5, 0.0),
                    at + egui::vec2(dx - 2.5, 4.0),
                ]);
            }
        }
        Icon::Bullets | Icon::Numbering => {
            // Three lines with a mark before each: a dot, or a digit's worth
            // of strokes.
            for row in 0..3 {
                let y = at.y - 5.0 + row as f32 * 5.0;
                seg(egui::pos2(at.x - 2.0, y), egui::pos2(at.x + 7.5, y));
                let mark = egui::pos2(at.x - 6.0, y);
                if icon == Icon::Bullets {
                    painter.circle_filled(mark, 1.5, ink);
                } else {
                    // A short vertical stroke standing in for the numeral.
                    painter.line_segment(
                        [mark + egui::vec2(0.0, -2.0), mark + egui::vec2(0.0, 2.0)],
                        thin,
                    );
                    painter.line_segment(
                        [mark + egui::vec2(-1.5, 2.0), mark + egui::vec2(1.5, 2.0)],
                        thin,
                    );
                }
            }
        }
        Icon::IndentIn | Icon::IndentOut => {
            // Four lines, the middle two pushed in, and an arrow in the gap.
            let full = SIZE * 0.85;
            let left = at.x - full / 2.0;
            for row in 0..4 {
                let y = at.y - 5.5 + row as f32 * 3.7;
                let inset = if row == 1 || row == 2 { 6.0 } else { 0.0 };
                seg(egui::pos2(left + inset, y), egui::pos2(left + full, y));
            }
            let sign = if icon == Icon::IndentIn { 1.0 } else { -1.0 };
            let tip = egui::pos2(left + 2.0 + if sign > 0.0 { 2.5 } else { 0.0 }, at.y - 0.5);
            painter.add(egui::Shape::line(
                vec![
                    tip + egui::vec2(-2.0 * sign, -2.0),
                    tip,
                    tip + egui::vec2(-2.0 * sign, 2.0),
                ],
                thin,
            ));
        }
        Icon::LineSpacing => {
            // Three lines and a double-headed arrow beside them.
            for row in 0..3 {
                let y = at.y - 5.0 + row as f32 * 5.0;
                seg(egui::pos2(at.x - 1.0, y), egui::pos2(at.x + 7.5, y));
            }
            let x = at.x - 5.5;
            seg(egui::pos2(x, at.y - 6.0), egui::pos2(x, at.y + 6.0));
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(x - 2.0, at.y - 4.0),
                    egui::pos2(x, at.y - 6.0),
                    egui::pos2(x + 2.0, at.y - 4.0),
                ],
                thin,
            ));
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(x - 2.0, at.y + 4.0),
                    egui::pos2(x, at.y + 6.0),
                    egui::pos2(x + 2.0, at.y + 4.0),
                ],
                thin,
            ));
        }
        Icon::TextColour => {
            // An A, standing above the colour bar the caller draws.
            let top = at + egui::vec2(0.0, -7.0);
            line(vec![
                top + egui::vec2(-5.0, 12.0),
                top,
                top + egui::vec2(5.0, 12.0),
            ]);
            seg(top + egui::vec2(-3.0, 7.5), top + egui::vec2(3.0, 7.5));
        }
        Icon::Highlight => {
            // A marker pen: a slanted barrel with a chisel tip, above the bar.
            let a = at + egui::vec2(-1.5, 3.5);
            line(vec![
                a + egui::vec2(-4.0, -1.0),
                a + egui::vec2(4.0, -9.0),
                a + egui::vec2(7.0, -6.0),
                a + egui::vec2(-1.0, 2.0),
                a + egui::vec2(-4.0, -1.0),
            ]);
            seg(a + egui::vec2(-4.0, -1.0), a + egui::vec2(-6.0, 2.0));
        }
        Icon::Table => {
            let rect = egui::Rect::from_center_size(at, egui::vec2(14.0, 12.0));
            painter.rect_stroke(rect, 1.0, stroke, egui::StrokeKind::Inside);
            seg(
                egui::pos2(rect.left(), rect.center().y),
                egui::pos2(rect.right(), rect.center().y),
            );
            for x in [rect.left() + 4.7, rect.left() + 9.3] {
                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    thin,
                );
            }
        }
        Icon::Picture => {
            let rect = egui::Rect::from_center_size(at, egui::vec2(14.0, 12.0));
            painter.rect_stroke(rect, 1.0, stroke, egui::StrokeKind::Inside);
            // A hill and a sun.
            line(vec![
                egui::pos2(rect.left() + 1.5, rect.bottom() - 1.5),
                egui::pos2(rect.left() + 5.5, rect.bottom() - 6.0),
                egui::pos2(rect.left() + 8.0, rect.bottom() - 3.5),
                egui::pos2(rect.left() + 10.0, rect.bottom() - 5.5),
                egui::pos2(rect.right() - 1.5, rect.bottom() - 1.5),
            ]);
            painter.circle_filled(egui::pos2(rect.right() - 4.0, rect.top() + 3.5), 1.5, ink);
        }
        Icon::Find => {
            let centre = at + egui::vec2(-1.5, -1.5);
            painter.circle_stroke(centre, 5.0, stroke);
            seg(centre + egui::vec2(3.6, 3.6), centre + egui::vec2(7.5, 7.5));
        }
        Icon::Comment => {
            // A speech bubble with its tail at the lower left.
            let rect =
                egui::Rect::from_center_size(at + egui::vec2(0.0, -1.0), egui::vec2(14.0, 10.0));
            line(vec![
                egui::pos2(rect.left() + 3.0, rect.bottom()),
                egui::pos2(rect.left() + 1.0, rect.bottom() + 3.0),
                egui::pos2(rect.left() + 1.0, rect.bottom()),
                rect.left_bottom(),
                rect.left_top(),
                rect.right_top(),
                rect.right_bottom(),
                egui::pos2(rect.left() + 3.0, rect.bottom()),
            ]);
        }
        Icon::TrackChanges => {
            // A line of text, and a pen over its end.
            seg(at + egui::vec2(-7.0, 5.0), at + egui::vec2(1.0, 5.0));
            line(vec![
                at + egui::vec2(-1.0, 4.0),
                at + egui::vec2(6.0, -3.0),
                at + egui::vec2(8.0, -1.0),
                at + egui::vec2(1.0, 6.0),
                at + egui::vec2(-1.5, 6.5),
                at + egui::vec2(-1.0, 4.0),
            ]);
        }
        Icon::Navigate => {
            // A page with a narrow pane down its left.
            let rect = egui::Rect::from_center_size(at, egui::vec2(14.0, 12.0));
            painter.rect_stroke(rect, 1.0, stroke, egui::StrokeKind::Inside);
            let x = rect.left() + 5.0;
            seg(egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom()));
            for row in 0..3 {
                let y = rect.top() + 2.5 + row as f32 * 3.5;
                painter.line_segment(
                    [egui::pos2(rect.left() + 1.5, y), egui::pos2(x - 1.5, y)],
                    thin,
                );
            }
        }
        Icon::Review => {
            // A page with a pane down its right, and a tick in the pane.
            let rect = egui::Rect::from_center_size(at, egui::vec2(14.0, 12.0));
            painter.rect_stroke(rect, 1.0, stroke, egui::StrokeKind::Inside);
            let x = rect.right() - 6.0;
            seg(egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom()));
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(x + 1.5, rect.center().y),
                    egui::pos2(x + 3.0, rect.center().y + 1.8),
                    egui::pos2(rect.right() - 1.2, rect.center().y - 2.0),
                ],
                thin,
            ));
        }
    }
}

/// A B, an I, a U or an S, drawn in the face it turns on.
pub fn emphasis(ui: &mut egui::Ui, letter: &str, on: bool, tip: &str) -> egui::Response {
    let bold = letter == "B";
    let italic = letter == "I";
    let font = egui::FontId::new(
        14.0,
        ui_kit::fonts::face(ui_kit::Family::Serif, bold, italic),
    );
    let (rect, response) = ui.allocate_exact_size(theme::TARGET, egui::Sense::click());
    paint_state(ui, rect, &response, on);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        letter,
        font,
        ink(ui),
    );
    // The underline and the strike are not in the glyph: drawn as rules.
    let width = 7.0;
    let ink = ink(ui);
    if letter == "U" {
        let y = rect.center().y + 6.0;
        ui.painter().hline(
            (rect.center().x - width / 2.0)..=(rect.center().x + width / 2.0),
            y,
            egui::Stroke::new(1.0, ink),
        );
    }
    if letter == "S" {
        let y = rect.center().y + 0.5;
        ui.painter().hline(
            (rect.center().x - width / 2.0)..=(rect.center().x + width / 2.0),
            y,
            egui::Stroke::new(1.0, ink),
        );
    }
    response.on_hover_text(tip)
}

/// Every icon there is, for the test below and for the toolbar's own walk.
#[cfg(test)]
pub const ALL: [Icon; 23] = [
    Icon::Undo,
    Icon::Redo,
    Icon::AlignLeft,
    Icon::AlignCenter,
    Icon::AlignRight,
    Icon::Justify,
    Icon::ChevronUp,
    Icon::ChevronDown,
    Icon::Bullets,
    Icon::Numbering,
    Icon::IndentIn,
    Icon::IndentOut,
    Icon::LineSpacing,
    Icon::TextColour,
    Icon::Highlight,
    Icon::Table,
    Icon::Picture,
    Icon::Find,
    Icon::Comment,
    Icon::TrackChanges,
    Icon::Navigate,
    Icon::Review,
    Icon::Overflow,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon draws *something*, which is the failure a hollow box is.
    #[test]
    fn every_icon_puts_ink_on_the_screen() {
        /// One toolbar button, alone in the window.
        struct Alone(Icon);
        impl ui_kit::drive::Driven for Alone {
            fn drive(&mut self, ui: &mut egui::Ui) {
                button(ui, self.0, false, "");
            }
        }
        let drive = ui_kit::drive::Driver::sized(egui::vec2(200.0, 60.0));
        for icon in ALL {
            let painted = drive.paint(&mut Alone(icon), Vec::new());
            let drew = painted.shapes().iter().any(|shape| {
                matches!(
                    shape,
                    egui::Shape::LineSegment { .. }
                        | egui::Shape::Path(_)
                        | egui::Shape::Circle(_)
                        | egui::Shape::Rect(_)
                )
            });
            assert!(
                drew,
                "{icon:?} drew nothing — a toolbar of hollow boxes is what this file exists to prevent"
            );
        }
    }
}
