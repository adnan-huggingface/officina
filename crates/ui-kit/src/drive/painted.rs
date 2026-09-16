//! What a frame painted, asked about the way a test asks.
//!
//! A test about how the window *looks* has nothing to go on but the shapes a
//! frame handed to the renderer. Walking them by hand was done eight times,
//! slightly differently each time, and one question could not be asked at all:
//! the colour a piece of text was painted in, which lives in the glyphs'
//! vertices rather than in anything with a name. This is that walk, once.

use eframe::egui;

/// A filled rectangle as it was painted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintedRect {
    pub rect: egui::Rect,
    /// What the frame let it paint into.
    pub clip: egui::Rect,
    pub fill: egui::Color32,
    /// Above zero for a shadow or a glow.
    pub blur: f32,
    pub stroke: egui::Stroke,
}

/// A piece of text as it was painted.
#[derive(Debug, Clone, PartialEq)]
pub struct PaintedText {
    pub text: String,
    /// Where it stands in the window, unrotated.
    pub rect: egui::Rect,
    /// What the frame let it paint into: a label clipped to nothing is
    /// painted and never seen.
    pub clip: egui::Rect,
    /// Each character the galley laid, and the colour it was painted in —
    /// `None` for a blank, which paints nothing to have a colour.
    pub letters: Vec<(char, Option<egui::Color32>)>,
}

impl PaintedText {
    /// The part of it that reaches the screen.
    pub fn shown(&self) -> egui::Rect {
        self.clip.intersect(self.rect)
    }

    /// The colours the non-blank characters of `word`'s first occurrence
    /// were painted in, in order, if the word is here.
    fn colours_of(&self, word: &str) -> Option<Vec<egui::Color32>> {
        colours_in(&self.letters, word)
    }
}

/// Every shape a frame painted, nested lists undone, in painting order.
#[derive(Debug, Clone, Default)]
pub struct Painted {
    shapes: Vec<egui::Shape>,
    clips: Vec<egui::Rect>,
}

impl Painted {
    pub fn new(clipped: &[egui::epaint::ClippedShape]) -> Painted {
        fn flatten(shape: &egui::Shape, into: &mut Vec<egui::Shape>) {
            match shape {
                egui::Shape::Vec(many) => many.iter().for_each(|one| flatten(one, into)),
                egui::Shape::Noop => {}
                other => into.push(other.clone()),
            }
        }
        let mut painted = Painted::default();
        for one in clipped {
            let before = painted.shapes.len();
            flatten(&one.shape, &mut painted.shapes);
            let added = painted.shapes.len() - before;
            painted
                .clips
                .extend(std::iter::repeat_n(one.clip_rect, added));
        }
        painted
    }

    /// The shapes themselves, for a question nothing below answers.
    pub fn shapes(&self) -> &[egui::Shape] {
        &self.shapes
    }

    /// Every rectangle, in painting order.
    pub fn rects(&self) -> Vec<PaintedRect> {
        self.shapes
            .iter()
            .zip(&self.clips)
            .filter_map(|(shape, clip)| match shape {
                egui::Shape::Rect(rect) => Some(PaintedRect {
                    rect: rect.rect,
                    clip: *clip,
                    fill: rect.fill,
                    blur: rect.blur_width,
                    stroke: rect.stroke,
                }),
                _ => None,
            })
            .collect()
    }

    /// The unblurred rectangles filled with `fill`, in painting order.
    pub fn filled(&self, fill: egui::Color32) -> Vec<egui::Rect> {
        self.rects()
            .into_iter()
            .filter(|rect| rect.fill == fill && rect.blur == 0.0)
            .map(|rect| rect.rect)
            .collect()
    }

    /// The largest unblurred rectangle filled with `fill` — the desk, a page,
    /// a panel — found by what it is rather than by a guess at its width.
    pub fn largest(&self, fill: egui::Color32) -> Option<egui::Rect> {
        self.filled(fill)
            .into_iter()
            .max_by(|a, b| a.area().total_cmp(&b.area()))
    }

    /// Every piece of text, in painting order.
    pub fn texts(&self) -> Vec<PaintedText> {
        self.shapes
            .iter()
            .zip(&self.clips)
            .filter_map(|(shape, clip)| match shape {
                egui::Shape::Text(text) => Some(read_text(text, *clip)),
                _ => None,
            })
            .collect()
    }

    /// Just the strings, in painting order.
    pub fn strings(&self) -> Vec<String> {
        self.texts().into_iter().map(|text| text.text).collect()
    }

    /// The first piece of text that is exactly `text`.
    pub fn text(&self, text: &str) -> Option<PaintedText> {
        self.texts()
            .into_iter()
            .find(|painted| painted.text == text)
    }

    /// The strings that begin with `prefix`, in painting order.
    pub fn strings_starting(&self, prefix: &str) -> Vec<String> {
        self.strings()
            .into_iter()
            .filter(|text| text.starts_with(prefix))
            .collect()
    }

    /// The one colour `word` was painted in: its first occurrence in a single
    /// piece of text, or else across the pieces in the order they were
    /// painted, which is how a line of runs is painted. `None` when the word
    /// is not on the screen. A word painted in more than one colour is a
    /// question the test did not mean to ask, and says so.
    pub fn colour_of(&self, word: &str) -> Option<egui::Color32> {
        let texts = self.texts();
        let colours = texts
            .iter()
            .find_map(|text| text.colours_of(word))
            .or_else(|| {
                let letters: Vec<(char, Option<egui::Color32>)> = texts
                    .iter()
                    .flat_map(|text| text.letters.iter().copied())
                    .collect();
                colours_in(&letters, word)
            })?;
        let first = *colours.first()?;
        assert!(
            colours.iter().all(|colour| *colour == first),
            "`{word}` is painted in more than one colour: {colours:?}"
        );
        Some(first)
    }

    /// The horizontal line segments painted in `colour`, as `(y, x0, x1)`
    /// with `x0` the left end.
    pub fn hlines(&self, colour: egui::Color32) -> Vec<(f32, f32, f32)> {
        self.shapes
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::LineSegment { points, stroke }
                    if stroke.color == colour && (points[0].y - points[1].y).abs() < 0.01 =>
                {
                    Some((
                        points[0].y,
                        points[0].x.min(points[1].x),
                        points[0].x.max(points[1].x),
                    ))
                }
                _ => None,
            })
            .collect()
    }
}

/// A text shape's characters and the colour the renderer will give each.
///
/// The colour is the glyph's own first vertex, with the shape's override and
/// fallback applied the way epaint's tessellator applies them. The galley's
/// sections cannot be asked instead: epaint invalidates a glyph's section
/// index once the galley is laid out.
fn read_text(shape: &egui::epaint::TextShape, clip: egui::Rect) -> PaintedText {
    let galley = &shape.galley;
    let mut letters = Vec::new();
    for placed in &galley.rows {
        let row = &placed.row;
        let vertices = &row.visuals.mesh.vertices;
        for glyph in &row.glyphs {
            let at = glyph.first_vertex as usize;
            let colour = match glyph.chr.is_whitespace() || at >= vertices.len() {
                true => None,
                false => {
                    let mut colour = vertices[at].color;
                    if let Some(over) = shape.override_text_color {
                        if row.visuals.glyph_vertex_range.contains(&at) {
                            colour = over;
                        }
                    } else if colour == egui::Color32::PLACEHOLDER {
                        colour = shape.fallback_color;
                    }
                    if shape.opacity_factor < 1.0 {
                        colour = colour.gamma_multiply(shape.opacity_factor);
                    }
                    Some(colour)
                }
            };
            letters.push((glyph.chr, colour));
        }
    }
    PaintedText {
        text: galley.text().to_owned(),
        rect: galley.rect.translate(shape.pos.to_vec2()),
        clip,
        letters,
    }
}

fn colours_in(letters: &[(char, Option<egui::Color32>)], word: &str) -> Option<Vec<egui::Color32>> {
    let wanted: Vec<char> = word.chars().collect();
    if wanted.is_empty() || letters.len() < wanted.len() {
        return None;
    }
    let start = (0..=letters.len() - wanted.len()).find(|&start| {
        letters[start..start + wanted.len()]
            .iter()
            .zip(&wanted)
            .all(|((have, _), want)| have == want)
    })?;
    Some(
        letters[start..start + wanted.len()]
            .iter()
            .filter_map(|(_, colour)| *colour)
            .collect(),
    )
}
