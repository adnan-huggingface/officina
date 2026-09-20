//! The first-run card: which helper Assist should use, asked once, in words.
//!
//! The rows are what the computer already has, looked for on a thread of its
//! own — `ant` can take seconds to print a login, and a server that is not
//! there a moment to say so — in the order `assist::ladder` gives them, with
//! the first preselected. The person the card is for has never needed the
//! words "model", "LLM", "token", "inference", "endpoint" or "GPU", and the
//! card does not use them.

use ::assist::{Installed, Row};
use eframe::egui;

use super::request::{Awaited, Background};
use super::transcript::{card_frame, radio};
use crate::{dialog, theme};

/// The card's one question.
pub(crate) const QUESTION: &str = "Which helper should Assist use?";

/// What stands in the transcript until a helper is chosen.
pub(crate) enum Choosing {
    /// The settings file is there and could not be read, for this reason.
    Unreadable(String),
    /// Looking at what the computer has: the rows, and how many looks were
    /// refused on the thread that looked.
    Looking(Background<(Vec<Row>, usize)>),
    Rows(Rows),
}

pub(crate) struct Rows {
    pub rows: Vec<Row>,
    pub picked: usize,
    /// Which of Ollama's models is picked, for its row.
    pub model: usize,
    /// What came of the last choice, when it came to nothing: the helper on
    /// this computer not being ready, a save that failed.
    pub said: Option<String>,
}

impl Rows {
    pub fn new(rows: Vec<Row>) -> Rows {
        // Lit first: the first row that can be chosen. A row that only says
        // why this computer has no helper of its own is not a choice.
        let picked = rows.iter().position(Row::is_ready).unwrap_or(0);
        Rows {
            rows,
            picked,
            model: 0,
            said: None,
        }
    }
}

/// What was pressed on the card.
pub(crate) enum Chose {
    /// Use the picked row, with the picked model of Ollama's.
    Use,
    /// Look again, after a settings file that could not be read.
    Again,
    NotNow,
}

/// The id of the card's "Use this", which takes the keyboard when the pane
/// is given it while the card is up.
pub(crate) fn use_this_id() -> egui::Id {
    egui::Id::new("ui-kit-assist-use-this")
}

/// Draws the card, and says what was pressed. The rows arrive from the look
/// here, and the looks the thread was refused are handed back to be counted
/// on this one.
pub(crate) fn card(
    ui: &mut egui::Ui,
    choosing: &mut Choosing,
    refused: &mut usize,
) -> Option<Chose> {
    if let Choosing::Looking(looking) = choosing {
        match looking.poll() {
            Awaited::Waiting => {}
            Awaited::Done((rows, times)) => {
                *refused += times;
                *choosing = Choosing::Rows(Rows::new(rows));
            }
            // A look that never answered is still a card: the rows that are
            // always offered — and not a helper on this computer, which the
            // look alone can vouch for.
            Awaited::Gone => {
                *choosing = Choosing::Rows(Rows::new(vec![Row::ClaudeWithKey, Row::Service]))
            }
        }
    }
    let mut chose = None;
    match choosing {
        Choosing::Unreadable(why) => {
            card_frame(ui, theme::INK_ERROR, |ui| {
                ui.label(egui::RichText::new("Assist's settings could not be read").strong());
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(why.as_str())
                            .size(theme::TEXT_SMALL)
                            .color(theme::INK_SOFT),
                    )
                    .wrap(),
                );
                ui.add(
                    egui::Label::new(
                        "Choose a helper again, and the old settings are kept beside the \
                         new ones, as assist.toml.unreadable.",
                    )
                    .wrap(),
                );
                ui.add_space(4.0);
                let again = ui.add(
                    egui::Button::new(egui::RichText::new("Choose Again").size(theme::TEXT))
                        .min_size(egui::vec2(96.0, 28.0)),
                );
                if again.clicked() {
                    chose = Some(Chose::Again);
                }
            });
        }
        Choosing::Looking(_) => {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
            card_frame(ui, theme::ACCENT, |ui| {
                heading(ui);
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0).color(theme::INK_SOFT));
                    ui.label(
                        egui::RichText::new("Looking at what this computer already has…")
                            .color(theme::INK_SOFT),
                    );
                });
            });
        }
        Choosing::Rows(rows) => {
            card_frame(ui, theme::ACCENT, |ui| {
                heading(ui);
                ui.spacing_mut().item_spacing.y = 5.0;
                let picked_before = rows.picked;
                for index in 0..rows.rows.len() {
                    let on = index == rows.picked;
                    if row_line(ui, &rows.rows[index], on).clicked() {
                        rows.picked = index;
                    }
                    if let Row::OllamaHere { models } = &rows.rows[index] {
                        for (at, model) in models.iter().enumerate() {
                            let lit = on && at == rows.model;
                            if model_line(ui, model, lit).clicked() {
                                rows.picked = index;
                                rows.model = at;
                            }
                        }
                    }
                }
                if rows.picked != picked_before {
                    rows.said = None;
                }
                if let Some(said) = &rows.said {
                    ui.add(
                        egui::Label::new(egui::RichText::new(said).color(theme::INK_ERROR)).wrap(),
                    );
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let use_this = ui
                        .push_id(use_this_id(), |ui| dialog::button(ui, "Use this", true))
                        .inner;
                    // The keyboard is sent to this button by id: remember it.
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(use_this_id(), use_this.id));
                    if use_this.clicked() {
                        chose = Some(Chose::Use);
                    }
                    if dialog::button(ui, "Not now", false).clicked() {
                        chose = Some(Chose::NotNow);
                    }
                });
            });
        }
    }
    chose
}

/// The widget id "Use this" was drawn with, once it has been.
pub(crate) fn use_this_widget(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|d| d.get_temp(use_this_id()))
}

fn heading(ui: &mut egui::Ui) {
    ui.label(egui::RichText::new(QUESTION).font(dialog::heading_font(theme::HEADING)));
    ui.add_space(2.0);
}

/// One row: a radio mark, the title, and what the row is, in a frame that is
/// lit when picked.
fn row_line(ui: &mut egui::Ui, row: &Row, on: bool) -> egui::Response {
    let inner = ui.scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| {
        // A label that can be selected takes the click from the row it is in.
        ui.style_mut().interaction.selectable_labels = false;
        let (fill, edge) = match on {
            true => (theme::TINT_ON, theme::ACCENT),
            false => (theme::FIELD, theme::CHROME_RULE),
        };
        egui::Frame::new()
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, edge))
            .corner_radius(theme::RADIUS_CONTROL)
            .inner_margin(egui::Margin {
                left: 26,
                right: 8,
                top: 5,
                bottom: 6,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new(row.title()).strong().color(theme::INK));
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(row.about())
                            .size(theme::TEXT_SMALL)
                            .color(theme::INK_SOFT),
                    )
                    .wrap(),
                );
            });
    });
    let rect = inner.response.rect;
    radio(
        ui.painter(),
        egui::pos2(rect.left() + 13.0, rect.top() + 14.0),
        on,
    );
    inner.response
}

/// What a model of Ollama's is called on the card: its name, its size, and
/// whether it is large.
pub(crate) fn model_words(model: &Installed) -> String {
    let mut words = model.name.clone();
    if let Some(size) = model.size() {
        words.push_str(&format!(" · {size}"));
    }
    if model.is_large() {
        words.push_str(" — large: needs a powerful computer");
    }
    words
}

/// One of Ollama's models, under its row.
pub(crate) fn model_line(ui: &mut egui::Ui, model: &Installed, on: bool) -> egui::Response {
    let inner = ui.scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| {
        ui.style_mut().interaction.selectable_labels = false;
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 44,
                right: 8,
                top: 1,
                bottom: 1,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(model_words(model))
                            .size(theme::TEXT_SMALL)
                            .color(theme::INK),
                    )
                    .wrap(),
                );
            });
    });
    let rect = inner.response.rect;
    radio(
        ui.painter(),
        egui::pos2(rect.left() + 32.0, rect.top() + 9.0),
        on,
    );
    inner.response
}
