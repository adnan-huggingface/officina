//! Reading `content.xml` while keeping the bytes each event came from.
//!
//! The third of these in the repository, after `ss_xlsx::write::splice` and
//! `wp_docx::write::splice`, and it exists for the reason both of those do: the
//! writer does not reprint the part, it *edits* it. An ODF document carries
//! change tracking, form controls, embedded objects, `<office:annotation>`,
//! `<text:soft-page-break>` and every element a newer version of the standard
//! has grown since this was written — none of which this crate models, all of
//! which is content nobody may invent. So the original bytes *are* the
//! document, and only the paragraphs that changed are replaced.
//!
//! Copying an event's exact source bytes keeps the producer's whitespace, its
//! choice of `<text:p/>` over `<text:p></text:p>`, its attribute order and its
//! entity escaping, so a save with no edits differs from the input nowhere at
//! all.

use quick_xml::events::Event;
use quick_xml::Reader;

/// Where in the part an event came from.
///
/// A span rather than a slice so that spans can be *joined*: an element's bytes
/// are its start tag's span through its end tag's, and stitching two `&[u8]`
/// back together is not something safe Rust will do.
pub(crate) type Span = std::ops::Range<usize>;

/// A UTF-8 byte-order mark.
///
/// Rarer here than in an OPC part — every producer of ODF observed writes
/// `content.xml` without one — but a mark that is there and is dropped moves
/// three bytes of the document, so it is handled rather than assumed away.
pub(crate) const BOM: &[u8] = b"\xEF\xBB\xBF";

/// An XML reader that hands back each event together with its source bytes.
pub(crate) struct Splicer<'a> {
    data: &'a [u8],
    reader: Reader<&'a [u8]>,
    /// Bytes of `data` the reader never saw — the byte-order mark.
    ///
    /// **quick-xml's `buffer_position` does not count the mark**, so every span
    /// it reports is three bytes short in a part that has one. Copying spans
    /// hides it perfectly — they still tile the input — and the moment one span
    /// is *replaced*, the output is cut three bytes early and three bytes of the
    /// next element are left behind. The reader is given the bytes after the
    /// mark and the offset is added back, so nothing downstream has to know.
    offset: usize,
}

impl<'a> Splicer<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        let offset = if data.starts_with(BOM) { BOM.len() } else { 0 };
        let mut reader = Reader::from_reader(&data[offset..]);
        reader.config_mut().trim_text(false);
        Splicer {
            data,
            reader,
            offset,
        }
    }

    /// The bytes before the XML itself — the byte-order mark, or nothing.
    pub(crate) fn preamble(&self) -> &'a [u8] {
        &self.data[..self.offset]
    }

    /// The next event and where it came from, or `None` at the end.
    pub(crate) fn next(&mut self) -> Option<(Event<'a>, Span)> {
        let from = self.reader.buffer_position() as usize + self.offset;
        let event = self.reader.read_event().ok()?;
        if matches!(event, Event::Eof) {
            return None;
        }
        let to = self.reader.buffer_position() as usize + self.offset;
        Some((event, from..to))
    }

    pub(crate) fn bytes(&self, span: Span) -> &'a [u8] {
        &self.data[span]
    }

    /// Reads to the end of the element whose start tag was just returned, and
    /// gives back the span of the *whole* element — start tag through end tag.
    ///
    /// Counting nesting, because a `<text:p>` holding a footnote holds
    /// `<text:p>`, a `<table:table>` in a cell holds `<table:table>`, and a
    /// reader that stopped at the first end tag would cut either in half.
    pub(crate) fn element(&mut self, name: &[u8], start: Span) -> Span {
        let mut depth = 1usize;
        let mut end = start.end;
        while let Some((event, span)) = self.next() {
            end = span.end;
            match event {
                Event::Start(e) if crate::xml::local_name(&e) == name => depth += 1,
                Event::End(e) if crate::xml::end_local_name(&e) == name => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        start.start..end
    }
}

/// Escapes text for an XML text node.
///
/// `>` is escaped as well as `<` and `&`. It does not have to be, and both
/// producers whose files are in the corpus do it.
pub(crate) fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

/// Escapes an attribute value, quotes included.
pub(crate) fn escape_attr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // A literal tab, newline or carriage return in an attribute is
            // folded to a space by every XML parser, so it has to be escaped to
            // survive being written and read back.
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_byte_order_mark_does_not_shift_every_span_by_three() {
        let mut xml = BOM.to_vec();
        xml.extend_from_slice(b"<office:text><text:p/></office:text>");
        let mut splicer = Splicer::new(&xml);
        assert_eq!(splicer.preamble(), BOM);
        let (_, body) = splicer.next().expect("the body");
        assert_eq!(splicer.bytes(body), b"<office:text>");
        let (_, paragraph) = splicer.next().expect("the paragraph");
        assert_eq!(splicer.bytes(paragraph), b"<text:p/>");
    }

    #[test]
    fn an_events_bytes_are_the_producers_own() {
        let xml = br#"<text:p text:style-name="P1">a<text:s/>b</text:p>"#;
        let mut splicer = Splicer::new(xml);
        let (event, span) = splicer.next().expect("a first event");
        assert!(matches!(event, Event::Start(_)));
        // Not "equivalent" — identical.
        assert_eq!(splicer.bytes(span), br#"<text:p text:style-name="P1">"#);
    }

    /// The case that made this count rather than search: a footnote's body is
    /// paragraphs, so the end tag of the *outer* paragraph is not the first one
    /// after its start tag.
    #[test]
    fn a_paragraph_holding_a_footnote_is_spliced_whole() {
        let xml = concat!(
            r#"<text:p>before<text:note><text:note-body>"#,
            r#"<text:p>the note</text:p></text:note-body></text:note>after</text:p>"#,
            r#"<text:p>next</text:p>"#
        )
        .as_bytes();
        let mut splicer = Splicer::new(xml);
        let (_, start) = splicer.next().expect("the outer paragraph");
        let whole = splicer.element(b"p", start);
        let held = String::from_utf8_lossy(splicer.bytes(whole)).into_owned();
        assert!(held.ends_with("after</text:p>"), "{held}");
        assert!(held.contains("the note"), "{held}");
    }

    #[test]
    fn text_is_escaped_the_way_the_format_escapes_it() {
        assert_eq!(escape_text("R&D < 5 > 3"), "R&amp;D &lt; 5 &gt; 3");
        assert_eq!(escape_attr(r#"a "b" & c"#), "a &quot;b&quot; &amp; c");
        assert_eq!(escape_attr("a\tb"), "a&#9;b");
    }
}
