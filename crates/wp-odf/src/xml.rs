//! Low-level XML helpers, the twin of `wp_docx::xml`.
//!
//! **Elements are matched by local name and, where it matters, by prefix.**
//! ODF gives every namespace a prefix the specification itself uses — `text:`,
//! `style:`, `fo:`, `table:`, `draw:` — and no producer has been observed
//! writing anything else. Resolving prefixes to namespace URIs on every event
//! would be more correct on paper and would change no answer here; matching the
//! way `wp-docx` matches keeps one idiom in the two readers, which is worth
//! more than a distinction nothing exercises. Where two namespaces do use the
//! same local name — `style:style` beside `text:list-style`, `svg:title` beside
//! a heading's own — the reader is already inside the element that tells them
//! apart, and [`prefix`] is here for the cases that are not.
//!
//! [`push_text`] is the same as its twin for the same reason: quick-xml does
//! not fold entities into text, so an accumulator that matches only
//! `Event::Text` silently drops every `&`, `<` and `>` in the document.

use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::XmlVersion;

/// OpenDocument is XML 1.0. The version only affects end-of-line handling.
pub(crate) const XML_VERSION: XmlVersion = XmlVersion::Explicit1_0;

/// Strips a namespace prefix: `text:p` -> `p`.
pub(crate) fn strip_prefix(name: &[u8]) -> &[u8] {
    match name.iter().position(|&b| b == b':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

/// The prefix itself, empty where there is none.
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

pub(crate) fn attributes<'a>(e: &'a BytesStart<'a>) -> impl Iterator<Item = Attribute<'a>> {
    // A duplicate attribute is a malformed-file problem and rejecting the file
    // is not this reader's call: the part is preserved byte for byte whatever
    // is read out of it. With the check left on, quick-xml aborts the iteration
    // and every attribute after the duplicate is never seen.
    let mut attrs = e.attributes();
    attrs.with_checks(false);
    attrs.flatten()
}

/// An attribute's value by local name, decoded, with entity references resolved.
pub(crate) fn attr(e: &BytesStart<'_>, want: &[u8]) -> Option<String> {
    for a in attributes(e) {
        if strip_prefix(a.key.as_ref()) == want {
            return a.normalized_value(XML_VERSION).ok().map(|v| v.into_owned());
        }
    }
    None
}

/// An attribute's value by prefix *and* local name.
///
/// `fo:break-before` and `style:break-before` are not the same attribute, and
/// `svg:width` on a frame is not `style:width` on a column.
pub(crate) fn attr_in(e: &BytesStart<'_>, ns: &[u8], want: &[u8]) -> Option<String> {
    for a in attributes(e) {
        let key = a.key.as_ref();
        if prefix(key) == ns && strip_prefix(key) == want {
            return a.normalized_value(XML_VERSION).ok().map(|v| v.into_owned());
        }
    }
    None
}

/// A length written the way XSL-FO writes one: a number and a unit.
///
/// ODF states every length with its unit — there is no bare-number-means-twips
/// convention to fall back on — so a value without one is a value this cannot
/// use rather than a value in some default.
pub(crate) fn length(text: &str) -> Option<wp_model::Twips> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    // `px` is legal in ODF and is not in `ST_UniversalMeasure`. It is a CSS
    // pixel, ninety-six to the inch, and it turns up in documents converted
    // from HTML.
    if let Some(number) = text.strip_suffix("px") {
        let value: f64 = number.trim().parse().ok()?;
        return Some(wp_model::Twips::from_inches(value / 96.0));
    }
    let ends_in_a_unit = text.chars().last().is_some_and(|c| c.is_ascii_alphabetic());
    match ends_in_a_unit {
        true => wp_model::units::parse_universal(text),
        false => None,
    }
}

pub(crate) fn attr_length(e: &BytesStart<'_>, want: &[u8]) -> Option<wp_model::Twips> {
    length(&attr(e, want)?)
}

/// A percentage, as hundredths of a percent — the unit the model keeps widths
/// and scales in.
pub(crate) fn percent(text: &str) -> Option<f64> {
    text.trim().strip_suffix('%')?.trim().parse().ok()
}

/// ODF's boolean, which is spelled out in full and never abbreviated.
pub(crate) fn boolean(text: &str) -> Option<bool> {
    match text.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

pub(crate) fn attr_bool(e: &BytesStart<'_>, want: &[u8]) -> Option<bool> {
    boolean(&attr(e, want)?)
}

pub(crate) fn attr_u32(e: &BytesStart<'_>, want: &[u8]) -> Option<u32> {
    attr(e, want)?.trim().parse().ok()
}

/// A colour written the way CSS writes one, or `transparent`.
pub(crate) fn color(text: &str) -> Option<wp_model::Color> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("transparent") {
        return None;
    }
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some(wp_model::Color::Rgb([
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    ]))
}

/// Appends one text-ish event to `out`, resolving entity references.
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
                    // Written back as it came in. Dropping it would corrupt the
                    // text; guessing would corrupt it differently.
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
/// The operation an ODF reader needs most: a list holds lists, a table holds
/// tables, and an element this crate does not model may contain ones it does.
pub(crate) fn skip_element(reader: &mut quick_xml::Reader<&[u8]>, name: &[u8]) {
    let mut depth = 1usize;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if local_name(&e) == name => depth += 1,
            Ok(Event::End(e)) if end_local_name(&e) == name => {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
            Ok(Event::Eof) | Err(_) => return,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::Twips;

    #[test]
    fn a_length_carries_its_unit_and_a_bare_number_is_not_a_length() {
        assert_eq!(length("8.5in"), Some(Twips(12240)));
        assert_eq!(length("1cm"), Some(Twips(567)));
        assert_eq!(length("12pt"), Some(Twips(240)));
        assert_eq!(length("-0.25in"), Some(Twips(-360)));
        assert_eq!(length("96px"), Some(Twips(1440)), "ninety-six to the inch");
        // ODF never writes a length without one, so a number alone is a value
        // this cannot use rather than a value in some assumed unit.
        assert_eq!(length("720"), None);
        assert_eq!(length(""), None);
    }

    #[test]
    fn a_boolean_is_spelled_out_and_a_colour_is_css() {
        assert_eq!(boolean("true"), Some(true));
        assert_eq!(boolean("false"), Some(false));
        assert_eq!(boolean("1"), None, "ODF does not abbreviate it");
        assert_eq!(color("#1e6f5c"), Some(wp_model::Color::Rgb([30, 111, 92])));
        assert_eq!(color("transparent"), None);
        assert_eq!(color("#abc"), None);
        assert_eq!(percent("62.5%"), Some(62.5));
    }

    #[test]
    fn prefixes_are_stripped_for_matching_and_available_when_needed() {
        assert_eq!(strip_prefix(b"text:p"), b"p");
        assert_eq!(prefix(b"style:break-before"), b"style");
        assert_eq!(prefix(b"p"), b"");
    }

    #[test]
    fn skipping_an_element_counts_its_own_kind_nested_inside_it() {
        let xml = "<text:list><text:list-item><text:list/></text:list-item></text:list><text:p/>";
        let mut reader = quick_xml::Reader::from_str(xml);
        assert!(matches!(reader.read_event().unwrap(), Event::Start(_)));
        skip_element(&mut reader, b"list");
        match reader.read_event().unwrap() {
            Event::Empty(e) => assert_eq!(local_name(&e), b"p"),
            other => panic!("stopped in the wrong place: {other:?}"),
        }
    }
}
