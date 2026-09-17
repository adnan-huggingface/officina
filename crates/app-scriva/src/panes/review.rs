//! The Review pane: every tracked change and comment as a card, in document
//! order, with what can be done about each on the card itself.
//!
//! Word's reviewing pane exists because a change the user cannot find is a
//! change they will not settle. The old pane listed cards with one "Go to"
//! each and settled changes only all at once or "the nearest"; a change is
//! accepted or rejected here on its own card. A comment is drafted *in* the
//! pane, at the place its card will take, with the words it is about already
//! washed on the page — not in a box over the document — and Ctrl+Enter
//! posts it, as Word's key does. The card of the change or comment at the
//! caret is outlined and kept in view as the caret moves.

use ui_kit::{dialog, egui, theme};
use wp_model::{Mark, Scope};

use crate::app::{Command, Draft, Keyboard, Scriva};

/// Which cards the pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Filter {
    #[default]
    All,
    Changes,
    Comments,
}

/// What a card stands for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CardKey {
    Change(Mark),
    Comment(u32),
    Draft,
}

/// One card as it was drawn this frame: what it is, where, whether it is the
/// caret's, and where its action buttons are — what a test presses instead
/// of a person.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CardDrawn {
    pub key: CardKey,
    pub rect: egui::Rect,
    pub at_caret: bool,
    pub actions: Vec<(&'static str, egui::Rect)>,
    /// The first action's widget id, for Tab to land on from the keyboard.
    pub first_action: Option<egui::Id>,
}

fn drawn_id() -> egui::Id {
    egui::Id::new("scriva-review-drawn")
}

/// The cards the last frame drew, in order.
#[cfg(test)]
pub(crate) fn drawn(ctx: &egui::Context) -> Vec<CardDrawn> {
    ctx.data(|d| d.get_temp::<Vec<CardDrawn>>(drawn_id()))
        .unwrap_or_default()
}

fn note(ui: &egui::Ui, card: CardDrawn) {
    ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<CardDrawn>>(drawn_id())
            .push(card)
    });
}

/// A card's place in the document, for ordering: the flow, the paragraph and
/// the offset. Flows in the order the document lists them, the text first.
type Place = (usize, usize, usize);

/// A change, gathered before the pane draws.
struct ChangeCard {
    place: Place,
    mark: Mark,
    what: &'static str,
    flow: Option<&'static str>,
    text: String,
    at_caret: bool,
}

/// A comment, gathered before the pane draws, with its replies under it.
struct CommentCard {
    place: Place,
    id: u32,
    author: String,
    date: Option<String>,
    text: String,
    done: bool,
    author_index: usize,
    at_caret: bool,
    replies: Vec<CommentCard>,
}

enum Card {
    Change(ChangeCard),
    Comment(CommentCard),
}

impl Card {
    fn place(&self) -> Place {
        match self {
            Card::Change(card) => card.place,
            Card::Comment(card) => card.place,
        }
    }
}

/// Which flow a card is in, for a list that shows more than one of them.
/// `None` for the text, which needs no saying.
fn flow_name(document: &wp_model::Document, scope: Scope) -> Option<&'static str> {
    let Scope::Chrome(id) = scope else {
        return None;
    };
    Some(match document.header(id)?.footer {
        true => "footer",
        false => "header",
    })
}

impl Scriva {
    /// The comment the caret stands in, if any: the innermost by start.
    pub(crate) fn comment_at_caret(&mut self) -> Option<u32> {
        let (scope, caret) = (self.scope, self.caret());
        self.comment_ranges_now()
            .iter()
            .filter(|range| {
                let (start, end) = range.range.ordered();
                range.scope == scope && start <= caret && caret <= end
            })
            .max_by_key(|range| range.range.ordered().0)
            .map(|range| range.id)
    }

    /// Everything the pane shows, read from the model in document order.
    fn cards(&mut self) -> Vec<Card> {
        let flows = self.document_ref().flows();
        let flow_index = |scope: Scope| flows.iter().position(|f| *f == scope).unwrap_or(0);
        let (scope, caret) = (self.scope, self.caret());
        let comment_here = self.comment_at_caret();
        let ranges = self.comment_ranges_now().to_vec();
        let document = self.document_ref();
        let authors = document.authors();
        let mut cards = Vec::new();

        // The caret's change: the nearest by offset among those in its
        // paragraph — a change is a range, and the one the caret stands in is
        // at no distance at all.
        let changes = crate::revise::tracked(document);
        let nearest = changes
            .iter()
            .filter(|change| change.scope == scope && change.paragraph == caret.paragraph)
            .min_by_key(|change| change.offset.abs_diff(caret.offset))
            .map(|change| change.mark.clone());
        for change in changes {
            cards.push(Card::Change(ChangeCard {
                place: (flow_index(change.scope), change.paragraph, change.offset),
                at_caret: comment_here.is_none() && nearest.as_ref() == Some(&change.mark),
                mark: change.mark,
                what: change.what,
                flow: flow_name(document, change.scope),
                text: change.text,
            }));
        }
        let card_of = |comment: &wp_model::Comment| {
            let range = ranges.iter().find(|range| range.id == comment.id);
            let place = range
                .map(|range| {
                    let start = range.range.ordered().0;
                    (flow_index(range.scope), start.paragraph, start.offset)
                })
                .unwrap_or((usize::MAX, 0, 0));
            CommentCard {
                place,
                id: comment.id,
                author: comment.author.to_string(),
                date: comment.date.as_ref().map(|d| d.to_string()),
                text: comment.text(),
                done: comment.done,
                author_index: authors
                    .iter()
                    .position(|known| *known == comment.author)
                    .unwrap_or(0),
                at_caret: comment_here == Some(comment.id),
                replies: Vec::new(),
            }
        };
        for comment in document.comments.iter().filter(|c| c.parent.is_none()) {
            let mut card = card_of(comment);
            card.replies = document
                .comments
                .iter()
                .filter(|reply| reply.parent == Some(comment.id))
                .map(card_of)
                .collect();
            cards.push(Card::Comment(card));
        }
        cards.sort_by_key(Card::place);
        cards
    }

    /// The pane down the right, and what was chosen on it.
    pub(crate) fn review_pane(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        ui.ctx()
            .data_mut(|d| d.insert_temp::<Vec<CardDrawn>>(drawn_id(), Vec::new()));
        let cards = self.cards();
        let mut draft = self.draft.clone();
        let draft_place = draft.as_ref().map(|draft| {
            let flows = self.document_ref().flows();
            let start = draft.range.ordered().0;
            (
                flows.iter().position(|f| *f == draft.scope).unwrap_or(0),
                start.paragraph,
                start.offset,
            )
        });
        let changes = cards
            .iter()
            .filter(|card| matches!(card, Card::Change(_)))
            .count();
        let comments = cards.len() - changes;
        let mut filter = self.review_filter;
        let mut chosen: Option<Command> = None;
        let mut held = false;
        let scrolled = self.review_scrolled.clone();
        let mut scroll_to: Option<CardKey> = scrolled.clone();

        // The keyboard in the pane: Up and Down walk the cards, Enter goes to
        // the lit card's place, Tab lands on its first button — from where
        // egui's own Tab walks the rest and Enter presses.
        let keyboard_here = self.keyboard == Keyboard::Review;
        let mut row_at = self.review_row.min(cards.len().saturating_sub(1));
        let mut go = false;
        let mut tab = false;
        if keyboard_here && !cards.is_empty() && draft.is_none() {
            let (up, down, enter, tabbed) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Tab),
                )
            });
            if up {
                row_at = row_at.saturating_sub(1);
            }
            if down {
                row_at = (row_at + 1).min(cards.len() - 1);
            }
            go = enter;
            tab = tabbed;
        }
        let lit = keyboard_here.then_some(row_at);

        // Drawn inside the right-hand side the window opens for it, which
        // Assist takes in turn.
        // Header: the tabs, the counts, the filter, the close.
        let header = egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(10, 5))
            .show(ui, |ui| {
                ui.set_min_height(theme::PANE_HEADER - 10.0);
                ui.horizontal(|ui| {
                    for (index, name) in crate::app::assisting::TABS.iter().enumerate() {
                        let label = egui::RichText::new(*name).strong().size(theme::TEXT);
                        if ui.selectable_label(index == 0, label).clicked() && index != 0 {
                            chosen = Some(Command::ShowAssist);
                        }
                    }
                    ui.label(
                        egui::RichText::new(format!(
                            "{} · {}",
                            plural(changes, "change"),
                            plural(comments, "comment")
                        ))
                        .color(theme::INK_SOFT)
                        .size(theme::TEXT_SMALL),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new("×").frame(false))
                            .on_hover_text("Close the pane")
                            .clicked()
                        {
                            chosen = Some(Command::Reviewer);
                        }
                    });
                });
                // The filter on a row of its own: beside the counts it
                // ran into them at the pane's width.
                ui.horizontal(|ui| {
                    for (label, value) in [
                        ("All", Filter::All),
                        ("Changes", Filter::Changes),
                        ("Comments", Filter::Comments),
                    ] {
                        if ui
                            .selectable_label(
                                filter == value,
                                egui::RichText::new(label).size(theme::TEXT_SMALL),
                            )
                            .clicked()
                        {
                            filter = value;
                        }
                    }
                });
            });
        let rule = header.response.rect.bottom() + 0.5;
        ui.painter().hline(
            header.response.rect.x_range(),
            rule,
            egui::Stroke::new(1.0, theme::CHROME_RULE),
        );

        // Footer first, so the body's scroll area takes what is left.
        egui::Panel::bottom("scriva-reviewer-foot")
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::CHROME)
                    .inner_margin(egui::Margin::symmetric(10, 8)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(changes > 0, |ui| {
                        if dialog::button(ui, "Accept All", false).clicked() {
                            chosen = Some(Command::AcceptAll);
                        }
                        if dialog::button(ui, "Reject All", false).clicked() {
                            chosen = Some(Command::RejectAll);
                        }
                    });
                });
            });

        ui_kit::scroll::show(
            ui,
            egui::ScrollArea::vertical().auto_shrink([false, false]),
            |ui| {
                ui.add_space(6.0);
                ui.spacing_mut().item_spacing.y = 6.0;
                let mut drafted = false;
                if cards.is_empty() && draft.is_none() {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        ui.label(egui::RichText::new("Nothing to review.").color(theme::INK_SOFT));
                    });
                }
                for (at, card) in cards.iter().enumerate() {
                    let is_lit = lit == Some(at);
                    // The draft goes where its comment will: before the
                    // first card that comes after it.
                    if let (Some(place), Some(d), false) = (draft_place, &mut draft, drafted) {
                        if d.reply_to.is_none() && card.place() >= place {
                            drafted = true;
                            let (command, holds) = draft_card(ui, d);
                            chosen = chosen.take().or(command);
                            held |= holds;
                        }
                    }
                    match card {
                        Card::Change(card) if filter != Filter::Comments => {
                            chosen = chosen.take().or(change_card(ui, card, is_lit));
                        }
                        Card::Comment(card) if filter != Filter::Changes => {
                            let reply_draft =
                                draft.as_mut().filter(|d| d.reply_to == Some(card.id));
                            let (command, holds) = comment_card(ui, card, reply_draft, is_lit);
                            chosen = chosen.take().or(command);
                            held |= holds;
                            if draft.as_ref().is_some_and(|d| d.reply_to == Some(card.id)) {
                                drafted = true;
                            }
                        }
                        _ => {}
                    }
                }
                if let (Some(d), false) = (&mut draft, drafted) {
                    let (command, holds) = draft_card(ui, d);
                    chosen = chosen.take().or(command);
                    held |= holds;
                }
                ui.add_space(6.0);

                // The card at the caret, kept in view as the caret moves —
                // once per arrival, not on every frame, or the list could
                // not be scrolled away from it.
                // With the keyboard here, the lit card instead.
                let drawn = drawn_now(ui.ctx());
                let follow = match lit {
                    Some(at) => drawn.get(at).cloned(),
                    None => drawn.iter().find(|card| card.at_caret).cloned(),
                };
                if let Some(card) = follow {
                    if scroll_to.as_ref() != Some(&card.key) {
                        ui.scroll_to_rect(card.rect, Some(egui::Align::Center));
                        scroll_to = Some(card.key.clone());
                    }
                    if go {
                        chosen = Some(match &card.key {
                            CardKey::Change(mark) => Command::GoToChange(mark.clone()),
                            CardKey::Comment(id) => Command::GoToComment(*id),
                            CardKey::Draft => Command::PostComment,
                        });
                    }
                    if tab {
                        if let Some(first) = card.first_action {
                            ui.ctx().memory_mut(|m| m.request_focus(first));
                            held = true;
                        }
                    }
                }
            },
        );

        self.review_filter = filter;
        self.review_scrolled = scroll_to;
        self.review_row = row_at;
        self.pane_held = held;
        if let Some(draft) = draft {
            self.draft = Some(draft);
        }
        chosen
    }
}

fn drawn_now(ctx: &egui::Context) -> Vec<CardDrawn> {
    ctx.data(|d| d.get_temp::<Vec<CardDrawn>>(drawn_id()))
        .unwrap_or_default()
}

fn plural(count: usize, what: &str) -> String {
    match count {
        1 => format!("1 {what}"),
        n => format!("{n} {what}s"),
    }
}

/// The frame every card sits in: the field colour, a rule round it, the
/// accent round the one at the caret, and a bar of the author's colour down
/// its left. Clicking the card anywhere but a button goes to its place.
fn card_frame(
    ui: &mut egui::Ui,
    at_caret: bool,
    lit: bool,
    bar: egui::Color32,
    add: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let inner = ui.scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| {
        // The card is the control. A selectable label senses clicks for its
        // text, and would take them from the card it is on.
        ui.style_mut().interaction.selectable_labels = false;
        let edge = if at_caret || lit {
            egui::Stroke::new(1.5, theme::ACCENT)
        } else {
            egui::Stroke::new(1.0, theme::CHROME_RULE)
        };
        let frame = egui::Frame::new()
            .fill(if lit { theme::TINT_HOVER } else { theme::FIELD })
            .stroke(edge)
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
                add(ui);
            });
        let rect = frame.response.rect;
        let strip = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 1.0, rect.top() + 4.0),
            egui::pos2(rect.left() + 4.0, rect.bottom() - 4.0),
        );
        ui.painter().rect_filled(strip, 1.5, bar);
    });
    inner.response
}

/// A small flat action on a card.
fn action(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(
        egui::RichText::new(label)
            .size(theme::TEXT_SMALL)
            .color(theme::ACCENT_DOWN),
    ))
}

fn change_card(ui: &mut egui::Ui, card: &ChangeCard, lit: bool) -> Option<Command> {
    let mut chosen = None;
    let mut actions = Vec::new();
    let mut first_action = None;
    let colour = theme::author(0);
    let response = card_frame(ui, card.at_caret, lit, colour, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(card.mark.author.to_string()).strong());
            let mut what = card.what.to_owned();
            if let Some(flow) = card.flow {
                what.push_str(&format!(" in the {flow}"));
            }
            ui.label(egui::RichText::new(what).color(theme::INK_SOFT));
            if let Some(date) = &card.mark.date {
                ui.label(
                    egui::RichText::new(short_date(date))
                        .color(theme::INK_SOFT)
                        .size(theme::TEXT_SMALL),
                );
            }
        });
        if !card.text.is_empty() {
            ui.add(egui::Label::new(egui::RichText::new(&card.text)).truncate());
        }
        ui.horizontal(|ui| {
            let accept = action(ui, "Accept");
            first_action = Some(accept.id);
            actions.push(("Accept", accept.rect));
            if accept.clicked() {
                chosen = Some(Command::AcceptChange(card.mark.clone()));
            }
            let reject = action(ui, "Reject");
            actions.push(("Reject", reject.rect));
            if reject.clicked() {
                chosen = Some(Command::RejectChange(card.mark.clone()));
            }
        });
    });
    note(
        ui,
        CardDrawn {
            key: CardKey::Change(card.mark.clone()),
            rect: response.rect,
            at_caret: card.at_caret,
            actions,
            first_action,
        },
    );
    if response.clicked() && chosen.is_none() {
        chosen = Some(Command::GoToChange(card.mark.clone()));
    }
    chosen
}

/// A comment card, its replies under it, and the reply being written under
/// those when there is one.
fn comment_card(
    ui: &mut egui::Ui,
    card: &CommentCard,
    reply_draft: Option<&mut Draft>,
    lit: bool,
) -> (Option<Command>, bool) {
    let mut chosen = None;
    let mut held = false;
    let mut actions = Vec::new();
    let mut first_action = None;
    let colour = theme::author(card.author_index);
    let response = card_frame(ui, card.at_caret, lit, colour, |ui| {
        comment_body(ui, card, 0.0);
        for reply in &card.replies {
            ui.add_space(4.0);
            comment_body(ui, reply, 14.0);
        }
        if let Some(draft) = reply_draft {
            ui.add_space(4.0);
            let (command, holds) = draft_field(ui, draft);
            chosen = command;
            held = holds;
        }
        ui.horizontal(|ui| {
            let reply = action(ui, "Reply");
            first_action = Some(reply.id);
            actions.push(("Reply", reply.rect));
            if reply.clicked() {
                chosen = Some(Command::ReplyComment(card.id));
            }
            let label = if card.done { "Reopen" } else { "Resolve" };
            let resolve = action(ui, label);
            actions.push((label, resolve.rect));
            if resolve.clicked() {
                chosen = Some(Command::ResolveComment(card.id, !card.done));
            }
            let delete = action(ui, "Delete");
            actions.push(("Delete", delete.rect));
            if delete.clicked() {
                chosen = Some(Command::DeleteCommentOf(card.id));
            }
        });
    });
    note(
        ui,
        CardDrawn {
            key: CardKey::Comment(card.id),
            rect: response.rect,
            at_caret: card.at_caret,
            actions,
            first_action,
        },
    );
    if response.clicked() && chosen.is_none() {
        chosen = Some(Command::GoToComment(card.id));
    }
    (chosen, held)
}

/// The author line and the text of one comment, indented for a reply.
fn comment_body(ui: &mut egui::Ui, card: &CommentCard, indent: f32) {
    let ink = if card.done {
        theme::INK_SOFT
    } else {
        theme::INK
    };
    ui.horizontal(|ui| {
        if indent > 0.0 {
            let (bar, _) = ui.allocate_exact_size(egui::vec2(3.0, 16.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(bar, 1.5, theme::author(card.author_index));
            ui.add_space(indent - 8.0);
        }
        ui.label(egui::RichText::new(&card.author).strong().color(ink));
        if let Some(date) = &card.date {
            ui.label(
                egui::RichText::new(short_date(date))
                    .color(theme::INK_SOFT)
                    .size(theme::TEXT_SMALL),
            );
        }
        if card.done {
            ui.label(
                egui::RichText::new("Resolved")
                    .color(theme::INK_SOFT)
                    .size(theme::TEXT_SMALL),
            );
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(indent);
        ui.add(egui::Label::new(egui::RichText::new(&card.text).color(ink)).wrap());
    });
}

/// A new comment's card: the field, with the keys that post and discard it.
fn draft_card(ui: &mut egui::Ui, draft: &mut Draft) -> (Option<Command>, bool) {
    let mut out = (None, false);
    let response = card_frame(ui, true, false, theme::ACCENT, |ui| {
        ui.label(egui::RichText::new("New comment").strong());
        out = draft_field(ui, draft);
    });
    note(
        ui,
        CardDrawn {
            key: CardKey::Draft,
            rect: response.rect,
            at_caret: false,
            actions: Vec::new(),
            first_action: None,
        },
    );
    out
}

/// The field a comment is written in. It takes the keyboard when it opens
/// and keeps it: Enter is a new line in a note and Tab is a tab, so the two
/// keys that leave are Word's — Ctrl+Enter posts, Escape discards — read
/// before the field can swallow them.
fn draft_field(ui: &mut egui::Ui, draft: &mut Draft) -> (Option<Command>, bool) {
    let id = egui::Id::new("scriva-comment-draft");
    let focused = ui.memory(|m| m.focused()) == Some(id);
    let mut chosen = None;
    if focused {
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
            chosen = Some(Command::PostComment);
        } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            chosen = Some(Command::DiscardComment);
        }
    }
    let field = ui
        .scope(|ui| {
            dialog::form_style(ui.style_mut());
            ui.add(
                egui::TextEdit::multiline(&mut draft.text)
                    .id(id)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text("What is there to say about this?"),
            )
        })
        .inner;
    if draft.focus {
        field.request_focus();
        draft.focus = false;
    }
    // Escape is read here, not by egui: its own rule gives up the focus on
    // Escape before any widget runs, and a field that has already lost the
    // keyboard cannot tell that the key meant "discard".
    ui.ctx().memory_mut(|m| {
        m.set_focus_lock_filter(
            id,
            egui::EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
            },
        );
    });
    ui.label(
        egui::RichText::new("Ctrl+Enter posts · Esc discards")
            .color(theme::INK_SOFT)
            .size(theme::TEXT_SMALL),
    );
    let acted = chosen.is_some();
    (chosen, focused || field.has_focus() || acted)
}

/// The day of an ISO date, which is what a card has room for.
fn short_date(iso: &str) -> String {
    iso.split('T').next().unwrap_or(iso).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{Caret, Selection};

    #[test]
    fn a_card_reads_the_day_and_not_the_second() {
        assert_eq!(short_date("2026-09-14T10:22:00Z"), "2026-09-14");
        assert_eq!(short_date("today"), "today");
    }

    #[test]
    fn counts_read_as_words() {
        assert_eq!(plural(1, "change"), "1 change");
        assert_eq!(plural(0, "comment"), "0 comments");
        assert_eq!(plural(3, "comment"), "3 comments");
    }

    #[test]
    fn the_caret_finds_the_comment_it_stands_in() {
        let mut app = Scriva::new();
        app.document.body = vec![wp_model::doc::Block::Paragraph(
            wp_model::doc::Paragraph::of("the quick fox"),
        )];
        let quick = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 4,
            },
            head: Caret {
                paragraph: 0,
                offset: 9,
            },
        };
        let id = crate::revise::add_comment(
            &mut app.document,
            &mut app.history,
            Scope::Body,
            quick,
            "A",
            "A",
            "?",
        );
        app.changed();
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 6,
        });
        assert_eq!(app.comment_at_caret(), Some(id));
        app.selection = Selection::at(Caret {
            paragraph: 0,
            offset: 1,
        });
        assert_eq!(app.comment_at_caret(), None);
    }
}
