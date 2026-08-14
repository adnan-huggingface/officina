//! Low-level XML helpers.
//!
//! A twin of `ss-xlsx::xml`, and deliberately not shared with it: that one is
//! tuned for a hot path of a million `<c>` elements whose attributes are
//! machine-written ASCII, and this one's hot path is deeply nested prose where
//! nearly every value is something a person typed. The two want different
//! defaults, and merging them would mean the spreadsheet reader paying for
//! decoding it does not need.
//!
//! What *is* the same, and must stay the same, is [`push_text`]. quick-xml 0.41
//! does not fold entities into text — `R&amp;D` arrives as three events — so any
//! accumulator that matches only `Event::Text` silently drops every `&`, `<` and
//! `>` in the document.

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::XmlVersion;

/// WordprocessingML is XML 1.0. The version only affects end-of-line handling.
pub(crate) const XML_VERSION: XmlVersion = XmlVersion::Explicit1_0;

/// Strips a namespace prefix: `w:p` -> `p`.
///
/// Prefix matching would be wrong. `w` is the convention and nothing requires
/// it; the same document uses `w14`, `w15`, `mc` and `wp` for elements that a
/// reader must tell apart by local name and namespace, not by prefix.
pub(crate) fn strip_prefix(name: &[u8]) -> &[u8] {
    match name.iter().position(|&b| b == b':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

/// The prefix itself, empty when there is none.
///
/// Needed because a handful of elements *are* distinguished by namespace and not
/// by local name: `<w:drawing>` and `<wp:anchor>`, `<m:t>` inside an equation
/// against `<w:t>` in a run.
pub(crate) fn prefix(name: &[u8]) -> &[u8] {
    match name.iter().position(|&b| b == b':') {
        Some(i) => &name[..i],
        None => b"",
    }
}

pub(crate) fn local_name<'a>(e: &'a BytesStart<'a>) -> &'a [u8] {
    strip_prefix(e.name().into_inner())
}

pub(crate) fn end_local_name<'a>(e: &'a BytesEnd<'a>) -> &'a [u8] {
    strip_prefix(e.name().into_inner())
}

pub(crate) fn attributes<'a>(
    e: &'a BytesStart<'a>,
) -> impl Iterator<Item = quick_xml::events::attributes::Attribute<'a>> {
    // A duplicate attribute is a malformed-file problem and rejecting the file
    // is not our call: the part is preserved byte for byte regardless of what we
    // read out of it. With the check left on, quick-xml aborts the iteration and
    // every attribute after the duplicate is never seen.
    let mut attrs = e.attributes();
    attrs.with_checks(false);
    attrs.flatten()
}

/// An attribute's value, decoded, with entity references resolved.
pub(crate) fn attr(e: &BytesStart<'_>, want: &[u8]) -> Option<String> {
    for a in attributes(e) {
        if strip_prefix(a.key.as_ref()) == want {
            return a.normalized_value(XML_VERSION).ok().map(|v| v.into_owned());
        }
    }
    None
}

/// The `w:val` attribute, which is where WordprocessingML puts nearly every
/// value it has.
pub(crate) fn val(e: &BytesStart<'_>) -> Option<String> {
    attr(e, b"val")
}

/// An on/off element's value: **absent means true**, because `<w:b/>` is bold.
pub(crate) fn on_off(e: &BytesStart<'_>) -> bool {
    wp_model::prop::on_off(val(e).as_deref())
}

pub(crate) fn attr_i32(e: &BytesStart<'_>, want: &[u8]) -> Option<i32> {
    wp_model::units::parse_i32(&attr(e, want)?)
}

pub(crate) fn attr_u32(e: &BytesStart<'_>, want: &[u8]) -> Option<u32> {
    attr_i32(e, want)?.try_into().ok()
}

/// A measurement attribute that may carry its own unit suffix.
pub(crate) fn attr_twips(e: &BytesStart<'_>, want: &[u8]) -> Option<wp_model::Twips> {
    wp_model::units::parse_universal(&attr(e, want)?)
}

/// Appends one text-ish event to `out`, resolving entity references.
///
/// See the module note. Returns `true` if the event was text and was consumed.
pub(crate) fn push_text(out: &mut String, ev: &Event<'_>) -> bool {
    match ev {
        Event::Text(t) => {
            if let Ok(s) = t.xml_content(XML_VERSION) {
                out.push_str(&s);
            }
            true
        }
        Event::CData(c) => {
            if let Ok(s) = c.xml_content(XML_VERSION) {
                out.push_str(&s);
            }
            true
        }
        Event::GeneralRef(r) => {
            if let Ok(Some(ch)) = r.resolve_char_ref() {
                out.push(ch);
            } else if let Ok(name) = r.decode() {
                match quick_xml::escape::resolve_predefined_entity(&name) {
                    Some(text) => out.push_str(text),
                    // An entity we cannot resolve is written back as it came in.
                    // Dropping it would corrupt the text; guessing would corrupt
                    // it differently.
                    None => {
                        out.push('&');
                        out.push_str(&name);
                        out.push(';');
                    }
                }
            }
            true
        }
        _ => false,
    }
}

/// Skips to the end of the element currently open, counting nesting.
///
/// The one operation a reader of this format needs constantly: WordprocessingML
/// nests deeply, and an element we do not model may contain ones we do.
pub(crate) fn skip_element(
    reader: &mut quick_xml::Reader<&[u8]>,
    name: &[u8],
) -> quick_xml::Result<()> {
    let mut depth = 1usize;
    loop {
        match reader.read_event()? {
            Event::Start(e) if local_name(&e) == name => depth += 1,
            Event::End(e) if end_local_name(&e) == name => {
                depth -= 1;
                if depth == 0 {
                    return Ok(());
                }
            }
            Event::Eof => return Ok(()),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quick_xml::Reader;

    fn text_of(xml: &str) -> String {
        let mut reader = Reader::from_str(xml);
        let mut out = String::new();
        let mut depth = 0usize;
        loop {
            match reader.read_event().expect("test xml parses") {
                Event::Start(_) => depth += 1,
                Event::End(_) => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                Event::Eof => break,
                ev => {
                    push_text(&mut out, &ev);
                }
            }
        }
        out
    }

    #[test]
    fn entities_survive_the_split_into_separate_events() {
        assert_eq!(text_of("<w:t>Smith &amp; Co</w:t>"), "Smith & Co");
        assert_eq!(text_of("<w:t>a &lt; b &gt; c</w:t>"), "a < b > c");
        assert_eq!(text_of("<w:t>&#8212;</w:t>"), "\u{2014}");
    }

    #[test]
    fn an_unresolvable_entity_is_kept_rather_than_dropped() {
        assert_eq!(text_of("<w:t>a&mystery;b</w:t>"), "a&mystery;b");
    }

    #[test]
    fn significant_whitespace_is_the_content() {
        // `xml:space="preserve"` runs are how Word writes the space between two
        // differently formatted words, and losing them joins the words.
        assert_eq!(text_of(r#"<w:t xml:space="preserve"> and </w:t>"#), " and ");
    }

    #[test]
    fn prefixes_are_stripped_for_matching_and_available_when_needed() {
        assert_eq!(strip_prefix(b"w:p"), b"p");
        assert_eq!(strip_prefix(b"p"), b"p");
        assert_eq!(prefix(b"w14:paraId"), b"w14");
        assert_eq!(prefix(b"p"), b"");
    }

    #[test]
    fn a_bare_on_off_element_is_true() {
        let bare = BytesStart::new("w:b");
        assert!(on_off(&bare));
        let off = BytesStart::from_content(r#"w:b w:val="0""#, 3);
        assert!(!on_off(&off));
    }

    #[test]
    fn skipping_an_element_counts_its_own_kind_nested_inside_it() {
        // A table inside a table cell is the case that breaks a naive skip.
        let xml = "<w:tbl><w:tr><w:tc><w:tbl><w:tr/></w:tbl></w:tc></w:tr></w:tbl><w:p/>";
        let mut reader = Reader::from_str(xml);
        assert!(matches!(reader.read_event().unwrap(), Event::Start(_)));
        skip_element(&mut reader, b"tbl").unwrap();
        match reader.read_event().unwrap() {
            Event::Empty(e) => assert_eq!(local_name(&e), b"p"),
            other => panic!("stopped in the wrong place: {other:?}"),
        }
    }
}
