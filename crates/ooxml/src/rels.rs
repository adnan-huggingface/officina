//! Relationship parts (`_rels/*.rels`) — the package's link graph.
//!
//! Relationships are *derived* on save. They are also the one place where a
//! preservation bug is invisible until it is catastrophic: drop a relationship and
//! the part it pointed at becomes unreachable, so Word silently ignores an image
//! or a header that is still sitting in the zip. Every relationship read is
//! therefore written back, including types we do not model.

use std::collections::BTreeMap;

use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

use crate::error::{Error, Result};
use crate::name::PartName;
use crate::xml::{attr, local_name};

const NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// Whether a relationship target lives inside the package or outside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetMode {
    /// A part in this package. `target` is a package-relative path.
    Internal,
    /// A URI outside the package (hyperlink, linked image, external workbook).
    /// Never resolved to a part, and never fetched.
    External,
}

#[derive(Debug, Clone)]
pub struct Relationship {
    /// `rId7`. Unique within the owning part; referenced from document XML.
    pub id: String,
    /// The relationship type URI.
    pub rel_type: String,
    /// Raw target string, exactly as written in the file.
    pub target: String,
    pub mode: TargetMode,
}

impl Relationship {
    /// Resolves an internal target to an absolute part name, relative to the
    /// directory holding the *owning* part.
    ///
    /// Returns `None` for external targets, which have no part.
    pub fn resolve(&self, owner: &PartName) -> Option<Result<PartName>> {
        if self.mode == TargetMode::External {
            return None;
        }
        let raw = if self.target.starts_with('/') {
            self.target.clone()
        } else {
            let dir = owner.parent();
            if dir == "/" {
                format!("/{}", self.target)
            } else {
                format!("{dir}/{}", self.target)
            }
        };
        Some(PartName::new(&normalize_dots(&raw)))
    }
}

/// Collapses `.` and `..` segments in a package-relative path.
///
/// Unlike `PartName`, which rejects them outright, relationship targets legitimately
/// use `../` — a header part at `/word/header1.xml` referring to `../customXml/item1.xml`
/// is normal and produced by Word itself. Traversal above the root is clamped, so a
/// hostile `../../../..` cannot escape the package.
fn normalize_dots(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.trim_start_matches('/').split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    format!("/{}", out.join("/"))
}

#[derive(Debug, Clone, Default)]
pub struct Relationships {
    /// Keyed by id, ordered so output is stable across saves.
    items: BTreeMap<String, Relationship>,
}

impl Relationships {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn get(&self, id: &str) -> Option<&Relationship> {
        self.items.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Relationship> {
        self.items.values()
    }

    /// All relationships of a given type, in id order.
    pub fn by_type<'a>(&'a self, rel_type: &'a str) -> impl Iterator<Item = &'a Relationship> {
        self.items.values().filter(move |r| r.rel_type == rel_type)
    }

    pub fn insert(&mut self, rel: Relationship) {
        self.items.insert(rel.id.clone(), rel);
    }

    /// Allocates an unused `rIdN`.
    pub fn next_id(&self) -> String {
        let mut n = self.items.len() + 1;
        loop {
            let candidate = format!("rId{n}");
            if !self.items.contains_key(&candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    pub fn parse(part: &PartName, xml: &[u8]) -> Result<Self> {
        let fail = |e: quick_xml::Error| Error::Xml {
            part: part.clone(),
            source: e.to_string(),
        };

        let mut reader = Reader::from_reader(xml);
        reader.config_mut().trim_text(true);

        let mut out = Relationships::new();
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf).map_err(fail)? {
                Event::Start(e) | Event::Empty(e) => {
                    if local_name(&e) == b"Relationship" {
                        let id = attr(&e, b"Id");
                        let rel_type = attr(&e, b"Type");
                        let target = attr(&e, b"Target");
                        let mode = match attr(&e, b"TargetMode") {
                            Some(m) if m.eq_ignore_ascii_case("External") => TargetMode::External,
                            _ => TargetMode::Internal,
                        };
                        if let (Some(id), Some(rel_type), Some(target)) = (id, rel_type, target) {
                            out.insert(Relationship {
                                id,
                                rel_type,
                                target,
                                mode,
                            });
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        Ok(out)
    }

    pub fn to_xml(&self) -> Vec<u8> {
        let mut w = Writer::new(Vec::new());
        let _ = w.write_event(Event::Decl(BytesDecl::new(
            "1.0",
            Some("UTF-8"),
            Some("yes"),
        )));

        let mut root = BytesStart::new("Relationships");
        root.push_attribute(("xmlns", NS));
        let _ = w.write_event(Event::Start(root));

        for rel in self.items.values() {
            let mut e = BytesStart::new("Relationship");
            e.push_attribute(("Id", rel.id.as_str()));
            e.push_attribute(("Type", rel.rel_type.as_str()));
            e.push_attribute(("Target", rel.target.as_str()));
            if rel.mode == TargetMode::External {
                e.push_attribute(("TargetMode", "External"));
            }
            let _ = w.write_event(Event::Empty(e));
        }

        let _ = w.write_event(Event::End(BytesEnd::new("Relationships")));
        w.into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/" TargetMode="External"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../customXml/item1.xml"/>
</Relationships>"#;

    fn owner() -> PartName {
        PartName::new("/word/document.xml").unwrap()
    }

    #[test]
    fn resolves_relative_targets_against_the_owning_part() {
        let rels = Relationships::parse(&owner(), SAMPLE).unwrap();
        let styles = rels
            .get("rId1")
            .unwrap()
            .resolve(&owner())
            .unwrap()
            .unwrap();
        assert_eq!(styles.as_str(), "/word/styles.xml");
    }

    #[test]
    fn resolves_parent_traversal_that_word_itself_emits() {
        let rels = Relationships::parse(&owner(), SAMPLE).unwrap();
        let item = rels
            .get("rId3")
            .unwrap()
            .resolve(&owner())
            .unwrap()
            .unwrap();
        assert_eq!(item.as_str(), "/customXml/item1.xml");
    }

    #[test]
    fn external_targets_resolve_to_no_part() {
        let rels = Relationships::parse(&owner(), SAMPLE).unwrap();
        let link = rels.get("rId2").unwrap();
        assert_eq!(link.mode, TargetMode::External);
        assert!(
            link.resolve(&owner()).is_none(),
            "a hyperlink is not a part"
        );
    }

    #[test]
    fn traversal_above_the_root_is_clamped() {
        assert_eq!(normalize_dots("/word/../../../etc/passwd"), "/etc/passwd");
        assert_eq!(normalize_dots("/a/./b/../c"), "/a/c");
    }

    #[test]
    fn round_trips_including_the_external_mode_flag() {
        let a = Relationships::parse(&owner(), SAMPLE).unwrap();
        let b = Relationships::parse(&owner(), &a.to_xml()).unwrap();
        assert_eq!(a.len(), b.len());
        for rel in a.iter() {
            let other = b.get(&rel.id).expect("every relationship must survive");
            assert_eq!(rel.rel_type, other.rel_type);
            assert_eq!(rel.target, other.target);
            assert_eq!(rel.mode, other.mode);
        }
    }

    #[test]
    fn next_id_avoids_collisions() {
        let rels = Relationships::parse(&owner(), SAMPLE).unwrap();
        let id = rels.next_id();
        assert!(rels.get(&id).is_none());
    }
}
