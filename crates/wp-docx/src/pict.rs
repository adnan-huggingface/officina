//! `<w:pict>`: the VML a watermark is still written as.
//!
//! **Word writes a watermark in a notation it deprecated twenty years ago.**
//! Everything else a modern document draws is DrawingML under `<w:drawing>`,
//! but Design ▸ Watermark writes a `<v:shape>` of type `_x0000_t136` —
//! WordArt — into the header, with the words in a `string` attribute on
//! `<v:textpath>`. A reader that knows only DrawingML keeps those bytes safe
//! and draws nothing, so a watermarked document opens looking like a document
//! with no watermark: the one difference a reader must never invent, invented
//! by omission.
//!
//! What is read here is only what makes the words appear: the string, the
//! face, the fill colour, the size, the turn and where on the page it sits.
//! The bytes are kept whole regardless — [`wp_model::doc::Drawing::source`] —
//! so a document nobody edited is written back exactly as it came, shadow,
//! locks, formulas and all.
//!
//! **A `<w:pict>` may also be a picture**, which is what a picture watermark
//! is: a `<v:imagedata>` naming the image part, with the washout stated as
//! `gain` and `blacklevel`. Drawing it without the washout would stamp a
//! photograph over the text at full strength, so the tone is read with it and
//! the picture is drawn through it — see [`wp_model::doc::Tone`]. A `<w:pict>`
//! that is neither keeps travelling as [`wp_model::doc::Piece::Embedded`],
//! preserved and undrawn.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use wp_model::doc::{Alignment, Drawing, DrawingPosition, Offset, RelativeTo, ShapeText, Wrap};
use wp_model::units::Emu;

use crate::xml::{attr, end_local_name, local_name};

/// Word's own grey, for a shape whose fill is a name this does not know.
const WATERMARK_GREY: [u8; 3] = [0xC0, 0xC0, 0xC0];

/// The drawing a `<w:pict>` holds: a shape of words, or a picture.
///
/// `None` for every other kind, which is the answer that leaves the element
/// travelling as opaque bytes.
pub(crate) fn shape(source: &[u8]) -> Option<Drawing> {
    let mut reader = Reader::from_reader(source);
    reader.config_mut().trim_text(false);
    // The `<v:shapetype>` that precedes the shape carries a `<v:textpath>` of
    // its own — the template's, with no string in it — so the two are told
    // apart by which element is open, not by which comes first.
    let mut shape: Option<Shape> = None;
    let mut in_shapetype = false;
    let mut found: Option<(String, Option<String>)> = None;
    let mut picture: Option<(String, Option<wp_model::doc::Tone>)> = None;
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) => match local_name(&e) {
                b"shapetype" => in_shapetype = true,
                b"shape" if !in_shapetype => shape = Some(read_shape(&e)),
                _ => {}
            },
            Event::Empty(e) if !in_shapetype => match local_name(&e) {
                b"shape" => shape = Some(read_shape(&e)),
                b"fill" => {
                    if let Some(shape) = shape.as_mut() {
                        shape.opacity = attr(&e, b"opacity").as_deref().and_then(opacity);
                    }
                }
                b"textpath" if shape.is_some() && found.is_none() => {
                    let text = attr(&e, b"string").filter(|text| !text.trim().is_empty());
                    if let Some(text) = text {
                        found = Some((text, attr(&e, b"style").as_deref().and_then(face)));
                    }
                }
                // A shape holding a picture is not words, whatever else it
                // holds — and the walk stops looking for a string, so a
                // picture watermark is never drawn as its own filename.
                b"imagedata" => {
                    let rel = attr(&e, b"id").or_else(|| attr(&e, b"pict"))?;
                    let gain = attr(&e, b"gain").as_deref().and_then(fixed);
                    let black = attr(&e, b"blacklevel").as_deref().and_then(fixed);
                    let tone = match (gain, black) {
                        (None, None) => None,
                        (gain, black) => {
                            let tone = wp_model::doc::Tone::of_vml(
                                gain.unwrap_or(1.0),
                                black.unwrap_or(0.0),
                            );
                            (!tone.is_plain()).then_some(tone)
                        }
                    };
                    picture = Some((rel, tone));
                    found = None;
                }
                _ => {}
            },
            Event::End(e) if end_local_name(&e) == b"shapetype" => in_shapetype = false,
            Event::Eof => break,
            _ => {}
        }
    }
    let shape = shape?;
    if let Some((rel, tone)) = picture {
        let mut drawn = drawing(&shape, "", None);
        drawn.text = None;
        drawn.rel = Some(rel.into());
        drawn.tone = tone;
        return Some(drawn);
    }
    let (text, face) = found?;
    Some(drawing(&shape, &text, face))
}

/// VML's 16.16 fixed point, written with an `f` on the end — `19661f` is
/// 19661/65536, which is Word's washout gain of three tenths. A bare number is
/// the value itself, which is how the same attributes are written by hand.
fn fixed(value: &str) -> Option<f64> {
    let value = value.trim();
    match value.strip_suffix('f') {
        Some(raw) => raw.trim().parse::<f64>().ok().map(|v| v / 65536.0),
        None => value.parse::<f64>().ok(),
    }
}

/// The `<v:shape>` attributes that decide where the words go and what they
/// look like. Everything else about the shape stays in the source bytes.
struct Shape {
    style: String,
    fill: Option<String>,
    /// `<v:fill opacity>`, which Word writes for a semitransparent watermark.
    opacity: Option<f64>,
}

fn read_shape(e: &BytesStart<'_>) -> Shape {
    Shape {
        style: attr(e, b"style").unwrap_or_default(),
        fill: attr(e, b"fillcolor"),
        opacity: None,
    }
}

/// The drawing a `<v:shape>` stands for, with its words when it has any.
fn drawing(shape: &Shape, text: &str, face: Option<String>) -> Drawing {
    let style = shape.style.as_str();
    let width = css_points(style, "width").unwrap_or(0.0).max(1.0);
    let height = css_points(style, "height").unwrap_or(0.0).max(1.0);
    // VML turns clockwise and so does `ShapeText`, so the number carries over
    // as it stands. Word's diagonal watermark is 315.
    let rotation = css_number(style, "rotation").unwrap_or(0.0);
    let rgb = fill(shape);
    Drawing {
        source: Vec::new().into(),
        tone: None,
        // `position:absolute` is what makes a shape float; without it the
        // shape sits in the line like a letter, which is ordinary WordArt.
        anchored: css_value(style, "position")
            .is_some_and(|value| value.eq_ignore_ascii_case("absolute")),
        extent: (Emu::from_points(width), Emu::from_points(height)),
        rel: None,
        chart: None,
        name: None,
        description: None,
        // A watermark states `<w10:wrap>` rather than a wrap element of its
        // own, and what it always means is that the text runs straight
        // through it.
        wrap: Wrap::None,
        distance: Default::default(),
        position: Some(Box::new(DrawingPosition {
            horizontal: axis(style, false),
            vertical: axis(style, true),
        })),
        // A negative z-index is VML's way of saying the shape is under the
        // text. It changes nothing for a watermark, which is in the header
        // and therefore under the body whatever it claims — but a `<w:pict>`
        // in the body means it, and the model should carry what the file
        // said.
        behind_text: css_number(style, "z-index").is_some_and(|z| z < 0.0),
        outline: None,
        text: Some(Box::new(ShapeText {
            text: text.into(),
            font: face.map(Into::into),
            color: Some(wp_model::Color::Rgb(rgb)),
            bold: false,
            italic: false,
            // VML cannot say otherwise: Word writes the same `<v:textpath>`
            // for a watermark and for a piece of WordArt that is stretched,
            // and draws a shape read back from one stretched.
            stretch: true,
            rotation,
        })),
    }
}

/// One axis of the shape's position, out of the `mso-position-*` properties.
fn axis(style: &str, vertical: bool) -> Offset {
    let (which, relative) = match vertical {
        true => ("mso-position-vertical", "mso-position-vertical-relative"),
        false => (
            "mso-position-horizontal",
            "mso-position-horizontal-relative",
        ),
    };
    let align = css_value(style, which).and_then(|value| {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "center" => Alignment::Center,
            "left" => Alignment::Left,
            "right" => Alignment::Right,
            "top" => Alignment::Top,
            "bottom" => Alignment::Bottom,
            "inside" => Alignment::Inside,
            "outside" => Alignment::Outside,
            _ => return None,
        })
    });
    let relative_to = css_value(style, relative)
        .and_then(|value| {
            Some(match value.trim().to_ascii_lowercase().as_str() {
                "margin" => RelativeTo::Margin,
                "page" => RelativeTo::Page,
                "text" => RelativeTo::Paragraph,
                "char" => RelativeTo::Character,
                "line" => RelativeTo::Line,
                "left-margin-area" => RelativeTo::LeftMargin,
                "right-margin-area" => RelativeTo::RightMargin,
                "top-margin-area" => RelativeTo::TopMargin,
                "bottom-margin-area" => RelativeTo::BottomMargin,
                _ => return None,
            })
        })
        .unwrap_or(RelativeTo::Margin);
    // `margin-left` and `margin-top` are the offset when there is no
    // alignment; Word writes both, and the alignment is the one that means
    // anything when it is there.
    let offset = match vertical {
        true => css_points(style, "margin-top"),
        false => css_points(style, "margin-left"),
    };
    Offset {
        relative_to,
        offset: align
            .is_none()
            .then(|| Emu::from_points(offset.unwrap_or(0.0))),
        align,
    }
}

/// The colour the words are drawn in, with a semitransparent fill folded into
/// it.
///
/// **The renderers draw a shape's words in one solid colour**, and Word's own
/// watermark is a light grey at half opacity over the paper. Folding the
/// opacity toward white here is nearer the truth than ignoring it — an
/// unfolded `silver` is twice as dark as Word draws it — and it is honest for
/// a watermark in particular, which is behind everything and therefore always
/// over the page rather than over other ink.
fn fill(shape: &Shape) -> [u8; 3] {
    let base = shape
        .fill
        .as_deref()
        .and_then(vml_color)
        .unwrap_or(WATERMARK_GREY);
    match shape.opacity {
        Some(opacity) if (0.0..1.0).contains(&opacity) => base.map(|channel| {
            let over_white = f64::from(channel) * opacity + 255.0 * (1.0 - opacity);
            over_white.round().clamp(0.0, 255.0) as u8
        }),
        _ => base,
    }
}

/// A VML colour: `#rrggbb`, `#rgb`, or one of CSS's sixteen names.
///
/// Word writes the name when there is one — a watermark's grey comes out as
/// `silver`, not as `#c0c0c0` — so a reader that only knows hex loses the
/// colour of the commonest watermark there is.
fn vml_color(text: &str) -> Option<[u8; 3]> {
    let text = text.trim();
    let named = match text.to_ascii_lowercase().as_str() {
        "black" => [0x00, 0x00, 0x00],
        "silver" => [0xC0, 0xC0, 0xC0],
        "gray" | "grey" => [0x80, 0x80, 0x80],
        "white" => [0xFF, 0xFF, 0xFF],
        "maroon" => [0x80, 0x00, 0x00],
        "red" => [0xFF, 0x00, 0x00],
        "purple" => [0x80, 0x00, 0x80],
        "fuchsia" => [0xFF, 0x00, 0xFF],
        "green" => [0x00, 0x80, 0x00],
        "lime" => [0x00, 0xFF, 0x00],
        "olive" => [0x80, 0x80, 0x00],
        "yellow" => [0xFF, 0xFF, 0x00],
        "navy" => [0x00, 0x00, 0x80],
        "blue" => [0x00, 0x00, 0xFF],
        "teal" => [0x00, 0x80, 0x80],
        "aqua" | "cyan" => [0x00, 0xFF, 0xFF],
        _ => {
            // `#rgb` is the three-digit shorthand, where each digit doubles.
            let hex = text.strip_prefix('#')?;
            if hex.len() == 3 {
                let mut out = [0u8; 3];
                for (slot, digit) in out.iter_mut().zip(hex.chars()) {
                    let value = digit.to_digit(16)? as u8;
                    *slot = value * 17;
                }
                return Some(out);
            }
            return match wp_model::Color::from_val(text) {
                Some(wp_model::Color::Rgb(rgb)) => Some(rgb),
                _ => None,
            };
        }
    };
    Some(named)
}

/// The face out of a `<v:textpath>` style: `font-family:"Calibri";font-size:1pt`.
///
/// The size is deliberately not read. Word writes `font-size:1pt` and lets the
/// shape's own box decide how big the letters are, which is what
/// `wp_layout::block::shape_words` measures — believing the stated size would
/// draw a watermark one point tall.
fn face(style: &str) -> Option<String> {
    let value = css_value(style, "font-family")?;
    let value = value.trim();
    let value = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(value);
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// One property out of a CSS `style` attribute, unparsed.
fn css_value(style: &str, want: &str) -> Option<String> {
    style
        .split(';')
        .filter_map(|pair| pair.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case(want))
        .map(|(_, value)| value.trim().to_owned())
}

/// A bare number out of a `style` property — `rotation:315`, `z-index:-251658752`.
fn css_number(style: &str, want: &str) -> Option<f64> {
    let value = css_value(style, want)?;
    let value = value.trim();
    let end = value
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-' && c != '+')
        .unwrap_or(value.len());
    value[..end].parse().ok()
}

/// One length out of a `style` property, in points.
///
/// CSS units, because VML's `style` is CSS. Word writes points here; the
/// others are cheap to accept.
fn css_points(style: &str, want: &str) -> Option<f64> {
    let value = css_value(style, want)?;
    let value = value.trim();
    let split = value
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-' && c != '+')
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let number: f64 = number.parse().ok()?;
    let scale = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "pt" => 1.0,
        "in" => 72.0,
        "pc" => 12.0,
        "cm" => 72.0 / 2.54,
        "mm" => 72.0 / 25.4,
        "px" => 72.0 / 96.0,
        _ => return None,
    };
    Some(number * scale)
}

/// `<v:fill opacity>`, which Word writes as a fraction — `.5` — and may also
/// write as a percentage or in VML's own sixteen-bit fixed point.
fn opacity(text: &str) -> Option<f64> {
    let text = text.trim();
    if let Some(percent) = text.strip_suffix('%') {
        return percent.trim().parse::<f64>().ok().map(|n| n / 100.0);
    }
    let number: f64 = text.parse().ok()?;
    // `f` marks VML's fixed point, where 65536 is fully opaque.
    match text.ends_with('f') || number > 1.0 {
        true => Some(number / 65536.0),
        false => Some(number),
    }
}

#[cfg(test)]
mod tests {
    /// Word's own watermark, as Design ▸ Watermark writes it — taken from a
    /// document this machine's Word produced, shapetype and all.
    const WORD: &[u8] = br##"<w:pict w14:anchorId="4556ADCE"><v:shapetype id="_x0000_t136" coordsize="21600,21600" o:spt="136" adj="10800" path="m@7,l@8,m@5,21600l@6,21600e"><v:path textpathok="t"/><v:textpath on="t" fitshape="t"/><o:lock v:ext="edit" text="t" shapetype="t"/></v:shapetype><v:shape id="PowerPlusWaterMarkObject1" o:spid="_x0000_s1025" type="#_x0000_t136" style="position:absolute;margin-left:0;margin-top:0;width:527.75pt;height:131.95pt;rotation:315;z-index:-251658752;mso-position-horizontal:center;mso-position-horizontal-relative:margin;mso-position-vertical:center;mso-position-vertical-relative:margin" fillcolor="silver" stroked="f"><v:fill opacity=".5"/><v:shadow color="#868686"/><v:textpath style="font-family:&quot;Calibri&quot;;font-size:1pt;v-text-kern:t" trim="t" fitpath="t" string="CONFIDENTIAL"/><w10:wrap anchorx="margin" anchory="margin"/></v:shape></w:pict>"##;

    #[test]
    fn words_are_read_out_of_the_shape_and_not_out_of_the_shapetype() {
        let drawing = super::shape(WORD).expect("a watermark is words");
        let text = drawing.text.expect("and it carries them");
        // The `<v:textpath>` inside `<v:shapetype>` comes first and has no
        // string; taking that one would find no watermark at all.
        assert_eq!(&*text.text, "CONFIDENTIAL");
        assert_eq!(text.font.as_deref(), Some("Calibri"));
        assert_eq!(text.rotation, 315.0);
        // `silver` at half opacity over the paper, which is what Word draws
        // and roughly twice as light as the name alone would give.
        assert_eq!(text.color, Some(wp_model::Color::Rgb([0xE0, 0xE0, 0xE0])));
    }

    #[test]
    fn the_shape_states_its_size_and_its_place_in_a_css_style() {
        let drawing = super::shape(WORD).expect("a watermark");
        assert_eq!(drawing.extent.0.points().round(), 528.0);
        assert_eq!(drawing.extent.1.points().round(), 132.0);
        assert!(drawing.anchored, "position:absolute floats it");
        assert!(drawing.behind_text, "a negative z-index is behind the text");
        let position = drawing.position.expect("centred on the margin box");
        assert_eq!(
            position.horizontal.align,
            Some(wp_model::doc::Alignment::Center)
        );
        assert_eq!(
            position.vertical.relative_to,
            wp_model::doc::RelativeTo::Margin
        );
    }

    #[test]
    fn a_pict_that_is_a_picture_is_a_picture_and_not_words() {
        // A picture watermark: the image part it names and the washout it is
        // drawn through, which is Word's own — `gain="19661f"` and
        // `blacklevel="22938f"`, which Word itself reads back as a brightness
        // of 0.85 and a contrast of 0.15.
        let picture = br##"<w:pict><v:shape id="WordPictureWatermark1" type="#_x0000_t75" style="width:400pt;height:300pt"><v:imagedata r:id="rId4" o:title="logo" gain="19661f" blacklevel="22938f"/></v:shape></w:pict>"##;
        let drawing = super::shape(picture).expect("a picture watermark");
        assert!(drawing.text.is_none(), "not a shape of words");
        assert_eq!(drawing.rel.as_deref(), Some("rId4"));
        let tone = drawing.tone.expect("washed out");
        assert!((tone.gain - 0.3).abs() < 1e-3);
        assert!(
            (tone.offset - 0.805).abs() < 1e-3,
            "black comes out at 205, which is what Word draws"
        );
        assert_eq!(tone.apply(0), 205);
        assert_eq!(tone.apply(255), 255, "and anything light is white");

        // An ordinary embedded object with no shape in it at all stays opaque.
        assert!(super::shape(br##"<w:pict><v:rect style="width:10pt"/></w:pict>"##).is_none());
    }

    #[test]
    fn a_picture_nobody_adjusted_carries_no_tone_at_all() {
        let plain = br##"<w:pict><v:shape id="p" type="#_x0000_t75" style="width:40pt;height:30pt"><v:imagedata r:id="rId9" o:title=""/></v:shape></w:pict>"##;
        let drawing = super::shape(plain).expect("a picture");
        assert_eq!(drawing.rel.as_deref(), Some("rId9"));
        assert_eq!(drawing.tone, None, "so nothing recolours it");
    }

    #[test]
    fn a_shape_with_no_words_in_it_is_not_a_watermark() {
        let empty = br##"<w:pict><v:shape id="s" type="#_x0000_t136" style="position:absolute;width:10pt;height:5pt"><v:textpath string=""/></v:shape></w:pict>"##;
        assert!(super::shape(empty).is_none());
    }

    #[test]
    fn a_fill_stated_as_hex_is_read_as_well_as_one_stated_by_name() {
        assert_eq!(super::vml_color("#1e6f5c"), Some([0x1E, 0x6F, 0x5C]));
        assert_eq!(super::vml_color("#abc"), Some([0xAA, 0xBB, 0xCC]));
        assert_eq!(super::vml_color("SILVER"), Some([0xC0, 0xC0, 0xC0]));
        assert_eq!(super::vml_color("chartreuse"), None);
    }
}
