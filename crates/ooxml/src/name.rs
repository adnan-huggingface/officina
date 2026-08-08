//! OPC part names.
//!
//! A part name is a `/`-rooted, `/`-separated path of non-empty segments, as
//! defined by ECMA-376 part 2 §9.1.1.1. Zip entry names are the same thing with
//! the leading `/` stripped, so this type is the boundary between the two.
//!
//! Comparison is ASCII-case-insensitive, because OPC says part names are equal if
//! they differ only by case — while the *original* casing is preserved for
//! writing, since some consumers (including older Word builds) are less relaxed
//! about this than the spec is.

use std::fmt;

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct PartName {
    /// Original casing, always leading-slash normalized. e.g. `/word/document.xml`
    raw: String,
    /// ASCII-lowercased form used for comparison, hashing, and lookup.
    key: String,
}

impl PartName {
    /// Parses a part name from either an OPC name (`/word/document.xml`) or a zip
    /// entry name (`word/document.xml`).
    pub fn new(s: &str) -> Result<Self> {
        let bad = |reason| Error::BadPartName {
            raw: s.to_string(),
            reason,
        };

        if s.is_empty() {
            return Err(bad("part name is empty"));
        }
        // Zip entries carry no leading slash; OPC names do. Accept both.
        let trimmed = s.strip_prefix('/').unwrap_or(s);
        if trimmed.is_empty() {
            return Err(bad("part name is just a separator"));
        }
        if trimmed.ends_with('/') {
            return Err(bad("part name must not end with a separator"));
        }
        if trimmed.contains('\\') {
            return Err(bad("part name must use `/`, not `\\`"));
        }
        for seg in trimmed.split('/') {
            if seg.is_empty() {
                return Err(bad("part name has an empty segment"));
            }
            if seg == "." || seg == ".." {
                // Rejected rather than resolved: a package that escapes its own
                // root is either malformed or hostile, and normalizing it away
                // would turn a zip-slip attempt into a silent write outside the
                // package.
                return Err(bad("part name must not contain `.` or `..` segments"));
            }
            if seg.ends_with('.') {
                return Err(bad("part name segment must not end with a dot"));
            }
        }

        let raw = format!("/{trimmed}");
        let key = raw.to_ascii_lowercase();
        Ok(PartName { raw, key })
    }

    /// The OPC form, with leading slash: `/word/document.xml`
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// The zip entry form, without leading slash: `word/document.xml`
    pub fn zip_entry(&self) -> &str {
        &self.raw[1..]
    }

    /// Lowercase extension without the dot, if any.
    pub fn extension(&self) -> Option<String> {
        let last = self.raw.rsplit('/').next()?;
        let dot = last.rfind('.')?;
        let ext = &last[dot + 1..];
        (!ext.is_empty()).then(|| ext.to_ascii_lowercase())
    }

    /// The directory portion, with leading slash and no trailing slash.
    /// `/word/document.xml` -> `/word`; `/doc.xml` -> `/`
    pub fn parent(&self) -> &str {
        match self.raw.rfind('/') {
            Some(0) | None => "/",
            Some(i) => &self.raw[..i],
        }
    }

    /// The `_rels/<file>.rels` part that holds this part's relationships.
    pub fn rels_part(&self) -> PartName {
        let file = self.raw.rsplit('/').next().unwrap_or_default();
        let dir = self.parent();
        let raw = if dir == "/" {
            format!("/_rels/{file}.rels")
        } else {
            format!("{dir}/_rels/{file}.rels")
        };
        let key = raw.to_ascii_lowercase();
        PartName { raw, key }
    }
}

impl PartialEq for PartName {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl Eq for PartName {}

impl PartialOrd for PartName {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PartName {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

impl std::hash::Hash for PartName {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

impl fmt::Display for PartName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_both_zip_and_opc_forms() {
        let a = PartName::new("/word/document.xml").unwrap();
        let b = PartName::new("word/document.xml").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.as_str(), "/word/document.xml");
        assert_eq!(a.zip_entry(), "word/document.xml");
    }

    #[test]
    fn comparison_is_case_insensitive_but_casing_is_preserved() {
        let a = PartName::new("/word/Document.xml").unwrap();
        let b = PartName::new("/WORD/document.XML").unwrap();
        assert_eq!(a, b);
        assert_eq!(
            a.as_str(),
            "/word/Document.xml",
            "original casing must survive"
        );
    }

    #[test]
    fn rejects_traversal_rather_than_normalizing_it() {
        // Silently resolving `..` would convert a zip-slip attempt into a write
        // outside the package root.
        assert!(PartName::new("/word/../../etc/passwd").is_err());
        assert!(PartName::new("/./document.xml").is_err());
        assert!(PartName::new("word\\document.xml").is_err());
    }

    #[test]
    fn rejects_malformed_names() {
        assert!(PartName::new("").is_err());
        assert!(PartName::new("/").is_err());
        assert!(PartName::new("/word//document.xml").is_err());
        assert!(PartName::new("/word/document.xml/").is_err());
        assert!(PartName::new("/word/document.").is_err());
    }

    #[test]
    fn derives_extension_and_parent() {
        let p = PartName::new("/word/document.XML").unwrap();
        assert_eq!(p.extension().as_deref(), Some("xml"));
        assert_eq!(p.parent(), "/word");

        let root = PartName::new("/doc.xml").unwrap();
        assert_eq!(root.parent(), "/");
        assert_eq!(
            PartName::new("/word/media/image1").unwrap().extension(),
            None
        );
    }

    #[test]
    fn derives_rels_part_location() {
        let doc = PartName::new("/word/document.xml").unwrap();
        assert_eq!(doc.rels_part().as_str(), "/word/_rels/document.xml.rels");

        // The package-level relationships part is the degenerate case: the
        // "part" is the root itself, so its rels live at /_rels/.rels
        let root = PartName::new("/x.xml").unwrap();
        assert_eq!(root.rels_part().as_str(), "/_rels/x.xml.rels");
    }
}
