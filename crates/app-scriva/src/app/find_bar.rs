//! The find bar: one bar for Find and Replace, on the row under the
//! toolbar, which holds the keyboard while it is open and hands it back to
//! the document when it closes.
//!
//! First the search: a glass, the field, how many and which, the two
//! arrows, the two switches — match case and whole word. Then, when Replace
//! is open, after a hairline on the same row: the replacement and its two
//! buttons. At the far end, the way into Replace and the way out. One row,
//! not two, because the row is the toolbar's and a second one would push
//! the page down. Enter is the next match and
//! Shift+Enter the one before, as F3 and Shift+F3 are; Tab goes between the
//! fields; Escape gives the keyboard back to the document and leaves the bar
//! open, and the document's own Escape then closes it.

use super::*;

use crate::app::Keyboard;
use crate::icons::{self, Icon};
use ui_kit::theme;

impl Scriva {
    /// The find bar, and whether it held the keyboard at any point this frame.
    ///
    /// *At any point*, not at the end: a key that moves the focus out of the
    /// bar arrives in the same frame it leaves, and the document must not have
    /// it too. Tab did exactly that — the focus went on to the arrow buttons,
    /// the bar said it no longer held the keyboard, and the same Tab was typed
    /// over the match the search had just selected.
    pub(super) fn find_bar(&mut self, ui: &mut egui::Ui) -> bool {
        // Wide enough for "Replaced 1000" in the bar's type.
        const COUNT_WIDTH: f32 = 96.0;
        self.refresh_matches();
        let total = self.find_matches.len();
        let current = self.find_matches.iter().position(|(scope, found)| {
            *scope == self.scope && found.ordered() == self.selection.ordered()
        });

        let Some(finder) = &self.finder else {
            return false;
        };
        let mut query = finder.query.clone();
        let mut replacement = finder.replacement.clone();
        let mut options = finder.options;
        let mut with_replace = finder.with_replace;
        let take_focus = finder.focus;
        let note = finder.note.clone();
        let bar_focused = self.finder_focused;
        let query_id = egui::Id::new("scriva-find-query");
        let replacement_id = egui::Id::new("scriva-find-replacement");
        let focused = ui.memory(|m| m.focused());
        let in_query = focused == Some(query_id);
        let in_replacement = with_replace && focused == Some(replacement_id);

        let mut close = false;
        let mut leave = false;
        let mut forward = false;
        let mut back = false;
        let mut replace_one = false;
        let mut replace_every = false;
        let mut tab = false;

        // Drawn straight onto the row under the toolbar, which is a frame's
        // child and not a nested panel: a panel put inside the toolbar's
        // content-sized panel is given no height, and clips everything in
        // it to nothing — the bar answered every key and was never on the
        // screen (LEARNINGS.md).
        let bar = ui.max_rect();
        // Read before the fields are drawn: a TextEdit consumes the Escape
        // and the Enter it is given, and by then the answer is gone.
        if bar_focused {
            let (escape, enter, f3, shift) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Escape),
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::F3),
                    i.modifiers.shift,
                )
            });
            if escape {
                leave = true;
            }
            if (enter && (in_query || in_replacement)) || f3 {
                if shift {
                    back = true;
                } else {
                    forward = true;
                }
            }
        }
        // Tab goes between the two fields, as it goes between a dialog's.
        // egui would hand the keyboard to the next widget along — an arrow
        // button — so its move is called off, and the key is taken so that
        // nothing after the bar sees it.
        if in_query || in_replacement {
            tab = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
            if tab {
                ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
            }
        }
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_space(8.0);
        let (glass, _) = ui.allocate_exact_size(
            egui::vec2(theme::ICON, theme::TARGET.y),
            egui::Sense::hover(),
        );
        icons::draw(ui.painter(), Icon::Find, glass.center(), theme::INK_SOFT);
        let field = ui.add(
            egui::TextEdit::singleline(&mut query)
                .id(query_id)
                .desired_width(220.0)
                .hint_text("Find"),
        );
        // Enter steps to the next match and leaves the keyboard in the
        // field it was pressed in; a single-line field gives it up on
        // Enter, so it is asked for back.
        if take_focus
            || ((forward || back) && in_query)
            || (tab && (in_replacement || !with_replace))
        {
            field.request_focus();
        }
        let standing = match &note {
            Some(note) => note.clone(),
            None if query.is_empty() => String::new(),
            None => match (current, total) {
                (Some(index), _) => format!("{} of {total}", index + 1),
                (None, 0) => "No matches".to_owned(),
                (None, 1) => "1 match".to_owned(),
                (None, n) => format!("{n} matches"),
            },
        };
        // A slot of its own width. The count's words change with every
        // step — "3 matches", "1 of 3", "Replaced 3" — and when the slot
        // followed them, every control after it moved along the bar under
        // a pointer that had not.
        ui.allocate_ui_with_layout(
            egui::vec2(COUNT_WIDTH, theme::TARGET.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(COUNT_WIDTH);
                ui.add(
                    egui::Label::new(egui::RichText::new(standing).color(theme::INK_SOFT))
                        .truncate(),
                );
            },
        );
        if icons::button(ui, Icon::ChevronUp, false, "Previous match  Shift+F3").clicked() {
            back = true;
        }
        if icons::button(ui, Icon::ChevronDown, false, "Next match  F3").clicked() {
            forward = true;
        }
        if switch(ui, "Aa", options.match_case, "Match case").clicked() {
            options.match_case = !options.match_case;
        }
        if switch(ui, "ab", options.whole_word, "Whole words only").clicked() {
            options.whole_word = !options.whole_word;
        }
        // Replace, when it is open, goes on along the same row after a
        // hairline rather than on a row of its own: a second row would
        // push the page down, and the row is wide enough for both.
        if with_replace {
            super::strips::divider(ui);
            let field = ui.add(
                egui::TextEdit::singleline(&mut replacement)
                    .id(replacement_id)
                    .desired_width(220.0)
                    .hint_text("Replace with"),
            );
            if ((forward || back) && in_replacement) || (tab && in_query) {
                field.request_focus();
            }
            if switch(ui, "Replace", false, "Replace this match and find the next").clicked() {
                replace_one = true;
            }
            if switch(ui, "Replace All", false, "Replace every match").clicked() {
                replace_every = true;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add_space(8.0);
            if switch(ui, "\u{00d7}", false, "Close  Esc").clicked() {
                close = true;
            }
            if switch(ui, "Replace", with_replace, "Replace  Ctrl+H").clicked() {
                with_replace = !with_replace;
            }
        });

        if let Some(finder) = &mut self.finder {
            if finder.query != query || finder.options != options {
                finder.note = None;
            }
            finder.query = query;
            finder.replacement = replacement;
            finder.options = options;
            finder.with_replace = with_replace;
            finder.focus = false;
        }
        // Anything in the bar holding the keyboard is the bar holding it: a
        // button pressed there keeps it, as a dialog's does, rather than
        // passing the next keystroke to a document whose caret is not showing.
        let focused_now = ui
            .memory(|m| m.focused())
            .and_then(|id| ui.ctx().read_response(id))
            .is_some_and(|widget| bar.contains_rect(widget.rect));
        self.finder_focused = focused_now;
        if focused_now {
            self.keyboard = Keyboard::Find;
        } else if self.keyboard == Keyboard::Find {
            self.keyboard = Keyboard::Document;
        }
        let held = bar_focused || focused_now || tab;
        // Escape leaves the bar open and gives the keyboard back to the
        // document: from anywhere but the document, Escape returns to it and
        // closes nothing; the document's own Escape then closes the bar.
        if leave {
            self.give_keyboard(Keyboard::Document, ui.ctx());
            return held;
        }
        if close {
            self.finder = None;
            self.finder_focused = false;
            if let Some(id) = self.surface_id {
                ui.ctx().memory_mut(|m| m.request_focus(id));
            }
            return held;
        }
        if replace_one {
            self.replace_current();
        }
        if replace_every {
            self.replace_all();
        }
        if forward {
            self.jump_match(true);
        }
        if back {
            self.jump_match(false);
        }
        held
    }
}

/// A small flat toggle with a word on it — `Aa`, `ab`, `Replace` — lit when
/// on, as the toolbar's toggles are.
fn switch(ui: &mut egui::Ui, label: &str, on: bool, tip: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(theme::TEXT),
        theme::INK,
    );
    let width = (galley.size().x + 14.0).max(theme::TARGET.x);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, theme::TARGET.y), egui::Sense::click());
    icons::paint_state(ui, rect, &response, on);
    let at = egui::pos2(
        rect.center().x - galley.size().x / 2.0,
        rect.center().y - galley.size().y / 2.0,
    );
    ui.painter().galley(at, galley, theme::INK);
    response.on_hover_text(tip)
}
