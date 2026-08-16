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
        return Vec::new();
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
    out.extend_from_slice(value.to_string().as_bytes());
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
    fn an_authored_drawing_with_no_source_emits_nothing() {
        let mut model = drawing("");
        model.source = Vec::new().into();
        assert!(patch(&model).is_empty());
    }
}
