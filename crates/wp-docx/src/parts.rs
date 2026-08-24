//! Locating a document's parts through the relationship graph.
//!
//! Never by path. `/word/document.xml` is a convention that Word happens to
//! follow and nothing requires: the Strict profile writes different names, and a
//! `.docx` produced by a report generator may put its document anywhere. The
//! only defined route is root relationships -> `officeDocument` -> that part's
//! own relationships, and following it is what makes a third-party file open
//! with no special case.

use std::collections::BTreeMap;

use ooxml::{Package, PartName, Relationship};

use crate::error::{Error, Result};

/// A relationship type reduced to its last path segment.
///
/// The full URIs differ between the Transitional and Strict profiles while the
/// final segment is identical in both, so matching on the segment reads either
/// without two tables of near-identical constants.
pub(crate) fn rel_kind(rel: &Relationship) -> &str {
    match rel.rel_type.rfind('/') {
        Some(i) => &rel.rel_type[i + 1..],
        None => &rel.rel_type,
    }
}

/// Where each part of a document lives.
#[derive(Debug, Clone)]
pub struct DocumentParts {
    pub document: PartName,
    pub styles: Option<PartName>,
    pub numbering: Option<PartName>,
    pub settings: Option<PartName>,
    pub theme: Option<PartName>,
    pub font_table: Option<PartName>,
    pub footnotes: Option<PartName>,
    pub endnotes: Option<PartName>,
    pub comments: Option<PartName>,
    /// `commentsExtended.xml` — where a comment's *resolved* flag lives, which
    /// is not in `comments.xml` at all.
    pub comments_extended: Option<PartName>,
    pub people: Option<PartName>,
    /// Headers and footers, in relationship order, each with its own id. A
    /// section names them by `r:id`, so the map is keyed that way.
    pub headers: BTreeMap<String, PartName>,
    pub footers: BTreeMap<String, PartName>,
    /// Everything else reachable from the document, by relationship id — images
    /// above all, which a drawing names with `r:embed`.
    pub by_rel_id: BTreeMap<String, (String, PartName)>,
    /// External targets, which are never fetched. Kept so a hyperlink can show
    /// where it points without the reader having to go and look.
    pub external: BTreeMap<String, String>,
}

impl DocumentParts {
    /// Where each part of `package` lives.
    pub fn locate_in(package: &Package) -> Result<DocumentParts> {
        locate(package)
    }

    pub fn target(&self, rel_id: &str) -> Option<&PartName> {
        self.by_rel_id.get(rel_id).map(|(_, name)| name)
    }

    pub fn external_target(&self, rel_id: &str) -> Option<&str> {
        self.external.get(rel_id).map(String::as_str)
    }
}

/// Walks root rels -> officeDocument -> the document's own rels.
pub fn locate(package: &Package) -> Result<DocumentParts> {
    let root = package.root_relationships()?;

    let doc_rel = root
        .iter()
        .find(|r| rel_kind(r) == "officeDocument")
        .ok_or(Error::NotADocument(
            "package has no officeDocument relationship",
        ))?;

    let document = doc_rel.resolve_from_root().ok_or(Error::NotADocument(
        "the officeDocument relationship is external",
    ))??;

    let part = package.part(&document).ok_or_else(|| Error::MissingPart {
        referenced_by: "/_rels/.rels".to_owned(),
        rel_id: doc_rel.id.clone(),
    })?;

    // The content type is what tells a document from a workbook. Checking the
    // file extension would accept an .xlsx renamed to .docx, and the failure
    // would then be reported as corruption rather than as the wrong file.
    if !is_document_content_type(&part.content_type) {
        return Err(Error::NotADocument(
            "the main document part is not a word processing document",
        ));
    }

    let mut found = DocumentParts {
        document: document.clone(),
        styles: None,
        numbering: None,
        settings: None,
        theme: None,
        font_table: None,
        footnotes: None,
        endnotes: None,
        comments: None,
        comments_extended: None,
        people: None,
        headers: BTreeMap::new(),
        footers: BTreeMap::new(),
        by_rel_id: BTreeMap::new(),
        external: BTreeMap::new(),
    };

    for rel in package.relationships(&document)?.iter() {
        let kind = rel_kind(rel).to_owned();
        let Some(resolved) = rel.resolve(&document) else {
            // External: a hyperlink, a linked image, a referenced document.
            // Never fetched — a document that reaches out to the network when it
            // is opened is a document that can be used to find out who opened it.
            found.external.insert(rel.id.clone(), rel.target.clone());
            continue;
        };
        let Ok(target) = resolved else {
            // A target we cannot even name. The part, if it exists, stays
            // retained and is written back untouched.
            continue;
        };
        match kind.as_str() {
            "styles" => found.styles = Some(target.clone()),
            "numbering" => found.numbering = Some(target.clone()),
            "settings" => found.settings = Some(target.clone()),
            "theme" => found.theme = Some(target.clone()),
            "fontTable" => found.font_table = Some(target.clone()),
            "footnotes" => found.footnotes = Some(target.clone()),
            "endnotes" => found.endnotes = Some(target.clone()),
            "comments" => found.comments = Some(target.clone()),
            "commentsExtended" => found.comments_extended = Some(target.clone()),
            "people" => found.people = Some(target.clone()),
            "header" => {
                found.headers.insert(rel.id.clone(), target.clone());
            }
            "footer" => {
                found.footers.insert(rel.id.clone(), target.clone());
            }
            _ => {}
        }
        found.by_rel_id.insert(rel.id.clone(), (kind, target));
    }

    // The numbering part has relationships of its own — a picture bullet's
    // image is one — and it numbers them from `rId1` exactly as the document
    // does. They cannot share a key, so the numbering part's go in under a
    // qualified one. See [`qualified`].
    if let Some(numbering) = found.numbering.clone() {
        if let Ok(rels) = package.relationships(&numbering) {
            for rel in rels.iter() {
                let Some(Ok(target)) = rel.resolve(&numbering) else {
                    continue;
                };
                let kind = rel_kind(rel).to_owned();
                found
                    .by_rel_id
                    .insert(qualified(NUMBERING, &rel.id), (kind, target));
            }
        }
    }

    Ok(found)
}

/// The part whose relationships a picture bullet's image belongs to.
pub const NUMBERING: &str = "numbering";

/// The key a relationship of some part other than the document goes under.
///
/// `rId1` of `numbering.xml` and `rId1` of `document.xml` are different
/// relationships of different parts. A picture bullet asking for a bare
/// `rId1` would be handed the document's first image instead of its own.
pub fn qualified(part: &str, rel_id: &str) -> String {
    format!("{part}:{rel_id}")
}

/// Accepts every word processing flavour: `.docx`, macro-enabled, and the two
/// template types.
fn is_document_content_type(ct: &str) -> bool {
    const PREFIXES: [&str; 2] = [
        "application/vnd.openxmlformats-officedocument.wordprocessingml.",
        "application/vnd.ms-word.",
    ];
    PREFIXES.iter().any(|p| ct.starts_with(p))
        && (ct.contains("document") || ct.contains("template"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relationship_type_is_matched_on_its_last_segment() {
        let transitional = Relationship {
            id: "rId1".into(),
            rel_type: "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles"
                .into(),
            target: "styles.xml".into(),
            mode: ooxml::TargetMode::Internal,
        };
        let strict = Relationship {
            rel_type: "http://purl.oclc.org/ooxml/officeDocument/relationships/styles".into(),
            ..transitional.clone()
        };
        assert_eq!(rel_kind(&transitional), "styles");
        assert_eq!(rel_kind(&strict), "styles");
    }

    #[test]
    fn a_workbook_renamed_to_docx_is_reported_as_the_wrong_file() {
        assert!(!is_document_content_type(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"
        ));
        assert!(is_document_content_type(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
        ));
        assert!(is_document_content_type(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml"
        ));
        assert!(is_document_content_type(
            "application/vnd.ms-word.document.macroEnabled.main+xml"
        ));
    }
}
