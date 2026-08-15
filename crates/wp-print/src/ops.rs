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

use wp_layout::block::{anchor_position, Page, Placed, Side};
use wp_layout::inline::Content;
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
    },
    /// An image in a box, by the relationship that names its bytes.
    Image {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        rel: String,
    },
}

/// Everything one page draws, in the order the screen draws it.
pub fn flatten(page: &Page) -> Vec<Op> {
    let mut ops = Vec::new();
    let theme = wp_model::Theme::default();
    for placement in page.everything() {
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
                for fragment in &line.fragments {
                    let x = placement.x + fragment.x;
                    if let Content::Object { height, rel, .. } = &fragment.content {
                        if let Some(rel) = rel {
                            // Inline drawings sit on the baseline like a very
                            // large letter.
                            ops.push(Op::Image {
                                x,
                                y: baseline - height,
                                width: fragment.width,
                                height: *height,
                                rel: rel.to_string(),
                            });
                        }
                        continue;
                    }
                    let (text, advances) = match &fragment.content {
                        Content::Text { text, advances, .. }
                        | Content::Label { text, advances } => (text, advances),
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
                    if let Some(rgb) = style.highlight {
                        ops.push(Op::Fill {
                            x,
                            y: placement.y,
                            width: fragment.width,
                            height: placement.height,
                            rgb,
                        });
                    }
                    let rgb = style.color.unwrap_or([0, 0, 0]);
                    ops.push(Op::Text {
                        x,
                        baseline: baseline - style.raise,
                        text: text.clone(),
                        advances: advances.clone(),
                        font: style.font.clone(),
                        rgb,
                    });
                    let base = baseline - style.raise;
                    if style.underline.draws() {
                        ops.push(Op::Rule {
                            from: (x, base + 2.0),
                            to: (x + fragment.width, base + 2.0),
                            thickness: 1.0,
                            rgb: style.underline_color.unwrap_or(rgb),
                        });
                    }
                    if style.strike || style.double_strike {
                        let middle = base - style.font.size * 0.3;
                        ops.push(Op::Rule {
                            from: (x, middle),
                            to: (x + fragment.width, middle),
                            thickness: 1.0,
                            rgb,
                        });
                    }
                }
            }
            Placed::Drawing { rel, anchor, .. } => {
                let Some(rel) = rel else { continue };
                let (x, y) = match anchor {
                    Some(drawing) => anchor_position(drawing, &page.geometry, placement.y),
                    None => (placement.x, placement.y),
                };
                ops.push(Op::Image {
                    x,
                    y,
                    width: placement.width,
                    height: placement.height,
                    rel: rel.to_string(),
                });
            }
            // Resolved or dropped at pagination; never on a page. The screen
            // paints neither and neither does paper.
            Placed::BreakEdge { .. } => {}
            Placed::FootnoteSeparator => {}
        }
    }
    ops
}

/// Every image relationship the pages mention, each once — what a caller
/// decodes before rendering.
pub fn image_rels<'a>(pages: impl Iterator<Item = &'a Page>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    for page in pages {
        for op in flatten(page) {
            if let Op::Image { rel, .. } = op {
                seen.insert(rel);
            }
        }
    }
    seen.into_iter().collect()
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
        }
    }

    fn text_line(text: &str) -> Placement {
        let style = TextStyle {
            font: FontRequest::new("Arial", 12.0),
            color: None,
            highlight: None,
            shading: None,
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
            },
        };
        let pages = [page_with(vec![drawing.clone()]), page_with(vec![drawing])];
        assert_eq!(image_rels(pages.iter()), vec!["rId7".to_owned()]);
    }
}
