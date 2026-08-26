//! One page flattened into draw operations no device has opinions about.
//!
//! The PDF writer and the GDI printer both draw what the screen shows, and the
//! way to make three renderers agree is for two of them to share every
//! geometric decision. This walk mirrors the screen painter's — same placement
//! order, same baseline arithmetic, same underline offsets — and what it emits
//! is plain enough that a backend only chooses ink: a coloured rectangle, a
//! stroked segment, a run of text on a baseline, an image in a box. Coordinates
//! are points from the page's top-left, exactly as the layout said them.
//!
//! What the screen does not draw, paper does not either: no carets, no
//! selections, no find highlights, no hidden text. A printed page is the page,
//! not the editing session.

use std::collections::HashMap;

use wp_layout::block::{anchor_position, Page, Placed, Side};
use wp_layout::inline::Content;
use wp_layout::shape::Shaper;
use wp_layout::FontRequest;

/// One device-independent drawing operation, in page points.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// A filled rectangle: shading, or a text highlight.
    Fill {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        rgb: [u8; 3],
    },
    /// A stroked segment: borders, underlines, strikes.
    Rule {
        from: (f64, f64),
        to: (f64, f64),
        thickness: f64,
        rgb: [u8; 3],
    },
    /// Text on a baseline. `advances` is per character and is the layout's own
    /// measurement — a backend that obeys them reproduces every line ending.
    Text {
        x: f64,
        baseline: f64,
        text: String,
        advances: Vec<f64>,
        font: FontRequest,
        rgb: [u8; 3],
        /// Degrees clockwise about `(x, baseline)`. Zero for every line of a
        /// paragraph; a shape's words — WordArt, and Word's watermark — are
        /// the one thing on a page that is set at an angle.
        rotation: f64,
        /// How much taller than its own proportion the face is drawn, applied
        /// before the turn. One for every line of a paragraph; a piece of
        /// WordArt is stretched to fill the shape it was drawn in and is the
        /// one thing on a page that is not set at its own proportion. See
        /// [`wp_layout::block::ShapeWords::stretch`].
        stretch: f64,
    },
    /// An image in a box, by the relationship that names its bytes.
    Image {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        rel: String,
    },
    /// A chart in a box, by the relationship that names its part.
    ///
    /// Unlike an image a chart has no bytes to hand a device: it is drawn,
    /// from numbers, in whatever type the page is set in. [`draw_charts`]
    /// turns one of these into the fills, rules and text every backend here
    /// already knows — a backend that never calls it simply leaves the box
    /// empty, which is what both of them did before charts were drawn at all.
    Chart {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        rel: String,
    },
    /// A filled polygon: a pie's slice, an area chart's band, a marker.
    ///
    /// Only charts need one, which is why it arrived with them — everything
    /// else a page draws is a rectangle, a rule or a word.
    Poly {
        points: Vec<(f64, f64)>,
        rgb: [u8; 3],
    },
}

/// How long the rule above a page's footnotes is: two inches, measured off
/// Word's own output.
const SEPARATOR_WIDTH: f64 = 144.0;

/// The stretches of a line that carry a fill, with neighbours of one colour
/// joined into one.
///
/// Shading first and highlight over it: Word's highlighter is a marker laid on
/// the page, and a run's own shading is the paper it is drawn on.
pub fn painted_runs(line: &wp_layout::inline::Line) -> Vec<(f64, f64, [u8; 3])> {
    let mut out: Vec<(f64, f64, [u8; 3])> = Vec::new();
    for which in 0..2 {
        let mut open: Option<(f64, f64, [u8; 3])> = None;
        for fragment in &line.fragments {
            let colour = if which == 0 {
                fragment.style.shading
            } else {
                fragment.style.highlight
            };
            let colour = colour.filter(|_| !fragment.style.hidden);
            match (open.as_mut(), colour) {
                // Abutting to within a rounding error counts as touching.
                (Some(run), Some(rgb)) if run.2 == rgb && fragment.x <= run.1 + 0.01 => {
                    run.1 = fragment.x + fragment.width;
                }
                (_, Some(rgb)) => {
                    out.extend(open.take());
                    open = Some((fragment.x, fragment.x + fragment.width, rgb));
                }
                (_, None) => out.extend(open.take()),
            }
        }
        out.extend(open);
    }
    out
}

/// Everything one page draws, in the order the screen draws it.
pub fn flatten(page: &Page) -> Vec<Op> {
    let mut ops = Vec::new();
    let theme = wp_model::Theme::default();
    for (_, placement) in page.painted() {
        match &placement.kind {
            Placed::Fill(rgb) => ops.push(Op::Fill {
                x: placement.x,
                y: placement.y,
                width: placement.width,
                height: placement.height,
                rgb: *rgb,
            }),
            Placed::Edge { border, side } => {
                let rgb = border
                    .color
                    .and_then(|c| c.resolve(&theme))
                    .unwrap_or([0, 0, 0]);
                let thickness = border.size.map(|s| s.points()).unwrap_or(0.5).max(0.5);
                let (x, y, w, h) = (placement.x, placement.y, placement.width, placement.height);
                // The same half-thickness overlap the screen uses, so abutting
                // band segments of one column rule stay one rule.
                let over = thickness / 2.0;
                let (from, to) = match side {
                    Side::Top => ((x, y), (x + w, y)),
                    Side::Bottom => ((x, y + h), (x + w, y + h)),
                    Side::Start => ((x, y - over), (x, y + h + over)),
                    Side::End => ((x + w, y - over), (x + w, y + h + over)),
                };
                ops.push(Op::Rule {
                    from,
                    to,
                    thickness,
                    rgb,
                });
            }
            Placed::Line { line, .. } => {
                let baseline = placement.y + line.baseline;
                // Shading and highlight are laid down for the whole stretch
                // they cover before any type goes on, and neighbours that
                // share a colour are drawn as one. Filling them fragment by
                // fragment leaves a hairline of paper between two words of an
                // inverse-video run, where the two rectangles fail to meet.
                for (from, to, rgb) in painted_runs(line) {
                    ops.push(Op::Fill {
                        x: placement.x + from,
                        y: placement.y,
                        width: to - from,
                        height: placement.height,
                        rgb,
                    });
                }
                for fragment in &line.fragments {
                    let x = placement.x + fragment.x;
                    if let Content::Object {
                        height, rel, chart, ..
                    } = &fragment.content
                    {
                        // Inline drawings sit on the baseline like a very
                        // large letter.
                        let (x, y) = (x, baseline - height);
                        let (width, height) = (fragment.width, *height);
                        if let Some(rel) = rel {
                            ops.push(Op::Image {
                                x,
                                y,
                                width,
                                height,
                                rel: rel.to_string(),
                            });
                        } else if let Some(rel) = chart {
                            ops.push(Op::Chart {
                                x,
                                y,
                                width,
                                height,
                                rel: rel.to_string(),
                            });
                        }
                        continue;
                    }
                    let (text, advances) = match &fragment.content {
                        Content::Text { text, advances, .. }
                        | Content::Label { text, advances }
                        // A leadered tab draws the dots of a table of
                        // contents, and is otherwise an empty stretch.
                        | Content::Tab {
                            fill: text,
                            advances,
                            ..
                        } => (text, advances),
                        _ => continue,
                    };
                    if text.is_empty() {
                        continue;
                    }
                    let style = &fragment.style;
                    if style.hidden {
                        // The screen shows hidden text only with formatting
                        // marks on; a printed page has no such mode.
                        continue;
                    }
                    if let Some(border) = style.border {
                        let thickness = border.size.map(|s| s.points()).unwrap_or(0.5);
                        let rgb = border
                            .color
                            .and_then(|c| c.resolve(&theme))
                            .unwrap_or([0, 0, 0]);
                        // The rule is stroked down the middle of the box edge,
                        // so the box is drawn half a thickness inside the room
                        // the line reserved for it.
                        let half = thickness / 2.0;
                        let (bx, by) = (x + half, placement.y + half);
                        let (bw, bh) = (
                            (fragment.width - thickness).max(0.0),
                            (placement.height - thickness).max(0.0),
                        );
                        for (from, to) in [
                            ((bx, by), (bx + bw, by)),
                            ((bx, by + bh), (bx + bw, by + bh)),
                            ((bx, by), (bx, by + bh)),
                            ((bx + bw, by), (bx + bw, by + bh)),
                        ] {
                            ops.push(Op::Rule {
                                from,
                                to,
                                thickness,
                                rgb,
                            });
                        }
                    }
                    let rgb = style.color.unwrap_or([0, 0, 0]);
                    ops.push(Op::Text {
                        x: x + fragment.lead,
                        baseline: baseline - style.raise,
                        text: text.clone(),
                        advances: advances.clone(),
                        font: style.font.clone(),
                        rgb,
                        rotation: 0.0,
                        stretch: 1.0,
                    });
                    let base = baseline - style.raise;
                    // A rule under or through the type follows the type, not
                    // the box a run border draws around it.
                    let ink = x + fragment.lead;
                    let ink_end = ink + advances.iter().sum::<f64>();
                    if style.underline.draws() {
                        ops.push(Op::Rule {
                            from: (ink, base + 2.0),
                            to: (ink_end, base + 2.0),
                            thickness: 1.0,
                            rgb: style.underline_color.unwrap_or(rgb),
                        });
                    }
                    if style.strike || style.double_strike {
                        let middle = base - style.font.size * 0.3;
                        ops.push(Op::Rule {
                            from: (ink, middle),
                            to: (ink_end, middle),
                            thickness: 1.0,
                            rgb,
                        });
                    }
                }
            }
            Placed::Drawing {
                rel, anchor, words, ..
            } => {
                let (x, y) = match anchor {
                    Some(drawing) => {
                        anchor_position(drawing, &page.geometry, (placement.x, placement.y))
                    }
                    None => (placement.x, placement.y),
                };
                let width = placement.width;
                let height = placement.height;
                if let Some(outline) = anchor.as_ref().and_then(|d| d.outline) {
                    if let Some(wp_model::Color::Rgb(rgb)) = outline.fill {
                        ops.push(Op::Fill {
                            x,
                            y,
                            width,
                            height,
                            rgb,
                        });
                    }
                    if let Some(wp_model::Color::Rgb(rgb)) = outline.line {
                        let thickness = outline.line_width.points().max(0.5);
                        let corners = [
                            ((x, y), (x + width, y)),
                            ((x + width, y), (x + width, y + height)),
                            ((x + width, y + height), (x, y + height)),
                            ((x, y + height), (x, y)),
                        ];
                        for (from, to) in corners {
                            ops.push(Op::Rule {
                                from,
                                to,
                                thickness,
                                rgb,
                            });
                        }
                    }
                }
                if let Some(words) = words {
                    let (x, baseline) = words.origin(x, y, width, height);
                    ops.push(Op::Text {
                        x,
                        baseline,
                        text: words.text.clone(),
                        advances: words.advances.clone(),
                        font: words.font.clone(),
                        rgb: words.rgb,
                        rotation: words.rotation,
                        stretch: words.stretch,
                    });
                } else if let Some(rel) = rel {
                    ops.push(Op::Image {
                        x,
                        y,
                        width,
                        height,
                        rel: rel.to_string(),
                    });
                } else if let Some(rel) = anchor.as_ref().and_then(|d| d.chart.as_deref()) {
                    // An anchored drawing keeps the whole `Drawing`, chart and
                    // all — the screen finds a chart the same way.
                    ops.push(Op::Chart {
                        x,
                        y,
                        width,
                        height,
                        rel: rel.to_string(),
                    });
                }
            }
            // Resolved or dropped at pagination; never on a page. The screen
            // paints neither and neither does paper.
            Placed::BreakEdge { .. } => {}
            Placed::FootnoteSeparator => {
                // Two inches of hairline against the left of the text column,
                // which is what Word draws above a page's notes.
                ops.push(Op::Rule {
                    from: (placement.x, placement.y),
                    to: (placement.x + SEPARATOR_WIDTH, placement.y),
                    thickness: 1.0,
                    rgb: [0, 0, 0],
                });
            }
        }
    }
    ops
}

/// Every image relationship the pages mention, each once — what a caller
/// decodes before rendering.
pub fn image_rels<'a>(pages: impl Iterator<Item = &'a Page>) -> Vec<String> {
    rels(pages, |op| match op {
        Op::Image { rel, .. } => Some(rel),
        _ => None,
    })
}

/// The same, for the chart parts — what a caller *reads* before rendering.
pub fn chart_rels<'a>(pages: impl Iterator<Item = &'a Page>) -> Vec<String> {
    rels(pages, |op| match op {
        Op::Chart { rel, .. } => Some(rel),
        _ => None,
    })
}

fn rels<'a>(pages: impl Iterator<Item = &'a Page>, of: fn(Op) -> Option<String>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    for page in pages {
        seen.extend(flatten(page).into_iter().filter_map(of));
    }
    seen.into_iter().collect()
}

/// The charts a document draws, and the shaper that measures their labels.
///
/// Both are needed and neither is this crate's to have: the plots are read
/// from parts of a package, and how wide a string is depends on fonts the
/// caller owns. Handing over the *same* shaper the screen laid the page with
/// is what makes a printed chart the one the user approved.
pub struct Charts<'a> {
    pub plots: &'a HashMap<String, chart::Plot>,
    pub shaper: &'a mut dyn Shaper,
}

/// The face a chart sets its own text in.
///
/// A chart's labels are not the document's type: the sample's chart part
/// names no face at all — no `txPr`, no typeface — and its package carries no
/// theme, so Word falls back to the minor latin font of its built-in one,
/// which is Calibri. Identified by rendering the same string in ten installed
/// sans faces, scaling each to the ink height Word drew, and overlapping it:
/// Calibri matched nine tenths of the ink, the next best two thirds, Arial a
/// seventh. The size that goes with it is [`chart::draw::TEXT`], and the
/// screen makes the same two choices.
const CHART_FACE: &str = "Calibri";

/// Turns every [`Op::Chart`] into the ink that draws it.
///
/// A backend calls this and then knows nothing about charts; one that does
/// not simply leaves the box empty, which is what all of them did before.
pub fn draw_charts(ops: Vec<Op>, charts: &mut Charts) -> Vec<Op> {
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        let Op::Chart {
            x,
            y,
            width,
            height,
            rel,
        } = &op
        else {
            out.push(op);
            continue;
        };
        // A chart part that could not be read leaves its box empty rather
        // than drawing a frame around nothing.
        let Some(plot) = charts.plots.get(rel) else {
            continue;
        };
        let prims = chart::draw::primitives(
            chart::draw::Rect::new(*x, *y, *width, *height),
            plot,
            &chart::draw::cached_series(plot),
            &chart::draw::Style {
                background: [255, 255, 255],
                outline: [0xC0, 0xC0, 0xC0],
                text: [0, 0, 0],
                grid: [0xC0, 0xC0, 0xC0],
                // Points on paper are the unit the geometry was written in.
                zoom: 1.0,
                label: chart::draw::plain_label,
            },
            &mut Pen(&mut *charts.shaper),
        );
        for prim in prims {
            ink(&mut out, prim, &mut *charts.shaper);
        }
    }
    out
}

/// Turns every [`Op::Image`] that names a metafile into the ink that draws it.
///
/// A metafile is not pixels either: it is a recording of the calls that drew a
/// diagram, and playing it gives the same fills, rules and words a chart is
/// drawn with. Which pictures are metafiles is the caller's knowledge — it is
/// the one holding the bytes — so an image this has never heard of is left
/// alone for a backend to decode as the raster it is.
pub fn draw_metafiles(ops: Vec<Op>, pictures: &HashMap<String, metafile::Picture>) -> Vec<Op> {
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        let Op::Image {
            x,
            y,
            width,
            height,
            rel,
        } = &op
        else {
            out.push(op);
            continue;
        };
        let Some(picture) = pictures.get(rel) else {
            out.push(op);
            continue;
        };
        // The recording states its own natural size, so a diagram dragged
        // smaller is drawn smaller rather than cropped.
        let scale = (width / picture.size.0, height / picture.size.1);
        for prim in &picture.prims {
            played(&mut out, prim, (*x, *y), scale);
        }
    }
    out
}

/// One primitive of a played metafile, placed in the box it was drawn for.
fn played(out: &mut Vec<Op>, prim: &metafile::Prim, at: (f64, f64), scale: (f64, f64)) {
    let place = |point: &(f64, f64)| (at.0 + point.0 * scale.0, at.1 + point.1 * scale.1);
    // A line has one width however unevenly the box was scaled.
    let along = (scale.0 + scale.1) / 2.0;
    match prim {
        metafile::Prim::Fill { points, rgb } => out.push(Op::Poly {
            points: points.iter().map(place).collect(),
            rgb: *rgb,
        }),
        metafile::Prim::Stroke { points, rgb, width } => {
            let placed: Vec<(f64, f64)> = points.iter().map(place).collect();
            for segment in placed.windows(2) {
                out.push(Op::Rule {
                    from: segment[0],
                    to: segment[1],
                    thickness: width * along,
                    rgb: *rgb,
                });
            }
        }
        metafile::Prim::Text {
            x,
            baseline,
            text,
            advances,
            family,
            size,
            bold,
            italic,
            rgb,
            rotation,
        } => {
            let (x, baseline) = place(&(*x, *baseline));
            out.push(Op::Text {
                x,
                baseline,
                text: text.clone(),
                advances: advances.iter().map(|width| width * scale.0).collect(),
                font: FontRequest {
                    family: family.as_str().into(),
                    size: size * along,
                    bold: *bold,
                    italic: *italic,
                },
                rgb: *rgb,
                rotation: *rotation,
                stretch: 1.0,
            });
        }
    }
}

/// Measures a chart's labels with the page's own shaper.
struct Pen<'a>(&'a mut dyn Shaper);

impl chart::draw::Measure for Pen<'_> {
    fn size(&mut self, text: &str, size: f64) -> (f64, f64) {
        let font = FontRequest::new(CHART_FACE, size);
        let metrics = self.0.metrics(&font);
        (self.0.width(text, &font), metrics.ascent + metrics.descent)
    }
}

/// One primitive as this page's drawing operations.
fn ink(out: &mut Vec<Op>, prim: chart::draw::Prim, shaper: &mut dyn Shaper) {
    use chart::draw::Prim;
    match prim {
        Prim::Fill { rect, rgb, .. } => out.push(Op::Fill {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            rgb,
        }),
        Prim::Frame {
            rect,
            rgb,
            thickness,
            ..
        } => {
            // Four sides rather than a stroked box: `Op::Rule` is the only
            // stroke either backend has, and a rectangle is four of them.
            let corners = [
                (rect.left(), rect.top()),
                (rect.right(), rect.top()),
                (rect.right(), rect.bottom()),
                (rect.left(), rect.bottom()),
            ];
            for side in 0..4 {
                out.push(Op::Rule {
                    from: corners[side],
                    to: corners[(side + 1) % 4],
                    thickness,
                    rgb,
                });
            }
        }
        Prim::Line {
            points,
            thickness,
            rgb,
        } => {
            for segment in points.windows(2) {
                out.push(Op::Rule {
                    from: segment[0],
                    to: segment[1],
                    thickness,
                    rgb,
                });
            }
        }
        Prim::Poly { points, rgb, edge } => {
            out.push(Op::Poly {
                points: points.clone(),
                rgb,
            });
            if let Some((rgb, thickness)) = edge {
                for segment in points.windows(2) {
                    out.push(Op::Rule {
                        from: segment[0],
                        to: segment[1],
                        thickness,
                        rgb,
                    });
                }
            }
        }
        Prim::Dot { at, radius, rgb } => {
            // A twelve-sided ring is a circle at the size a marker is drawn,
            // and it needs no operation of its own.
            let points = (0..12)
                .map(|step| {
                    let angle = std::f64::consts::TAU * f64::from(step) / 12.0;
                    (at.0 + angle.cos() * radius, at.1 + angle.sin() * radius)
                })
                .collect();
            out.push(Op::Poly { points, rgb });
        }
        Prim::Text {
            at,
            size,
            text,
            rgb,
        } => {
            let font = FontRequest::new(CHART_FACE, size);
            let mut measured = Vec::new();
            shaper.advances(&text, &font, &mut measured);
            out.push(Op::Text {
                x: at.0,
                // The geometry places text by its top-left corner, which is
                // the corner that needs no font to compute.
                baseline: at.1 + shaper.metrics(&font).ascent,
                text,
                advances: measured.iter().map(|a| a.width).collect(),
                font,
                rgb,
                rotation: 0.0,
                stretch: 1.0,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_layout::block::Placement;
    use wp_layout::inline::{Fragment, Line};
    use wp_layout::TextStyle;
    use wp_model::PageBox;

    fn page_with(content: Vec<Placement>) -> Page {
        Page {
            number: 1,
            section: 0,
            geometry: PageBox {
                width: 612.0,
                height: 792.0,
                top: 72.0,
                bottom: 72.0,
                start: 72.0,
                end: 72.0,
            },
            content,
            header: Vec::new(),
            footer: Vec::new(),
            footnotes: Vec::new(),
            header_body: None,
            footer_body: None,
        }
    }

    fn text_line(text: &str) -> Placement {
        let style = TextStyle {
            font: FontRequest::new("Arial", 12.0),
            color: None,
            highlight: None,
            shading: None,
            border: None,
            underline: wp_model::prop::UnderlineKind::None,
            underline_color: None,
            strike: false,
            double_strike: false,
            caps: false,
            small_caps: false,
            raise: 0.0,
            letter_spacing: 0.0,
            hidden: false,
            rtl: false,
        };
        let advances = vec![6.0; text.chars().count()];
        let width: f64 = advances.iter().sum();
        Placement {
            x: 72.0,
            y: 100.0,
            width,
            height: 14.0,
            kind: Placed::Line {
                line: Box::new(Line {
                    fragments: vec![Fragment {
                        x: 0.0,
                        width,
                        lead: 0.0,
                        style,
                        content: Content::Text {
                            text: text.to_owned(),
                            advances,
                            hyphen: false,
                        },
                        source: None,
                        field: None,
                    }],
                    y: 0.0,
                    baseline: 11.0,
                    height: 14.0,
                    ascent: 11.0,
                    descent: 3.0,
                    x: 0.0,
                    width,
                    ideal: 0.0,
                    ended_by: None,
                }),
                paragraph: 0,
            },
        }
    }

    #[test]
    fn a_line_flattens_to_text_on_its_baseline() {
        let ops = flatten(&page_with(vec![text_line("hello")]));
        let [Op::Text {
            x,
            baseline,
            text,
            advances,
            ..
        }] = ops.as_slice()
        else {
            panic!("one text op, got {ops:?}");
        };
        assert_eq!(*x, 72.0);
        assert_eq!(*baseline, 111.0, "placement y plus the line's baseline");
        assert_eq!(text, "hello");
        assert_eq!(advances.len(), 5);
    }

    #[test]
    fn hidden_text_stays_off_the_paper() {
        let mut placement = text_line("secret");
        if let Placed::Line { line, .. } = &mut placement.kind {
            line.fragments[0].style.hidden = true;
        }
        assert!(flatten(&page_with(vec![placement])).is_empty());
    }

    #[test]
    fn an_underline_hangs_two_points_below_the_baseline() {
        let mut placement = text_line("under");
        if let Placed::Line { line, .. } = &mut placement.kind {
            line.fragments[0].style.underline = wp_model::prop::UnderlineKind::Single;
        }
        let ops = flatten(&page_with(vec![placement]));
        let rule = ops
            .iter()
            .find_map(|op| match op {
                Op::Rule { from, to, .. } => Some((from.1, to.1)),
                _ => None,
            })
            .expect("an underline rule");
        assert_eq!(rule, (113.0, 113.0), "baseline 111 plus two");
    }

    /// An inline drawing that is a chart rather than a picture.
    fn chart_line(rel: &str) -> Placement {
        let mut placement = text_line("");
        if let Placed::Line { line, .. } = &mut placement.kind {
            line.fragments[0].width = 300.0;
            line.fragments[0].content = Content::Object {
                height: 200.0,
                rel: None,
                chart: Some(rel.into()),
                nth: Some(0),
            };
        }
        placement
    }

    fn bar_chart() -> chart::Plot {
        chart::Plot {
            kind: chart::ChartKind::Bar,
            series: vec![chart::Series {
                name: Some("Sales".to_owned()),
                values: vec![Some(1.0), Some(2.0), Some(3.0)],
                categories: vec!["Q1".into(), "Q2".into(), "Q3".into()],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn a_chart_is_an_op_of_its_own_until_someone_draws_it() {
        let ops = flatten(&page_with(vec![chart_line("rId5")]));
        let [Op::Chart {
            x,
            y,
            width,
            height,
            rel,
        }] = ops.as_slice()
        else {
            panic!("one chart op, got {ops:?}");
        };
        assert_eq!((*x, *y), (72.0, 100.0 + 11.0 - 200.0), "on the baseline");
        assert_eq!((*width, *height), (300.0, 200.0));
        assert_eq!(rel, "rId5");
        assert_eq!(
            chart_rels([page_with(vec![chart_line("rId5")])].iter()),
            ["rId5"]
        );
    }

    #[test]
    fn a_chart_becomes_the_ink_that_draws_it() {
        // The whole reason `Op::Chart` exists: a chart has no bytes to hand a
        // device, so it has to arrive as the fills, rules and words every
        // backend already knows how to put down.
        let page = page_with(vec![chart_line("rId5")]);
        let plots = HashMap::from([("rId5".to_owned(), bar_chart())]);
        let mut shaper = wp_layout::shape::Fixed;
        let ops = draw_charts(
            flatten(&page),
            &mut Charts {
                plots: &plots,
                shaper: &mut shaper,
            },
        );

        assert!(
            !ops.iter().any(|op| matches!(op, Op::Chart { .. })),
            "nothing is left for a backend to skip"
        );
        let fills = ops
            .iter()
            .filter(|op| matches!(op, Op::Fill { .. }))
            .count();
        assert!(fills >= 4, "the ground and three bars, at least: {fills}");
        let labels: Vec<&str> = ops
            .iter()
            .filter_map(|op| match op {
                Op::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(labels.contains(&"Q1"), "the categories: {labels:?}");
        assert!(labels.contains(&"0"), "the value axis: {labels:?}");
        // Every piece of it lands inside the box the layout gave it.
        for op in &ops {
            if let Op::Fill { x, y, .. } = op {
                assert!(
                    (72.0..=372.0).contains(x) && (-89.0..=111.0).contains(y),
                    "outside its own box: {op:?}"
                );
            }
        }
    }

    #[test]
    fn a_chart_part_that_could_not_be_read_leaves_its_box_empty() {
        let page = page_with(vec![chart_line("rId5")]);
        let mut shaper = wp_layout::shape::Fixed;
        let ops = draw_charts(
            flatten(&page),
            &mut Charts {
                plots: &HashMap::new(),
                shaper: &mut shaper,
            },
        );
        assert!(ops.is_empty(), "no frame around nothing: {ops:?}");
    }

    #[test]
    fn every_image_is_listed_once_for_decoding() {
        let drawing = Placement {
            x: 100.0,
            y: 200.0,
            width: 50.0,
            height: 25.0,
            kind: Placed::Drawing {
                rel: Some("rId7".into()),
                anchor: None,
                paragraph: 0,
                nth: 0,
                words: None,
            },
        };
        let pages = [page_with(vec![drawing.clone()]), page_with(vec![drawing])];
        assert_eq!(image_rels(pages.iter()), vec!["rId7".to_owned()]);
    }

    #[test]
    fn a_metafile_is_played_into_the_box_the_page_gave_it() {
        // Half the recording's natural size on both axes: every coordinate,
        // every line width and every type size halves with it, and the ink
        // starts at the box's own corner rather than at the page's.
        let picture = metafile::Picture {
            size: (200.0, 100.0),
            prims: vec![
                metafile::Prim::Stroke {
                    points: vec![(0.0, 0.0), (200.0, 100.0)],
                    rgb: [1, 2, 3],
                    width: 4.0,
                },
                metafile::Prim::Text {
                    x: 100.0,
                    baseline: 50.0,
                    text: "hi".to_owned(),
                    advances: vec![10.0, 6.0],
                    family: "Arial".to_owned(),
                    size: 20.0,
                    bold: true,
                    italic: false,
                    rgb: [0, 0, 0],
                    rotation: 0.0,
                },
            ],
        };
        let mut pictures = HashMap::new();
        pictures.insert("rId7".to_owned(), picture);
        let ops = draw_metafiles(
            vec![
                Op::Image {
                    x: 30.0,
                    y: 40.0,
                    width: 100.0,
                    height: 50.0,
                    rel: "rId7".to_owned(),
                },
                Op::Image {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                    rel: "rId9".to_owned(),
                },
            ],
            &pictures,
        );
        let Op::Rule {
            from,
            to,
            thickness,
            ..
        } = &ops[0]
        else {
            panic!("{ops:?}");
        };
        assert_eq!((*from, *to), ((30.0, 40.0), (130.0, 90.0)));
        assert_eq!(*thickness, 2.0);
        let Op::Text {
            x,
            baseline,
            advances,
            font,
            ..
        } = &ops[1]
        else {
            panic!("{ops:?}");
        };
        assert_eq!((*x, *baseline), (80.0, 65.0));
        assert_eq!(advances, &[5.0, 3.0]);
        assert_eq!(font.size, 10.0);
        assert!(font.bold);
        // A picture this has never heard of is left for a backend to decode.
        assert!(matches!(&ops[2], Op::Image { rel, .. } if rel == "rId9"));
    }
}
