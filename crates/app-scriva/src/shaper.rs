//! Measuring text with the faces the application actually has.
//!
//! `wp-layout` asks for advances and metrics and knows nothing about fonts; this
//! answers with epaint's, which are the machine's own files registered by
//! `ui_kit::fonts`. The same faces the spreadsheet draws with.
//!
//! **Every glyph width is cached.** A page of prose asks for a few thousand of
//! them and a document asks per page, per frame; epaint's own lookup goes
//! through a lock and a hash of the whole font id, and doing that afresh for
//! every character of every line is the difference between scrolling and
//! stuttering.

use std::collections::HashMap;

use ui_kit::egui;
use wp_layout::shape::{Advance, FontRequest, Metrics, Shaper};

/// How many measured strings to keep before starting again.
///
/// A document reuses its words, so the cache hits hard and stays small; the
/// ceiling is there so that a hundred pages of distinct tokens cannot grow it
/// without bound.
const CACHE_LIMIT: usize = 20_000;

/// A shaper over an egui context.
pub struct Egui {
    ctx: egui::Context,
    /// Measured per *string* rather than per character, so the face's kerning is
    /// kept. The layout engine asks in break units — words — and a document
    /// repeats its words, so this hits far more often than it misses.
    runs: HashMap<(Key, String), Vec<f64>>,
    metrics: HashMap<Key, Metrics>,
}

/// A font, reduced to what epaint distinguishes.
///
/// The size is quantised to a twentieth of a point so that a zoomed view does
/// not miss the cache on every frame — the difference is far below what a
/// pixel can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    /// The three families, as a number: `ui_kit::Family` is not hashable, and
    /// making it so would be a change to a crate this one only reads from.
    family: u8,
    size: u32,
    bold: bool,
    italic: bool,
}

impl Key {
    fn of(font: &FontRequest) -> Key {
        Key {
            family: match ui_kit::Family::of(&font.family) {
                ui_kit::Family::Sans => 0,
                ui_kit::Family::Serif => 1,
                ui_kit::Family::Mono => 2,
            },
            size: (font.size * 20.0).round().max(1.0) as u32,
            bold: font.bold,
            italic: font.italic,
        }
    }

    fn family(self) -> ui_kit::Family {
        match self.family {
            1 => ui_kit::Family::Serif,
            2 => ui_kit::Family::Mono,
            _ => ui_kit::Family::Sans,
        }
    }

    fn font_id(self) -> egui::FontId {
        egui::FontId::new(
            self.size as f32 / 20.0,
            ui_kit::fonts::face(self.family(), self.bold, self.italic),
        )
    }
}

impl Egui {
    pub fn new(ctx: &egui::Context) -> Egui {
        Egui {
            ctx: ctx.clone(),
            runs: HashMap::new(),
            metrics: HashMap::new(),
        }
    }

    /// The epaint font a request resolves to, for a painter.
    pub fn font_id(&self, font: &FontRequest) -> egui::FontId {
        Key::of(font).font_id()
    }

    /// The advance of each character of `text`, measured together so the face's
    /// kerning applies.
    fn measure(&mut self, key: Key, text: &str) -> &[f64] {
        if !self.runs.contains_key(&(key, text.to_owned())) {
            if self.runs.len() >= CACHE_LIMIT {
                self.runs.clear();
            }
            let id = key.font_id();
            // `fonts_mut`: laying text out fills epaint's own caches, so the
            // read is a write.
            let galley = self
                .ctx
                .fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), id, egui::Color32::BLACK));
            let mut widths = Vec::with_capacity(text.chars().count());
            for row in &galley.rows {
                for glyph in &row.glyphs {
                    widths.push(glyph.advance_width as f64);
                }
            }
            // A glyph is not a character: a ligature is one glyph for two, and a
            // combining mark is a glyph of no width. The caret has to land
            // between *characters*, so a mismatch is filled from the total
            // rather than left to put the caret in the wrong place.
            let count = text.chars().count();
            if widths.len() != count {
                let total: f64 = widths.iter().sum();
                let each = if count > 0 { total / count as f64 } else { 0.0 };
                widths = vec![each; count];
            }
            self.runs.insert((key, text.to_owned()), widths);
        }
        &self.runs[&(key, text.to_owned())]
    }
}

impl Shaper for Egui {
    fn metrics(&mut self, font: &FontRequest) -> Metrics {
        let key = Key::of(font);
        if let Some(&metrics) = self.metrics.get(&key) {
            return metrics;
        }
        let id = key.font_id();
        let height = self.ctx.fonts_mut(|fonts| fonts.row_height(&id)) as f64;
        // epaint reports one number for a row and does not expose the face's own
        // ascent and descent. The split is Word's usual proportion for a text
        // face, and it decides where a baseline sits rather than how tall a line
        // is — which is what `row_height` already answers.
        let metrics = Metrics {
            ascent: height * 0.8,
            descent: height * 0.2,
            line_gap: 0.0,
        };
        self.metrics.insert(key, metrics);
        metrics
    }

    fn advances(&mut self, text: &str, font: &FontRequest, into: &mut Vec<Advance>) {
        let key = Key::of(font);
        let widths = self.measure(key, text).to_vec();
        for (index, (offset, _)) in text.char_indices().enumerate() {
            into.push(Advance {
                offset,
                width: widths.get(index).copied().unwrap_or(0.0),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A context with the *names* registered but no font files read — epaint
    /// refuses to substitute for a family it has never heard of, and reading a
    /// hundred megabytes of type is not what this is testing.
    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        ui_kit::fonts::register(&ctx, &[]);
        // egui has no fonts until a frame has been run, and a shaper that asks
        // before then panics rather than measuring.
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        // epaint panics if a frame's texture deltas are dropped unapplied —
        // there is no GPU here to apply them to.
        out.textures_delta.clear();
        ctx
    }

    #[test]
    fn a_font_request_becomes_the_face_the_application_registered() {
        let ctx = context();
        let shaper = Egui::new(&ctx);
        let id = shaper.font_id(&FontRequest {
            family: "Times New Roman".into(),
            size: 12.0,
            bold: true,
            italic: false,
        });
        assert_eq!(id.size, 12.0);
        assert_eq!(
            id.family,
            ui_kit::fonts::face(ui_kit::Family::Serif, true, false)
        );
    }

    #[test]
    fn measuring_the_same_text_twice_asks_the_font_once() {
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let font = FontRequest::new("Arial", 11.0);
        let first = shaper.width("hello world", &font);
        let cached = shaper.runs.len();
        let second = shaper.width("hello world", &font);
        assert_eq!(first, second);
        assert_eq!(shaper.runs.len(), cached, "nothing new was measured");
        assert!(first > 0.0, "and it measured something");
    }

    #[test]
    fn a_size_that_differs_below_a_twentieth_of_a_point_is_the_same_font() {
        // A zoomed view produces sizes that differ in the seventh decimal place,
        // and missing the cache on every frame is the difference between
        // scrolling and stuttering.
        let a = Key::of(&FontRequest::new("Arial", 11.000_001));
        let b = Key::of(&FontRequest::new("Arial", 11.0));
        assert_eq!(a, b);
        assert_ne!(a, Key::of(&FontRequest::new("Arial", 11.5)));
    }

    #[test]
    fn a_line_is_as_tall_as_the_face_says_and_the_baseline_sits_inside_it() {
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let metrics = shaper.metrics(&FontRequest::new("Arial", 12.0));
        assert!(metrics.line_height() > 0.0);
        assert!(metrics.ascent > metrics.descent);
        assert!((metrics.ascent + metrics.descent - metrics.line_height()).abs() < 1e-9);
    }

    #[test]
    fn advances_are_one_per_character_and_byte_indexed() {
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let mut out = Vec::new();
        shaper.advances("aéb", &FontRequest::new("Arial", 10.0), &mut out);
        assert_eq!(out.len(), 3);
        assert_eq!(out.iter().map(|a| a.offset).collect::<Vec<_>>(), [0, 1, 3]);
    }
}
