//! Locating the parts of a workbook through the relationship graph.
//!
//! Part *paths* are never assumed. `/xl/workbook.xml` is only a convention —
//! Excel itself writes `/xl/workbook.xml`, but the Strict Open XML profile and
//! several third-party producers do not, and a workbook is perfectly valid with
//! its sheets at `/spreadsheet/s1.xml`. Everything here is reached by following
//! relationships from the package root, which is the only way defined by the spec.

use std::collections::BTreeMap;

use ooxml::{Package, PartName, Relationship};

use crate::error::{Error, Result};

/// The relationship *type* of a relationship, reduced to its last path segment.
///
/// The full URIs differ between the Transitional profile
/// (`http://schemas.openxmlformats.org/officeDocument/2006/relationships/...`)
/// and the Strict profile (`http://purl.oclc.org/ooxml/officeDocument/relationships/...`),
/// while the final segment is identical in both. Matching on the segment reads
/// both profiles without carrying two tables of near-identical constants.
fn rel_kind(rel: &Relationship) -> &str {
    match rel.rel_type.rfind('/') {
        Some(i) => &rel.rel_type[i + 1..],
        None => &rel.rel_type,
    }
}

/// Where each part of a workbook lives.
#[derive(Debug)]
pub(crate) struct WorkbookParts {
    pub workbook: PartName,
    pub shared_strings: Option<PartName>,
    pub styles: Option<PartName>,
    /// Relationship id -> part, for everything reachable from workbook.xml.
    ///
    /// Keyed by rel id rather than by index because `<sheet r:id="rId3">` is the
    /// only thing tying a sheet's name to its part.
    pub by_rel_id: BTreeMap<String, (String, PartName)>,
}

impl WorkbookParts {
    /// The part and relationship kind for a `<sheet>`'s `r:id`.
    pub fn sheet_target(&self, rel_id: &str) -> Option<(&str, &PartName)> {
        self.by_rel_id
            .get(rel_id)
            .map(|(kind, name)| (kind.as_str(), name))
    }
}

/// Walks root rels -> officeDocument -> the workbook's own rels.
pub(crate) fn locate(package: &Package) -> Result<WorkbookParts> {
    let root = package.root_relationships()?;

    let doc_rel = root
        .iter()
        .find(|r| rel_kind(r) == "officeDocument")
        .ok_or(Error::NotAWorkbook(
            "package has no officeDocument relationship",
        ))?;

    // Root relationships anchor at the package root, not at `/_rels`.
    let workbook = doc_rel.resolve_from_root().ok_or(Error::NotAWorkbook(
        "the officeDocument relationship is external",
    ))??;

    let part = package.part(&workbook).ok_or(Error::MissingPart {
        referenced_by: "/_rels/.rels".to_owned(),
        rel_id: doc_rel.id.clone(),
    })?;

    // The content type is what distinguishes a workbook from a Word document.
    // Checking the file extension would accept a .docx renamed to .xlsx, and the
    // resulting error would be reported as corruption rather than wrong file type.
    if !is_workbook_content_type(&part.content_type) {
        return Err(Error::NotAWorkbook(
            "the main document part is not a spreadsheet",
        ));
    }

    let mut found = WorkbookParts {
        workbook: workbook.clone(),
        shared_strings: None,
        styles: None,
        by_rel_id: BTreeMap::new(),
    };

    for rel in package.relationships(&workbook)?.iter() {
        let Some(resolved) = rel.resolve(&workbook) else {
            continue; // external target: a linked workbook, never fetched
        };
        let Ok(target) = resolved else {
            continue; // a target we cannot even name; the part stays retained
        };
        let kind = rel_kind(rel);
        match kind {
            "sharedStrings" => found.shared_strings = Some(target.clone()),
            "styles" => found.styles = Some(target.clone()),
            _ => {}
        }
        found
            .by_rel_id
            .insert(rel.id.clone(), (kind.to_owned(), target));
    }

    Ok(found)
}

/// Accepts every spreadsheet flavour: .xlsx, macro-enabled, and templates.
fn is_workbook_content_type(ct: &str) -> bool {
    const PREFIXES: [&str; 2] = [
        "application/vnd.openxmlformats-officedocument.spreadsheetml.",
        "application/vnd.ms-excel.",
    ];
    PREFIXES.iter().any(|p| ct.starts_with(p))
        && (ct.contains("sheet") || ct.contains("template") || ct.contains("addin"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ooxml::TargetMode;

    fn rel(rel_type: &str) -> Relationship {
        Relationship {
            id: "rId1".into(),
            rel_type: rel_type.into(),
            target: "workbook.xml".into(),
            mode: TargetMode::Internal,
        }
    }

    #[test]
    fn both_ooxml_profiles_reduce_to_the_same_kind() {
        // Strict and Transitional differ only in the URI stem.
        assert_eq!(
            rel_kind(&rel(
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument"
            )),
            "officeDocument"
        );
        assert_eq!(
            rel_kind(&rel(
                "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument"
            )),
            "officeDocument"
        );
        assert_eq!(rel_kind(&rel("worksheet")), "worksheet");
    }

    #[test]
    fn workbook_content_types_are_recognized_across_flavours() {
        for ct in [
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.template.main+xml",
            "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
            "application/vnd.ms-excel.template.macroEnabled.main+xml",
            "application/vnd.ms-excel.addin.macroEnabled.main+xml",
        ] {
            assert!(is_workbook_content_type(ct), "should accept {ct}");
        }
    }

    #[test]
    fn a_word_document_is_not_a_workbook() {
        assert!(!is_workbook_content_type(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
        ));
        assert!(!is_workbook_content_type(
            "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"
        ));
    }
}
