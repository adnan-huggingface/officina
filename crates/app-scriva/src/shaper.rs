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
use wp_layout::shape::{Advance, FontRequest, Ink, Metrics, Pitch, Shaper};

/// How many measured strings to keep before starting again.
///
/// A document reuses its words, so the cache hits hard and stays small; the
/// ceiling is there so that a hundred pages of distinct tokens cannot grow it
/// without bound.
const CACHE_LIMIT: usize = 20_000;

/// A shaper over an egui context.
pub struct Egui {
    ctx: egui::Context,
    /// Measured per *string* rather than per character, because a string is
    /// where shaping happens. The layout engine asks in break units — words —
    /// and a document repeats its words, so this hits far more often than it
    /// misses.
    runs: HashMap<(Key, String), Vec<f64>>,
    /// What one character advances on its own, which is what a line is
    /// measured by — see [`Egui::measure`]. Tiny: an alphabet per face.
    alone: HashMap<(Key, char), f64>,
    /// The ascent and row height epaint reports, per *drawn* face. The line
    /// gap is deliberately not kept here: several names can share one drawn
    /// face, and the gap belongs to the file the name resolves to, so caching
    /// it beside these handed the second name the first one's gap.
    rows: HashMap<Key, (f64, f64)>,
    /// Which named face each (family, bold, italic) resolved to, if the machine
    /// has it registered — interned so a `Key` stays `Copy` and cheap to hash.
    /// `None` remembers that the name has only the generic families to fall to.
    codes: HashMap<(String, bool, bool), Option<u16>>,
    named: Vec<egui::FontFamily>,
    /// Word's laid and ideal line pitches per face and size — resolved from
    /// the font file once, `None` when the machine has no file to ask.
    #[allow(clippy::type_complexity)]
    pitches: HashMap<(String, bool, bool, u32), Option<(f64, f64)>>,
    /// Each face's `hhea` line gap as a fraction of its em, which is where
    /// Word puts a line's baseline — see [`Egui::gap`]. Size-independent, so
    /// one lookup serves every size the document sets the face in.
    gaps: HashMap<(String, bool, bool), f64>,
    /// The ink of whole strings, in ems, per face — asked once per watermark
    /// and per shape of words, so the cache is tiny and the parse is not
    /// repeated on every frame. `None` remembers a face whose outlines this
    /// cannot read.
    #[allow(clippy::type_complexity)]
    inks: HashMap<(String, bool, bool, String), Option<Ink>>,
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
    /// Whether the pairs are closed up — part of the key because the same face
    /// at the same size measures differently on either side of it.
    kern: bool,
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
            alone: HashMap::new(),
            rows: HashMap::new(),
            codes: HashMap::new(),
            gaps: HashMap::new(),
            inks: HashMap::new(),
            named: Vec::new(),
            pitches: HashMap::new(),
        }
    }

    /// The font file behind a name, the document's own copy first.
    ///
    /// A face the document embedded is the one the screen shaped with, so it
    /// is the one whose tables must be read; going to the machine's fonts
    /// instead answers with a substitute's metrics for a face that is right
    /// here in the package.
    fn face_bytes(family: &str, bold: bool, italic: bool) -> Option<Vec<u8>> {
        if let Some(bytes) = ui_kit::fonts::document_face_file(family, bold, italic) {
            return Some(bytes.to_vec());
        }
        ui_kit::fonts::face_file(family, bold, italic).map(|(_, bytes)| bytes)
    }

    /// The face's `hhea` line gap, as a fraction of its em.
    ///
    /// Word lays the baseline of a line at the face's ascent *plus its line
    /// gap*, so the gap has to be known apart from the descent — and epaint
    /// only ever reports the two added together as a row height.
    fn gap(&mut self, font: &FontRequest) -> f64 {
        let ask = (font.family.to_ascii_lowercase(), font.bold, font.italic);
        if let Some(&known) = self.gaps.get(&ask) {
            return known;
        }
        let found = Egui::face_bytes(&font.family, font.bold, font.italic)
            .and_then(|bytes| {
                let face = wp_print::ttf::Face::parse(&bytes)?;
                Some(f64::from(face.line_gap) / face.units_per_em())
            })
            .unwrap_or(0.0);
        self.gaps.insert(ask, found);
        found
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
            kern: font.kern,
        }
    }

    /// The interned code of the exactly-named face for this request, if the
    /// machine registered one.
    fn named_code(&mut self, family: &str, bold: bool, italic: bool) -> Option<u16> {
        let ask = (family.to_ascii_lowercase(), bold, italic);
        if let Some(&code) = self.codes.get(&ask) {
            return code;
        }
        let code = ui_kit::fonts::named_face(&ask.0, bold, italic)
            .filter(|face| self.registered(face))
            .map(|face| {
                self.named.push(face);
                NAMED_BASE + (self.named.len() - 1) as u16
            });
        self.codes.insert(ask, code);
        code
    }

    /// Whether epaint has really been given this family.
    ///
    /// A document that carries its own type registers it and asks for it in
    /// the same breath, and `set_fonts` does not take effect until the next
    /// frame begins. Until then the family exists everywhere except where it
    /// is drawn — and epaint answers a family it has never been given with a
    /// panic rather than a substitute, so the question has to be asked before
    /// the name is used. Asked once per name; the answer is cached with it.
    fn registered(&self, face: &egui::FontFamily) -> bool {
        self.ctx.fonts_mut(|fonts| fonts.families().contains(face))
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

    /// The advance of each character of `text`.
    ///
    /// **Not the shaped width of the run.** Word does not close up a kerning
    /// pair unless the run asks it to, and nothing in an ordinary document
    /// asks; epaint shapes through HarfBuzz, which kerns whatever the face
    /// offers. Left alone, a line of prose measures a fraction of a point
    /// narrower here than there — enough, twice in the demonstration
    /// document, to pull onto a line a word Word puts on the next. So a run of
    /// letters that stand on their own is measured character by character,
    /// which is the same thing without the kerning, and a run of a script
    /// whose letters change shape beside each other is left shaped, where
    /// taking the characters apart would measure forms the reader never sees.
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
            } else if !key.kern && text.chars().all(stands_alone) {
                widths.clear();
                for ch in text.chars() {
                    widths.push(self.on_its_own(key, ch));
                }
            } else {
                share_ligatures(text, &mut widths);
            }
            self.runs.insert((key, text.to_owned()), widths);
        }
        &self.runs[&(key, text.to_owned())]
    }

    /// What one character advances with nothing beside it.
    fn on_its_own(&mut self, key: Key, ch: char) -> f64 {
        if let Some(&width) = self.alone.get(&(key, ch)) {
            return width;
        }
        // A combining mark is drawn over the letter before it and advances
        // nothing. On its own epaint has nothing to put it over and gives it
        // a width, which would push everything after it along.
        let width = match is_combining(ch) {
            true => 0.0,
            false => {
                let id = self.id_of(key);
                let galley = self.ctx.fonts_mut(|fonts| {
                    fonts.layout_no_wrap(ch.to_string(), id, egui::Color32::BLACK)
                });
                galley
                    .rows
                    .iter()
                    .flat_map(|row| &row.glyphs)
                    .map(|glyph| glyph.advance_width as f64)
                    .sum()
            }
        };
        self.alone.insert((key, ch), width);
        width
    }
}

/// Whether a character keeps its shape whatever stands next to it.
///
/// Latin, Greek, Cyrillic and the punctuation among them are drawn one letter
/// at a time, so the sum of the letters is the width of the word. Past that
/// come the scripts that join, reorder and combine — Arabic, Hebrew with its
/// points, the Indic scripts — where a letter measured alone is not the letter
/// that would be drawn.
fn stands_alone(ch: char) -> bool {
    (ch as u32) < 0x0590
}

/// Shares a ligature's advance out over the characters it stands for.
///
/// epaint draws Calibri's `ti` and `tt` as one glyph and hands the whole pair's
/// advance to the first character, leaving the second none — one entry per
/// character, so the count agrees and nothing looks wrong until a caret has to
/// go between them. It could not: `section` measured as though the `i` were not
/// there, a click in front of it landed after it, and a selection that ended
/// there covered a letter it did not include.
///
/// A combining mark keeps its nothing. Its advance is genuinely zero — it is
/// drawn over the letter before it rather than after it — and a caret belongs at
/// the end of the pair, not inside it.
fn share_ligatures(text: &str, widths: &mut [f64]) {
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0usize;
    while at < widths.len() {
        let mut run = at + 1;
        while run < widths.len() && widths[run] == 0.0 && !is_combining(chars[run]) {
            run += 1;
        }
        if run > at + 1 && widths[at] > 0.0 {
            let each = widths[at] / (run - at) as f64;
            widths[at..run].fill(each);
        }
        at = run;
    }
}

fn is_combining(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x0483..=0x0489 | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF | 0x20D0..=0x20F0 | 0xFE20..=0xFE2F)
}

impl Shaper for Egui {
    fn metrics(&mut self, font: &FontRequest) -> Metrics {
        let key = self.key(font);
        // epaint hands back a row height with the face's line gap already
        // folded into it. The gap is taken back out here, because Word puts it
        // above the baseline and the rest of the descent below: the row is the
        // same height, the split is not. It is never taken from anywhere but
        // below the baseline, so a name whose file states a larger gap than
        // the face actually drawn leaves room for cannot grow the line.
        let stated = self.gap(font) * font.size;
        let split = |ascent: f64, row_height: f64| {
            let gap = stated.clamp(0.0, (row_height - ascent).max(0.0));
            Metrics {
                ascent,
                descent: row_height - ascent - gap,
                line_gap: gap,
            }
        };
        if let Some(&(ascent, row_height)) = self.rows.get(&key) {
            return split(ascent, row_height);
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
        self.rows.insert(key, (ascent, row_height));
        split(ascent, row_height)
    }

    /// The box the letters of `text` really fill, from the face's own outlines.
    ///
    /// Measured in the font's units and returned in points at the size asked
    /// for, so one parse of the file serves every size. The pen walks by the
    /// face's *design* advances rather than the shaper's measured ones: this
    /// is a question about the glyphs, and mixing in epaint's pixel-grid
    /// rounding would put the answer on a different footing from the outlines
    /// it is being compared against.
    fn ink(&mut self, text: &str, font: &FontRequest) -> Option<Ink> {
        let ask = (
            font.family.to_ascii_lowercase(),
            font.bold,
            font.italic,
            text.to_owned(),
        );
        if !self.inks.contains_key(&ask) {
            let found = Egui::face_bytes(&font.family, font.bold, font.italic)
                .and_then(|bytes| ink_of(&bytes, text));
            self.inks.insert(ask.clone(), found);
        }
        let unit = (*self.inks.get(&ask)?)?;
        Some(Ink {
            left: unit.left * font.size,
            right: unit.right * font.size,
            top: unit.top * font.size,
            bottom: unit.bottom * font.size,
        })
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
            let natural = metrics.line_height();
            return Pitch {
                base: natural,
                ideal: natural,
            };
        }
        // The exact hhea sum, from the same file the screen resolved the name
        // to. epaint's metrics went through f32 and a pixel grid; the
        // accumulator needs the design value to the unit.
        let computed = Egui::face_bytes(&font.family, font.bold, font.italic).and_then(|bytes| {
            let face = wp_print::ttf::Face::parse(&bytes)?;
            let units = f64::from(face.ascent) - f64::from(face.descent) + f64::from(face.line_gap);
            let ideal = units / face.units_per_em() * font.size;
            let base = measured_base(&ask.0, ask.3).unwrap_or((ideal * 24.0).round() / 24.0);
            Some((base, ideal))
        });
        self.pitches.insert(ask, computed);
        match computed {
            Some((base, ideal)) => Pitch { base, ideal },
            None => {
                let metrics = self.metrics(font);
                let natural = metrics.line_height();
                Pitch {
                    base: natural,
                    ideal: natural,
                }
            }
        }
    }
}

/// The ink of a string in one face, in ems.
///
/// `None` when the face states no outlines this can read — a CFF OpenType, a
/// collection — or when the string draws nothing at all, which a space does.
fn ink_of(bytes: &[u8], text: &str) -> Option<Ink> {
    let face = wp_print::ttf::Face::parse(bytes)?;
    let em = face.units_per_em();
    let mut pen = 0i32;
    let mut box_: Option<[f64; 4]> = None;
    for c in text.chars() {
        let glyph = face.glyph(c);
        if let Some([x0, y0, x1, y1]) = face.glyph_box(glyph) {
            let (left, right) = (
                f64::from(pen + i32::from(x0)),
                f64::from(pen + i32::from(x1)),
            );
            let (bottom, top) = (f64::from(y0), f64::from(y1));
            box_ = Some(match box_ {
                None => [left, bottom, right, top],
                Some([l, b, r, t]) => [l.min(left), b.min(bottom), r.max(right), t.max(top)],
            });
        }
        pen += i32::from(face.advance(glyph));
    }
    let [left, bottom, right, top] = box_?;
    Some(Ink {
        left: left / em,
        right: right / em,
        top: top / em,
        bottom: bottom / em,
    })
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
///
/// **Aptos is measured and deliberately absent.** Word's default face since
/// 2024 fits the same law with a base of exactly 1.2 times the size and a
/// correction of *six* tenths of a point — `tools/probe` writes the
/// probes and `fit.py` returns it to no residual at all — but the accumulator
/// here pays halves, and entering the measured base with the wrong correction
/// wobbled every line half a point where the rounded ideal drifts by a
/// twentieth. Its ideal, the `hhea` sum, is within a thousandth of Word's own
/// average pitch; the drift that is left is the rounding, not the face.
fn measured_base(family: &str, half_points: u32) -> Option<f64> {
    const MEASURED: &[(&str, u32, f64)] = &[
        ("verdana", 16, 9.5662),
        ("verdana", 20, 12.0847),
        ("verdana", 24, 14.6017),
        ("verdana", 28, 17.1194),
        ("arial", 20, 11.5808),
        ("arial", 21, 12.0839),
        ("times new roman", 20, 11.5808),
        // Symbol draws a list's bullet, so its pitch decides the height of
        // every bulleted line even though the words on that line are in
        // another face. Thirty lines of `U+F0B7` per size, printed and read
        // back off the page.
        ("symbol", 20, 12.2524),
        ("symbol", 22, 13.4772),
        ("symbol", 24, 14.6979),
        ("symbol", 28, 17.1517),
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
            kern: false,
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
            kern: false,
        });
        assert_eq!(
            id.family,
            ui_kit::fonts::face(ui_kit::Family::Sans, false, true)
        );
    }

    #[test]
    fn a_face_registered_this_frame_is_not_asked_for_until_epaint_has_it() {
        // What opening a document that carries its own type does: register the
        // faces and lay the page out in them. `set_fonts` lands between
        // frames, so for the length of this one the family is known everywhere
        // except where it is drawn — and epaint answers a family it has never
        // been given by panicking. The name falls back to its shape for the
        // one frame, and the window stays up.
        let ctx = context();
        let name = "Nonesuch Grotesk";
        let faces = [(name.to_owned(), false, false, vec![0u8; 64])];
        ui_kit::fonts::embed_document(&ctx, &faces, &[]);
        let mut shaper = Egui::new(&ctx);
        let font = FontRequest::new(name, 12.0);
        assert_eq!(
            shaper.font_id(&font).family,
            ui_kit::fonts::face(ui_kit::Family::Sans, false, false),
            "the embedded face is registered but not yet live"
        );
        assert!(shaper.width("some words", &font) > 0.0, "and it measured");
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
        // The three parts are kept apart because Word seats the baseline at
        // the ascent *plus the line gap* and puts the rest of the descent
        // below it. They still sum to the row height epaint reports.
        assert!(metrics.line_gap >= 0.0);
        assert!(metrics.ascent + metrics.line_gap < metrics.line_height());
    }

    #[test]
    fn a_faces_line_gap_is_taken_out_of_the_descent_rather_than_added_to_it() {
        // Verdana has no line gap at all and Calibri has one, so the pair
        // shows both that the gap is found and that the row height a caller
        // lays out with does not change when it is.
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        for family in ["Verdana", "Calibri", "Arial"] {
            let font = FontRequest::new(family, 12.0);
            let metrics = shaper.metrics(&font);
            let gap = shaper.gap(&font) * 12.0;
            assert!(
                metrics.line_gap <= gap + 1e-9,
                "{family}: never more gap than the face's own file states"
            );
            assert!(metrics.descent >= 0.0, "{family}: descent stays positive");
            assert!(metrics.descent >= 0.0, "{family}: descent stays positive");
        }
    }

    #[test]
    fn one_faces_line_gap_does_not_leak_into_another_that_draws_the_same() {
        // With no fonts registered every name falls back to one built-in face
        // and so shares a cache key. The ascent they share is real — it is the
        // face being drawn — but the gap is not: it belongs to the file the
        // *name* resolves to, and caching it beside the ascent handed Calibri's
        // gap to Arial and Verdana.
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let of = |shaper: &mut Egui, family: &str| shaper.metrics(&FontRequest::new(family, 12.0));
        let calibri = of(&mut shaper, "Calibri");
        let verdana = of(&mut shaper, "Verdana");
        let again = of(&mut shaper, "Calibri");
        assert_eq!(
            calibri.line_gap, again.line_gap,
            "asking twice gives one answer"
        );
        assert_eq!(
            calibri.ascent, verdana.ascent,
            "the drawn face is shared, so its ascent is"
        );
        assert_eq!(
            calibri.ascent + calibri.descent + calibri.line_gap,
            verdana.ascent + verdana.descent + verdana.line_gap,
            "and the row they are drawn on is one height however it is split"
        );
    }

    #[test]
    fn a_ligatures_width_is_shared_by_the_characters_it_stands_for() {
        // Calibri draws `ti` as one glyph, and epaint reports the pair's whole
        // advance on the `t` and nothing on the `i` — one entry per character,
        // so the count agrees and only a caret notices. Measured here without a
        // font at all: the arithmetic is the thing under test, and which pairs a
        // given face ligates is the face's business.
        let mut widths = vec![4.0, 6.0, 0.0, 5.0];
        share_ligatures("stio", &mut widths);
        assert_eq!(widths, vec![4.0, 3.0, 3.0, 5.0], "the t and the i halve it");
        assert_eq!(
            widths.iter().sum::<f64>(),
            15.0,
            "and the line is no wider than it was"
        );
    }

    #[test]
    fn a_combining_mark_keeps_its_nothing() {
        // It is drawn *over* the letter before it, not after it, so its advance
        // is genuinely zero and the caret belongs at the end of the pair.
        let mut widths = vec![5.0, 0.0, 4.0];
        share_ligatures("e\u{0301}b", &mut widths);
        assert_eq!(widths, vec![5.0, 0.0, 4.0]);
    }

    #[test]
    fn a_kerning_pair_measures_as_the_two_letters_it_is_made_of() {
        // Word closes up a pair only where the run asks it to, and an ordinary
        // document never asks. The face this context falls back to kerns `AV`
        // by three points at this size, which is five times what it takes to
        // pull a word onto a line Word leaves it off.
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let font = FontRequest::new("Arial", 64.0);
        let width = |shaper: &mut Egui, text: &str| {
            let mut out = Vec::new();
            shaper.advances(text, &font, &mut out);
            out.iter().map(|advance| advance.width).sum::<f64>()
        };
        let apart = width(&mut shaper, "A") + width(&mut shaper, "V");
        assert!(
            (width(&mut shaper, "AV") - apart).abs() < 0.001,
            "the pair measured {} where its letters measure {apart}",
            width(&mut shaper, "AV")
        );
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
