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
}
