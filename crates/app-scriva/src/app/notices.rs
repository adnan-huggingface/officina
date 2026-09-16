//! The three tiers of telling the user something, and the two lighter ones
//! drawn here: a status notice, which is a sentence at the left of the
//! status bar for four seconds, and a notice, which takes the row under the
//! toolbar for a fact about this document and stays until dismissed.
//!
//! A modal is for a question whose wrong answer loses work and for a
//! failure that stops an action; everything else was a modal too, and a box
//! that has to be dismissed to say "no tracked changes" is a box in the
//! way. `dialog::message` keeps the third tier.

use super::*;
use ui_kit::theme;

/// How long a status notice stands, in seconds.
pub(crate) const NOTICE_SECONDS: f64 = 4.0;

/// A fact about this document, in the band under the toolbar, with the
/// one thing that can be done about it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Notice {
    pub text: String,
    /// A button's label and what it runs — `Export…` on the print notice.
    pub action: Option<(String, Command)>,
}

impl Scriva {
    /// Says `text` in the status bar for the next four seconds. A queue of
    /// one: a new notice replaces the old.
    pub(crate) fn say(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), None));
    }

    /// Puts a fact about this document in the band under the toolbar,
    /// where it stays until its `×` or the document is closed.
    pub(crate) fn post_notice(&mut self, text: impl Into<String>, action: Option<(&str, Command)>) {
        let text = text.into();
        if self.notices.iter().any(|notice| notice.text == text) {
            return;
        }
        self.notices.push(Notice {
            text,
            action: action.map(|(label, command)| (label.to_owned(), command)),
        });
    }

    /// The status notice, drawn where the status bar puts it: stamped with
    /// the clock on its first frame, gone four seconds after.
    pub(super) fn status_notice(&mut self, ui: &mut egui::Ui) {
        let Some((text, since)) = self.notice.clone() else {
            return;
        };
        let now = ui.input(|i| i.time);
        let since = since.unwrap_or(now);
        let remaining = NOTICE_SECONDS - (now - since);
        if remaining <= 0.0 {
            self.notice = None;
            return;
        }
        self.notice = Some((text.clone(), Some(since)));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(remaining.max(0.05)));
        ui.add(
            egui::Label::new(
                egui::RichText::new(text)
                    .size(theme::TEXT)
                    .color(theme::INK_SOFT),
            )
            .truncate(),
        );
    }

    /// The notice, on the row under the toolbar: the oldest one still
    /// standing, with its action and its way out; the next is behind it.
    pub(super) fn notice_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let notice = self.notices.first().cloned()?;
        let row = ui.max_rect();
        ui.painter().rect_filled(row, 0.0, theme::NOTICE);
        ui.painter().hline(
            row.x_range(),
            row.bottom() - 0.5,
            egui::Stroke::new(1.0, theme::NOTICE_RULE),
        );
        let mut chosen = None;
        let mut dismissed = false;
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.add_space(10.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(&notice.text)
                    .size(theme::TEXT)
                    .color(theme::INK),
            )
            .truncate(),
        );
        if let Some((label, command)) = &notice.action {
            if ui
                .add(egui::Button::new(
                    egui::RichText::new(label)
                        .size(theme::TEXT)
                        .color(theme::ACCENT),
                ))
                .clicked()
            {
                chosen = Some(command.clone());
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(4.0);
            let (rect, response) = ui.allocate_exact_size(theme::TARGET, egui::Sense::click());
            crate::icons::paint_state(ui, rect, &response, false);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "\u{00d7}",
                egui::FontId::proportional(theme::TEXT + 2.0),
                theme::INK,
            );
            if response.on_hover_text("Dismiss").clicked() {
                dismissed = true;
            }
        });
        if dismissed {
            self.notices.remove(0);
        }
        chosen
    }
}

/// A count with thousands separated, as the status bar prints one.
pub(crate) fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_count_is_printed_with_its_thousands_separated() {
        assert_eq!(super::thousands(0), "0");
        assert_eq!(super::thousands(999), "999");
        assert_eq!(super::thousands(1204), "1,204");
        assert_eq!(super::thousands(1_000_000), "1,000,000");
    }
}
