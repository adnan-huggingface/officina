//! Reading a part while keeping the bytes it was read from.
//!
//! The writer does not reprint a part from the model. It walks the original
//! bytes and substitutes only the regions it owns, because everything else in a
//! worksheet — autofilters, conditional formatting, data validation, hyperlinks,
//! the drawing anchor, page setup, `extLst` — is content we do not model and
//! must not invent. Copying an event's exact source bytes also keeps the
//! producer's whitespace, its choice of `<c/>` over `<c></c>`, and its entity
//! escaping, so a save with no edits differs from the input nowhere at all.
//!
//! Byte spans come from the reader's position rather than from re-serializing
//! the event: quick-xml can rebuild an equivalent tag, but "equivalent" is
//! exactly the word this crate is not allowed to use.

use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::error::{xml_err, Result};
use crate::xml::{attributes, strip_prefix};

/// Where in the part an event came from.
///
/// A span rather than a slice so that spans can be *joined*: an element's bytes
/// are its start tag's span through its end tag's, and stitching two `&[u8]`
/// back together is not something safe Rust will do.
pub(crate) type Span = std::ops::Range<usize>;

/// An XML reader that hands back each event together with its source bytes.
pub(crate) struct Splicer<'a> {
    part: &'a str,
    data: &'a [u8],
    reader: Reader<&'a [u8]>,
}

impl<'a> Splicer<'a> {
    pub(crate) fn new(part: &'a str, data: &'a [u8]) -> Self {
        let mut reader = Reader::from_reader(data);
        reader.config_mut().check_end_names = true;
        Splicer { part, data, reader }
    }

    /// The next event and where it came from, or `None` at the end.
    pub(crate) fn next(&mut self) -> Result<Option<(Event<'a>, Span)>> {
        let from = self.reader.buffer_position() as usize;
        let event = self
            .reader
            .read_event()
            .map_err(|e| xml_err(self.part, e))?;
        if matches!(event, Event::Eof) {
            return Ok(None);
        }
        let to = self.reader.buffer_position() as usize;
        Ok(Some((event, from..to)))
    }

    /// The bytes a span covers.
    pub(crate) fn bytes(&self, span: Span) -> &'a [u8] {
        &self.data[span]
    }
}

/// An attribute to add, change, or drop on a start tag.
pub(crate) struct Set<'a> {
    pub name: &'a [u8],
    /// `None` removes the attribute.
    pub value: Option<String>,
}

impl<'a> Set<'a> {
    pub(crate) fn to(name: &'a [u8], value: impl Into<String>) -> Self {
        Set {
            name,
            value: Some(value.into()),
        }
    }

    pub(crate) fn off(name: &'a [u8]) -> Self {
        Set { name, value: None }
    }

    /// Sets the attribute when there is a value and removes it when there is not.
    ///
    /// Excel omits rather than writes a default — `hidden="0"` and `hidden`
    /// absent mean the same thing, but only one of them is what Excel wrote.
    pub(crate) fn maybe(name: &'a [u8], value: Option<String>) -> Self {
        Set { name, value }
    }
}

/// Rewrites a start tag, keeping the attributes that were not named.
///
/// The element's own qualified name is copied through, prefix included: the
/// prefix is the producer's choice and rewriting `x:row` as `row` would be
/// correct XML and a diff on every line.
pub(crate) fn retag(e: &BytesStart<'_>, sets: &[Set<'_>], empty: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(e.len() + 16);
    out.push(b'<');
    out.extend_from_slice(e.name().into_inner());

    let mut written = vec![false; sets.len()];
    for a in attributes(e) {
        match sets
            .iter()
            .position(|s| s.name == strip_prefix(a.key.as_ref()))
        {
            Some(i) => {
                written[i] = true;
                if let Some(value) = &sets[i].value {
                    push_attr(&mut out, a.key.as_ref(), escape_value(value).as_bytes());
                }
            }
            // Not ours: the raw bytes go back exactly as they arrived, still
            // escaped the way the producer escaped them.
            None => push_attr(&mut out, a.key.as_ref(), &a.value),
        }
    }
    for (set, done) in sets.iter().zip(written) {
        if done {
            continue;
        }
        if let Some(value) = &set.value {
            push_attr(&mut out, set.name, escape_value(value).as_bytes());
        }
    }

    if empty {
        out.push(b'/');
    }
    out.push(b'>');
    out
}

/// The namespace prefix on an element name, colon included, or nothing.
///
/// Kept so that elements we author match the ones around them. Excel writes no
/// prefix on spreadsheetML, but a file that does should not come back mixing
/// `<row>` and `<x:row>`.
pub(crate) fn prefix_of(e: &BytesStart<'_>) -> Vec<u8> {
    let name = e.name().into_inner();
    match name.iter().position(|&b| b == b':') {
        Some(i) => name[..=i].to_vec(),
        None => Vec::new(),
    }
}

/// Writes a start tag we are authoring rather than editing.
pub(crate) fn open(out: &mut Vec<u8>, prefix: &[u8], name: &[u8], sets: &[Set<'_>], empty: bool) {
    out.push(b'<');
    out.extend_from_slice(prefix);
    out.extend_from_slice(name);
    for set in sets {
        if let Some(value) = &set.value {
            push_attr(out, set.name, escape_value(value).as_bytes());
        }
    }
    if empty {
        out.push(b'/');
    }
    out.push(b'>');
}

/// Writes the matching end tag.
pub(crate) fn close(out: &mut Vec<u8>, prefix: &[u8], name: &[u8]) {
    out.extend_from_slice(b"</");
    out.extend_from_slice(prefix);
    out.extend_from_slice(name);
    out.push(b'>');
}

fn push_attr(out: &mut Vec<u8>, key: &[u8], value: &[u8]) {
    out.push(b' ');
    out.extend_from_slice(key);
    out.extend_from_slice(b"=\"");
    // The value may have been single-quoted in the original, in which case it
    // can hold a bare `"`. We always double-quote, so that one byte has to be
    // escaped; everything else is already escaped as the file had it.
    for &b in value {
        match b {
            b'"' => out.extend_from_slice(b"&quot;"),
            _ => out.push(b),
        }
    }
    out.push(b'"');
}

/// Escapes text for an attribute value we are authoring ourselves.
pub(crate) fn escape_value(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // A literal tab or newline in an attribute is folded to a space by
            // XML attribute-value normalization, so it has to be a character
            // reference to survive.
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Escapes character data we are authoring ourselves.
pub(crate) fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            _ => out.push(ch),
        }
    }
    out
}

/// The value of `want`, raw and still escaped, from a start tag.
pub(crate) fn raw_attr<'a>(e: &'a BytesStart<'a>, want: &[u8]) -> Option<Attribute<'a>> {
    attributes(e).find(|a| strip_prefix(a.key.as_ref()) == want)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(xml: &str) -> Vec<String> {
        let mut splicer = Splicer::new("test.xml", xml.as_bytes());
        let mut out = Vec::new();
        while let Some((_, span)) = splicer.next().expect("parses") {
            out.push(String::from_utf8(splicer.bytes(span).to_vec()).expect("utf-8"));
        }
        out
    }

    #[test]
    fn the_source_bytes_of_every_event_reassemble_the_document() {
        // This is the whole guarantee: a writer that copies every event through
        // produces the input, byte for byte.
        let xml = concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            "\n<worksheet xmlns=\"http://x\"><sheetData>\n",
            r#"  <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1"/></row>"#,
            "\n</sheetData><!-- kept --></worksheet>",
        );
        assert_eq!(events(xml).concat(), xml);
    }

    #[test]
    fn spans_cover_cdata_entities_and_processing_instructions() {
        let xml = r#"<a><![CDATA[<b>]]>&amp;<?pi go?></a>"#;
        assert_eq!(events(xml).concat(), xml);
    }

    fn start(xml: &str) -> BytesStart<'static> {
        let mut reader = Reader::from_str(xml);
        match reader.read_event().expect("parses") {
            Event::Start(e) | Event::Empty(e) => e.into_owned(),
            other => panic!("expected a start tag, got {other:?}"),
        }
    }

    #[test]
    fn retagging_changes_what_it_is_asked_to_and_nothing_else() {
        let e = start(r#"<row r="1" spans="1:4" customFormat="1" s="7">"#);
        let out = retag(&e, &[Set::to(b"r", "9"), Set::off(b"s")], false);
        assert_eq!(
            String::from_utf8(out).expect("utf-8"),
            r#"<row r="9" spans="1:4" customFormat="1">"#,
            "spans and customFormat are not ours to touch"
        );
    }

    #[test]
    fn a_new_attribute_is_appended_and_the_prefix_survives() {
        let e = start(r#"<x:row x:r="1">"#);
        let out = retag(&e, &[Set::to(b"ht", "20")], true);
        assert_eq!(
            String::from_utf8(out).expect("utf-8"),
            r#"<x:row x:r="1" ht="20"/>"#
        );
    }

    #[test]
    fn a_single_quoted_value_survives_being_double_quoted() {
        // Legal XML, and it means the value may contain a bare double quote.
        let e = start(r#"<c r='A1' t='say "hi"'>"#);
        let out = retag(&e, &[], false);
        assert_eq!(
            String::from_utf8(out).expect("utf-8"),
            r#"<c r="A1" t="say &quot;hi&quot;">"#
        );
    }

    #[test]
    fn authored_text_is_escaped_but_not_over_escaped() {
        assert_eq!(escape_text("R&D <x>"), "R&amp;D &lt;x&gt;");
        assert_eq!(escape_text("plain"), "plain");
        assert_eq!(
            escape_value("a\tb"),
            "a&#9;b",
            "a tab would fold to a space"
        );
    }
}
