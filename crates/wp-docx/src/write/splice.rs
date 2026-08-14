//! Reading a part while keeping the bytes it was read from.
//!
//! The same primitive as `ss-xlsx::write::splice`, and for the same reason: the
//! writer does not reprint `document.xml`, it *edits* it. Everything in a real
//! document that this crate does not model — `w:rsid*` on every element,
//! `<w:proofErr>`, smart tags, `<mc:AlternateContent>`, VML fallbacks, the
//! contents of `<m:oMath>`, and whatever the current version of Word puts in
//! `extLst` — is content we must not invent, so the original bytes *are* the
//! document and only the paragraphs that changed are replaced.
//!
//! Copying an event's exact source bytes also keeps the producer's whitespace,
//! its choice of `<w:p/>` over `<w:p></w:p>`, and its entity escaping, so a save
//! with no edits differs from the input nowhere at all.

use quick_xml::events::Event;
use quick_xml::Reader;

/// Where in the part an event came from.
///
/// A span rather than a slice so that spans can be *joined*: an element's bytes
/// are its start tag's span through its end tag's, and stitching two `&[u8]`
/// back together is not something safe Rust will do.
pub(crate) type Span = std::ops::Range<usize>;

/// A UTF-8 byte-order mark. Several of the templates Office ships begin with
/// one, and Word reads and writes it.
pub(crate) const BOM: &[u8] = b"\xEF\xBB\xBF";

/// An XML reader that hands back each event together with its source bytes.
pub(crate) struct Splicer<'a> {
    data: &'a [u8],
    reader: Reader<&'a [u8]>,
    /// Bytes of `data` the reader never saw — the byte-order mark.
    ///
    /// **quick-xml's `buffer_position` does not count the BOM**, so every span
    /// it reports is three bytes short in a document that has one. Copying spans
    /// hides it perfectly — they still tile the input, so the concatenation is
    /// byte-identical — and the moment one span is *replaced*, the output is cut
    /// three bytes early and three bytes of the next element are left behind.
    /// The reader is given the bytes after the mark and the offset is added
    /// back, so nothing downstream has to know.
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
    /// Counting nesting, because `<w:tbl>` inside `<w:tc>` inside `<w:tbl>` is
    /// ordinary and a reader that stops at the first `</w:tbl>` cuts a table in
    /// half.
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
/// `>` is escaped as well as `<` and `&`. It does not have to be, and Word does
/// it, and the point of this writer is to look like Word.
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

/// Whether a `<w:t>` needs `xml:space="preserve"`.
///
/// Decided from the text rather than remembered from the file. A reader that
/// recorded the attribute and a writer that emitted it faithfully would still be
/// wrong the moment the text was edited — and text whose leading space is
/// dropped joins two words that the document keeps apart.
pub(crate) fn needs_space_preserve(text: &str) -> bool {
    text.starts_with(|c: char| c.is_whitespace())
        || text.ends_with(|c: char| c.is_whitespace())
        || text.contains(['\t', '\n', '\r'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_byte_order_mark_does_not_shift_every_span_by_three() {
        // quick-xml does not count the mark, so without this every span in a
        // document that has one is three bytes short — invisible while spans are
        // only copied, and three bytes of damage the first time one is replaced.
        let mut xml = b"\xEF\xBB\xBF".to_vec();
        xml.extend_from_slice(b"<w:body><w:p/></w:body>");
        let mut splicer = Splicer::new(&xml);
        assert_eq!(splicer.preamble(), b"\xEF\xBB\xBF");
        let (_, body) = splicer.next().expect("the body");
        assert_eq!(splicer.bytes(body), b"<w:body>");
        let (_, paragraph) = splicer.next().expect("the paragraph");
        assert_eq!(splicer.bytes(paragraph), b"<w:p/>");
    }

    #[test]
    fn an_events_bytes_are_the_producers_own() {
        let xml = br#"<w:p><w:r><w:t xml:space="preserve"> a </w:t></w:r></w:p>"#;
        let mut splicer = Splicer::new(xml);
        let (event, span) = splicer.next().expect("a first event");
        assert!(matches!(event, Event::Start(_)));
        // Not "equivalent" — identical.
        assert_eq!(splicer.bytes(span), b"<w:p>");
    }

    #[test]
    fn an_element_span_counts_its_own_kind_nested_inside_it() {
        let xml = b"<w:tbl><w:tr><w:tc><w:tbl><w:tr/></w:tbl></w:tc></w:tr></w:tbl><w:p/>";
        let mut splicer = Splicer::new(xml);
        let (_, start) = splicer.next().expect("the outer table");
        let whole = splicer.element(b"tbl", start);
        assert_eq!(
            splicer.bytes(whole),
            &xml[..xml.len() - b"<w:p/>".len()],
            "the outer table, all of it"
        );
    }

    #[test]
    fn text_is_escaped_the_way_word_escapes_it() {
        assert_eq!(escape_text("R&D < 5 > 3"), "R&amp;D &lt; 5 &gt; 3");
        assert_eq!(escape_attr(r#"a "b" & c"#), "a &quot;b&quot; &amp; c");
        assert_eq!(escape_attr("a\tb"), "a&#9;b");
    }

    #[test]
    fn space_preserve_is_decided_from_the_text_rather_than_remembered() {
        assert!(needs_space_preserve(" leading"));
        assert!(needs_space_preserve("trailing "));
        assert!(needs_space_preserve("a\tb"));
        assert!(!needs_space_preserve("plain"));
        assert!(!needs_space_preserve("two words"));
    }
}
