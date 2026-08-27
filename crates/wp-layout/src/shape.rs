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

/// The two heights Word gives a single-spaced line.
///
/// Word does not lay lines at the font's design height. Each line is laid at a
/// quantized `base` — hinted metrics, a hair off the design value — while an
/// accumulator tracks the exact `ideal` (the hhea sum, scaled). Whenever the
/// two have drifted half a point apart, one line is made half a point taller
/// or shorter to pay the debt. Thirty single-spaced lines of Verdana measure
/// this directly: pitches of 12.083pt with a 12.583pt line every seventh, and
/// the average is the design height to the third decimal.
///
/// A shaper that answers with `base == ideal` opts out: the drift is always
/// zero and every line is its natural height, which is what the fixed test
/// shaper and any font this cannot be measured for do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pitch {
    /// What a line of this font is actually laid at, in points.
    pub base: f64,
    /// What the accumulator counts toward, in points.
    pub ideal: f64,
}

/// The box a string's drawn outline fills, in points, measured from the pen
/// position and the baseline.
///
/// **Not the line box.** `left`/`right` exclude the side bearings the advances
/// carry, and `top`/`bottom` are where the ink of *these letters* reaches
/// rather than where the face says a line begins and ends — so "xxxx" is as
/// tall as an x and "Hg" reaches from the cap to the descender. Word's WordArt
/// fits this box to the shape it is drawn in, which is the only thing that
/// needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ink {
    pub left: f64,
    pub right: f64,
    /// Above the baseline, positive up.
    pub top: f64,
    /// Below the baseline, negative — a string with no descender says a small
    /// positive number, which is the overshoot of a round letter.
    pub bottom: f64,
}

impl Ink {
    pub fn width(&self) -> f64 {
        self.right - self.left
    }

    pub fn height(&self) -> f64 {
        self.top - self.bottom
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

    /// The laid and ideal single-line heights — see [`Pitch`].
    ///
    /// The default answers with the natural height for both, which disables
    /// the half-point dance and keeps every line at its design height.
    fn pitch(&mut self, font: &FontRequest) -> Pitch {
        let metrics = self.metrics(font);
        let natural = metrics.line_height();
        Pitch {
            base: natural,
            ideal: natural,
        }
    }

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

    /// The box the drawn outline of `text` fills — see [`Ink`].
    ///
    /// The default answers `None`, which is a shaper saying it cannot look
    /// inside the glyphs. Every caller must have something to do without it,
    /// because a face whose outlines are not `glyf` — and the fixed test
    /// shaper, which has no glyphs at all — will never answer.
    fn ink(&mut self, text: &str, font: &FontRequest) -> Option<Ink> {
        let _ = (text, font);
        None
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
