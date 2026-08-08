//! Low-level XML helpers shared by the part readers.
//!
//! These exist instead of a generic XML-to-struct layer because the hot path is
//! `<c>` elements — over a million of them in a large workbook — and what costs
//! there is *re-walking the tag*. Machine-written attributes (`r`, `s`, `t`) are
//! read as raw bytes and parsed in place; only human-authored values (sheet
//! names, formula text) take the decoding path.
//!
//! Measured on a 55 MB sheet: reading `<c>`'s three attributes with three
//! separate lookups, each with quick-xml's default duplicate checking, cost more
//! than everything else in the reader put together — 3.6 s against 2.1 s once
//! both were fixed. See `tests/large_workbook.rs`, which keeps that number
//! honest, and `xml_scan_floor`, which shows what the scan alone costs.

use std::borrow::Cow;

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::XmlVersion;

use crate::error::Result;

/// xlsx parts are XML 1.0; the version only affects end-of-line normalization.
pub(crate) const XML_VERSION: XmlVersion = XmlVersion::Explicit1_0;

/// Strips a namespace prefix: `x:c` -> `c`.
///
/// Prefix-based matching would be wrong — the prefix is arbitrary, and Excel,
/// LibreOffice, and Google Sheets each pick different ones.
pub(crate) fn strip_prefix(name: &[u8]) -> &[u8] {
    match name.iter().position(|&b| b == b':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

pub(crate) fn local_name<'a>(e: &'a BytesStart<'a>) -> &'a [u8] {
    strip_prefix(e.name().into_inner())
}

pub(crate) fn end_local_name<'a>(e: &'a BytesEnd<'a>) -> &'a [u8] {
    strip_prefix(e.name().into_inner())
}

/// Iterates a start tag's attributes with duplicate-checking switched off.
///
/// quick-xml checks for duplicate attribute names by default, which compares
/// every attribute against all the ones before it — quadratic, and re-scanning
/// the tag each time. On `<c>` elements that cost dominated the whole reader.
///
/// Dropping the check is safe here: a duplicate attribute is a malformed-file
/// problem, and rejecting the file is not our call to make. The part is retained
/// byte-for-byte regardless of what we read out of it.
pub(crate) fn attributes<'a>(
    e: &'a BytesStart<'a>,
) -> impl Iterator<Item = quick_xml::events::attributes::Attribute<'a>> {
    // `with_checks` returns a `&mut Self`, so the iterator has to be bound before
    // it is adapted or it borrows a temporary.
    let mut attrs = e.attributes();
    attrs.with_checks(false);
    attrs.flatten()
}

/// Raw attribute bytes, undecoded.
///
/// Correct only for values that cannot contain entity references or non-ASCII —
/// which covers every attribute on the hot path.
pub(crate) fn attr_raw<'a>(e: &'a BytesStart<'a>, want: &[u8]) -> Option<Cow<'a, [u8]>> {
    for a in attributes(e) {
        if strip_prefix(a.key.as_ref()) == want {
            return Some(a.value);
        }
    }
    None
}

/// Attribute decoded to text, with entity references resolved.
///
/// Use for anything a person typed: sheet names, defined names, formula text.
pub(crate) fn attr_text(e: &BytesStart<'_>, want: &[u8]) -> Option<String> {
    for a in attributes(e) {
        if strip_prefix(a.key.as_ref()) == want {
            return a.normalized_value(XML_VERSION).ok().map(|v| v.into_owned());
        }
    }
    None
}

pub(crate) fn attr_u32(e: &BytesStart<'_>, want: &[u8]) -> Option<u32> {
    let raw = attr_raw(e, want)?;
    parse_u32(&raw)
}

pub(crate) fn attr_f64(e: &BytesStart<'_>, want: &[u8]) -> Option<f64> {
    parse_f64(&attr_raw(e, want)?)
}

/// Reads an xsd:boolean attribute.
///
/// Both spellings are legal and both occur in the wild: Excel writes `1`,
/// several other producers write `true`.
pub(crate) fn parse_bool(raw: &[u8]) -> Option<bool> {
    match raw {
        b"1" | b"true" | b"TRUE" | b"True" => Some(true),
        b"0" | b"false" | b"FALSE" | b"False" => Some(false),
        _ => None,
    }
}

pub(crate) fn parse_f64(raw: &[u8]) -> Option<f64> {
    std::str::from_utf8(raw).ok()?.trim().parse().ok()
}

pub(crate) fn parse_u32(bytes: &[u8]) -> Option<u32> {
    let mut acc: u32 = 0;
    let mut any = false;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        acc = acc.checked_mul(10)?.checked_add((b - b'0') as u32)?;
        any = true;
    }
    any.then_some(acc)
}

/// Appends one text-ish event to `out`, resolving entity references.
///
/// quick-xml 0.41 does not fold entities into text: `Smith &amp; Co` arrives as
/// three events, `Text("Smith ")`, `GeneralRef("amp")`, `Text(" Co")`. Handling
/// only `Text` would silently drop every `&`, `<`, and `>` from cell strings —
/// invisible until a customer's "R&D" column came back as "RD".
///
/// Returns `true` if the event was text and was consumed.
pub(crate) fn push_text(out: &mut String, ev: &Event<'_>) -> Result<bool> {
    match ev {
        Event::Text(t) => {
            if let Ok(s) = t.xml_content(XML_VERSION) {
                out.push_str(&s);
            }
            Ok(true)
        }
        Event::CData(c) => {
            if let Ok(s) = c.xml_content(XML_VERSION) {
                out.push_str(&s);
            }
            Ok(true)
        }
        Event::GeneralRef(r) => {
            if let Ok(Some(ch)) = r.resolve_char_ref() {
                out.push(ch);
            } else if let Ok(name) = r.decode() {
                match quick_xml::escape::resolve_predefined_entity(&name) {
                    Some(text) => out.push_str(text),
                    // An entity we cannot resolve is written back as it came in.
                    // Losing it outright would corrupt the text; guessing would
                    // corrupt it differently.
                    None => {
                        out.push('&');
                        out.push_str(&name);
                        out.push(';');
                    }
                }
            }
            Ok(true)
        }
        _ => Ok(false),
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
                    push_text(&mut out, &ev).expect("text appends");
                }
            }
        }
        out
    }

    #[test]
    fn entities_survive_the_split_into_separate_events() {
        assert_eq!(text_of("<t>Smith &amp; Co</t>"), "Smith & Co");
        assert_eq!(text_of("<t>a &lt; b &gt; c</t>"), "a < b > c");
        assert_eq!(text_of("<t>&quot;quoted&quot;</t>"), "\"quoted\"");
    }

    #[test]
    fn numeric_character_references_resolve() {
        assert_eq!(text_of("<t>&#65;&#x42;</t>"), "AB");
        assert_eq!(text_of("<t>&#x4F60;&#x597D;</t>"), "\u{4F60}\u{597D}");
    }

    #[test]
    fn an_unresolvable_entity_is_kept_rather_than_dropped() {
        assert_eq!(text_of("<t>a&mystery;b</t>"), "a&mystery;b");
    }

    #[test]
    fn cdata_is_text_too() {
        assert_eq!(text_of("<t><![CDATA[a & b]]></t>"), "a & b");
    }

    #[test]
    fn significant_whitespace_is_kept() {
        // xml:space="preserve" cells are common and the spaces are the content.
        assert_eq!(text_of(r#"<t xml:space="preserve">  pad  </t>"#), "  pad  ");
    }

    #[test]
    fn namespace_prefixes_are_ignored_when_matching() {
        assert_eq!(strip_prefix(b"x:c"), b"c");
        assert_eq!(strip_prefix(b"c"), b"c");
        assert_eq!(strip_prefix(b"ns0:sheetData"), b"sheetData");
    }

    #[test]
    fn u32_parsing_rejects_junk_and_overflow() {
        assert_eq!(parse_u32(b"0"), Some(0));
        assert_eq!(parse_u32(b"1048576"), Some(1_048_576));
        assert_eq!(parse_u32(b""), None);
        assert_eq!(parse_u32(b"12a"), None);
        assert_eq!(parse_u32(b"-1"), None);
        assert_eq!(parse_u32(b"99999999999999999999"), None);
    }

    #[test]
    fn booleans_accept_both_spellings() {
        // Excel writes `1`; several other producers write `true`.
        assert_eq!(parse_bool(b"1"), Some(true));
        assert_eq!(parse_bool(b"true"), Some(true));
        assert_eq!(parse_bool(b"TRUE"), Some(true));
        assert_eq!(parse_bool(b"0"), Some(false));
        assert_eq!(parse_bool(b"false"), Some(false));
        assert_eq!(parse_bool(b"x"), None);
        assert_eq!(parse_bool(b""), None);
    }

    #[test]
    fn duplicate_attributes_do_not_abort_the_read() {
        // With quick-xml's duplicate checking left on, this element yields an
        // error mid-iteration and the later attributes are never seen.
        let e = BytesStart::from_content(r#"c r="A1" r="B2" s="4""#, 1);
        assert_eq!(attr_u32(&e, b"s"), Some(4));
    }

    #[test]
    fn attribute_text_resolves_entities() {
        let e = BytesStart::from_content(r#"sheet name="R&amp;D" val="plain""#, 5);
        assert_eq!(attr_text(&e, b"name").as_deref(), Some("R&D"));
        assert_eq!(attr_text(&e, b"val").as_deref(), Some("plain"));
    }
}
