//! The transcript: what was asked, what the helper said and did, and the
//! changes it made, in the order they happened.
//!
//! It belongs to the window. Nothing in it is saved, with the document or
//! anywhere else: a conversation about a document is not part of it, and
//! there is no second place it belongs.

use eframe::egui;

use crate::theme;

/// One thing in the transcript.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// What the person asked, in their own words.
    Asked(String),
    /// What the helper said. `kept` goes false when the request it belongs to
    /// ends without finishing: the conversation forgets the request, so the
    /// next one does not build on these words, and the screen says so.
    Said { words: String, kept: bool },
    /// What a tool did, in a line: "read paragraphs 12–14".
    Did(String),
    /// A change the application made at the helper's request, with what can be
    /// done about it.
    Card(Card),
    /// A sentence from the pane itself — a failure, a refusal, a stop, a helper
    /// chosen — and the one thing that helps, when there is one.
    Note {
        sentence: String,
        action: Option<Action>,
    },
}

/// What a note offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// The settings box: a key refused, a helper not ready.
    Settings,
    /// The same request again: a helper busy, a connection dropped.
    Retry,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Action::Settings => "Open Settings",
            Action::Retry => "Try Again",
        }
    }
}

/// A change, as its card shows it. The application makes the card and
/// settles it; the pane draws it and says which action was pressed.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    /// The application's own name for the change.
    pub id: u64,
    pub title: String,
    pub body: String,
    /// The buttons, in order: "Accept", "Reject"; "Undo".
    pub actions: Vec<String>,
    /// What became of it, once settled: "Accepted". A settled card has no
    /// buttons.
    pub verdict: Option<String>,
}

/// What was pressed on an entry.
pub(crate) enum Pressed {
    Card { card: u64, action: usize },
    Note(Action),
}

/// Draws one entry, and says what was pressed on it. While `busy` — a
/// request under way, or waiting to be agreed to — nothing can be tried again.
pub(crate) fn entry(ui: &mut egui::Ui, entry: &Entry, busy: bool) -> Option<Pressed> {
    match entry {
        Entry::Asked(words) => {
            asked(ui, words);
            None
        }
        Entry::Said { words, kept } => {
            said(ui, words, *kept);
            None
        }
        Entry::Did(line) => {
            indented(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(line)
                            .size(theme::TEXT_SMALL)
                            .color(theme::INK_SOFT),
                    )
                    .wrap(),
                );
            });
            None
        }
        Entry::Card(card) => change(ui, card),
        Entry::Note { sentence, action } => {
            let mut pressed = None;
            indented(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(sentence).color(theme::INK)).wrap());
                if let Some(action) = action {
                    let enabled = !(busy && *action == Action::Retry);
                    let button = ui
                        .add_enabled_ui(enabled, |ui| flat_action(ui, action.label()))
                        .inner;
                    if button.clicked() {
                        pressed = Some(Pressed::Note(*action));
                    }
                }
            });
            pressed
        }
    }
}

/// The person's words: on the right, on the tint that marks a thing as theirs.
fn asked(ui: &mut egui::Ui, words: &str) {
    let width = ui.available_width();
    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
        ui.add_space(8.0);
        egui::Frame::new()
            .fill(theme::TINT_ON)
            .corner_radius(theme::RADIUS_MENU)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_max_width((width * 0.8).max(80.0));
                ui.add(egui::Label::new(egui::RichText::new(words).color(theme::INK)).wrap());
            });
    });
}

/// The helper's words, on the left; greyed, and said to be forgotten, when
/// the request did not finish.
fn said(ui: &mut egui::Ui, words: &str, kept: bool) {
    if words.trim().is_empty() {
        return;
    }
    let ink = if kept { theme::INK } else { theme::INK_SOFT };
    indented(ui, |ui| {
        ui.add(egui::Label::new(egui::RichText::new(words.trim()).color(ink)).wrap());
        if !kept {
            ui.label(
                egui::RichText::new(NOT_KEPT)
                    .size(theme::TEXT_SMALL)
                    .color(theme::INK_SOFT),
            );
        }
    });
}

/// The line under the words of a request that did not finish.
pub const NOT_KEPT: &str = "Not kept: the assistant will not remember this answer.";

fn change(ui: &mut egui::Ui, card: &Card) -> Option<Pressed> {
    let mut pressed = None;
    card_frame(ui, theme::author(0), |ui| {
        ui.label(egui::RichText::new(&card.title).strong());
        if !card.body.is_empty() {
            ui.add(egui::Label::new(egui::RichText::new(&card.body).color(theme::INK)).wrap());
        }
        match &card.verdict {
            Some(verdict) => {
                ui.label(
                    egui::RichText::new(verdict)
                        .size(theme::TEXT_SMALL)
                        .color(theme::INK_SOFT),
                );
            }
            None => {
                ui.horizontal(|ui| {
                    for (index, label) in card.actions.iter().enumerate() {
                        if flat_action(ui, label).clicked() {
                            pressed = Some(Pressed::Card {
                                card: card.id,
                                action: index,
                            });
                        }
                    }
                });
            }
        }
    });
    pressed
}

/// A line of the transcript that is not the person's: from the left edge, a
/// margin in.
pub(crate) fn indented<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 10,
            right: 10,
            top: 0,
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// The frame a card sits in, the Review pane's: the field colour, a rule
/// round it, and a bar of `bar` down its left.
pub(crate) fn card_frame<R>(
    ui: &mut egui::Ui,
    bar: egui::Color32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let inner = egui::Frame::new()
        .fill(theme::FIELD)
        .stroke(egui::Stroke::new(1.0, theme::CHROME_RULE))
        .corner_radius(theme::RADIUS_MENU as f32)
        .inner_margin(egui::Margin {
            left: 12,
            right: 8,
            top: 6,
            bottom: 6,
        })
        .outer_margin(egui::Margin::symmetric(8, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        });
    let rect = inner.response.rect;
    let strip = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 1.0, rect.top() + 4.0),
        egui::pos2(rect.left() + 4.0, rect.bottom() - 4.0),
    );
    ui.painter().rect_filled(strip, 1.5, bar);
    inner.inner
}

/// A small flat action, as a card's are.
pub(crate) fn flat_action(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(
        egui::RichText::new(label)
            .size(theme::TEXT_SMALL)
            .color(theme::ACCENT_DOWN),
    ))
}

/// A chip: a small pill with an edge, for a quick verb or a scope.
pub(crate) fn chip(ui: &mut egui::Ui, label: &str, lit: bool) -> egui::Response {
    let (fill, edge) = match lit {
        true => (theme::TINT_ON, theme::ACCENT),
        false => (theme::FIELD, theme::FIELD_EDGE),
    };
    ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .size(theme::TEXT_SMALL)
                .color(theme::INK),
        )
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, edge))
        .corner_radius(11.0),
    )
}

/// A compact primary button: the accent, white words.
pub(crate) fn primary(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        for (state, fill) in [
            (&mut widgets.inactive, theme::ACCENT),
            (&mut widgets.hovered, theme::ACCENT_HOVER),
            (&mut widgets.active, theme::ACCENT_DOWN),
        ] {
            state.weak_bg_fill = fill;
            state.bg_stroke = egui::Stroke::new(1.0, fill);
            state.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
        }
        ui.add(
            egui::Button::new(egui::RichText::new(label).size(theme::TEXT))
                .min_size(egui::vec2(64.0, 24.0)),
        )
    })
    .inner
}

/// A small flat button with an edge, for the header's Stop.
pub(crate) fn plain(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).size(theme::TEXT_SMALL))
            .fill(theme::FIELD)
            .stroke(egui::Stroke::new(1.0, theme::FIELD_EDGE))
            .min_size(egui::vec2(48.0, 22.0)),
    )
}

/// Three dots, drawn: the chrome's face has no `⋯`, and a glyph a face does
/// not have is a box on the screen and a pass in a test.
pub(crate) fn dots(ui: &mut egui::Ui) -> egui::Response {
    let response = ui.add(egui::Button::new("").min_size(theme::TARGET));
    let centre = response.rect.center();
    for step in [-5.0, 0.0, 5.0] {
        ui.painter()
            .circle_filled(centre + egui::vec2(step, 0.0), 1.6, theme::INK);
    }
    response
}

/// A chevron pointing down, drawn, after the words of a control that opens
/// a list.
pub(crate) fn chevron(painter: &egui::Painter, at: egui::Pos2, ink: egui::Color32) {
    let stroke = egui::Stroke::new(theme::ICON_STROKE, ink);
    painter.line_segment(
        [at + egui::vec2(-3.5, -1.8), at + egui::vec2(0.0, 1.8)],
        stroke,
    );
    painter.line_segment(
        [at + egui::vec2(0.0, 1.8), at + egui::vec2(3.5, -1.8)],
        stroke,
    );
}

/// A radio mark, drawn: a ring, filled in the accent when `on`.
pub(crate) fn radio(painter: &egui::Painter, centre: egui::Pos2, on: bool) {
    let ring = if on {
        theme::ACCENT
    } else {
        theme::FIELD_EDGE_HOT
    };
    painter.circle(centre, 6.0, theme::FIELD, egui::Stroke::new(1.5, ring));
    if on {
        painter.circle_filled(centre, 3.2, theme::ACCENT);
    }
}
