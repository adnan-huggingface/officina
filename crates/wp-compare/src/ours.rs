//! Where Scriva put every word of a document.
//!
//! Laid with the application's own shaper rather than the arithmetic one, and
//! through the application's own view rather than a hand-built layout context:
//! the whole worth of this measurement is that it is the page the screen would
//! have shown, and a stand-in for either would answer for a different document.
//! No window and no GPU are involved — see [`fonts`].

use std::path::Path;

use ui_kit::egui;
use wp_layout::block::{Page, Placed, Placement};
use wp_layout::inline::Content;

use crate::diff::{Band, Word};

/// Lays the document out and reports where each of its words landed.
pub fn read(path: &Path) -> Result<Vec<Word>, String> {
    let document = open(path)?;
    let ctx = fonts();
    let mut shaper = scriva::shaper::Egui::new(&ctx);
    let mut view = scriva::view::View::default();
    view.refresh(&document, &wp_layout::FieldValues::new(), 1, &mut shaper);

    let mut words = Vec::new();
    for page in view.pages() {
        collect(page, &mut words);
    }
    Ok(words)
}

fn open(path: &Path) -> Result<wp_model::Document, String> {
    let legacy = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("doc"));
    match legacy {
        true => wp_doc::open(path)
            .map(|(document, _media)| document)
            .map_err(|e| format!("{}: {e}", path.display())),
        false => wp_docx::open(path)
            .map(|(document, _package)| document)
            .map_err(|e| format!("{}: {e}", path.display())),
    }
}

/// An egui context with the machine's real faces and nothing else.
///
/// The same preparation the `anchors` test does, and for the same reason: a
/// position is a statement about metrics, so the faces have to be the ones the
/// application draws with. egui has no fonts until a frame has run, and the
/// texture deltas of that frame have to be dropped deliberately because there
/// is no GPU here to apply them to.
fn fonts() -> egui::Context {
    let ctx = egui::Context::default();
    ui_kit::fonts::install(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
    out.textures_delta.clear();
    ctx
}

fn collect(page: &Page, into: &mut Vec<Word>) {
    for (band, placements) in [
        (Band::Body, &page.content),
        (Band::Header, &page.header),
        (Band::Footer, &page.footer),
        (Band::Note, &page.footnotes),
    ] {
        for placement in placements {
            words_of(page.number, band, placement, into);
        }
    }
}

/// One character of a line, and the left edge it was drawn at.
///
/// A word is gathered from these rather than from any one fragment, because a
/// fragment is a stretch of one *format* — a word with one bold letter in the
/// middle of it is three fragments and still one word, and Word counts it as
/// one.
type Ink = (char, f64);

fn words_of(page: u32, band: Band, placement: &Placement, into: &mut Vec<Word>) {
    let Placed::Line { line, .. } = &placement.kind else {
        return;
    };

    let mut ink: Vec<Ink> = Vec::new();
    for fragment in &line.fragments {
        // The document's own words and a list's number: both are type, and the
        // comparison is against a rendered page, which has the number on it. A
        // leadered tab's dots are deliberately not here — they are a rule that
        // happens to be made of full stops, both sides draw as many as fit
        // rather than a number either of them chose, and counting them as
        // words buries every real difference under them. Everything else — an
        // inline drawing, a chart — is not type at all, and still separates
        // the words on either side of it.
        let (text, advances) = match &fragment.content {
            Content::Text { text, advances, .. } | Content::Label { text, advances } => {
                (text, advances)
            }
            _ => {
                ink.push((' ', 0.0));
                continue;
            }
        };
        if text.is_empty() {
            ink.push((' ', 0.0));
            continue;
        }
        if fragment.style.hidden {
            ink.push((' ', 0.0));
            continue;
        }
        let mut x = placement.x + fragment.x + fragment.lead;
        for (index, ch) in text.chars().enumerate() {
            ink.push((ch, x));
            x += advances.get(index).copied().unwrap_or(0.0);
        }
    }

    // The baseline, because that is the one horizontal Word's rendering can be
    // asked about without either side having to guess at the other's idea of
    // where a line begins. Same arithmetic as `wp_print::ops::flatten`.
    let baseline = placement.y + line.baseline;
    into.extend(split(&ink).map(|(x, text)| Word {
        page,
        band: Some(band),
        x,
        baseline,
        text,
    }));
}

/// The ink of one line, cut into words at its whitespace.
///
/// A word keeps the left edge of its *first* character, which is where the pen
/// was put down — the same thing a rendered page reports as that character's
/// origin, and so the same thing on both sides of the comparison.
fn split(ink: &[Ink]) -> impl Iterator<Item = (f64, String)> + '_ {
    let mut words = Vec::new();
    let mut text = String::new();
    let mut start = 0.0;
    for (ch, x) in ink {
        if ch.is_whitespace() {
            if !text.is_empty() {
                words.push((start, std::mem::take(&mut text)));
            }
            continue;
        }
        if text.is_empty() {
            start = *x;
        }
        text.push(*ch);
    }
    if !text.is_empty() {
        words.push((start, text));
    }
    words.into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ink(text: &str, from: f64, advance: f64) -> Vec<Ink> {
        text.chars()
            .enumerate()
            .map(|(index, ch)| (ch, from + advance * index as f64))
            .collect()
    }

    #[test]
    fn a_line_is_cut_into_words_at_its_whitespace() {
        let words: Vec<_> = split(&ink("media options,", 72.0, 5.0)).collect();
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].1, "media");
        assert_eq!(words[1].1, "options,");
        assert!((words[0].0 - 72.0).abs() < 0.001);
        // Six characters in, at five points each.
        assert!((words[1].0 - 102.0).abs() < 0.001, "{}", words[1].0);
    }

    /// A fragment is a stretch of one format, not a word: the run boundary in
    /// the middle of a bold letter must not become a word boundary.
    #[test]
    fn a_word_split_across_two_formats_is_still_one_word() {
        // "Chamber" in one format, "lain" in another: two fragments, no space.
        let mut letters = ink("Chamber", 72.0, 5.0);
        letters.extend(ink("lain", 107.0, 5.0));
        let words: Vec<_> = split(&letters).collect();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].1, "Chamberlain");
        assert!((words[0].0 - 72.0).abs() < 0.001);
    }

    /// A tab or an inline drawing arrives as a break with no width, and parts
    /// what is on either side of it.
    #[test]
    fn something_that_is_not_type_parts_the_words_around_it() {
        let mut letters = ink("13.1.", 72.0, 5.0);
        letters.push((' ', 0.0));
        letters.extend(ink("Scope", 144.0, 5.0));
        let words: Vec<_> = split(&letters).collect();
        assert_eq!(words.len(), 2);
        assert_eq!(words[1].1, "Scope");
        assert!((words[1].0 - 144.0).abs() < 0.001);
    }
}
