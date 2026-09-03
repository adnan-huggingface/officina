//! `<draw:frame>` — the box a picture is in.
//!
//! Everything a document draws sits in a frame, and the frame carries the size
//! and where it is anchored; what is inside says what to draw. Only a
//! `<draw:image>` is read: a frame holding an embedded object, a chart or a
//! text box is skipped for reading and kept for writing, which is what the
//! preservation rule means by unsupported.
//!
//! **The picture's bytes are in the package and the model names them by a
//! relationship.** ODF has no relationships — a frame gives the path the
//! picture sits at — so a name is minted while reading and the bytes come out
//! beside the document, exactly as `wp-doc` does for a format with no package
//! of its own. A picture drawn twice is carried once.
//!
//! A frame may also carry the picture *inside itself*, base64 in an
//! `<office:binary-data>`. That is how flat ODF has to do it and it is legal in
//! a package too, so it is decoded here rather than left as a box with nothing
//! in it — a blank rectangle where a logo should be is the kind of failure that
//! reads as our fault and is not.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use wp_model::doc::{Drawing, Wrap};
use wp_model::prop::{ParaProps, RunProps};
use wp_model::style::StyleKind;
use wp_model::units::Emu;

use crate::xml::{attr_in, end_local_name, local_name, push_text, skip_element};
use crate::Ctx;

/// Reads one `<draw:frame>`, whose start tag began at `at` and which the caller
/// has just seen.
///
/// `at` is what lets the drawing keep the bytes it was written as — see
/// [`Drawing::source`], which is the preservation vault applied inside a part
/// this crate models. A frame carries a graphic style, a title, an anchor and
/// possibly an object nothing here can draw, and editing the paragraph a
/// picture sits in is an ordinary thing to do.
pub fn frame(
    reader: &mut Reader<&[u8]>,
    e: &BytesStart<'_>,
    ctx: &mut Ctx<'_>,
    at: usize,
) -> Option<Drawing> {
    let width = attr_in(e, b"svg", b"width")
        .as_deref()
        .and_then(crate::xml::length);
    let height = attr_in(e, b"svg", b"height")
        .as_deref()
        .and_then(crate::xml::length);
    let anchor = attr_in(e, b"text", b"anchor-type").unwrap_or_else(|| "as-char".into());
    let name = attr_in(e, b"draw", b"name");
    let style = attr_in(e, b"draw", b"style-name");

    let mut drawing = Drawing {
        source: Vec::new().into(),
        // `as-char` is a picture in the line of text; everything else — to a
        // paragraph, to a page, to a character — floats. The distinction is the
        // one the layout engine turns on.
        anchored: anchor != "as-char",
        extent: (
            Emu::from_points(width.map(|w| w.points()).unwrap_or(0.0)),
            Emu::from_points(height.map(|h| h.points()).unwrap_or(0.0)),
        ),
        rel: None,
        chart: None,
        name: name.map(Into::into),
        description: None,
        wrap: Wrap::None,
        distance: (Emu(0), Emu(0), Emu(0), Emu(0)),
        position: None,
        behind_text: false,
        text: None,
        tone: None,
        outline: None,
    };
    if let Some(wrap) = style
        .as_deref()
        .and_then(|s| ctx.styles.graphics.get(s))
        .and_then(|graphic| graphic.wrap)
    {
        drawing.wrap = wrap;
    }

    let (rel, description) = inside(reader, ctx);
    drawing.rel = rel;
    drawing.description = description;
    let to = reader.buffer_position() as usize;
    if let Some(source) = ctx.source.get(at..to) {
        drawing.source = source.into();
    }
    // A frame this crate cannot draw is not a frame to put an empty box on the
    // page for. It is still in the package, and a save writes it back.
    drawing.rel.as_ref()?;
    Some(drawing)
}

/// Reads one `<draw:custom-shape>`, whose start tag began at `at`.
///
/// **Only a shape whose geometry says its text is drawn along a path.** That is
/// what a watermark is — Word exports one as a custom shape whose
/// `<draw:enhanced-geometry>` states `draw:text-path="true"` — and it is the one
/// kind of shape whose *words* are the whole of what it puts on the page. Every
/// other autoshape is skipped for reading and kept for writing, exactly as
/// before: drawing a shape whose geometry is not understood is worse than
/// drawing nothing, because the wrong outline is a claim and a blank is not.
///
/// The letters are filled with the *shape's* colour rather than the text's. ODF
/// says so by giving the shape `draw:fill-color` and `draw:opacity` while the
/// text style states a nominal size the path overrules — Word's own export
/// writes the watermark's grey as `#c0c0c0` at fifty per cent on the shape and
/// a one-point font on the run.
pub fn custom_shape(
    reader: &mut Reader<&[u8]>,
    e: &BytesStart<'_>,
    ctx: &mut Ctx<'_>,
    at: usize,
) -> Option<Drawing> {
    let width = attr_in(e, b"svg", b"width")
        .as_deref()
        .and_then(crate::xml::length);
    let height = attr_in(e, b"svg", b"height")
        .as_deref()
        .and_then(crate::xml::length);
    let anchor = attr_in(e, b"text", b"anchor-type").unwrap_or_else(|| "paragraph".into());
    let style = attr_in(e, b"draw", b"style-name");
    let rotation = attr_in(e, b"draw", b"transform")
        .as_deref()
        .and_then(rotation)
        .unwrap_or(0.0);

    let words = shape_inside(reader, ctx);
    if !words.on_a_path || words.text.is_empty() {
        return None;
    }

    let graphic = style.as_deref().and_then(|s| ctx.styles.graphics.get(s));
    let fill = graphic
        .and_then(|graphic| graphic.fill)
        .unwrap_or(wp_model::Color::Rgb(WATERMARK_GREY));
    let opacity = graphic.and_then(|graphic| graphic.opacity);
    let mut drawing = Drawing {
        source: Vec::new().into(),
        anchored: anchor != "as-char",
        extent: (
            Emu::from_points(width.map(|w| w.points()).unwrap_or(0.0)),
            Emu::from_points(height.map(|h| h.points()).unwrap_or(0.0)),
        ),
        rel: None,
        chart: None,
        name: attr_in(e, b"draw", b"name").map(Into::into),
        description: None,
        // A watermark is drawn through: `style:wrap="run-through"` is what the
        // export writes, and it is what the graphic style resolves to anyway.
        wrap: graphic
            .and_then(|graphic| graphic.wrap)
            .unwrap_or(wp_model::doc::Wrap::None),
        distance: (Emu(0), Emu(0), Emu(0), Emu(0)),
        position: graphic.and_then(|graphic| graphic.position).map(Box::new),
        // `style:run-through="background"` says the shape is under the text.
        behind_text: true,
        text: Some(Box::new(wp_model::doc::ShapeText {
            text: words.text.trim().into(),
            font: words.run.fonts.ascii.clone(),
            color: Some(washed(fill, opacity)),
            bold: words
                .run
                .toggles
                .get(wp_model::prop::Toggle::Bold)
                .unwrap_or(false),
            italic: words
                .run
                .toggles
                .get(wp_model::prop::Toggle::Italic)
                .unwrap_or(false),
            // `draw:text-path-mode="shape"` is the letters pulled about until
            // they fill the box, which is what the other format's reader takes
            // a `<v:textpath>` to mean and what Word draws for a watermark.
            stretch: true,
            rotation,
        })),
        tone: None,
        outline: None,
    };
    let to = reader.buffer_position() as usize;
    if let Some(source) = ctx.source.get(at..to) {
        drawing.source = source.into();
    }
    Some(drawing)
}

/// The grey a watermark is when the shape does not say otherwise — Word's own,
/// and the same constant the VML reader falls back to.
const WATERMARK_GREY: [u8; 3] = [0xC0, 0xC0, 0xC0];

/// A half-transparent fill, resolved against the paper it is drawn on.
///
/// The model keeps a colour and no opacity, so the blend is done here — the
/// same trade `wp_docx::pict` makes for `<v:fill opacity>`, and the same
/// arithmetic.
fn washed(color: wp_model::Color, opacity: Option<f64>) -> wp_model::Color {
    let wp_model::Color::Rgb(base) = color else {
        return color;
    };
    match opacity {
        Some(share) if (0.0..1.0).contains(&share) => wp_model::Color::Rgb(base.map(|channel| {
            let over_white = f64::from(channel) * share + 255.0 * (1.0 - share);
            over_white.round().clamp(0.0, 255.0) as u8
        })),
        _ => color,
    }
}

/// `draw:transform` — a list of `translate`, `rotate`, `scale` and `skew`, of
/// which only the rotation moves a watermark's words.
///
/// **The angle is in radians and turns anticlockwise** (ODF 1.4 part 3
/// §19.228), where the model keeps degrees clockwise. Word's own diagonal
/// watermark comes out of its ODF export as `rotate(-5.49779)`, which is the
/// 315 degrees the same watermark states in a `.docx`.
fn rotation(transform: &str) -> Option<f64> {
    let at = transform.find("rotate")?;
    let open = transform[at..].find('(')? + at;
    let close = transform[open..].find(')')? + open;
    let radians: f64 = transform[open + 1..close].trim().parse().ok()?;
    let degrees = -radians.to_degrees();
    Some(degrees.rem_euclid(360.0))
}

/// What a shape says, how it is set, and whether its geometry draws it along a
/// path.
#[derive(Default)]
struct Words {
    text: String,
    /// Resolved rather than direct: ODF puts a face in a style and a shape's
    /// label names one, so what the model wants is what the chain comes to.
    run: RunProps,
    on_a_path: bool,
}

/// Everything between the shape's start tag and its end.
///
/// **A shape's label is paragraphs of its own, and one face for all of them.**
/// The model keeps a single face, weight and slope for a shape's words, so the
/// first style that names a face is the one taken and later ones are left; a
/// watermark is one word in one span and has no second opinion to lose. The
/// *size* is not taken at all, because a text path sets its own out of the box
/// it has to fill — Word's export says so plainly by writing a one-point font
/// on a label an inch and a half high.
fn shape_inside(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>) -> Words {
    let mut words = Words::default();
    let mut paragraph = wp_model::Layers::default();
    while let Ok(event) = reader.read_event() {
        if push_text(&mut words.text, &event) {
            continue;
        }
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                match name.as_slice() {
                    b"enhanced-geometry" => {
                        words.on_a_path = attr_in(&e, b"draw", b"text-path")
                            .as_deref()
                            .and_then(crate::xml::boolean)
                            .unwrap_or(false);
                        if !empty {
                            skip_element(reader, &name);
                        }
                    }
                    b"p" | b"h" => {
                        if !words.text.is_empty() {
                            words.text.push(' ');
                        }
                        paragraph = layers(&e, ctx);
                        let resolved = ctx.table.resolve_run(&paragraph, &RunProps::default());
                        set_face(&mut words, resolved);
                    }
                    b"span" => {
                        let direct = RunProps {
                            style: attr_in(&e, b"text", b"style-name").map(|name| {
                                ctx.styles.id(&mut ctx.table, &name, StyleKind::Character)
                            }),
                            ..RunProps::default()
                        };
                        set_face(&mut words, ctx.table.resolve_run(&paragraph, &direct));
                    }
                    // The same collapsing rule the body is read under: every
                    // space after the first is an element of its own.
                    b"s" => {
                        let count = attr_in(&e, b"text", b"c")
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(1u32);
                        for _ in 0..count {
                            words.text.push(' ');
                        }
                        if !empty {
                            skip_element(reader, &name);
                        }
                    }
                    b"tab" | b"line-break" => {
                        words.text.push(' ');
                        if !empty {
                            skip_element(reader, &name);
                        }
                    }
                    // `<svg:title>` and `<svg:desc>` carry words a person wrote
                    // about the shape rather than words the shape draws, and
                    // harvesting them would put the file's path across the page.
                    _ if !empty => skip_element(reader, &name),
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == b"custom-shape" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    words
}

/// What a paragraph of a shape's label inherits, so that a span inside it
/// resolves against the same chain a paragraph of the body would.
fn layers(e: &BytesStart<'_>, ctx: &mut Ctx<'_>) -> wp_model::Layers {
    let style = attr_in(e, b"text", b"style-name")
        .map(|name| ctx.styles.id(&mut ctx.table, &name, StyleKind::Paragraph));
    let direct = ParaProps {
        style,
        ..ParaProps::default()
    };
    ctx.table.resolve_paragraph(&direct, None)
}

/// The first face named wins, and the weight and slope come with it.
fn set_face(words: &mut Words, resolved: RunProps) {
    if words.run.fonts.ascii.is_none() {
        words.run = resolved;
    }
}

/// What is in the frame: an image, a title, a description.
fn inside(
    reader: &mut Reader<&[u8]>,
    ctx: &mut Ctx<'_>,
) -> (Option<std::sync::Arc<str>>, Option<std::sync::Arc<str>>) {
    let mut rel = None;
    let mut description = None;
    while let Ok(event) = reader.read_event() {
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                match name.as_slice() {
                    b"image" => match attr_in(&e, b"xlink", b"href") {
                        Some(href) => {
                            rel = crate::content::adopt(ctx, href.trim_start_matches("./"));
                            if !empty {
                                skip_element(reader, &name);
                            }
                        }
                        None if !empty => rel = carried(reader, ctx),
                        None => {}
                    },
                    b"desc" | b"title" if !empty => {
                        let text = crate::content::text_of(reader, &name);
                        if description.is_none() && !text.is_empty() {
                            description = Some(text.into());
                        }
                    }
                    _ if !empty => skip_element(reader, &name),
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == b"frame" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    (rel, description)
}

/// A picture the frame carries itself, as base64 in `<office:binary-data>`.
fn carried(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>) -> Option<std::sync::Arc<str>> {
    let mut encoded = String::new();
    while let Ok(event) = reader.read_event() {
        match &event {
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"binary-data" => {}
            Event::End(e) if end_local_name(e) == b"image" => break,
            Event::Eof => break,
            other => {
                push_text(&mut encoded, other);
            }
        }
    }
    let data = decode(&encoded)?;
    let rel: std::sync::Arc<str> = format!("odf-picture-{}", ctx.media.len() + 1).into();
    ctx.media.push(crate::Media {
        rel: rel.to_string(),
        content_type: sniff(&data),
        data,
    });
    Some(rel)
}

/// Base64, and nothing more of it than this needs.
fn decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut bits: u32 = 0;
    let mut have = 0u32;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(value);
        have += 6;
        if have >= 8 {
            have -= 8;
            out.push((bits >> have) as u8);
        }
    }
    match out.is_empty() {
        true => None,
        false => Some(out),
    }
}

/// What a picture is, from the first bytes of it.
///
/// A carried picture has no file name to take an extension from, and the model
/// hands the type on to whatever decodes it.
fn sniff(data: &[u8]) -> &'static str {
    match data {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xff, 0xd8, 0xff, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'B', b'M', ..] => "image/bmp",
        [0x01, 0x00, 0x00, 0x00, ..] => "image/x-emf",
        [0xd7, 0xcd, 0xc6, 0x9a, ..] => "image/x-wmf",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_comes_back_as_the_bytes_it_stood_for() {
        assert_eq!(decode("aGVsbG8="), Some(b"hello".to_vec()));
        assert_eq!(decode("aGVsbG8h"), Some(b"hello!".to_vec()));
        // Whitespace is how a long picture is written, one line at a time.
        assert_eq!(decode("aGVs\n bG8="), Some(b"hello".to_vec()));
        assert_eq!(decode(""), None);
        assert_eq!(decode("not base64!"), None);
    }

    #[test]
    fn a_picture_says_what_it_is_by_its_first_bytes() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n"), "image/png");
        assert_eq!(sniff(b"\xff\xd8\xff\xe0"), "image/jpeg");
        assert_eq!(sniff(b"nothing"), "application/octet-stream");
    }

    /// The shape a watermark is, written as Word's own ODF export writes one:
    /// styles at the top of the part, the shape inline in a header paragraph,
    /// and everything about how it is drawn in a graphic style beside it.
    fn styles_with_a_shape(geometry: &str) -> Vec<wp_model::doc::HeaderFooter> {
        let xml = format!(
            concat!(
                r#"<office:document-styles>"#,
                r#"<office:automatic-styles>"#,
                r#"<style:style style:family="text" style:name="a1">"#,
                r##"<style:text-properties fo:color="#c0c0c0" fo:font-family="Calibri" fo:font-size="0.01389in"/>"##,
                r#"</style:style>"#,
                r#"<style:style style:family="paragraph" style:name="a2">"#,
                r#"<style:paragraph-properties fo:text-align="center"/></style:style>"#,
                r#"<style:style style:family="graphic" style:name="a3">"#,
                r##"<style:graphic-properties draw:fill="solid" draw:fill-color="#c0c0c0" draw:opacity="50%""##,
                r#" style:wrap="run-through" style:run-through="background""#,
                r#" style:horizontal-rel="page-content" style:vertical-rel="page-content""#,
                r#" style:horizontal-pos="center" style:vertical-pos="middle"/></style:style>"#,
                r#"</office:automatic-styles>"#,
                r#"<office:master-styles>"#,
                r#"<style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header>"#,
                r#"<text:p text:style-name="Header"><text:span text:style-name="T49">"#,
                r#"<draw:custom-shape svg:width="7.33125in" svg:height="1.83264in" draw:style-name="a3""#,
                r#" draw:transform="translate(-3.66562in -0.91632in) rotate(-5.49779) translate(3.66562in 0.91632in)""#,
                r#" draw:name="PowerPlusWaterMarkObject357476642" text:anchor-type="paragraph">"#,
                r#"<svg:title/><svg:desc>a note about the shape, not a word it draws</svg:desc>"#,
                r#"<text:p text:style-name="a2"><text:span text:style-name="a1">IPSUMDOLORSI</text:span></text:p>"#,
                "{}",
                r#"</draw:custom-shape></text:span></text:p>"#,
                r#"</style:header></style:master-page>"#,
                r#"</office:master-styles></office:document-styles>"#
            ),
            geometry
        );
        let container = crate::container::Container::empty(crate::container::TEXT_MIMETYPE);
        let mut ctx = Ctx::for_tests(&container);
        crate::content::part(xml.as_bytes(), &mut ctx, crate::content::Which::Styles)
            .expect("the stylesheet parses");
        ctx.headers
    }

    /// The one drawing in a band, if the band drew one at all.
    fn drawn(headers: &[wp_model::doc::HeaderFooter]) -> Option<&Drawing> {
        headers.iter().find_map(|band| {
            band.content.iter().find_map(|block| match block {
                wp_model::doc::Block::Paragraph(paragraph) => {
                    paragraph.content.iter().find_map(|inline| match inline {
                        wp_model::doc::Inline::Run(run) => {
                            run.content.iter().find_map(|piece| match piece {
                                wp_model::doc::Piece::Drawing(drawing) => Some(&**drawing),
                                _ => None,
                            })
                        }
                        _ => None,
                    })
                }
                _ => None,
            })
        })
    }

    #[test]
    fn a_custom_shape_on_a_text_path_is_the_watermark_it_draws() {
        let headers = styles_with_a_shape(concat!(
            r#"<draw:enhanced-geometry draw:type="non-primitive" draw:text-path-mode="shape""#,
            r#" draw:modifiers="50000" draw:text-path="true">"#,
            r#"<draw:equation draw:name="f0" draw:formula="left"/>"#,
            r#"</draw:enhanced-geometry>"#
        ));
        let drawing = drawn(&headers).expect("the shape is drawn");
        let text = drawing.text.as_ref().expect("it carries words");
        assert_eq!(&*text.text, "IPSUMDOLORSI");
        assert_eq!(text.font.as_deref(), Some("Calibri"));
        // Half of Word's own grey over white, because the model keeps a colour
        // and no opacity. See `washed`.
        assert_eq!(text.color, Some(wp_model::Color::Rgb([0xE0, 0xE0, 0xE0])));
        assert!((text.rotation - 315.0).abs() < 0.01, "{}", text.rotation);
        assert!(text.stretch);
        // The words are the shape's, and the note a person left on it is not
        // one of them.
        assert!(!text.text.contains("note"));
        assert!(drawing.behind_text);
        assert!(drawing.anchored);
        assert_eq!(drawing.wrap, Wrap::None);
        assert_eq!(drawing.extent.0, Emu::from_points(7.33125 * 72.0));
        let position = drawing.position.as_ref().expect("the page places it");
        assert_eq!(
            position.horizontal.align,
            Some(wp_model::doc::Alignment::Center)
        );
        assert_eq!(
            position.horizontal.relative_to,
            wp_model::doc::RelativeTo::Margin
        );
        // Kept as the bytes it arrived as, so that editing the header it sits
        // in does not rewrite a shape this crate models four attributes of.
        assert!(std::str::from_utf8(&drawing.source)
            .expect("the source is text")
            .starts_with("<draw:custom-shape"));
    }

    /// Every other autoshape, which is a geometry this cannot draw and words
    /// that are not the whole of what it puts on the page.
    #[test]
    fn a_custom_shape_that_is_not_on_a_text_path_is_left_to_the_bytes_it_came_as() {
        let headers = styles_with_a_shape(
            r#"<draw:enhanced-geometry draw:type="ellipse" svg:viewBox="0 0 21600 21600"/>"#,
        );
        assert!(drawn(&headers).is_none(), "an ellipse is not drawn");
        // And the paragraph it stood in is still there, with the shape's own
        // label kept out of it — a drawing's words are not the paragraph's.
        let paragraph = match &headers[0].content[0] {
            wp_model::doc::Block::Paragraph(paragraph) => paragraph,
            other => panic!("the header holds a paragraph: {other:?}"),
        };
        assert!(!paragraph.text().contains("IPSUMDOLORSI"));
    }

    /// ODF turns anticlockwise in radians and the model turns clockwise in
    /// degrees, and the two agree on nothing but zero.
    #[test]
    fn a_transform_states_its_rotation_in_radians_the_other_way_round() {
        assert_eq!(rotation("rotate(0)"), Some(0.0));
        let quarter = rotation("translate(1in 1in) rotate(-1.5707963) translate(2in 2in)");
        assert!((quarter.expect("there is a rotation") - 90.0).abs() < 0.01);
        assert_eq!(rotation("translate(1in 1in)"), None);
    }

    #[test]
    fn a_fill_that_is_half_there_is_blended_with_the_paper_under_it() {
        let grey = wp_model::Color::Rgb([0xC0, 0xC0, 0xC0]);
        assert_eq!(washed(grey, Some(0.5)), wp_model::Color::Rgb([0xE0; 3]));
        assert_eq!(washed(grey, None), grey);
        assert_eq!(washed(grey, Some(1.0)), grey);
    }
}
