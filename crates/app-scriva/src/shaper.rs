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
use wp_layout::shape::{Advance, FontRequest, Metrics, Pitch, Shaper};

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
    /// Which named face each (family, bold, italic) resolved to, if the machine
    /// has it registered — interned so a `Key` stays `Copy` and cheap to hash.
    /// `None` remembers that the name has only the generic families to fall to.
    codes: HashMap<(String, bool, bool), Option<u16>>,
    named: Vec<egui::FontFamily>,
    /// Word's laid and ideal line pitches per face and size — resolved from
    /// the font file once, `None` when the machine has no file to ask.
    #[allow(clippy::type_complexity)]
    pitches: HashMap<(String, bool, bool, u32), Option<(f64, f64)>>,
}

/// A font, reduced to what epaint distinguishes.
///
/// The size is quantised to a twentieth of a point so that a zoomed view does
/// not miss the cache on every frame — the difference is far below what a
/// pixel can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    /// 0–2 are the three generic families; `NAMED_BASE + n` is the `n`th
    /// exactly-named face this shaper has resolved. A named code already
    /// accounts for bold and italic — those pick which *file* the family is —
    /// but the flags stay on the key so a generic fallback stays styled.
    family: u16,
    size: u32,
    bold: bool,
    italic: bool,
}

/// Where the named-face codes start.
const NAMED_BASE: u16 = 3;

impl Key {
    fn family(self) -> ui_kit::Family {
        match self.family {
            1 => ui_kit::Family::Serif,
            2 => ui_kit::Family::Mono,
            _ => ui_kit::Family::Sans,
        }
    }
}

impl Egui {
    pub fn new(ctx: &egui::Context) -> Egui {
        Egui {
            ctx: ctx.clone(),
            runs: HashMap::new(),
            metrics: HashMap::new(),
            codes: HashMap::new(),
            named: Vec::new(),
            pitches: HashMap::new(),
        }
    }

    fn key(&mut self, font: &FontRequest) -> Key {
        let family = match self.named_code(&font.family, font.bold, font.italic) {
            Some(code) => code,
            None => match ui_kit::Family::of(&font.family) {
                ui_kit::Family::Sans => 0,
                ui_kit::Family::Serif => 1,
                ui_kit::Family::Mono => 2,
            },
        };
        Key {
            family,
            size: (font.size * 20.0).round().max(1.0) as u32,
            bold: font.bold,
            italic: font.italic,
        }
    }

    /// The interned code of the exactly-named face for this request, if the
    /// machine registered one.
    fn named_code(&mut self, family: &str, bold: bool, italic: bool) -> Option<u16> {
        let ask = (family.to_ascii_lowercase(), bold, italic);
        if let Some(&code) = self.codes.get(&ask) {
            return code;
        }
        let code = ui_kit::fonts::named_face(&ask.0, bold, italic).map(|face| {
            self.named.push(face);
            NAMED_BASE + (self.named.len() - 1) as u16
        });
        self.codes.insert(ask, code);
        code
    }

    fn id_of(&self, key: Key) -> egui::FontId {
        let family = if key.family >= NAMED_BASE {
            self.named[(key.family - NAMED_BASE) as usize].clone()
        } else {
            ui_kit::fonts::face(key.family(), key.bold, key.italic)
        };
        egui::FontId::new(key.size as f32 / 20.0, family)
    }

    /// The epaint font a request resolves to, for a painter.
    pub fn font_id(&mut self, font: &FontRequest) -> egui::FontId {
        let key = self.key(font);
        self.id_of(key)
    }

    /// The advance of each character of `text`, measured together so the face's
    /// kerning applies.
    fn measure(&mut self, key: Key, text: &str) -> &[f64] {
        if !self.runs.contains_key(&(key, text.to_owned())) {
            if self.runs.len() >= CACHE_LIMIT {
                self.runs.clear();
            }
            let id = self.id_of(key);
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
        let key = self.key(font);
        if let Some(&metrics) = self.metrics.get(&key) {
            return metrics;
        }
        let id = self.id_of(key);
        let pixels_per_point = self.ctx.pixels_per_point();
        // `FontsView::row_height` throws away the split between ascent and
        // descent that produced it. A fixed 80/20 guess at that split put a
        // real face's cap-height above the ascent line we assumed — invisible
        // in ordinary text, but a capital letter drawn flush against a cell's
        // top border (no padding, no leading) then pokes through the rule
        // above it. `styled_metrics` is the same computation `row_height`
        // already does, with the face's own ascent kept rather than discarded.
        let (ascent, row_height) = self.ctx.fonts_mut(|fonts| {
            let styled = fonts.fonts.font(&id.family).styled_metrics(
                pixels_per_point,
                id.size,
                &egui::epaint::text::VariationCoords::default(),
            );
            (styled.ascent as f64, styled.row_height as f64)
        });
        let metrics = Metrics {
            ascent,
            descent: (row_height - ascent).max(0.0),
            line_gap: 0.0,
        };
        self.metrics.insert(key, metrics);
        metrics
    }

    fn advances(&mut self, text: &str, font: &FontRequest, into: &mut Vec<Advance>) {
        let key = self.key(font);
        let widths = self.measure(key, text).to_vec();
        for (index, (offset, _)) in text.char_indices().enumerate() {
            into.push(Advance {
                offset,
                width: widths.get(index).copied().unwrap_or(0.0),
            });
        }
    }

    fn pitch(&mut self, font: &FontRequest) -> Pitch {
        // The measured bases are recorded under the face that actually draws:
        // a missing face's pitch is its substitute's pitch, exactly as its
        // glyphs are the substitute's glyphs. A `Liberation Sans;Arial` chain
        // is asked for by its first name, the way Word reads the attribute.
        let mut family = font
            .family
            .split(';')
            .next()
            .unwrap_or(&font.family)
            .trim()
            .to_ascii_lowercase();
        if ui_kit::fonts::exact_face(&family, font.bold, font.italic).is_none() {
            if let Some(sub) = ui_kit::fonts::substitute(&family) {
                family = sub.to_ascii_lowercase();
            }
        }
        let ask = (
            family,
            font.bold,
            font.italic,
            (font.size * 2.0).round().max(1.0) as u32,
        );
        if let Some(&cached) = self.pitches.get(&ask) {
            if let Some((base, ideal)) = cached {
                return Pitch { base, ideal };
            }
            let metrics = self.metrics(font);
            let natural = metrics.ascent + metrics.descent;
            return Pitch {
                base: natural,
                ideal: natural,
            };
        }
        // The exact hhea sum, from the same file the screen resolved the name
        // to. epaint's metrics went through f32 and a pixel grid; the
        // accumulator needs the design value to the unit.
        let computed = ui_kit::fonts::face_file(&font.family, font.bold, font.italic).and_then(
            |(_, bytes)| {
                let face = wp_print::ttf::Face::parse(&bytes)?;
                let units =
                    f64::from(face.ascent) - f64::from(face.descent) + f64::from(face.line_gap);
                let ideal = units / face.units_per_em() * font.size;
                let base = measured_base(&ask.0, ask.3).unwrap_or((ideal * 24.0).round() / 24.0);
                Some((base, ideal))
            },
        );
        self.pitches.insert(ask, computed);
        match computed {
            Some((base, ideal)) => Pitch { base, ideal },
            None => {
                let metrics = self.metrics(font);
                let natural = metrics.ascent + metrics.descent;
                Pitch {
                    base: natural,
                    ideal: natural,
                }
            }
        }
    }
}

/// Word's laid line pitch, measured rather than derived.
///
/// Thirty to fifty-five single-spaced lines of one face at one size, positions
/// read back over COM to the twip, and the pitch plus its half-point
/// corrections fitted to within reporting noise — see the probe machinery in
/// the session notes. No formula over the font's tables reproduces these
/// numbers (they are hinted, per-ppem quantities); a face and size not in this
/// table is laid at its ideal rounded to a twenty-fourth of a point, and the
/// half-point accumulator bounds the difference from Word below half a point
/// either way.
fn measured_base(family: &str, half_points: u32) -> Option<f64> {
    const MEASURED: &[(&str, u32, f64)] = &[
        ("verdana", 16, 9.5662),
        ("verdana", 20, 12.0847),
        ("verdana", 24, 14.6017),
        ("verdana", 28, 17.1194),
        ("arial", 20, 11.5808),
        ("arial", 21, 12.0839),
        ("times new roman", 20, 11.5808),
    ];
    MEASURED
        .iter()
        .find(|(name, half, _)| *name == family && *half == half_points)
        .map(|(_, _, base)| *base)
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
        let mut shaper = Egui::new(&ctx);
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
    fn a_name_with_no_registered_face_falls_back_to_its_shape() {
        // In tests nothing is loaded from disk, so even Verdana has no named
        // face — the request lands on the generic sans family, styled. On the
        // user's machine the same request resolves to verdana.ttf itself.
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let id = shaper.font_id(&FontRequest {
            family: "Verdana".into(),
            size: 10.0,
            bold: false,
            italic: true,
        });
        assert_eq!(
            id.family,
            ui_kit::fonts::face(ui_kit::Family::Sans, false, true)
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
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let a = shaper.key(&FontRequest::new("Arial", 11.000_001));
        let b = shaper.key(&FontRequest::new("Arial", 11.0));
        assert_eq!(a, b);
        assert_ne!(a, shaper.key(&FontRequest::new("Arial", 11.5)));
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
