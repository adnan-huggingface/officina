//! Semantic comparison of two packages — the engine behind the fidelity harness.
//!
//! A byte-level zip diff is useless for this job: compression levels, entry
//! ordering, and attribute ordering all vary without changing what the document
//! *means*. Comparing rendered output is the opposite problem — too weak to catch
//! a dropped custom-XML part that nothing on page 1 depends on.
//!
//! So this compares packages the way a consumer sees them: same set of parts, and
//! for each part, XML that is equivalent under the rules XML itself says are
//! insignificant.
//!
//! What is deliberately treated as **significant**, because Word cares:
//! - element order (`<w:p>` before `<w:tbl>` is a different document)
//! - text content, exactly, including whitespace inside a leaf element
//! - attribute values
//!
//! What is treated as **insignificant**:
//! - attribute order within an element
//! - whitespace-only text between sibling elements (indentation)
//! - the XML declaration's exact spelling
//! - zip entry order and compression method

use std::collections::BTreeSet;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::name::PartName;
use crate::package::Package;

/// One way in which two packages differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Difference {
    /// Present in the original, absent from the rewrite. The failure that matters.
    PartLost(PartName),
    /// Absent from the original, present in the rewrite.
    PartAdded(PartName),
    /// Same part name, different content type declared.
    ContentTypeChanged {
        part: PartName,
        before: String,
        after: String,
    },
    /// Same part name, semantically different XML.
    XmlChanged { part: PartName, detail: String },
    /// Same part name, different bytes in a non-XML part.
    BinaryChanged {
        part: PartName,
        before_len: usize,
        after_len: usize,
    },
}

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Difference::PartLost(p) => write!(f, "part lost: {p}"),
            Difference::PartAdded(p) => write!(f, "part added: {p}"),
            Difference::ContentTypeChanged {
                part,
                before,
                after,
            } => {
                write!(f, "content type changed for {part}: {before} -> {after}")
            }
            Difference::XmlChanged { part, detail } => write!(f, "xml changed in {part}: {detail}"),
            Difference::BinaryChanged {
                part,
                before_len,
                after_len,
            } => write!(
                f,
                "binary content changed in {part}: {before_len} -> {after_len} bytes"
            ),
        }
    }
}

/// One part of a package, as much of it as a comparison needs.
///
/// A borrowed view rather than a trait, so that a container which is not an OPC
/// package — an ODF one, which has a manifest where this has content types and
/// no relationships at all — is compared by the same code and to the same
/// standard. The alternative was a second comparison beside this one, which is
/// the fastest way to end up with two definitions of "faithful".
#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    pub name: &'a PartName,
    /// The content type for OPC, the manifest's media type for ODF. Both are
    /// the same claim: what a consumer will take these bytes to be.
    pub kind: &'a str,
    pub data: &'a [u8],
}

/// Compares `before` against `after`, returning every difference found.
///
/// An empty result means the rewrite is faithful.
pub fn diff(before: &Package, after: &Package) -> Vec<Difference> {
    fn entries(package: &Package) -> Vec<Entry<'_>> {
        package
            .parts()
            .map(|part| Entry {
                name: &part.name,
                kind: &part.content_type,
                data: part.data(),
            })
            .collect()
    }
    diff_entries(&entries(before), &entries(after))
}

/// The same comparison, over whatever a container can hand out.
pub fn diff_entries(before: &[Entry<'_>], after: &[Entry<'_>]) -> Vec<Difference> {
    let mut out = Vec::new();
    let find = |entries: &[Entry<'_>], name: &PartName| -> Option<(String, Vec<u8>)> {
        entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| (entry.kind.to_string(), entry.data.to_vec()))
    };

    let names_before: BTreeSet<&PartName> = before.iter().map(|e| e.name).collect();
    let names_after: BTreeSet<&PartName> = after.iter().map(|e| e.name).collect();

    for lost in names_before.difference(&names_after) {
        out.push(Difference::PartLost((*lost).clone()));
    }
    for added in names_after.difference(&names_before) {
        out.push(Difference::PartAdded((*added).clone()));
    }

    for name in names_before.intersection(&names_after) {
        let a = find(before, name).expect("name came from this list");
        let b = find(after, name).expect("name came from this list");

        if a.0 != b.0 {
            out.push(Difference::ContentTypeChanged {
                part: (*name).clone(),
                before: a.0.clone(),
                after: b.0.clone(),
            });
        }

        if a.1 == b.1 {
            continue; // fast path: identical bytes are identical meaning
        }

        if looks_like_xml(&a.1) && looks_like_xml(&b.1) {
            match (canonicalize(&a.1), canonicalize(&b.1)) {
                (Ok(ca), Ok(cb)) if equivalent(&ca, &cb) => {}
                (Ok(ca), Ok(cb)) => out.push(Difference::XmlChanged {
                    part: (*name).clone(),
                    detail: first_divergence(&ca, &cb),
                }),
                // Unparseable XML on either side is itself the finding.
                (a_res, b_res) => out.push(Difference::XmlChanged {
                    part: (*name).clone(),
                    detail: format!(
                        "could not canonicalize: before={:?} after={:?}",
                        a_res.err(),
                        b_res.err()
                    ),
                }),
            }
        } else {
            out.push(Difference::BinaryChanged {
                part: (*name).clone(),
                before_len: a.1.len(),
                after_len: b.1.len(),
            });
        }
    }

    out
}

/// Cheap sniff: does this part look like XML rather than an image or OLE blob?
fn looks_like_xml(data: &[u8]) -> bool {
    let head = &data[..data.len().min(256)];
    let start = head
        .iter()
        .position(|b| !b.is_ascii_whitespace() && *b != 0xEF && *b != 0xBB && *b != 0xBF);
    matches!(start, Some(i) if head[i] == b'<')
}

/// A normalized token stream for one XML part.
///
/// Comparing token streams rather than strings makes the first divergence easy to
/// report, which is what a preservation bug needs in order to be fixed.
type Canonical = Vec<String>;

fn canonicalize(data: &[u8]) -> std::result::Result<Canonical, String> {
    let mut reader = Reader::from_reader(data);
    // Left off deliberately: trimming here would erase significant whitespace
    // inside `<w:t xml:space="preserve">`. Ignorable whitespace is dropped below
    // by inspecting context instead.
    reader.config_mut().trim_text(false);

    let mut out: Canonical = Vec::new();
    let mut buf = Vec::new();
    // Text accumulated since the last element boundary, held until we know whether
    // the enclosing element has element children (making it indentation) or not
    // (making it content).
    let mut pending_text = String::new();
    let mut saw_child_element = vec![false];

    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("{e} at byte {}", reader.buffer_position()))?;

        match event {
            Event::Start(e) => {
                open_element(&mut out, &mut pending_text, &mut saw_child_element, &e);
                saw_child_element.push(false);
            }
            Event::Empty(e) => {
                // `<a/>` has no End event, so it opens and closes in one step and
                // must not leave a frame on the depth stack.
                open_element(&mut out, &mut pending_text, &mut saw_child_element, &e);
                out.push("</>".to_string());
            }
            Event::End(_) => {
                flush_text(&mut out, &mut pending_text, &saw_child_element, false);
                saw_child_element.pop();
                out.push("</>".to_string());
            }
            Event::Text(t) => {
                let decoded = t
                    .decode()
                    .map_err(|e| format!("undecodable text: {e}"))?
                    .into_owned();
                pending_text.push_str(&decoded);
            }
            Event::CData(c) => {
                pending_text.push_str(&String::from_utf8_lossy(&c));
            }
            // Declaration, comments, processing instructions and DTDs carry no
            // document meaning for our purposes.
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) | Event::DocType(_) => {}
            Event::GeneralRef(_) => {}
            Event::Eof => break,
        }
        buf.clear();
    }

    Ok(out)
}

/// Emits the opening token for an element: local name plus attributes sorted by
/// name, with namespace declarations dropped.
fn open_element(
    out: &mut Canonical,
    pending_text: &mut String,
    saw_child_element: &mut [bool],
    e: &quick_xml::events::BytesStart<'_>,
) {
    flush_text(out, pending_text, saw_child_element, true);
    if let Some(last) = saw_child_element.last_mut() {
        *last = true;
    }

    let name = String::from_utf8_lossy(strip_ns(e.name().into_inner())).into_owned();

    let mut attrs: Vec<(String, String)> = Vec::new();
    for a in e.attributes().flatten() {
        // Namespace declarations are structural noise here: a prefix rename does
        // not change meaning, and local names are what we compare.
        if a.key.as_ref() == b"xmlns" || a.key.as_ref().starts_with(b"xmlns:") {
            continue;
        }
        let key = String::from_utf8_lossy(strip_ns(a.key.as_ref())).into_owned();
        let val = a
            .normalized_value(crate::xml::XML_VERSION)
            .map(|v| v.into_owned())
            .unwrap_or_else(|_| String::from_utf8_lossy(&a.value).into_owned());
        attrs.push((key, val));
    }
    attrs.sort();

    let rendered = attrs
        .iter()
        .map(|(k, v)| format!("{k}={v:?}"))
        .collect::<Vec<_>>()
        .join(" ");
    out.push(format!("<{name} {rendered}>"));
}

/// Emits accumulated text, unless it is whitespace-only indentation.
///
/// `element_follows` is true when this flush happens because an element tag is
/// about to be emitted. That case needs no lookahead: whitespace sitting directly
/// before a tag is indentation by construction, including the run before a
/// parent's *first* child, which a "has the parent seen an element yet" test
/// would wrongly keep.
///
/// At a closing tag there is no such guarantee, so the parent's history decides:
/// whitespace inside a leaf element is content. `<w:t xml:space="preserve"> </w:t>`
/// is a real space in a real document, and dropping it is a real bug.
fn flush_text(
    out: &mut Canonical,
    pending: &mut String,
    saw_child_element: &[bool],
    element_follows: bool,
) {
    if pending.is_empty() {
        return;
    }
    let is_ws_only = pending.chars().all(char::is_whitespace);
    let parent_has_element_children = saw_child_element.last().copied().unwrap_or(false);

    let ignorable = is_ws_only && (element_follows || parent_has_element_children);
    if !ignorable {
        out.push(format!("#{pending}"));
    }
    pending.clear();
}

fn strip_ns(qname: &[u8]) -> &[u8] {
    match qname.iter().position(|&b| b == b':') {
        Some(i) => &qname[i + 1..],
        None => qname,
    }
}

/// Whether two canonical streams say the same thing.
///
/// Token-for-token, except that two text nodes holding the same `f64` are the
/// same text node whatever digits they are spelled with. Excel writes the cached
/// value of a percentage cell as `0.42709999999999998`; the shortest string that
/// reads back as that exact double is `0.4271`, and a writer that produces the
/// short form has changed nothing about the file except its length.
///
/// The relaxation is deliberately confined to text. Attribute values are
/// compared exactly, because an attribute that looks like a number is often an
/// index or an id, where `007` and `7` are not interchangeable at all.
fn equivalent(a: &Canonical, b: &Canonical) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same_token(x, y))
}

fn same_token(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (a.strip_prefix('#'), b.strip_prefix('#')) {
        (Some(x), Some(y)) => match (x.trim().parse::<f64>(), y.trim().parse::<f64>()) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        },
        _ => false,
    }
}

/// Human-readable description of where two token streams first diverge.
fn first_divergence(a: &Canonical, b: &Canonical) -> String {
    // The same rule `equivalent` used, so the reported position is a real
    // disagreement and not one this function invented.
    let at = a.iter().zip(b.iter()).position(|(x, y)| !same_token(x, y));
    match at {
        Some(i) => format!(
            "diverges at token {i}: {:?} vs {:?}",
            a.get(i).map(String::as_str).unwrap_or("<end>"),
            b.get(i).map(String::as_str).unwrap_or("<end>")
        ),
        None => format!(
            "identical for {} tokens, then lengths differ ({} vs {})",
            a.len().min(b.len()),
            a.len(),
            b.len()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canon(s: &str) -> Canonical {
        canonicalize(s.as_bytes()).expect("test input must parse")
    }

    #[test]
    fn attribute_order_is_insignificant() {
        assert_eq!(
            canon(r#"<w:p a="1" b="2"/>"#),
            canon(r#"<w:p b="2" a="1"/>"#)
        );
    }

    #[test]
    fn indentation_between_elements_is_insignificant() {
        assert_eq!(
            canon("<root><a/><b/></root>"),
            canon("<root>\n  <a/>\n  <b/>\n</root>")
        );
    }

    #[test]
    fn whitespace_inside_a_leaf_element_is_significant() {
        // The bug this guards against: "normalizing" a document until a real
        // space in `<w:t xml:space="preserve"> </w:t>` disappears.
        assert_ne!(canon("<w:t> </w:t>"), canon("<w:t></w:t>"));
        assert_ne!(canon("<w:t>a b</w:t>"), canon("<w:t>ab</w:t>"));
    }

    #[test]
    fn element_order_is_significant() {
        assert_ne!(
            canon("<root><a/><b/></root>"),
            canon("<root><b/><a/></root>")
        );
    }

    #[test]
    fn namespace_prefix_renaming_is_insignificant() {
        assert_eq!(
            canon(r#"<w:p xmlns:w="urn:x"><w:r/></w:p>"#),
            canon(r#"<z:p xmlns:z="urn:x"><z:r/></z:p>"#)
        );
    }

    #[test]
    fn xml_declaration_and_comments_are_ignored() {
        assert_eq!(
            canon(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><a/>"#),
            canon("<!-- a comment --><a/>")
        );
    }

    #[test]
    fn attribute_values_are_significant() {
        assert_ne!(canon(r#"<a v="1"/>"#), canon(r#"<a v="2"/>"#));
    }

    #[test]
    fn entity_escaping_style_is_insignificant() {
        assert_eq!(canon(r#"<a v="&amp;"/>"#), canon(r#"<a v="&#38;"/>"#));
    }

    #[test]
    fn detects_a_binary_part_by_its_leading_bytes() {
        assert!(looks_like_xml(b"<?xml version=\"1.0\"?><a/>"));
        assert!(
            looks_like_xml(b"\xEF\xBB\xBF<a/>"),
            "BOM must not fool the sniff"
        );
        assert!(looks_like_xml(b"  \n<a/>"));
        assert!(!looks_like_xml(b"\x89PNG\r\n\x1a\n"));
        assert!(!looks_like_xml(&[]));
    }

    #[test]
    fn divergence_report_points_at_the_first_difference() {
        let a = canon("<root><a/><b/></root>");
        let b = canon("<root><a/><c/></root>");
        let msg = first_divergence(&a, &b);
        assert!(msg.contains("diverges at token"), "got: {msg}");
        assert!(msg.contains('b') && msg.contains('c'), "got: {msg}");
    }
}
