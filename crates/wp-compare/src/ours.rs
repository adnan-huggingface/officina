//! Where Scriva put every word of a document.
//!
//! Laid with the application's own shaper rather than the arithmetic one, and
//! through the application's own view rather than a hand-built layout context:
//! the whole worth of this measurement is that it is the page the screen would
//! have shown, and a stand-in for either would answer for a different document.
//! No window and no GPU are involved — see [`fonts`].
//!
//! Three kinds of type are gathered, because Word's rendering has all three:
//! the words of a line, the words a shape *is* — a watermark, a piece of
//! WordArt — and the words inside a pasted diagram, which is a recording of the
//! calls that drew it and not pixels. The last two are placed by
//! [`wp_print::ops`] rather than by arithmetic restated here, so a change to
//! how paper draws them cannot leave this measuring against a page nobody
//! prints. A chart's labels are the one kind left out: drawing them needs the
//! plot as well as the page, and the box a chart sits in is compared as a box.
//!
//! And the page's furniture with them — see [`furniture`]. A rule, a shading
//! and a picture's box are ink too, and until they were gathered a border could
//! move an inch and no number moved with it.

use std::collections::HashMap;
use std::path::Path;

use ui_kit::egui;
use wp_layout::block::{anchor_position, Page, Placed, Placement};
use wp_layout::inline::Content;
use wp_print::ops::Op;

use crate::diff::{Band, Word};
use crate::marks::{Mark, Rect, HAIRLINE};
use crate::Reading;

/// A document, and what is needed to find the bytes of the pictures it draws.
struct Opened {
    document: wp_model::Document,
    package: Option<ooxml::Package>,
    parts: Option<wp_docx::DocumentParts>,
    loose: HashMap<String, Vec<u8>>,
}

/// Every metafile the pages draw, played once, by the name a page draws it by.
type Pictures = HashMap<String, metafile::Picture>;

/// Lays the document out and reports where each of its marks landed.
pub fn read(path: &Path) -> Result<Reading, String> {
    let opened = open(path)?;
    let ctx = fonts();
    let mut shaper = scriva::shaper::Egui::new(&ctx);
    let mut view = scriva::view::View::default();
    view.refresh(
        &opened.document,
        &wp_layout::FieldValues::new(),
        1,
        &mut shaper,
    );

    // The same map the PDF writer is handed, built by the application's own
    // code: a picture is either pixels or a recording, and only the second
    // kind has words in it.
    let pictures = scriva::publish::metafiles(
        opened.package.as_ref(),
        opened.parts.as_ref(),
        &opened.loose,
        view.pages(),
    );

    let mut words = Vec::new();
    let mut marks = Vec::new();
    for page in view.pages() {
        collect(page, &pictures, &mut words);
        furniture(page, &mut marks);
    }
    Ok(Reading { words, marks })
}

/// Every rectangle of ink on a page that is not type.
///
/// Straight out of [`wp_print::ops::flatten`], which is the paper renderer's
/// own account of the page, for the same reason the diagrams' words go through
/// its metafile player: a border that moves on paper has to move here too, and
/// a walk of the page restated in this file would sooner or later be measuring
/// a page nobody prints.
///
/// A picture and a chart are the boxes they were put in, and are not opened.
/// What fills them is not ink the page laid — it is a recording playing, or a
/// chart drawing itself — and [`crate::marks::answered`] is where the two
/// renderings are let off answering for each other about it.
fn furniture(page: &Page, into: &mut Vec<Mark>) {
    for op in wp_print::ops::flatten(page) {
        let (rect, picture) = match op {
            Op::Fill {
                x,
                y,
                width,
                height,
                ..
            } => (Rect::new(x, y, x + width, y + height), false),
            // A stroke is drawn along its centre and lays ink down on either
            // side of it, so the rule a page sees is half the thickness wider
            // than the line the layout asked for — and no longer, because a
            // stroke's cap stops at its end point. `pdfink.py` reduces Word's
            // strokes by the same rule.
            Op::Rule {
                from,
                to,
                thickness,
                ..
            } => {
                let half = thickness.max(HAIRLINE) / 2.0;
                let (x0, x1) = (from.0.min(to.0), from.0.max(to.0));
                let (y0, y1) = (from.1.min(to.1), from.1.max(to.1));
                match x1 - x0 >= y1 - y0 {
                    true => (Rect::new(x0, y0 - half, x1, y1 + half), false),
                    false => (Rect::new(x0 - half, y0, x1 + half, y1), false),
                }
            }
            Op::Image {
                x,
                y,
                width,
                height,
                ..
            }
            | Op::Chart {
                x,
                y,
                width,
                height,
                ..
            } => (Rect::new(x, y, x + width, y + height), true),
            Op::Text { .. } | Op::Poly { .. } => continue,
        };
        into.push(Mark {
            page: page.number,
            rect,
            picture,
        });
    }
}

fn open(path: &Path) -> Result<Opened, String> {
    let legacy = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("doc"));
    match legacy {
        true => {
            let (document, media) =
                wp_doc::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Opened {
                document,
                package: None,
                parts: None,
                // A legacy file has no package: its pictures come out of the
                // stream loose, under the names its drawings ask for.
                loose: media
                    .into_iter()
                    .map(|picture| (picture.rel, picture.data))
                    .collect(),
            })
        }
        false => {
            let (document, package) =
                wp_docx::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let parts = wp_docx::DocumentParts::locate_in(&package).ok();
            Ok(Opened {
                document,
                package: Some(package),
                parts,
                loose: HashMap::new(),
            })
        }
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

fn collect(page: &Page, pictures: &Pictures, into: &mut Vec<Word>) {
    for (band, placements) in [
        (Band::Body, &page.content),
        (Band::Header, &page.header),
        (Band::Footer, &page.footer),
        (Band::Note, &page.footnotes),
    ] {
        for placement in placements {
            words_of(page, band, placement, pictures, into);
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

fn words_of(
    page: &Page,
    band: Band,
    placement: &Placement,
    pictures: &Pictures,
    into: &mut Vec<Word>,
) {
    match &placement.kind {
        Placed::Line { line, .. } => {
            // The baseline, because that is the one horizontal Word's rendering
            // can be asked about without either side having to guess at the
            // other's idea of where a line begins. Same arithmetic as
            // `wp_print::ops::flatten`.
            let baseline = placement.y + line.baseline;
            let mut ink: Vec<Ink> = Vec::new();
            for fragment in &line.fragments {
                // The document's own words and a list's number: both are type,
                // and the comparison is against a rendered page, which has the
                // number on it. A leadered tab's dots are deliberately not
                // gathered — they are a rule that happens to be made of full
                // stops, both sides draw as many as fit rather than a number
                // either of them chose, and counting them as words buries every
                // real difference under them.
                let (text, advances) = match &fragment.content {
                    Content::Text { text, advances, .. } | Content::Label { text, advances } => {
                        (text, advances)
                    }
                    // A diagram pasted into the run of the text: it sits on the
                    // baseline like a very large letter, and what is recorded
                    // inside it is type Word's rendering draws.
                    Content::Object { height, rel, .. } => {
                        if let Some(rel) = rel {
                            inside(
                                page,
                                band,
                                (placement.x + fragment.x, baseline - height),
                                (fragment.width, *height),
                                rel,
                                pictures,
                                into,
                            );
                        }
                        ink.push((' ', 0.0));
                        continue;
                    }
                    _ => {
                        ink.push((' ', 0.0));
                        continue;
                    }
                };
                if text.is_empty() || fragment.style.hidden {
                    ink.push((' ', 0.0));
                    continue;
                }
                let mut x = placement.x + fragment.x + fragment.lead;
                for (index, ch) in text.chars().enumerate() {
                    ink.push((ch, x));
                    x += advances.get(index).copied().unwrap_or(0.0);
                }
            }
            emit(page, band, baseline, &ink, into);
        }
        Placed::Drawing {
            rel, anchor, words, ..
        } => {
            // An anchored drawing floats: where it sits is a question about the
            // page, and `anchor_position` is the answer the screen and the
            // paper both give.
            let (x, y) = match anchor.as_deref() {
                Some(drawing) => {
                    anchor_position(drawing, &page.geometry, (placement.x, placement.y))
                }
                None => (placement.x, placement.y),
            };
            // A shape's own words are deliberately not gathered, and the
            // reason is a measurement rather than a preference: Word draws a
            // WordArt watermark into a PDF as *outlines*, so the rendering has
            // no such text in it at all. `watermark.docx`, `picture-watermark
            // .docx` and the demonstration document all export pages whose only
            // words are the body's. Gathering ours would put sixteen words on
            // one side of the comparison that nothing on the other side could
            // ever answer — the leader-dot mistake in the other direction. A
            // shape whose words Word does export as text would show up as words
            // Word laid and we did not, which is visible rather than silent.
            if let Some(rel) = rel.as_ref().filter(|_| words.is_none()) {
                let (width, height) = (placement.width, placement.height);
                inside(page, band, (x, y), (width, height), rel, pictures, into);
            }
        }
        _ => {}
    }
}

/// The words of a metafile, in the box the page gave it.
///
/// Handed to the paper renderer's own player rather than restating how a
/// recording is scaled into its box: a diagram that moves on paper has to move
/// here too, or this measures a page nobody prints.
///
/// **Each call of the recording is left where it is.** A recording has no
/// words in it, only marks, and it is tempting to join the ones that abut into
/// the word a reader would see. It was tried, and what it produced was a token
/// neither side could answer: a diagram draws its labels in whatever order it
/// pleases, so "SPI" was run together with a "Radio" fifty-three points to its
/// *left*, and the fabricated "SPIRadio" then matched nothing on either side.
/// Where the two sides cut a word differently, `diff::glued` pairs them.
/// Nothing here guesses at a boundary neither renderer wrote down.
fn inside(
    page: &Page,
    band: Band,
    (x, y): (f64, f64),
    (width, height): (f64, f64),
    rel: &str,
    pictures: &Pictures,
    into: &mut Vec<Word>,
) {
    if !pictures.contains_key(rel) {
        return;
    }
    let image = Op::Image {
        x,
        y,
        width,
        height,
        rel: rel.to_string(),
    };
    for op in wp_print::ops::draw_metafiles(vec![image], pictures) {
        let Op::Text {
            x,
            baseline,
            text,
            advances,
            ..
        } = op
        else {
            continue;
        };
        let ink = laid(x, &text, &advances);
        emit(page, band, baseline, &ink, into);
    }
}

/// A run of type as ink, carried along by its own advances.
fn laid(x: f64, text: &str, advances: &[f64]) -> Vec<Ink> {
    let mut ink = Vec::with_capacity(text.chars().count());
    let mut pen = x;
    for (index, ch) in text.chars().enumerate() {
        ink.push((ch, pen));
        pen += advances.get(index).copied().unwrap_or(0.0);
    }
    ink
}

fn emit(page: &Page, band: Band, baseline: f64, ink: &[Ink], into: &mut Vec<Word>) {
    into.extend(split(ink).map(|(x, text)| Word {
        page: page.number,
        band: Some(band),
        x,
        baseline,
        // A symbol font's glyph number is not a character — see `diff::spelled`.
        text: crate::diff::spelled(&text),
    }));
}

/// The ink of one line, cut into words at its whitespace.
///
/// A word is measured at its **left edge** — the leftmost of its characters,
/// not the first of them. For type set left to right the two are the same
/// thing; for a right-to-left run they are opposite ends of the same word, and
/// `pdfink.py` measures the left edge for exactly the same reason.
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
        start = match text.is_empty() {
            true => *x,
            false => start.min(*x),
        };
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

    /// A shape's words and a diagram's arrive as one run and its advances,
    /// rather than as a line's fragments, and are cut up the same way.
    #[test]
    fn a_run_becomes_ink_at_its_own_advances() {
        let words: Vec<_> = split(&laid(100.0, "ab c", &[6.0, 6.0, 3.0, 6.0])).collect();
        assert_eq!(words.len(), 2);
        assert!((words[0].0 - 100.0).abs() < 0.001);
        assert!((words[1].0 - 115.0).abs() < 0.001, "{}", words[1].0);
    }
}
