//! The Word Count box, from the status bar's count: pages, words,
//! characters with and without spaces, paragraphs and lines — and, with a
//! selection, the selection beside the document.

use super::*;

/// The six figures Word's box gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Count {
    pub pages: Option<usize>,
    pub words: usize,
    pub characters: usize,
    pub with_spaces: usize,
    pub paragraphs: usize,
    pub lines: Option<usize>,
}

impl Count {
    /// Counts a text whose paragraphs are separated by newlines.
    pub(crate) fn of_text(text: &str) -> Count {
        let words = text
            .split(|c: char| c.is_whitespace() || c == '/')
            .filter(|word| !word.is_empty())
            .count();
        let with_spaces = text.chars().filter(|c| *c != '\n').count();
        let characters = text.chars().filter(|c| !c.is_whitespace()).count();
        let paragraphs = text.lines().filter(|line| !line.trim().is_empty()).count();
        Count {
            pages: None,
            words,
            characters,
            with_spaces,
            paragraphs,
            lines: None,
        }
    }
}

/// One row of the box: how to read its figure from a count.
type Figure = fn(&Count) -> Option<usize>;

impl Scriva {
    /// The document's figures: the text as the word count reads it, the
    /// pages and lines as the layout laid them.
    pub(crate) fn document_count(&self) -> Count {
        let text: String = self
            .document
            .paragraphs()
            .iter()
            .map(|paragraph| plain_text(paragraph))
            .collect::<Vec<_>>()
            .join("\n");
        let mut count = Count::of_text(&text);
        let pages = self.view.pages();
        count.pages = Some(pages.len().max(1));
        count.lines = Some(
            pages
                .iter()
                .map(|page| {
                    page.content
                        .iter()
                        .filter(|placement| {
                            matches!(placement.kind, wp_layout::block::Placed::Line { .. })
                        })
                        .count()
                })
                .sum(),
        );
        count
    }

    pub(super) fn word_count_dialog(&mut self, ctx: &egui::Context) {
        let document = self.document_count();
        let selection = self.selected_plain_text().map(|text| Count::of_text(&text));
        let mut close = false;
        egui::Modal::new(egui::Id::new("scriva-word-count"))
            .frame(dialog::frame(ctx))
            .show(ctx, |ui| {
                dialog::form_style(ui.style_mut());
                dialog::body(ui, |ui| {
                    ui.set_width(300.0);
                    ui.label(egui::RichText::new("Word Count").font(dialog::heading_font(16.0)));
                    ui.add_space(8.0);
                    let figure = |n: Option<usize>| match n {
                        Some(n) => n.to_string(),
                        None => "\u{2013}".to_owned(),
                    };
                    egui::Grid::new("scriva-word-count-grid")
                        .num_columns(if selection.is_some() { 3 } else { 2 })
                        .spacing([18.0, 6.0])
                        .show(ui, |ui| {
                            if selection.is_some() {
                                ui.label("");
                                ui.label(egui::RichText::new("Selection").strong());
                                ui.label(egui::RichText::new("Document").strong());
                                ui.end_row();
                            }
                            let rows: [(&str, Figure); 6] = [
                                ("Pages", |c| c.pages),
                                ("Words", |c| Some(c.words)),
                                ("Characters (no spaces)", |c| Some(c.characters)),
                                ("Characters (with spaces)", |c| Some(c.with_spaces)),
                                ("Paragraphs", |c| Some(c.paragraphs)),
                                ("Lines", |c| c.lines),
                            ];
                            for (label, read) in rows {
                                ui.label(label);
                                if let Some(selection) = &selection {
                                    ui.label(figure(read(selection)));
                                }
                                ui.label(figure(read(&document)));
                                ui.end_row();
                            }
                        });
                    if dialog::row(ui, |ui| dialog::button(ui, "Close", true).clicked()) {
                        close = true;
                    }
                    close |= dialog::answered(ui).is_some();
                });
            });
        if close {
            self.word_count_up = false;
        }
    }
}

/// A paragraph's text as the count reads it: tabs and breaks as spaces,
/// symbols as their character, nothing for what is not a character.
fn plain_text(paragraph: &Paragraph) -> String {
    use wp_model::doc::Piece;
    let mut text = String::new();
    for run in paragraph.runs() {
        for piece in &run.content {
            match piece {
                Piece::Text(t) => text.push_str(t),
                Piece::Tab | Piece::Break(_) => text.push(' '),
                Piece::Symbol { ch, .. } => text.push(*ch),
                Piece::Hyphen { .. } => text.push('-'),
                _ => {}
            }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::Count;

    #[test]
    fn a_count_reads_words_characters_and_paragraphs() {
        let count = Count::of_text("the quick fox\n\njumps over");
        assert_eq!(count.words, 5);
        assert_eq!(count.characters, 20);
        assert_eq!(count.with_spaces, 23);
        assert_eq!(count.paragraphs, 2);
    }
}
