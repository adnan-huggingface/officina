//! Edit ▸ Go To… (Ctrl+G): a small popover on the status bar's page count.
//! `Page [ ] of 12`, Enter goes; `+3` and `-2` are relative; a heading can
//! be picked from a list. Not a modal — it is a question about where to
//! stand, and the page should stay in sight while it is asked.

use super::*;
use ui_kit::menu;

impl Scriva {
    /// Draws the popover under the status bar's page label while it is
    /// open, and does what it is answered with.
    pub(super) fn goto_popover(&mut self, ui: &mut egui::Ui, anchor: &egui::Response) {
        let Some(mut draft) = self.goto.clone() else {
            return;
        };
        let pages = self.view.pages().len().max(1);
        let here = self.caret_page() + 1;
        let mut answer: Option<Option<String>> = None;
        let mut heading: Option<usize> = None;
        let headings = wp_model::outline::headings(&self.document);
        let shown = egui::Popup::from_response(anchor)
            .id(egui::Id::new("scriva-goto"))
            .kind(egui::PopupKind::Popup)
            .open(true)
            .align(egui::RectAlign::TOP_START)
            .frame(dialog::frame(ui.ctx()))
            .show(|ui| {
                dialog::form_style(ui.style_mut());
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label("Page");
                            dialog::first_field(ui, "scriva-goto", &mut draft, 48.0);
                            ui.label(format!("of {pages}"));
                            ui.add_space(6.0);
                            let list =
                                ui.add_enabled(!headings.is_empty(), egui::Button::new("Heading…"));
                            if let Some(Some(index)) = menu::under(&list, |ui| {
                                let mut picked = None;
                                ui_kit::scroll::show(
                                    ui,
                                    egui::ScrollArea::vertical().max_height(320.0),
                                    |ui| {
                                        for (index, heading) in headings.iter().enumerate() {
                                            let label = format!(
                                                "{}{}",
                                                "   ".repeat(
                                                    heading.level.saturating_sub(1) as usize
                                                ),
                                                heading.text
                                            );
                                            if menu::item(ui, &label, "").clicked() {
                                                picked = Some(index);
                                            }
                                        }
                                    },
                                );
                                picked
                            }) {
                                heading = Some(index);
                            }
                            // Enter goes, Escape closes; a heading's row is
                            // its own answer. Taken from the input so that the
                            // document does not see them.
                            // The field is the only thing to type into, so
                            // Enter anywhere in the popover means go — and
                            // the field has already let its focus go by the
                            // time Enter can be read, as a single-line field
                            // does on Enter.
                            let (enter, escape) = ui.input_mut(|i| {
                                (
                                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                                    i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                                )
                            });
                            if enter {
                                answer = Some(Some(draft.clone()));
                            } else if escape {
                                answer = Some(None);
                            }
                        });
                        ui.label(
                            egui::RichText::new(format!(
                                "You are on page {here}. +3 or -2 moves from here."
                            ))
                            .size(ui_kit::theme::TEXT_SMALL)
                            .color(ui_kit::theme::INK_SOFT),
                        );
                    });
            });
        // A click anywhere else is the other way to say no.
        if let Some(shown) = &shown {
            if shown.response.clicked_elsewhere() && !egui::Popup::is_any_open(ui.ctx()) {
                answer.get_or_insert(None);
            }
        }
        self.goto = Some(draft);
        if let Some(index) = heading {
            self.goto = None;
            if let Some(heading) = headings.get(index) {
                self.run(Command::GoTo(wp_model::Scope::Body, heading.paragraph));
            }
            return;
        }
        match answer {
            Some(Some(text)) => {
                self.goto = None;
                match page_asked(&text, here, pages) {
                    Some(page) => self.go_to_page(page),
                    None => {
                        self.say(format!(
                            "No such page: type 1 to {pages}, or +3 or -2 to move from here"
                        ));
                    }
                }
            }
            Some(None) => self.goto = None,
            None => {}
        }
    }

    /// The caret to the first line of page `page`, one-based, in the text.
    pub(super) fn go_to_page(&mut self, page: usize) {
        let index = page.saturating_sub(1);
        let Some(laid) = self.view.pages().get(index) else {
            return;
        };
        let spot = view::Spot {
            page: index,
            x: laid.geometry.start,
            y: laid.geometry.top,
        };
        self.close_band();
        let Some(caret) = view::caret_at(&self.view, wp_model::Scope::Body, spot) else {
            return;
        };
        self.go_to(wp_model::Scope::Body, caret);
        self.reveal = Some(caret);
        self.reveal_on = Some(index);
    }
}

/// The page a Go To answer names: a number, or a step from `here`.
fn page_asked(text: &str, here: usize, pages: usize) -> Option<usize> {
    let text = text.trim();
    let page = match text.chars().next()? {
        '+' => here.checked_add(text[1..].trim().parse::<usize>().ok()?)?,
        '-' => here.checked_sub(text[1..].trim().parse::<usize>().ok()?)?,
        _ => text.parse::<usize>().ok()?,
    };
    (1..=pages).contains(&page).then_some(page)
}

#[cfg(test)]
mod tests {
    use super::page_asked;

    #[test]
    fn a_page_is_asked_for_by_number_or_by_a_step_from_here() {
        assert_eq!(page_asked("5", 2, 12), Some(5));
        assert_eq!(page_asked(" +3 ", 2, 12), Some(5));
        assert_eq!(page_asked("-2", 5, 12), Some(3));
        assert_eq!(page_asked("-9", 5, 12), None);
        assert_eq!(page_asked("13", 5, 12), None);
        assert_eq!(page_asked("0", 5, 12), None);
        assert_eq!(page_asked("five", 5, 12), None);
    }
}
