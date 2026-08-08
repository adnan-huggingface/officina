//! Small XML helpers shared by the package-level parsers.
//!
//! OOXML in the wild is inconsistent about namespace prefixes — the same element
//! appears bare, as `r:`, or under a producer-specific prefix — so these match on
//! local names and let the namespace go.

use quick_xml::events::BytesStart;
use quick_xml::XmlVersion;

/// OOXML packages are XML 1.0 throughout; ECMA-376 does not permit 1.1.
const XML_VERSION: XmlVersion = XmlVersion::Explicit1_0;

/// Strips any namespace prefix from a qualified name.
fn strip_prefix(qname: &[u8]) -> &[u8] {
    match qname.iter().position(|&b| b == b':') {
        Some(i) => &qname[i + 1..],
        None => qname,
    }
}

/// Element name with any namespace prefix removed.
///
/// `into_inner` rather than `as_ref`: the latter would borrow from the temporary
/// `QName` and not outlive this call.
pub(crate) fn local_name<'a>(e: &'a BytesStart<'a>) -> &'a [u8] {
    strip_prefix(e.name().into_inner())
}

/// Attribute value by local name, with XML attribute-value normalization applied
/// (entities resolved, literal tabs and newlines folded to spaces per the XML spec).
///
/// Returns `None` for a missing attribute or one whose escaping is malformed —
/// callers treat both as "producer did not give us this".
pub(crate) fn attr(e: &BytesStart<'_>, want: &[u8]) -> Option<String> {
    for a in e.attributes().flatten() {
        if strip_prefix(a.key.as_ref()) == want {
            return a.normalized_value(XML_VERSION).ok().map(|v| v.into_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_namespace_prefixes() {
        assert_eq!(strip_prefix(b"Relationship"), b"Relationship");
        assert_eq!(strip_prefix(b"r:embed"), b"embed");
        assert_eq!(strip_prefix(b"w:p"), b"p");
    }

    #[test]
    fn reads_attributes_regardless_of_prefix() {
        let mut e = BytesStart::new("Relationship");
        e.push_attribute(("Id", "rId1"));
        e.push_attribute(("r:embed", "rId9"));
        assert_eq!(attr(&e, b"Id").as_deref(), Some("rId1"));
        assert_eq!(attr(&e, b"embed").as_deref(), Some("rId9"));
        assert_eq!(attr(&e, b"Missing"), None);
    }

    #[test]
    fn unescapes_attribute_values() {
        // Built from raw content rather than `push_attribute`, which escapes on
        // write and would leave the entity double-encoded.
        let e = BytesStart::from_content(
            r#"Relationship Target="a&amp;b&#32;c""#,
            "Relationship".len(),
        );
        assert_eq!(attr(&e, b"Target").as_deref(), Some("a&b c"));
    }
}
