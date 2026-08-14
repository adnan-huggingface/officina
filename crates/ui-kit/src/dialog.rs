//! The look of every dialog both apps put on the screen.
//!
//! A dialog is the one place an application talks in sentences, and it is
//! judged as a piece of typography rather than as a control: a heading in the
//! text's own weight, a line of body copy hard against the frame, and three
//! borderless words in a row where the buttons should be, is read as an
//! unfinished program no matter how right the sentence is.
//!
//! So the frame, the padding, the type and the buttons live here rather than at
//! each of the twenty call sites, which is also the only way they stay the same
//! as each other.
//!
//! The metrics are Windows': a 96 by 32 button, a 24 pixel gutter, the action
//! row in a band of its own along the bottom, and the default action leftmost
//! in a group pushed to the right.

use eframe::egui;

use crate::fonts::{face, Family};

/// Excel's green, which is this workspace's accent everywhere else too.
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x21, 0x73, 0x46);
const ACCENT_HOVER: egui::Color32 = egui::Color32::from_rgb(0x2B, 0x8B, 0x56);
const ACCENT_DOWN: egui::Color32 = egui::Color32::from_rgb(0x19, 0x5A, 0x36);

/// A message box's width, and the least a form dialog may be.
///
/// Fixed rather than fitted: a box that is as wide as its longest sentence is a
/// different shape every time it appears, and a paragraph set to sixty
/// characters is easier to read than one set to a hundred and forty.
pub const WIDTH: f32 = 460.0;

const GUTTER: i8 = 24;

/// What kind of news the box is carrying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Something happened and the user needs to know. No decision to make.
    Info,
    /// A question whose wrong answer loses work.
    Warning,
    /// Something was refused or failed.
    Error,
}

impl Severity {
    fn color(self) -> egui::Color32 {
        match self {
            Severity::Info => egui::Color32::from_rgb(0x05, 0x63, 0xC1),
            Severity::Warning => egui::Color32::from_rgb(0xC7, 0x8A, 0x00),
            Severity::Error => egui::Color32::from_rgb(0xC4, 0x2B, 0x1C),
        }
    }
}

/// One button in a dialog's action row.
#[derive(Debug, Clone, Copy)]
pub struct Choice<'a> {
    pub label: &'a str,
    /// Drawn filled, and pressed by Enter. At most one per box.
    pub primary: bool,
    /// Pressed by Escape. At most one per box.
    pub escapes: bool,
}

impl<'a> Choice<'a> {
    pub fn new(label: &'a str) -> Self {
        Choice {
            label,
            primary: false,
            escapes: false,
        }
    }

    /// The action the box exists to offer: filled, and under Enter.
    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    /// The way out: under Escape, and under the window's close button in
    /// spirit, since a dialog that cannot be dismissed is a trap.
    pub fn escapes(mut self) -> Self {
        self.escapes = true;
        self
    }
}

/// A message box: an icon, a heading, a paragraph, and a row of answers.
///
/// `detail` is for the machine's own words — an error code, a parser's
/// complaint. It is set smaller and greyer underneath, because it is the one
/// line in the box the user did not ask for and cannot act on, and putting it
/// in the same type as the sentence that *can* be acted on buries the sentence.
///
/// Returns the index of the button pressed, by click or by key.
pub fn message(
    ctx: &egui::Context,
    id: &str,
    severity: Severity,
    heading: &str,
    body: &str,
    detail: Option<&str>,
    choices: &[Choice<'_>],
) -> Option<usize> {
    let mut chosen = None;
    egui::Modal::new(egui::Id::new(("ui-kit-message", id)))
        .frame(frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: GUTTER,
                    right: GUTTER,
                    top: 22,
                    bottom: 22,
                })
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        badge(ui, severity);
                        ui.add_space(14.0);
                        ui.vertical(|ui| {
                            // Bounded so the paragraph wraps against the gutter
                            // rather than against the screen: a `Modal` grows to
                            // whatever its widest child asks for.
                            ui.set_max_width(WIDTH - f32::from(GUTTER) * 2.0 - 46.0);
                            ui.label(egui::RichText::new(heading).font(heading_font(16.0)));
                            ui.add_space(7.0);
                            paragraph(ui, body);
                            if let Some(detail) = detail.filter(|d| !d.trim().is_empty()) {
                                ui.add_space(10.0);
                                ui.label(
                                    egui::RichText::new(detail)
                                        .size(11.5)
                                        .color(egui::Color32::from_gray(0x77)),
                                );
                            }
                        });
                    });
                });
            chosen = actions(ui, |ui| one_of(ui, choices));
        });

    // The keys, after the buttons, so a click this frame wins over a key that
    // arrived in the same one.
    let (enter, escape) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
        )
    });
    chosen
        .or_else(|| {
            enter
                .then(|| choices.iter().position(|c| c.primary))
                .flatten()
        })
        .or_else(|| {
            escape
                .then(|| choices.iter().position(|c| c.escapes))
                .flatten()
        })
}

/// The frame a dialog sits in: the window colour, a hairline, a soft shadow.
pub fn frame(ctx: &egui::Context) -> egui::Frame {
    let fill = ctx.style_of(ctx.theme()).visuals.window_fill;
    egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(0xC4)))
        .corner_radius(egui::CornerRadius::same(8))
        .shadow(egui::Shadow {
            offset: [0, 8],
            blur: 28,
            spread: 0,
            color: egui::Color32::from_black_alpha(46),
        })
        // None at all: the parts inside lay out their own, and the action band
        // has to reach the edges of the frame to be a band.
        .inner_margin(0)
}

/// A dialog's heading, in the real bold face rather than a darker grey.
pub fn heading_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, face(Family::Sans, true, false))
}

/// The action row: a band along the bottom, buttons pushed to the right.
///
/// The first button added is the leftmost, which is where Windows puts the
/// default action.
pub fn actions<T>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> T) -> T {
    let hairline = egui::Stroke::new(1.0, egui::Color32::from_gray(0xDD));
    let band = egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: 8,
            se: 8,
        })
        .inner_margin(egui::Margin {
            left: 20,
            right: 20,
            top: 14,
            bottom: 14,
        });
    let inner = band.show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        // Right to left so the group sits against the right edge; the closure
        // still reads left to right because `one_of` hands the buttons over in
        // reverse. A caller adding its own buttons gets the same order.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.set_min_width(ui.available_width());
            add(ui)
        })
        .inner
    });
    let rect = inner.response.rect;
    ui.painter()
        .hline(rect.x_range(), rect.top() - 0.5, hairline);
    inner.inner
}

/// A form dialog's action row: a rule across the dialog, buttons pushed right.
///
/// No band, unlike a message box: a message box is a footer with a sentence
/// above it, and a form is a page of controls whose buttons belong to the page.
///
/// Laid out right to left, so the *first* button added is the rightmost. That
/// is the way round it has to be for the group to sit against the right edge
/// without measuring it first, so a caller adds Cancel first and the action
/// after it.
pub fn row<T>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> T) -> T {
    ui.add_space(14.0);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 1.0), egui::Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        egui::Stroke::new(1.0, egui::Color32::from_gray(0xE0)),
    );
    ui.add_space(12.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.set_min_width(ui.available_width());
        add(ui)
    })
    .inner
}

/// The action row nearly every form ends with: one action, and a way out.
///
/// `Some(true)` is the action, `Some(false)` is Cancel.
pub fn confirm(ui: &mut egui::Ui, action: &str) -> Option<bool> {
    row(ui, |ui| {
        let cancel = button(ui, "Cancel", false).clicked();
        let go = button(ui, action, true).clicked();
        match (go, cancel) {
            (true, _) => Some(true),
            (_, true) => Some(false),
            _ => None,
        }
    })
}

/// Draws `choices` into an action row and reports which one was pressed.
pub fn one_of(ui: &mut egui::Ui, choices: &[Choice<'_>]) -> Option<usize> {
    let mut chosen = None;
    // Reversed because the row is laid out right to left: the caller's first
    // choice must end up furthest left.
    for (index, choice) in choices.iter().enumerate().rev() {
        if button(ui, choice.label, choice.primary).clicked() {
            chosen = Some(index);
        }
    }
    chosen
}

/// A dialog button: a real button, sized as one, with a state to hover into.
///
/// The application's own theme deliberately draws buttons flat and borderless —
/// a toolbar of forty framed buttons is a wall of boxes — but the same treatment
/// in a dialog leaves three words floating in the corner, which is what makes a
/// box look unfinished. Here they are drawn as buttons.
pub fn button(ui: &mut egui::Ui, label: &str, primary: bool) -> egui::Response {
    ui.scope(|ui| {
        let v = &mut ui.visuals_mut().widgets;
        if primary {
            v.inactive.weak_bg_fill = ACCENT;
            v.inactive.bg_stroke = egui::Stroke::new(1.0, ACCENT);
            v.hovered.weak_bg_fill = ACCENT_HOVER;
            v.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT_HOVER);
            v.active.weak_bg_fill = ACCENT_DOWN;
            v.active.bg_stroke = egui::Stroke::new(1.0, ACCENT_DOWN);
            for w in [&mut v.inactive, &mut v.hovered, &mut v.active] {
                w.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
            }
        } else {
            v.inactive.weak_bg_fill = egui::Color32::WHITE;
            v.inactive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(0xC4));
            v.hovered.weak_bg_fill = egui::Color32::from_gray(0xF0);
            v.hovered.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(0xAE));
            v.active.weak_bg_fill = egui::Color32::from_gray(0xE4);
            v.active.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(0xAE));
        }
        for w in [&mut v.inactive, &mut v.hovered, &mut v.active] {
            w.corner_radius = egui::CornerRadius::same(4);
        }
        ui.add(
            egui::Button::new(egui::RichText::new(label).size(13.5))
                .min_size(egui::vec2(96.0, 32.0)),
        )
    })
    .inner
}

/// Body text: one label per line, an empty line as a paragraph break.
///
/// Not one wrapped label, because the lines are written as lines — a path, then
/// what to do about it — and a paragraph laid out by a modal comes back centred
/// when the modal is wider than the text.
pub fn paragraph(ui: &mut egui::Ui, body: &str) {
    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        for line in body.lines() {
            if line.is_empty() {
                ui.add_space(7.0);
            } else {
                ui.label(egui::RichText::new(line).size(13.5));
            }
        }
    });
}

/// The round badge beside the heading, drawn rather than set in a glyph.
///
/// A font is not guaranteed to have a warning sign, and a box whose icon comes
/// out as a hollow rectangle is worse than one with no icon at all.
fn badge(ui: &mut egui::Ui, severity: Severity) {
    let size = 32.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter();
    let color = severity.color();
    let centre = rect.center();

    match severity {
        Severity::Warning => {
            // A triangle, because that is what a warning is everywhere.
            let half = size * 0.5;
            let top = egui::pos2(centre.x, centre.y - half * 0.92);
            let left = egui::pos2(centre.x - half * 0.96, centre.y + half * 0.74);
            let right = egui::pos2(centre.x + half * 0.96, centre.y + half * 0.74);
            painter.add(egui::Shape::convex_polygon(
                vec![top, right, left],
                color,
                egui::Stroke::NONE,
            ));
            bang(painter, centre.y + size * 0.06, centre.x, size, true);
        }
        Severity::Error => {
            painter.circle_filled(centre, size * 0.5, color);
            let arm = size * 0.19;
            let stroke = egui::Stroke::new(2.4, egui::Color32::WHITE);
            painter.line_segment(
                [
                    centre + egui::vec2(-arm, -arm),
                    centre + egui::vec2(arm, arm),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    centre + egui::vec2(arm, -arm),
                    centre + egui::vec2(-arm, arm),
                ],
                stroke,
            );
        }
        Severity::Info => {
            painter.circle_filled(centre, size * 0.5, color);
            // An `i` is an exclamation mark the other way up.
            bang(painter, centre.y, centre.x, size, false);
        }
    }
}

/// The bar-and-dot of an `!` (dot below) or an `i` (dot above), drawn white.
fn bang(painter: &egui::Painter, middle: f32, x: f32, size: f32, dot_below: bool) {
    let white = egui::Color32::WHITE;
    let bar = size * 0.30;
    let gap = size * 0.075;
    let dot = size * 0.058;
    let half_width = size * 0.052;
    let top = middle - (bar + gap + dot * 2.0) * 0.5;

    let (bar_top, dot_centre) = if dot_below {
        (top, top + bar + gap + dot)
    } else {
        (top + dot * 2.0 + gap, top + dot)
    };
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(x - half_width, bar_top),
            egui::pos2(x + half_width, bar_top + bar),
        ),
        egui::CornerRadius::same(1),
        white,
    );
    painter.circle_filled(egui::pos2(x, dot_centre), dot, white);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one frame of a message box and hands back what it drew.
    fn shown(choices: &[Choice<'_>], input: egui::RawInput) -> (Option<usize>, Vec<egui::Shape>) {
        let ctx = egui::Context::default();
        // The names only, with no directories to read them from: the heading
        // asks for the bold sans face, and epaint panics on a family nobody
        // has registered rather than substituting for it.
        crate::fonts::register(&ctx, &[]);
        // A modal fades in, and a colour read mid-fade is the colour times the
        // opacity so far. Nothing here is testing the animation.
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        let mut chosen = None;
        let mut frame = |input: egui::RawInput| {
            ctx.run_ui(input, |_ui| {
                chosen = message(
                    &ctx,
                    "test",
                    Severity::Warning,
                    "Save changes to budget.xlsx?",
                    "Your changes will be lost if you don't save them.",
                    Some("os error 32"),
                    choices,
                );
            })
        };
        // Twice: a centred `Area` spends its first frame working out how big it
        // is, and a sizing pass throws its shapes away.
        let mut warm = frame(egui::RawInput {
            screen_rect: input.screen_rect,
            ..Default::default()
        });
        warm.textures_delta.clear();
        let mut out = frame(input);
        out.textures_delta.clear();
        // Flattened: a `Ui` hands back nested `Shape::Vec`s, and a button's
        // body is inside one of them.
        fn flatten(shape: egui::Shape, into: &mut Vec<egui::Shape>) {
            match shape {
                egui::Shape::Vec(many) => {
                    for one in many {
                        flatten(one, into);
                    }
                }
                one => into.push(one),
            }
        }
        let mut shapes = Vec::new();
        for clipped in out.shapes {
            flatten(clipped.shape, &mut shapes);
        }
        (chosen, shapes)
    }

    fn keyed(key: egui::Key) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn enter_takes_the_default_and_escape_the_way_out() {
        let choices = [
            Choice::new("Save").primary(),
            Choice::new("Don't Save"),
            Choice::new("Cancel").escapes(),
        ];
        assert_eq!(shown(&choices, keyed(egui::Key::Enter)).0, Some(0));
        assert_eq!(shown(&choices, keyed(egui::Key::Escape)).0, Some(2));
    }

    #[test]
    fn a_box_with_no_way_out_is_not_dismissed_by_a_key_that_has_no_button() {
        // Every box should offer one; the point is that a key never invents an
        // answer the caller did not list.
        let choices = [Choice::new("Retry").primary()];
        assert_eq!(shown(&choices, keyed(egui::Key::Escape)).0, None);
    }

    /// The complaint this module exists to answer: buttons that draw no button.
    #[test]
    fn the_buttons_are_drawn_as_buttons() {
        let choices = [
            Choice::new("Save").primary(),
            Choice::new("Cancel").escapes(),
        ];
        let (_, shapes) = shown(
            &choices,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 700.0),
                )),
                ..Default::default()
            },
        );

        let filled: Vec<&egui::epaint::RectShape> = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Rect(r) => Some(r),
                _ => None,
            })
            .filter(|r| {
                let size = r.rect.size();
                (size.x - 96.0).abs() < 8.0 && (size.y - 32.0).abs() < 4.0
            })
            .collect();
        assert!(
            filled.len() >= 2,
            "both buttons should paint a 96x32 body, found {}",
            filled.len()
        );
        assert!(
            filled.iter().any(|r| r.fill == ACCENT),
            "the default action is the filled one: {:?}",
            filled.iter().map(|r| r.fill).collect::<Vec<_>>()
        );
        assert!(
            filled.iter().any(|r| r.stroke.width > 0.0),
            "and the other has an edge to it"
        );
    }
}
