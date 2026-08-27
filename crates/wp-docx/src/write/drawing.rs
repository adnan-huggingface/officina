//! Putting a drawing back, with the two things a user can change to it applied.
//!
//! A `<w:drawing>` is a whole DrawingML document and is kept as opaque bytes
//! (`Drawing::source`). That is right for everything about it *except* the two
//! numbers the editor lets a user change: how big it is, and where it sits. If
//! those were only in the model, a move would be shown on screen and thrown away
//! on save — the worst kind of bug, because the document looks right until it is
//! reopened.
//!
//! So the bytes are spliced rather than re-authored: the four `cx`/`cy`
//! attributes that state the size and the `<wp:posOffset>` values that state the
//! position are overwritten in place, and every other byte — effects, crops,
//! rotations, the VML fallback, the SmartArt — is copied. A drawing nobody
//! touched comes back byte-for-byte, because nothing disagrees and nothing is
//! rewritten.

use std::fmt::Write as _;

use quick_xml::events::Event;
use wp_model::doc::Drawing;
use wp_model::Emu;

use super::splice::Splicer;

/// The drawing's bytes, with size and position brought up to date.
///
/// Byte-identical to `drawing.source` when the model agrees with it, which is
/// every drawing in a document nobody has dragged.
pub fn patch(drawing: &Drawing) -> Vec<u8> {
    let source: &[u8] = &drawing.source;
    if source.is_empty() {
        return author(drawing);
    }
    // A `<w:pict>` is VML, and none of the DrawingML elements spliced below
    // exist in it — a watermark states its size and its place in a CSS
    // `style` attribute instead. There is nothing here that could edit one,
    // so it goes back as it came; the watermark box replaces the whole shape
    // rather than patching it, which arrives as an empty `source` and is
    // authored afresh.
    if is_vml(source) {
        return source.to_vec();
    }
    let mut out = Vec::with_capacity(source.len());
    let mut splicer = Splicer::new(source);
    out.extend_from_slice(splicer.preamble());
    // Which axis the enclosing `<wp:positionH>` / `<wp:positionV>` is about. A
    // `<wp:posOffset>` says a number and nothing else; only its parent says
    // which direction the number is in.
    let mut horizontal = true;

    while let Some((event, span)) = splicer.next() {
        let bytes = &source[span.clone()];
        match &event {
            Event::Start(tag) | Event::Empty(tag) => match local(tag.name().as_ref()) {
                b"positionH" => {
                    horizontal = true;
                    out.extend_from_slice(bytes);
                }
                b"positionV" => {
                    horizontal = false;
                    out.extend_from_slice(bytes);
                }
                // `<wp:extent>` is the drawing's size in the document; `<a:ext>`
                // is the same size inside the shape's own transform. Word keeps
                // them equal, and so does this.
                b"extent" | b"ext" => {
                    out.extend_from_slice(&resized(bytes, drawing.extent));
                }
                // `<c:chart>` names the chart part by relationship. The model
                // disagrees with the source for exactly one reason: a pasted
                // chart, whose part was cloned under a fresh relationship —
                // see `media::clone_chart` — and must be named by the clone.
                b"chart" => match drawing.chart.as_deref() {
                    Some(rel) => out.extend_from_slice(&attribute_text(bytes, b"r:id", rel)),
                    None => out.extend_from_slice(bytes),
                },
                b"posOffset" => {
                    let offset = drawing
                        .position
                        .as_ref()
                        .map(|position| match horizontal {
                            true => position.horizontal.offset,
                            false => position.vertical.offset,
                        })
                        .unwrap_or_default();
                    out.extend_from_slice(bytes);
                    if let Some(Emu(value)) = offset {
                        // Take the element's own text with it, and write ours.
                        skip_text(&mut splicer, source, &mut out, b"posOffset", value);
                        continue;
                    }
                }
                _ => out.extend_from_slice(bytes),
            },
            Event::Eof => break,
            _ => out.extend_from_slice(bytes),
        }
    }
    out
}

/// A `<w:drawing>` for a picture this application put in the document.
///
/// Written out rather than spliced, because there is nothing to splice into: a
/// pasted picture has no bytes Word authored and, by construction, nothing in it
/// that the model does not hold. It is the smallest inline picture Word accepts
/// — anything more would be inventing formatting the user did not ask for.
///
/// Every namespace is declared on the element that uses it rather than assumed
/// of the root. `document.xml` in a file Word wrote declares all of them at the
/// top; one written by something else may declare only `w`, and a `<w:drawing>`
/// naming an undeclared prefix is not a picture Word cannot draw, it is a file
/// Word offers to repair.
fn author(drawing: &Drawing) -> Vec<u8> {
    if drawing.text.is_some() {
        return author_watermark(drawing);
    }
    if drawing.chart.is_some() {
        return author_chart(drawing);
    }
    // A drawing with no relationship has no bytes to draw: writing the frame
    // without the picture would produce exactly the dangling `r:embed` that
    // makes Word call a document broken.
    let Some(rel) = &drawing.rel else {
        return Vec::new();
    };
    // The file names a relationship by its bare id — which part's it is has
    // already been settled by which part the element sits in.
    let rel = crate::parts::plain_rel(rel);
    let (cx, cy) = (drawing.extent.0 .0.max(1), drawing.extent.1 .0.max(1));
    let name = drawing.name.as_deref().unwrap_or("Picture");
    let name = crate::write::escape_attr(name);
    format!(
        "<w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\" \
           xmlns:wp=\"{WP}\">\
           <wp:extent cx=\"{cx}\" cy=\"{cy}\"/>\
           <wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>\
           <wp:docPr id=\"1\" name=\"{name}\"/>\
           <wp:cNvGraphicFramePr>\
             <a:graphicFrameLocks xmlns:a=\"{A}\" noChangeAspect=\"1\"/>\
           </wp:cNvGraphicFramePr>\
           <a:graphic xmlns:a=\"{A}\">\
             <a:graphicData uri=\"{PIC}\">\
               <pic:pic xmlns:pic=\"{PIC}\">\
                 <pic:nvPicPr>\
                   <pic:cNvPr id=\"0\" name=\"{name}\"/>\
                   <pic:cNvPicPr/>\
                 </pic:nvPicPr>\
                 <pic:blipFill>\
                   <a:blip r:embed=\"{rel}\" xmlns:r=\"{R}\"/>\
                   <a:stretch><a:fillRect/></a:stretch>\
                 </pic:blipFill>\
                 <pic:spPr>\
                   <a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
                   <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>\
                 </pic:spPr>\
               </pic:pic>\
             </a:graphicData>\
           </a:graphic>\
         </wp:inline></w:drawing>"
    )
    .into_bytes()
}

/// A `<w:drawing>` for a chart this application put in the document.
///
/// The same shape as the picture above with the graphic swapped: where a
/// picture holds `<pic:pic>` with a blip, a chart holds a single `<c:chart>`
/// naming its part by relationship. Word draws the part, not the frame, so
/// this is everything there is to write. The namespaces — `c` and `r` both —
/// are declared on the elements that use them, because a prefix the root did
/// not bind is a file Word offers to repair, and `document.xml` written by
/// this crate binds only `w`.
fn author_chart(drawing: &Drawing) -> Vec<u8> {
    let Some(rel) = &drawing.chart else {
        return Vec::new();
    };
    let (cx, cy) = (drawing.extent.0 .0.max(1), drawing.extent.1 .0.max(1));
    let name = drawing.name.as_deref().unwrap_or("Chart");
    let name = crate::write::escape_attr(name);
    format!(
        "<w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\" \
           xmlns:wp=\"{WP}\">\
           <wp:extent cx=\"{cx}\" cy=\"{cy}\"/>\
           <wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>\
           <wp:docPr id=\"1\" name=\"{name}\"/>\
           <wp:cNvGraphicFramePr>\
             <a:graphicFrameLocks xmlns:a=\"{A}\" noChangeAspect=\"1\"/>\
           </wp:cNvGraphicFramePr>\
           <a:graphic xmlns:a=\"{A}\">\
             <a:graphicData uri=\"{C}\">\
               <c:chart xmlns:c=\"{C}\" xmlns:r=\"{R}\" r:id=\"{rel}\"/>\
             </a:graphicData>\
           </a:graphic>\
         </wp:inline></w:drawing>"
    )
    .into_bytes()
}

/// Whether a drawing's bytes are a `<w:pict>` rather than a `<w:drawing>`.
///
/// The first element decides it, so a leading comment or processing
/// instruction cannot fool this into splicing VML.
fn is_vml(source: &[u8]) -> bool {
    let mut reader = quick_xml::Reader::from_reader(source);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                return local(e.name().as_ref()) == b"pict"
            }
            Ok(Event::Eof) | Err(_) => return false,
            _ => {}
        }
    }
}

/// The WordArt shape type a watermark is an instance of, exactly as Word
/// writes it.
///
/// **The `<o:lock shapetype="t"/>` at the end is not decoration.** Without it
/// Word counts the template itself as a second shape in the header — a
/// watermarked page with two objects on it, one of them un-indexable — and
/// its own Remove Watermark then leaves that one behind. The formulas define
/// the `@7`, `@8` and the rest that the `path` refers to; a shape type whose
/// path names formulas it does not carry is not a shape type Word can draw
/// text along.
///
/// Taken from what this machine's Word emitted for its own watermark, the
/// same way `corpus/generate.ps1` gets everything else here.
const WORDART_SHAPETYPE: &str = concat!(
    r#"<v:shapetype id="_x0000_t136" coordsize="21600,21600" o:spt="136" adj="10800""#,
    r#" path="m@7,l@8,m@5,21600l@6,21600e">"#,
    r#"<v:formulas>"#,
    r##"<v:f eqn="sum #0 0 10800"/><v:f eqn="prod #0 2 1"/><v:f eqn="sum 21600 0 @1"/>"##,
    r#"<v:f eqn="sum 0 0 @2"/><v:f eqn="sum 21600 0 @3"/><v:f eqn="if @0 @3 0"/>"#,
    r#"<v:f eqn="if @0 21600 @1"/><v:f eqn="if @0 0 @2"/><v:f eqn="if @0 @4 21600"/>"#,
    r#"<v:f eqn="mid @5 @6"/><v:f eqn="mid @8 @5"/><v:f eqn="mid @7 @8"/>"#,
    r#"<v:f eqn="mid @6 @7"/><v:f eqn="sum @6 0 @5"/>"#,
    r#"</v:formulas>"#,
    r#"<v:path textpathok="t" o:connecttype="custom""#,
    r#" o:connectlocs="@9,0;@10,10800;@11,21600;@12,10800" o:connectangles="270,180,90,0"/>"#,
    r#"<v:textpath on="t" fitshape="t"/>"#,
    r##"<v:handles><v:h position="#0,bottomRight" xrange="6629,14971"/></v:handles>"##,
    r#"<o:lock v:ext="edit" text="t" shapetype="t"/>"#,
    r#"</v:shapetype>"#,
);

/// A `<w:pict>` for a watermark this application put in the document.
///
/// **Word still writes a watermark as VML**, so this does too. Authoring the
/// DrawingML equivalent would be a document Word opens and shows correctly
/// and its own Design ▸ Watermark ▸ Remove Watermark cannot find — a file
/// that looks right and behaves wrong, which is worse than one that looks
/// wrong.
///
/// The shape carries the `PowerPlusWaterMarkObject` name Word gives its own:
/// that name is how Word tells a watermark from a piece of art someone drew,
/// and one written without it cannot be removed from Word's own menu.
///
/// There is no whitespace anywhere in what this emits. VML is not
/// whitespace-sensitive, but a `<w:pict>` full of indentation is a `<w:pict>`
/// that no longer round-trips against the bytes it was read from.
fn author_watermark(drawing: &Drawing) -> Vec<u8> {
    let Some(shape) = drawing.text.as_deref() else {
        return Vec::new();
    };
    let text = crate::write::escape_attr(&shape.text);
    let face = crate::write::escape_attr(shape.font.as_deref().unwrap_or("Calibri"));
    let width = drawing.extent.0.points();
    let height = drawing.extent.1.points();
    let rotation = shape.rotation;
    let fill = match shape.color {
        Some(wp_model::Color::Rgb([r, g, b])) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => "silver".to_owned(),
    };
    // A negative z-index puts the shape under the words, which is the whole
    // point of a watermark: the document must stay legible over it.
    let style = format!(
        "position:absolute;margin-left:0;margin-top:0;\
         width:{width:.2}pt;height:{height:.2}pt;rotation:{rotation};z-index:-251658752;\
         mso-position-horizontal:center;mso-position-horizontal-relative:margin;\
         mso-position-vertical:center;mso-position-vertical-relative:margin"
    );
    let mut out = String::with_capacity(WORDART_SHAPETYPE.len() + 512);
    // The three VML namespaces are declared here, on the one element this
    // authors that encloses everything using them. `document.xml` in a file
    // Word wrote declares them at the root; one written by something else may
    // declare only `w`, and a prefix nothing declares is not a watermark Word
    // draws badly — it is a file Word refuses to open.
    let _ = write!(
        out,
        r#"<w:pict xmlns:v="{V}" xmlns:o="{O}" xmlns:w10="{W10}">"#
    );
    out.push_str(WORDART_SHAPETYPE);
    out.push_str(r##"<v:shape id="PowerPlusWaterMarkObject1" type="#_x0000_t136""##);
    let _ = write!(out, r#" style="{style}" fillcolor="{fill}" stroked="f">"#);
    let _ = write!(
        out,
        r#"<v:textpath style="font-family:&quot;{face}&quot;;font-size:1pt" trim="t" fitpath="t" string="{text}"/>"#
    );
    out.push_str(r#"<w10:wrap anchorx="margin" anchory="margin"/>"#);
    out.push_str("</v:shape></w:pict>");
    out.into_bytes()
}

const V: &str = "urn:schemas-microsoft-com:vml";
const O: &str = "urn:schemas-microsoft-com:office:office";
const W10: &str = "urn:schemas-microsoft-com:office:word";
const WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// The local name of an element, without its prefix.
fn local(name: &[u8]) -> &[u8] {
    match name.iter().position(|b| *b == b':') {
        Some(colon) => &name[colon + 1..],
        None => name,
    }
}

/// Replaces the `cx` and `cy` attributes of a start or empty tag.
///
/// Done on the raw bytes rather than by re-emitting the tag, so an element that
/// carries other attributes keeps them, in their order.
fn resized(bytes: &[u8], extent: (Emu, Emu)) -> Vec<u8> {
    let mut out = attribute(bytes, b"cx", extent.0 .0);
    out = attribute(&out, b"cy", extent.1 .0);
    out
}

fn attribute(bytes: &[u8], name: &[u8], value: i64) -> Vec<u8> {
    attribute_text(bytes, name, &value.to_string())
}

fn attribute_text(bytes: &[u8], name: &[u8], value: &str) -> Vec<u8> {
    let mut needle = Vec::with_capacity(name.len() + 2);
    needle.push(b' ');
    needle.extend_from_slice(name);
    needle.push(b'=');
    let Some(at) = find(bytes, &needle) else {
        return bytes.to_vec();
    };
    let open = at + needle.len();
    let Some(quote) = bytes.get(open).copied() else {
        return bytes.to_vec();
    };
    let Some(close) = bytes[open + 1..].iter().position(|b| *b == quote) else {
        return bytes.to_vec();
    };
    let mut out = Vec::with_capacity(bytes.len() + 8);
    out.extend_from_slice(&bytes[..open + 1]);
    out.extend_from_slice(value.as_bytes());
    out.extend_from_slice(&bytes[open + 1 + close..]);
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Writes `value` as the element's text, and drops the text that was there.
fn skip_text(splicer: &mut Splicer<'_>, source: &[u8], out: &mut Vec<u8>, name: &[u8], value: i64) {
    out.extend_from_slice(value.to_string().as_bytes());
    while let Some((event, span)) = splicer.next() {
        match &event {
            Event::End(tag) if local(tag.name().as_ref()) == name => {
                out.extend_from_slice(&source[span]);
                return;
            }
            Event::Eof => return,
            // Anything that is not the element's own text is not ours to drop.
            Event::Text(_) => {}
            _ => out.extend_from_slice(&source[span]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::doc::{DrawingPosition, Offset, RelativeTo};

    fn drawing(source: &str) -> Drawing {
        Drawing {
            source: source.as_bytes().to_vec().into(),
            anchored: true,
            extent: (Emu(914400), Emu(457200)),
            rel: None,
            chart: None,
            name: None,
            description: None,
            wrap: wp_model::Wrap::Square,
            distance: Default::default(),
            position: None,
            behind_text: false,
            text: None,
            tone: None,
            outline: None,
        }
    }

    #[test]
    fn a_drawing_nobody_touched_comes_back_byte_for_byte() {
        // This is the whole reason the source is kept: the parts of it that are
        // not modelled cannot survive being re-authored.
        let source = r#"<w:drawing><wp:inline distT="0"><wp:extent cx="914400" cy="457200"/><a:graphic><a:weird custom="yes"/></a:graphic></wp:inline></w:drawing>"#;
        let out = patch(&drawing(source));
        assert_eq!(String::from_utf8(out).expect("utf-8"), source);
    }

    #[test]
    fn a_resized_drawing_has_both_statements_of_its_size_rewritten() {
        // `<wp:extent>` and `<a:ext>` say the same thing twice. Word keeps them
        // equal; a file where they disagree renders at one size and prints at
        // the other.
        let source = r#"<w:drawing><wp:extent cx="914400" cy="457200"/><a:ext cx="914400" cy="457200"/></w:drawing>"#;
        let mut model = drawing(source);
        model.extent = (Emu(1828800), Emu(228600));
        let out = String::from_utf8(patch(&model)).expect("utf-8");
        assert_eq!(out.matches(r#"cx="1828800""#).count(), 2);
        assert_eq!(out.matches(r#"cy="228600""#).count(), 2);
        assert!(!out.contains("914400"));
    }

    #[test]
    fn other_attributes_of_a_resized_element_keep_their_place() {
        let source =
            r#"<w:drawing><wp:extent xmlns:wp="u" cx="10" cy="20" custom="keep"/></w:drawing>"#;
        let mut model = drawing(source);
        model.extent = (Emu(30), Emu(40));
        let out = String::from_utf8(patch(&model)).expect("utf-8");
        assert!(
            out.contains(r#"<wp:extent xmlns:wp="u" cx="30" cy="40" custom="keep"/>"#),
            "{out}"
        );
    }

    #[test]
    fn a_pasted_chart_names_the_cloned_part_and_an_untouched_one_names_its_own() {
        // Word refuses a document where two drawings share one chart part, so
        // a pasted chart's part is cloned and the model holds the clone's
        // relationship. The source still says the original's; the model wins.
        let source = r#"<w:drawing><wp:inline><wp:extent cx="914400" cy="457200"/><a:graphic><a:graphicData><c:chart xmlns:c="c" xmlns:r="r" r:id="rId3"/></a:graphicData></a:graphic></wp:inline></w:drawing>"#;
        let mut model = drawing(source);
        model.chart = Some("rId8".into());
        let out = String::from_utf8(patch(&model)).expect("utf-8");
        assert!(out.contains(r#"r:id="rId8""#), "{out}");
        assert!(!out.contains("rId3"));

        // A chart nobody pasted agrees with its source and comes back
        // byte-for-byte.
        let mut model = drawing(source);
        model.chart = Some("rId3".into());
        let out = String::from_utf8(patch(&model)).expect("utf-8");
        assert_eq!(out, source);
    }

    #[test]
    fn an_authored_chart_drawing_declares_every_prefix_it_uses() {
        // The namespace lesson, kept as a tripwire: quick_xml is
        // namespace-blind and will bless an element whose prefix nothing
        // bound, and Word will refuse the whole file over it.
        let mut model = drawing("");
        model.anchored = false;
        model.chart = Some("rId9".into());
        let out = String::from_utf8(patch(&model)).expect("utf-8");
        assert!(out.contains(r#"r:id="rId9""#), "{out}");
        for prefix in ["wp", "a", "c", "r"] {
            assert!(out.contains(&format!("xmlns:{prefix}=")), "{prefix}: {out}");
        }
        assert!(
            out.contains(r#"<wp:extent cx="914400" cy="457200"/>"#),
            "{out}"
        );
        assert!(
            out.contains(r#"uri="http://schemas.openxmlformats.org/drawingml/2006/chart""#),
            "{out}"
        );
    }

    #[test]
    fn a_moved_drawing_has_the_offset_of_each_axis_rewritten() {
        let source = concat!(
            r#"<w:drawing><wp:anchor>"#,
            r#"<wp:positionH relativeFrom="column"><wp:posOffset>100</wp:posOffset></wp:positionH>"#,
            r#"<wp:positionV relativeFrom="paragraph"><wp:posOffset>200</wp:posOffset></wp:positionV>"#,
            r#"<wp:extent cx="914400" cy="457200"/></wp:anchor></w:drawing>"#,
        );
        let mut model = drawing(source);
        model.position = Some(Box::new(DrawingPosition {
            horizontal: Offset {
                relative_to: RelativeTo::Column,
                offset: Some(Emu(4444)),
                align: None,
            },
            vertical: Offset {
                relative_to: RelativeTo::Paragraph,
                offset: Some(Emu(8888)),
                align: None,
            },
        }));
        let out = String::from_utf8(patch(&model)).expect("utf-8");
        assert!(out.contains("<wp:posOffset>4444</wp:posOffset>"), "{out}");
        assert!(out.contains("<wp:posOffset>8888</wp:posOffset>"), "{out}");
        // And the axis each offset belongs to was not confused for the other.
        assert!(
            out.contains(r#"relativeFrom="column"><wp:posOffset>4444"#),
            "{out}"
        );
    }

    #[test]
    fn a_pict_is_handed_back_exactly_as_it_came() {
        // None of the DrawingML this splices exists in VML, and a watermark
        // states its size in a CSS `style` instead. Byte-for-byte is the
        // whole invariant: an untouched watermark must not be re-authored.
        let source: &[u8] = br##"<w:pict><v:shape id="PowerPlusWaterMarkObject1" type="#_x0000_t136" style="position:absolute;width:527.75pt;height:131.95pt;rotation:315" fillcolor="silver" stroked="f"><v:fill opacity=".5"/><v:textpath style="font-family:&quot;Calibri&quot;" string="CONFIDENTIAL"/></v:shape></w:pict>"##;
        let mut model = drawing("");
        model.source = source.into();
        model.extent = (Emu::from_points(1.0), Emu::from_points(1.0));
        assert_eq!(patch(&model), source, "not one byte moved");
    }

    #[test]
    fn an_authored_watermark_is_vml_this_can_read_back() {
        let mut model = drawing("");
        model.source = Vec::new().into();
        model.rel = None;
        model.extent = (Emu::from_points(529.5), Emu::from_points(132.4));
        model.text = Some(Box::new(wp_model::doc::ShapeText {
            text: "CONFIDENTIAL".into(),
            font: Some("Verdana".into()),
            color: Some(wp_model::Color::Rgb([0xE0, 0xE0, 0xE0])),
            bold: false,
            italic: false,
            stretch: true,
            rotation: 315.0,
        }));
        let out = patch(&model);
        let text = String::from_utf8(out.clone()).expect("utf-8");
        // Word finds its own watermarks by this name, and Remove Watermark
        // will not touch a shape that lacks it.
        assert!(text.contains("PowerPlusWaterMarkObject"), "{text}");
        assert!(text.contains("_x0000_t136"), "the WordArt shape type");

        let read = crate::pict::shape(&out).expect("and it reads back as words");
        let shape = read.text.expect("with its words");
        assert_eq!(&*shape.text, "CONFIDENTIAL");
        assert_eq!(shape.font.as_deref(), Some("Verdana"));
        assert_eq!(shape.rotation, 315.0);
        assert_eq!(read.extent.0.points().round(), 530.0);
        assert!(read.behind_text, "written behind the words");
    }

    #[test]
    fn an_authored_drawing_with_no_source_emits_nothing() {
        let mut model = drawing("");
        model.source = Vec::new().into();
        assert!(patch(&model).is_empty());
    }
}
