//! Reading and writing the notes on a sheet's cells.
//!
//! A note lives in three places at once, which is the whole difficulty:
//!
//! * `xl/comments1.xml` holds the author list and the text.
//! * `xl/drawings/vmlDrawing1.vml` holds the yellow box that draws it — VML,
//!   a format Microsoft deprecated in 2007 and which Excel still requires
//!   here. A comments part with no shape beside it opens, but Excel offers to
//!   repair the file, which is worse than the note being plain.
//! * The worksheet points at the VML with `<legacyDrawing r:id="…"/>`.
//!
//! Only the first is really modeled. The VML is authored when a sheet gains
//! its first note and left alone afterwards: it says where a box would appear
//! if the user asked to see it, and nothing in Calx moves those boxes.

use quick_xml::events::Event;
use quick_xml::Reader;

use ss_model::{CellRef, Comment};

use crate::error::{xml_err, Result};
use crate::xml::{attr_raw, local_name, parse_u32, push_text};

/// Parses a comments part into the notes it holds.
pub(crate) fn parse(part: &str, data: &[u8]) -> Result<Vec<Comment>> {
    let mut reader = Reader::from_reader(data);
    reader.config_mut().check_end_names = false;

    let mut authors: Vec<String> = Vec::new();
    let mut out: Vec<Comment> = Vec::new();
    let mut buf = Vec::new();

    let mut in_author = false;
    let mut in_text = false;
    // `<t>` inside `<rPh>` is a phonetic reading of the run beside it, not part
    // of what the note says — the same trap shared strings hold.
    let mut in_phonetic = 0usize;
    let mut text = String::new();
    let mut author_text = String::new();
    let mut at = CellRef::new(0, 0);
    let mut author = 0u32;

    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err(part, e))?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => match local_name(e) {
                b"author" => {
                    in_author = true;
                    author_text.clear();
                }
                b"comment" => {
                    at = attr_raw(e, b"ref")
                        .as_deref()
                        .and_then(crate::sheet::parse_a1_bytes)
                        .unwrap_or(CellRef::new(0, 0));
                    author = attr_raw(e, b"authorId")
                        .and_then(|v| parse_u32(&v))
                        .unwrap_or(0);
                    text.clear();
                }
                b"text" => in_text = true,
                b"rPh" => in_phonetic += 1,
                _ => {}
            },
            Event::End(ref e) => match crate::xml::end_local_name(e) {
                b"author" => {
                    in_author = false;
                    authors.push(std::mem::take(&mut author_text));
                }
                b"text" => in_text = false,
                b"rPh" => in_phonetic = in_phonetic.saturating_sub(1),
                b"comment" => out.push(Comment {
                    at,
                    author: authors.get(author as usize).cloned().unwrap_or_default(),
                    text: std::mem::take(&mut text),
                }),
                _ => {}
            },
            Event::Eof => break,
            ref other => {
                if in_author {
                    push_text(&mut author_text, other)?;
                } else if in_text && in_phonetic == 0 {
                    push_text(&mut text, other)?;
                }
            }
        }
        buf.clear();
    }
    Ok(out)
}

/// Authors a whole comments part.
///
/// Written whole rather than spliced, unlike almost everything else here. A
/// note is text and an address and nothing else, so there is nothing in the
/// part worth preserving that the model does not already hold — and the
/// alternative, editing one `<comment>` inside a list whose author indices all
/// shift, is more ways to be wrong than writing four lines of XML.
pub(crate) fn write(comments: &[Comment]) -> Vec<u8> {
    let mut authors: Vec<&str> = Vec::new();
    for note in comments {
        if !authors.contains(&note.author.as_str()) {
            authors.push(&note.author);
        }
    }

    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><authors>"#,
    );
    for author in &authors {
        out.push_str(&format!("<author>{}</author>", escape(author)));
    }
    out.push_str("</authors><commentList>");
    for note in comments {
        let id = authors
            .iter()
            .position(|a| *a == note.author)
            .unwrap_or_default();
        out.push_str(&format!(
            r#"<comment ref="{}" authorId="{id}"><text><r><t xml:space="preserve">{}</t></r></text></comment>"#,
            note.at.to_a1(),
            escape(&note.text),
        ));
    }
    out.push_str("</commentList></comments>");
    out.into_bytes()
}

/// The VML shape Excel wants beside each note.
///
/// Every box is written hidden and the same size, which is what Excel does for
/// a note nobody has dragged. Positions are given as an anchor in the units
/// VML uses — cell, offset, cell, offset — rather than in points, so a note
/// stays beside its cell when the column is widened.
pub(crate) fn vml(comments: &[Comment]) -> Vec<u8> {
    let mut out = String::from(
        r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel">"#,
    );
    out.push_str(r#"<o:shapelayout v:ext="edit"><o:idmap v:ext="edit" data="1"/></o:shapelayout>"#);
    out.push_str(
        r#"<v:shapetype id="_x0000_t202" coordsize="21600,21600" o:spt="202" path="m,l,21600r21600,l21600,xe"><v:stroke joinstyle="miter"/><v:path gradientshapeok="t" o:connecttype="rect"/></v:shapetype>"#,
    );
    for (index, note) in comments.iter().enumerate() {
        let id = 1025 + index;
        out.push_str(&format!(
            r##"<v:shape id="_x0000_s{id}" type="#_x0000_t202" style="position:absolute;width:108pt;height:59.25pt;z-index:{};visibility:hidden" fillcolor="#ffffe1" o:insetmode="auto">"##,
            index + 1
        ));
        out.push_str(r##"<v:fill color2="#ffffe1"/><v:shadow on="t" color="black" obscured="t"/>"##);
        out.push_str(r#"<v:path o:connecttype="none"/><v:textbox style="mso-direction-alt:auto"><div style="text-align:left"></div></v:textbox>"#);
        out.push_str(r#"<x:ClientData ObjectType="Note"><x:MoveWithCells/><x:SizeWithCells/>"#);
        // The eight numbers are the box's corners: column, offset, row, offset
        // for each. Two columns right of the cell and two rows down, which is
        // where Excel puts a note it has just made.
        out.push_str(&format!(
            "<x:Anchor>{}, 15, {}, 2, {}, 15, {}, 4</x:Anchor>",
            note.at.col + 1,
            note.at.row,
            note.at.col + 3,
            note.at.row + 4,
        ));
        out.push_str("<x:AutoFill>False</x:AutoFill>");
        out.push_str(&format!(
            "<x:Row>{}</x:Row><x:Column>{}</x:Column>",
            note.at.row, note.at.col
        ));
        out.push_str("</x:ClientData></v:shape>");
    }
    out.push_str("</xml>");
    out.into_bytes()
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_is_read_with_its_author_and_its_text() {
        let xml = br#"<?xml version="1.0"?>
            <comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
              <authors><author>Ada</author><author>Grace</author></authors>
              <commentList>
                <comment ref="B2" authorId="1"><text><r><rPr><b/></rPr><t>Grace:</t></r><r><t xml:space="preserve">
check this</t></r></text></comment>
              </commentList>
            </comments>"#;
        let notes = parse("/xl/comments1.xml", xml).expect("parses");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].at, CellRef::from_a1("B2").expect("valid"));
        assert_eq!(notes[0].author, "Grace");
        assert_eq!(notes[0].text, "Grace:\ncheck this");
        assert_eq!(notes[0].body(), "check this");
    }

    #[test]
    fn a_note_written_here_reads_back_as_itself() {
        let notes = vec![
            Comment::new(CellRef::new(1, 1), "Ada", "Ada:\nlooks high & wide"),
            Comment::new(CellRef::new(4, 0), "Grace", "Grace:\nfrom the ledger"),
        ];
        let read = parse("/xl/comments1.xml", &write(&notes)).expect("parses");
        assert_eq!(read, notes);
    }

    #[test]
    fn two_notes_by_one_author_list_that_author_once() {
        let notes = vec![
            Comment::new(CellRef::new(0, 0), "Ada", "one"),
            Comment::new(CellRef::new(1, 0), "Ada", "two"),
        ];
        let text = String::from_utf8(write(&notes)).expect("utf-8");
        assert_eq!(text.matches("<author>").count(), 1, "{text}");
        assert_eq!(text.matches(r#"authorId="0""#).count(), 2, "{text}");
    }

    #[test]
    fn every_note_gets_a_shape_of_its_own_with_its_own_id() {
        let notes = vec![
            Comment::new(CellRef::new(0, 0), "Ada", "one"),
            Comment::new(CellRef::new(9, 3), "Ada", "two"),
        ];
        let text = String::from_utf8(vml(&notes)).expect("utf-8");
        assert!(text.contains(r#"id="_x0000_s1025""#), "{text}");
        assert!(text.contains(r#"id="_x0000_s1026""#), "{text}");
        assert!(
            text.contains("<x:Row>9</x:Row><x:Column>3</x:Column>"),
            "a shape names the cell it belongs to: {text}"
        );
    }
}
