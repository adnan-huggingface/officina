//! The find bar: one bar for Find and Replace, which holds the keyboard
//! while it is open and hands it back to the document when it closes.

use super::*;

use crate::app::Keyboard;

impl Scriva {
    /// The find bar across the top of the document.
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
        let with_replace = finder.with_replace;
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

        let panel = egui::Panel::top("scriva-find").show(ui, |ui| {
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
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Find");
                let field = ui.add(
                    egui::TextEdit::singleline(&mut query)
                        .id(query_id)
                        .desired_width(220.0)
                        .hint_text("Find in document"),
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
                if crate::icons::button(
                    ui,
                    crate::icons::Icon::ChevronUp,
                    false,
                    "Previous match (Shift+F3)",
                )
                .clicked()
                {
                    back = true;
                }
                if crate::icons::button(
                    ui,
                    crate::icons::Icon::ChevronDown,
                    false,
                    "Next match (F3)",
                )
                .clicked()
                {
                    forward = true;
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
                // step — "3 matches", "1 of 3", "Replaced 3" — and when the
                // slot followed them, every control after it moved along the
                // bar under a pointer that had not: a click aimed at Replace
                // All pressed Replace, whose first press only finds, and the
                // shorter count it left slid Replace All under the pointer to
                // look as though it had been pressed and done nothing.
                let height = ui.spacing().interact_size.y;
                ui.allocate_ui_with_layout(
                    egui::vec2(COUNT_WIDTH, height),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_width(COUNT_WIDTH);
                        ui.add(egui::Label::new(egui::RichText::new(standing).weak()).truncate());
                    },
                );
                if with_replace {
                    ui.separator();
                    ui.label("Replace with");
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut replacement)
                            .id(replacement_id)
                            .desired_width(180.0),
                    );
                    if ((forward || back) && in_replacement) || (tab && in_query) {
                        field.request_focus();
                    }
                    if ui.button("Replace").clicked() {
                        replace_one = true;
                    }
                    if ui.button("Replace All").clicked() {
                        replace_every = true;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("Close (Esc)").clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(6.0);
            // Measured from inside: a panel nested in the toolbar's panel
            // reports a rectangle of no height, and a bar of no height
            // holds nothing — its Enter went to the document.
            ui.min_rect()
        });

        if let Some(finder) = &mut self.finder {
            if finder.query != query {
                finder.note = None;
            }
            finder.query = query;
            finder.replacement = replacement;
            finder.focus = false;
        }
        // Anything in the bar holding the keyboard is the bar holding it: a
        // button pressed there keeps it, as a dialog's does, rather than
        // passing the next keystroke to a document whose caret is not showing.
        let bar = panel.inner;
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
