//! What the layout engine needs from a font, and nothing more.
//!
//! Measurement is a *trait* rather than a dependency. Two reasons, and the
//! second is the important one.
//!
//! The obvious one: this crate would otherwise have to choose a font stack, and
//! the application already has one — the same `epaint` faces the spreadsheet
//! draws with, registered from the machine's own font files.
//!
//! The one that matters: **a layout engine tested against a real font is tested
//! against a moving target.** Line breaks would change with the font version,
//! the hinting, and the machine. [`Fixed`] is a shaper whose every glyph is
//! exactly half its point size, so a test can say "this line holds eleven
//! characters" and mean it. `LEARNINGS.md` §5 — ask the laid-out result, and be
//! able to.

use std::sync::Arc;

/// A face, at a size, in a weight and a slope.
#[derive(Debug, Clone, PartialEq)]
pub struct FontRequest {
    pub family: Arc<str>,
    /// Points.
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
}

impl FontRequest {
    pub fn new(family: impl Into<Arc<str>>, size: f64) -> FontRequest {
        FontRequest {
            family: family.into(),
            size,
            bold: false,
            italic: false,
        }
    }

    /// The size a superscript or subscript is actually drawn at.
    ///
    /// Word stores no size for these — `<w:vertAlign>` is a position — and
    /// shrinks the glyphs itself. The ratio is Word's own.
    pub fn shrunk(&self) -> FontRequest {
        FontRequest {
            size: self.size * 0.65,
            ..self.clone()
        }
    }
}

/// A face's vertical metrics at a given size, in points.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Metrics {
    /// Distance from the baseline to the top of the tallest glyph, positive up.
    pub ascent: f64,
    /// Baseline to the bottom, positive down.
    pub descent: f64,
    /// The face's recommended extra space between lines.
    pub line_gap: f64,
}

impl Metrics {
    /// The natural height of one line in this face — what `w:lineRule="auto"`
    /// multiplies.
    pub fn line_height(&self) -> f64 {
        self.ascent + self.descent + self.line_gap
    }
}

/// One character's contribution to the width of a string.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Advance {
    /// Byte offset of the character within the string that was measured.
    pub offset: usize,
    /// How far the pen moves, in points.
    pub width: f64,
}

/// Everything the layout engine asks of a font.
///
/// Deliberately small. Kerning, ligatures and bidi reordering all live below
/// this line, inside whatever implements it — which is why the trait talks about
/// *advances* per character rather than about glyphs: the caret has to land
/// between characters of the document, not between glyphs of a font.
pub trait Shaper {
    fn metrics(&mut self, font: &FontRequest) -> Metrics;

    /// Appends one entry per character of `text`.
    ///
    /// Appends rather than returns so a caller measuring a paragraph reuses one
    /// buffer instead of allocating per run.
    fn advances(&mut self, text: &str, font: &FontRequest, into: &mut Vec<Advance>);

    /// Total width of `text`.
    fn width(&mut self, text: &str, font: &FontRequest) -> f64 {
        let mut buffer = Vec::new();
        self.advances(text, font, &mut buffer);
        buffer.iter().map(|a| a.width).sum()
    }
}

/// A shaper with no fonts, for tests.
///
/// Every character is half the point size wide and every face has the same
/// metrics, so a line's contents are arithmetic rather than typography. A test
/// written against this says something about the *engine*; one written against a
/// real face says something about the face.
#[derive(Debug, Clone, Copy, Default)]
pub struct Fixed;

impl Shaper for Fixed {
    fn metrics(&mut self, font: &FontRequest) -> Metrics {
        // Quarters rather than fifths: 0.75 and 0.25 are exact in binary, so
        // `ascent + descent == size` and a test may compare with `==`.
        Metrics {
            ascent: font.size * 0.75,
            descent: font.size * 0.25,
            line_gap: 0.0,
        }
    }

    fn advances(&mut self, text: &str, font: &FontRequest, into: &mut Vec<Advance>) {
        for (offset, _) in text.char_indices() {
            into.push(Advance {
                offset,
                width: font.size * 0.5,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixed_shaper_makes_a_lines_contents_arithmetic() {
        let mut shaper = Fixed;
        let font = FontRequest::new("any", 12.0);
        assert_eq!(shaper.width("hello", &font), 30.0);
        assert_eq!(shaper.metrics(&font).line_height(), 12.0);
    }

    #[test]
    fn advances_are_byte_offsets_so_a_caret_can_land_between_characters() {
        let mut shaper = Fixed;
        let font = FontRequest::new("any", 10.0);
        let mut out = Vec::new();
        // Three characters, five bytes: the middle one is two bytes wide in
        // UTF-8 and one character wide on the page.
        shaper.advances("aéb", &font, &mut out);
        assert_eq!(out.len(), 3);
        assert_eq!(out.iter().map(|a| a.offset).collect::<Vec<_>>(), [0, 1, 3]);
    }

    #[test]
    fn a_superscript_is_smaller_without_the_document_saying_so() {
        let font = FontRequest::new("Calibri", 11.0);
        assert!(font.shrunk().size < font.size);
        assert_eq!(font.shrunk().family, font.family);
    }
}
