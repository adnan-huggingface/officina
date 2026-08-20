//! Toolbar icons, drawn rather than typed.
//!
//! The obvious way to put an icon on a button is to find a character that looks
//! like one — `⏴` for align-left, `⏎` for wrap, `↶` for undo. It does not work.
//! Those live in blocks (Miscellaneous Technical, Arrows) that Arial, Segoe UI,
//! and the fonts egui ships all decline to cover, so the button renders as a
//! hollow box, and it does so *silently*: the code compiles, the tests pass, and
//! the toolbar reads as a row of empty rectangles.
//!
//! So the icons are drawn. Each one is a handful of lines and rectangles, which
//! is all these glyphs ever were, and the ones that genuinely are letters — B,
//! I, U, S — are drawn as letters in the real bold and italic faces, which is
//! what those buttons mean anyway.

use ui_kit::egui;

/// Side of the square an icon is drawn in.
pub const SIZE: f32 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Undo,
    Redo,
    AlignLeft,
    AlignCenter,
    AlignRight,
    Wrap,
    IndentMore,
    IndentLess,
    Merge,
    Borders,
    Freeze,
    TextColor,
    FillColor,
    InsertRow,
    DeleteRow,
    InsertColumn,
    DeleteColumn,
    Save,
    Open,
    New,
    Sum,
    Filter,
    /// A to Z with a downward arrow, and its opposite.
    SortAscending,
    SortDescending,
}

/// A toolbar button carrying a drawn icon.
///
/// `on` gives it the pressed look, so the same call site serves a command and a
/// toggle — which is what bold is, and what align-left is.
pub fn button(ui: &mut egui::Ui, icon: Icon, on: bool, tip: &str) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(SIZE + 8.0, SIZE + 4.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let fill = if on {
            ui.visuals().widgets.active.weak_bg_fill
        } else {
            visuals.weak_bg_fill
        };
        ui.painter().rect(
            rect,
            visuals.corner_radius,
            fill,
            if on {
                visuals.bg_stroke
            } else {
                egui::Stroke::NONE
            },
            egui::StrokeKind::Inside,
        );
        draw(ui.painter(), icon, rect, visuals.fg_stroke.color);
    }
    response.on_hover_text(tip)
}

/// The left half of Excel's colour control: the glyph, with a band of the
/// colour under it that the button would apply.
///
/// The band is the whole point. A colour button with no band is a button whose
/// effect you find out by pressing it, which for a colour is one undo per
/// guess; with the band, the toolbar answers "what will this do" before it is
/// touched. Pair it with [`arrow`] for the half that changes the colour.
pub fn color_button(ui: &mut egui::Ui, icon: Icon, rgb: [u8; 3], tip: &str) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(SIZE + 6.0, SIZE + 4.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        ui.painter().rect(
            rect,
            visuals.corner_radius,
            visuals.weak_bg_fill,
            egui::Stroke::NONE,
            egui::StrokeKind::Inside,
        );
        // The glyph sits high so that the band has room to be a band rather
        // than an underline nobody notices.
        let glyph = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.max.y - 5.0));
        draw(ui.painter(), icon, glyph, visuals.fg_stroke.color);
        let band = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 4.0, rect.bottom() - 6.0),
            egui::pos2(rect.right() - 4.0, rect.bottom() - 2.0),
        );
        let [r, g, b] = rgb;
        ui.painter()
            .rect_filled(band, 1.0, egui::Color32::from_rgb(r, g, b));
        // White on white would be an invisible band, and "no fill" is a real
        // and common answer, so the band is always outlined.
        ui.painter().rect_stroke(
            band,
            1.0,
            egui::Stroke::new(1.0, egui::Color32::from_gray(0x8A)),
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text(tip)
}

/// A toolbar control that opens a menu: the glyph, and the chevron that says
/// so.
///
/// The chevron is not decoration. A dropdown drawn as a plain icon button is
/// indistinguishable from a command, so the user learns what it is by clicking
/// it and being surprised.
pub fn menu_button(ui: &mut egui::Ui, icon: Icon, tip: &str) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(SIZE + 17.0, SIZE + 4.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        ui.painter().rect(
            rect,
            visuals.corner_radius,
            visuals.weak_bg_fill,
            egui::Stroke::NONE,
            egui::StrokeKind::Inside,
        );
        let glyph = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x - 11.0, rect.max.y));
        draw(ui.painter(), icon, glyph, visuals.fg_stroke.color);
        chevron(
            ui.painter(),
            egui::pos2(rect.right() - 7.0, rect.center().y),
            visuals.fg_stroke.color,
        );
    }
    response.on_hover_text(tip)
}

/// The right half of a split control: a chevron, and nothing else.
pub fn arrow(ui: &mut egui::Ui, tip: &str) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(13.0, SIZE + 4.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        ui.painter().rect(
            rect,
            visuals.corner_radius,
            visuals.weak_bg_fill,
            egui::Stroke::NONE,
            egui::StrokeKind::Inside,
        );
        chevron(ui.painter(), rect.center(), visuals.fg_stroke.color);
    }
    response.on_hover_text(tip)
}

/// The little downward triangle that means "there is a list behind this".
fn chevron(painter: &egui::Painter, at: egui::Pos2, color: egui::Color32) {
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(at.x - 3.5, at.y - 1.5),
            egui::pos2(at.x + 3.5, at.y - 1.5),
            egui::pos2(at.x, at.y + 2.5),
        ],
        color,
        egui::Stroke::NONE,
    ));
}

/// A letter button that wears the formatting it applies.
///
/// The button for bold is the letter B *in the bold face*, drawn from the same
/// font the cells use — so the toolbar is a preview of what the machine can
/// actually render, and a missing bold face would be visible here first.
pub struct Letter<'a> {
    pub text: &'a str,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// Whether the cursor's cell already has this formatting.
    pub on: bool,
    pub tip: &'a str,
}

impl Letter<'static> {
    /// An unformatted letter, to spread over.
    pub fn plain() -> Self {
        Letter {
            text: "",
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            on: false,
            tip: "",
        }
    }
}

pub fn letter(ui: &mut egui::Ui, letter: Letter<'_>) -> egui::Response {
    let Letter {
        text,
        bold,
        italic,
        underline,
        strike,
        on,
        tip,
    } = letter;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(SIZE + 4.0, SIZE + 4.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let fill = if on {
            ui.visuals().widgets.active.weak_bg_fill
        } else {
            visuals.weak_bg_fill
        };
        ui.painter().rect(
            rect,
            visuals.corner_radius,
            fill,
            if on {
                visuals.bg_stroke
            } else {
                egui::Stroke::NONE
            },
            egui::StrokeKind::Inside,
        );
        let color = visuals.fg_stroke.color;
        let font = egui::FontId::new(
            14.0,
            ui_kit::fonts::face(ui_kit::Family::Serif, bold, italic),
        );
        let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
        let at = rect.center() - galley.size() / 2.0;
        let width = galley.size().x;
        ui.painter().galley(at, galley, color);
        if underline {
            ui.painter().hline(
                at.x..=at.x + width,
                rect.center().y + 7.0,
                egui::Stroke::new(1.0, color),
            );
        }
        if strike {
            ui.painter().hline(
                at.x - 1.0..=at.x + width + 1.0,
                rect.center().y,
                egui::Stroke::new(1.0, color),
            );
        }
    }
    response.on_hover_text(tip)
}

fn draw(painter: &egui::Painter, icon: Icon, rect: egui::Rect, color: egui::Color32) {
    let box_ = egui::Rect::from_center_size(rect.center(), egui::vec2(SIZE, SIZE)).shrink(3.0);
    let stroke = egui::Stroke::new(1.4, color);
    let thin = egui::Stroke::new(1.0, color);
    let (l, r, t, b) = (box_.left(), box_.right(), box_.top(), box_.bottom());
    let w = box_.width();
    let h = box_.height();

    // Four evenly spaced text lines, which is what most of these icons are
    // made of: alignment, wrapping, indenting.
    let line = |i: usize| t + h * (0.18 + 0.22 * i as f32);
    let bar = |painter: &egui::Painter, i: usize, from: f32, to: f32| {
        painter.hline(l + w * from..=l + w * to, line(i), thin);
    };

    match icon {
        Icon::AlignLeft => {
            for (i, to) in [1.0, 0.65, 1.0, 0.65].into_iter().enumerate() {
                bar(painter, i, 0.0, to);
            }
        }
        Icon::AlignRight => {
            for (i, from) in [0.0, 0.35, 0.0, 0.35].into_iter().enumerate() {
                bar(painter, i, from, 1.0);
            }
        }
        Icon::AlignCenter => {
            for (i, (from, to)) in [(0.0, 1.0), (0.18, 0.82), (0.0, 1.0), (0.18, 0.82)]
                .into_iter()
                .enumerate()
            {
                bar(painter, i, from, to);
            }
        }
        Icon::Wrap => {
            bar(painter, 0, 0.0, 1.0);
            bar(painter, 1, 0.0, 0.75);
            // The turn-and-return arrow that says the line came back.
            let y = line(2);
            painter.line_segment(
                [
                    egui::pos2(l + w * 0.75, line(1)),
                    egui::pos2(l + w * 0.75, y),
                ],
                thin,
            );
            painter.hline(l + w * 0.25..=l + w * 0.75, y, thin);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(l + w * 0.25, y),
                    egui::pos2(l + w * 0.4, y - 4.0),
                    egui::pos2(l + w * 0.4, y + 4.0),
                ],
                color,
                egui::Stroke::NONE,
            ));
            bar(painter, 3, 0.0, 1.0);
        }
        Icon::IndentMore | Icon::IndentLess => {
            for i in 0..4 {
                bar(painter, i, 0.4, 1.0);
            }
            let mid = (line(1) + line(2)) / 2.0;
            let (tip, back) = if icon == Icon::IndentMore {
                (l + w * 0.28, l + w * 0.02)
            } else {
                (l + w * 0.02, l + w * 0.28)
            };
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(tip, mid),
                    egui::pos2(back, mid - 5.0),
                    egui::pos2(back, mid + 5.0),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        Icon::Undo | Icon::Redo => {
            // Three-quarters of a circle with a head on one end.
            let centre = box_.center() + egui::vec2(0.0, 2.0);
            let radius = w * 0.36;
            let mut points = Vec::new();
            for step in 0..=24 {
                let sweep = std::f32::consts::PI * 1.15;
                let start = if icon == Icon::Undo {
                    std::f32::consts::PI * 1.0
                } else {
                    -std::f32::consts::PI * 0.15
                };
                let angle = start + sweep * step as f32 / 24.0;
                points.push(centre + egui::vec2(angle.cos() * radius, -angle.sin() * radius));
            }
            painter.add(egui::Shape::line(points.clone(), stroke));
            let head = if icon == Icon::Undo {
                points[0]
            } else {
                points[points.len() - 1]
            };
            let flip = if icon == Icon::Undo { 1.0 } else { -1.0 };
            painter.add(egui::Shape::convex_polygon(
                vec![
                    head + egui::vec2(0.0, -5.0),
                    head + egui::vec2(-4.0 * flip, 2.0),
                    head + egui::vec2(4.0 * flip, 2.0),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        Icon::Merge => {
            painter.rect_stroke(box_, 0.0, thin, egui::StrokeKind::Inside);
            // Two arrows meeting in the middle: what a merge does to two cells.
            let y = box_.center().y;
            painter.hline(l + 2.0..=l + w * 0.42, y, thin);
            painter.hline(r - w * 0.42..=r - 2.0, y, thin);
            for (tip, back) in [(l + w * 0.45, l + w * 0.3), (r - w * 0.45, r - w * 0.3)] {
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        egui::pos2(tip, y),
                        egui::pos2(back, y - 4.0),
                        egui::pos2(back, y + 4.0),
                    ],
                    color,
                    egui::Stroke::NONE,
                ));
            }
        }
        Icon::Borders => {
            painter.rect_stroke(box_, 0.0, stroke, egui::StrokeKind::Inside);
            let dotted = egui::Stroke::new(1.0, color.gamma_multiply(0.55));
            painter.hline(l..=r, box_.center().y, dotted);
            painter.vline(box_.center().x, t..=b, dotted);
        }
        Icon::Freeze => {
            painter.rect_stroke(box_, 0.0, thin, egui::StrokeKind::Inside);
            // The frozen bands, filled, and the seam between them heavier.
            let seam_y = t + h * 0.34;
            let seam_x = l + w * 0.34;
            painter.rect_filled(
                egui::Rect::from_min_max(box_.min, egui::pos2(r, seam_y)),
                0.0,
                color.gamma_multiply(0.25),
            );
            painter.rect_filled(
                egui::Rect::from_min_max(box_.min, egui::pos2(seam_x, b)),
                0.0,
                color.gamma_multiply(0.25),
            );
            painter.hline(l..=r, seam_y, stroke);
            painter.vline(seam_x, t..=b, stroke);
        }
        Icon::TextColor => {
            let font =
                egui::FontId::new(13.0, ui_kit::fonts::face(ui_kit::Family::Sans, true, false));
            painter.text(
                egui::pos2(box_.center().x, t + h * 0.36),
                egui::Align2::CENTER_CENTER,
                "A",
                font,
                color,
            );
        }
        Icon::FillColor => {
            // A bucket, tipped as if pouring. The character that used to stand
            // in for this — `▪` — drew a three-pixel square: technically a
            // filled shape, and unreadable at any size anyone builds a toolbar
            // out of.
            let bucket = vec![
                egui::pos2(l + w * 0.12, t + h * 0.30),
                egui::pos2(r - w * 0.12, t + h * 0.30),
                egui::pos2(r - w * 0.30, t + h * 0.80),
                egui::pos2(l + w * 0.30, t + h * 0.80),
            ];
            painter.add(egui::Shape::convex_polygon(
                bucket,
                color.gamma_multiply(0.22),
                thin,
            ));
            // The handle: half a loop over the mouth.
            let mouth = egui::Rect::from_min_max(
                egui::pos2(l + w * 0.24, t + h * 0.04),
                egui::pos2(r - w * 0.24, t + h * 0.30),
            );
            let mut arc = Vec::new();
            for step in 0..=10 {
                let angle = std::f32::consts::PI * step as f32 / 10.0;
                arc.push(egui::pos2(
                    mouth.center().x - angle.cos() * mouth.width() / 2.0,
                    mouth.bottom() - angle.sin() * mouth.height(),
                ));
            }
            painter.add(egui::Shape::line(arc, thin));
        }
        Icon::InsertRow | Icon::DeleteRow | Icon::InsertColumn | Icon::DeleteColumn => {
            painter.rect_stroke(box_, 0.0, thin, egui::StrokeKind::Inside);
            let horizontal = matches!(icon, Icon::InsertRow | Icon::DeleteRow);
            let band = if horizontal {
                egui::Rect::from_min_max(egui::pos2(l, t + h * 0.36), egui::pos2(r, t + h * 0.64))
            } else {
                egui::Rect::from_min_max(egui::pos2(l + w * 0.36, t), egui::pos2(l + w * 0.64, b))
            };
            let adding = matches!(icon, Icon::InsertRow | Icon::InsertColumn);
            painter.rect_filled(
                band,
                0.0,
                if adding {
                    egui::Color32::from_rgb(0x1E, 0x6F, 0x5C).gamma_multiply(0.45)
                } else {
                    egui::Color32::from_rgb(0xC0, 0x39, 0x2B).gamma_multiply(0.45)
                },
            );
            let c = band.center();
            painter.hline(c.x - 4.0..=c.x + 4.0, c.y, stroke);
            if adding {
                painter.vline(c.x, c.y - 4.0..=c.y + 4.0, stroke);
            }
        }
        Icon::New | Icon::Open | Icon::Save => {
            let page =
                egui::Rect::from_min_max(egui::pos2(l + w * 0.15, t), egui::pos2(r - w * 0.15, b));
            painter.rect_stroke(page, 0.0, thin, egui::StrokeKind::Inside);
            match icon {
                Icon::New => {
                    let c = page.center();
                    painter.hline(c.x - 4.0..=c.x + 4.0, c.y, stroke);
                    painter.vline(c.x, c.y - 4.0..=c.y + 4.0, stroke);
                }
                Icon::Save => {
                    // The shutter of a floppy disk, which is still what "save"
                    // looks like forty years after anyone last saw one.
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(page.left() + 3.0, page.top() + 1.0),
                            egui::pos2(page.right() - 3.0, page.top() + h * 0.35),
                        ),
                        0.0,
                        color.gamma_multiply(0.5),
                    );
                    painter.rect_stroke(
                        egui::Rect::from_min_max(
                            egui::pos2(page.left() + 2.0, page.bottom() - h * 0.42),
                            egui::pos2(page.right() - 2.0, page.bottom() - 1.0),
                        ),
                        0.0,
                        thin,
                        egui::StrokeKind::Inside,
                    );
                }
                _ => {
                    for i in 0..3 {
                        painter.hline(
                            page.left() + 3.0..=page.right() - 3.0,
                            page.top() + h * (0.3 + 0.2 * i as f32),
                            thin,
                        );
                    }
                }
            }
        }
        Icon::Sum => {
            painter.text(
                box_.center(),
                egui::Align2::CENTER_CENTER,
                "Σ",
                egui::FontId::new(
                    15.0,
                    ui_kit::fonts::face(ui_kit::Family::Sans, false, false),
                ),
                color,
            );
        }
        Icon::SortAscending | Icon::SortDescending => {
            // Excel's own: the two letters stacked in reading order, with an
            // arrow beside them pointing the way the sort runs. Drawn rather
            // than typed because the arrow glyphs are in a Unicode block the
            // system sans faces do not cover.
            let up = matches!(icon, Icon::SortAscending);
            let font =
                egui::FontId::new(8.0, ui_kit::fonts::face(ui_kit::Family::Sans, false, false));
            let x = l + 3.0;
            for (i, letter) in ["A", "Z"].iter().enumerate() {
                let letter = if up { *letter } else { ["Z", "A"][i] };
                painter.text(
                    egui::pos2(x, t + 2.0 + i as f32 * (h * 0.45)),
                    egui::Align2::LEFT_TOP,
                    letter,
                    font.clone(),
                    color,
                );
            }
            // The arrow always points down: it is the reading direction of the
            // letters beside it, not the direction of the comparison.
            let ax = r - 3.0;
            painter.vline(ax, t + 2.0..=b - 2.0, stroke);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(ax - 2.5, b - 5.0),
                    egui::pos2(ax + 2.5, b - 5.0),
                    egui::pos2(ax, b - 1.0),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        Icon::Filter => {
            // A funnel.
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(l, t + 2.0),
                    egui::pos2(r, t + 2.0),
                    egui::pos2(box_.center().x + 2.0, box_.center().y),
                    egui::pos2(box_.center().x + 2.0, b),
                    egui::pos2(box_.center().x - 2.0, b - 3.0),
                    egui::pos2(box_.center().x - 2.0, box_.center().y),
                ],
                egui::Color32::TRANSPARENT,
                stroke,
            ));
        }
    }
}
