//! Authoring a package for a document that has never been in a file.
//!
//! "Never write a part we did not author or retain" cuts both ways: there is no
//! original here to preserve, so these parts are ours to write — and being ours,
//! they are written as small as Word will accept rather than as a copy of
//! whatever a template happened to hold.
//!
//! Only the skeleton is built. `document.xml` goes out with an empty body and a
//! section, and the paragraphs are then put in by the same splice writer that
//! edits a real file. One code path writes paragraphs, whether the document came
//! from Word or from nothing — which is the only way the new-document path can
//! be trusted, because it is the path everything else already uses.
//!
//! This is also what a `.doc` is saved through: a legacy document is read, and
//! the words it gave up are written into a package authored here.

use ooxml::{Package, PartName, Relationship, Relationships, TargetMode};
use wp_model::Document;

use crate::error::Result;

const REL_BASE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const WML: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

const DOCUMENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
const STYLES_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml";
const SETTINGS_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml";

const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";

/// Builds a package holding the document's shape but none of its paragraphs.
pub fn package_for(document: &Document) -> Result<Package> {
    let mut package = Package::empty();

    let mut root = Relationships::new();
    root.insert(Relationship {
        id: "rId1".to_string(),
        rel_type: format!("{REL_BASE}/officeDocument"),
        target: "word/document.xml".to_string(),
        mode: TargetMode::Internal,
    });
    put(&mut package, "/_rels/.rels", "", root.to_xml())?;

    let mut rels = Relationships::new();
    rels.insert(Relationship {
        id: "rId1".to_string(),
        rel_type: format!("{REL_BASE}/styles"),
        target: "styles.xml".to_string(),
        mode: TargetMode::Internal,
    });
    rels.insert(Relationship {
        id: "rId2".to_string(),
        rel_type: format!("{REL_BASE}/settings"),
        target: "settings.xml".to_string(),
        mode: TargetMode::Internal,
    });

    put(
        &mut package,
        "/word/document.xml",
        DOCUMENT_TYPE,
        document_part(document),
    )?;
    put(
        &mut package,
        "/word/_rels/document.xml.rels",
        "",
        rels.to_xml(),
    )?;
    put(&mut package, "/word/styles.xml", STYLES_TYPE, styles())?;
    put(
        &mut package,
        "/word/settings.xml",
        SETTINGS_TYPE,
        settings(),
    )?;
    Ok(package)
}

fn put(package: &mut Package, name: &str, content_type: &str, data: Vec<u8>) -> Result<()> {
    let name = PartName::new(name).map_err(crate::Error::Package)?;
    let content_type = match content_type.is_empty() {
        // Relationship parts are covered by the `rels` extension default;
        // declaring them again would only add noise to [Content_Types].xml.
        true => package
            .content_types()
            .get(&name)
            .unwrap_or("application/xml")
            .to_string(),
        false => content_type.to_string(),
    };
    package.put_part(name, &content_type, data);
    Ok(())
}

/// An empty body, and the section the document says it has.
///
/// Empty on purpose: `write::document_out` then appends every paragraph the
/// model holds, exactly as it would for a document read from a file.
fn document_part(document: &Document) -> Vec<u8> {
    let mut out = String::from(DECL);
    out.push_str(&format!("<w:document xmlns:w=\"{WML}\"><w:body>"));
    crate::write::section(&mut out, &document.section);
    out.push_str("</w:body></w:document>");
    out.into_bytes()
}

/// The smallest stylesheet Word will open: the document defaults, and nothing.
///
/// A document with no `styles.xml` at all opens, but Word rebuilds one on the
/// first save and the result is a file this project did not write.
fn styles() -> Vec<u8> {
    let mut out = String::from(DECL);
    out.push_str(&format!("<w:styles xmlns:w=\"{WML}\">"));
    out.push_str(
        "<w:docDefaults><w:rPrDefault><w:rPr>\
         <w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\"/>\
         <w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/>\
         </w:rPr></w:rPrDefault>\
         <w:pPrDefault><w:pPr>\
         <w:spacing w:after=\"160\" w:line=\"259\" w:lineRule=\"auto\"/>\
         </w:pPr></w:pPrDefault></w:docDefaults>",
    );
    out.push_str("</w:styles>");
    out.into_bytes()
}

fn settings() -> Vec<u8> {
    let mut out = String::from(DECL);
    out.push_str(&format!(
        "<w:settings xmlns:w=\"{WML}\"><w:defaultTabStop w:val=\"720\"/></w:settings>"
    ));
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::doc::{Block, Inline, Paragraph, Run};

    #[test]
    fn a_document_authored_from_nothing_reads_back_as_itself() {
        // The whole claim of this module: the parts it writes are a document
        // this project's own reader accepts, and the paragraphs come out again.
        let mut document = Document::new();
        let mut paragraph = Paragraph::new();
        paragraph.content = vec![Inline::Run(Run::of("Written from nothing."))];
        document.body = vec![Block::Paragraph(paragraph)];

        let mut package = package_for(&document).expect("a package");
        crate::write::flush(&document, &mut package).expect("it writes");
        let path = std::env::temp_dir().join("wp-docx-authored.docx");
        package.save(&path).expect("saved");
        let (read, _) = crate::open(&path).expect("it opens");
        let _ = std::fs::remove_file(&path);

        let text: Vec<String> = read
            .paragraphs()
            .iter()
            .map(|paragraph| paragraph.text())
            .collect();
        assert_eq!(text, vec!["Written from nothing.".to_string()]);
    }

    #[test]
    fn the_section_the_document_states_is_the_one_that_is_written() {
        // A new document that came from a `.doc` has the legacy file's page
        // setup, and losing it on the first save would resize every page.
        let mut document = Document::new();
        document.section.page.width = wp_model::Twips(11906);
        document.section.page.height = wp_model::Twips(16838);
        let package = package_for(&document).expect("a package");
        let part = ooxml::PartName::new("/word/document.xml").expect("a name");
        let bytes = package.part(&part).expect("the part").data();
        let text = String::from_utf8_lossy(bytes);
        assert!(text.contains("w:w=\"11906\""), "{text}");
    }
}
