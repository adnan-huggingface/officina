//! Header and footer parts: rewritten when edited, authored when new.
//!
//! A header body the app edited is rewritten whole — its part is small and
//! the edit replaced its content, so "changed means emitted" is the same
//! bargain the document part's paragraphs strike. Whether it changed is
//! decided the splice writer's way: the part is *re-read* and the result
//! compared against the model, so an untouched header keeps its exact
//! bytes, decorations and all.
//!
//! A header born in the app has no part and no relationship — the model
//! says so with two `None`s — and this is where both are assigned: a free
//! `headerN.xml` beside the document part, a relationship from it, and the
//! section reference updated to name that relationship before the document
//! part is written.

use ooxml::{Package, PartName, Relationship, TargetMode};
use wp_model::doc::Block;
use wp_model::style::StyleTable;
use wp_model::Document;

use crate::ctx::{Ctx, HeaderIndex};
use crate::error::{Error, Result};
use crate::parts::DocumentParts;

use super::emit;

const HEADER_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml";
const FOOTER_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml";
const REL_BASE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const RELS_TYPE: &str = "application/vnd.openxmlformats-package.relationships+xml";
const WML: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";

pub(crate) fn flush(
    document: &mut Document,
    package: &mut Package,
    located: &DocumentParts,
) -> Result<()> {
    if document.headers.is_empty() {
        return Ok(());
    }
    let (headers, styles) = (&mut document.headers, &mut document.styles);
    let mut assigned: Vec<(wp_model::section::HeaderId, std::sync::Arc<str>)> = Vec::new();
    for header in headers.iter_mut() {
        match header.part.clone() {
            Some(part_name) => {
                let name = PartName::new(&part_name).map_err(Error::Package)?;
                let Some(part) = package.part(&name) else {
                    continue;
                };
                // Changed means "would read back differently" — the same
                // definition the document part's paragraphs use.
                let existing = {
                    let mut index = HeaderIndex::default();
                    // Read under the part's own name, exactly as the reader
                    // did: a picture's relationship key is part of what a
                    // header *is*, so comparing a scoped model against an
                    // unscoped re-read would call every header changed and
                    // rewrite it on a save that touched nothing.
                    let scope = crate::parts::scope_of(&name);
                    let mut ctx = Ctx::of_named_part(styles, &mut index, part.data(), &scope);
                    crate::read_header(part.data(), &mut ctx, header.footer)
                };
                if existing == header.content {
                    continue;
                }
                // A block the emitter cannot say is a block the rewrite
                // would drop; those bytes stand rather than lose it.
                if !emitable(&header.content) {
                    continue;
                }
                let content_type = part.content_type.clone();
                let data = part_out(header.footer, &header.content, styles);
                package.put_part(name, &content_type, data);
            }
            None => {
                if !emitable(&header.content) {
                    continue;
                }
                let (name, target) = free_name(package, &located.document, header.footer)?;
                let content_type = if header.footer {
                    FOOTER_TYPE
                } else {
                    HEADER_TYPE
                };
                let data = part_out(header.footer, &header.content, styles);
                package.put_part(name.clone(), content_type, data);

                let mut rels = package.relationships(&located.document)?;
                let rel_id = rels.next_id();
                let kind = if header.footer { "footer" } else { "header" };
                rels.insert(Relationship {
                    id: rel_id.clone(),
                    rel_type: format!("{REL_BASE}/{kind}"),
                    target,
                    mode: TargetMode::Internal,
                });
                package.put_part(located.document.rels_part(), RELS_TYPE, rels.to_xml());

                header.part = Some(name.as_str().into());
                header.rel = Some(rel_id.as_str().into());
                assigned.push((header.id, header.rel.clone().expect("just set")));
            }
        }
    }
    // The section references that named these bodies before they had a
    // relationship now get one, so the sectPr the document part writes can
    // say `r:id` — a reference without one is skipped there, and the header
    // would exist in the package while no page used it.
    //
    // **Every section, not only the last.** All but one of a document's
    // sections hang off the paragraph that ends them, and a header made by
    // unlinking one of those from the section before it is named from there
    // alone — patching only `Document::section` wrote the part and then left
    // nothing pointing at it.
    for (id, rel) in assigned {
        for index in 0.. {
            let Some(section) = document.section_mut(index) else {
                break;
            };
            for reference in section.headers.iter_mut().chain(section.footers.iter_mut()) {
                if reference.body == id && reference.rel.is_none() {
                    reference.rel = Some(rel.clone());
                }
            }
        }
    }
    Ok(())
}

/// Whether every block is one the emitter can write. A content control or an
/// `altChunk` inside a header is vocabulary the rewrite would lose.
fn emitable(content: &[Block]) -> bool {
    content
        .iter()
        .all(|block| matches!(block, Block::Paragraph(_) | Block::Table(_)))
}

fn part_out(footer: bool, content: &[Block], styles: &StyleTable) -> Vec<u8> {
    let root = if footer { "ftr" } else { "hdr" };
    let mut out = String::from(DECL);
    // `xmlns:r` up front because an edited header may keep a picture, whose
    // drawing names its part by `r:embed`.
    out.push_str(&format!(
        r#"<w:{root} xmlns:w="{WML}" xmlns:r="{REL_BASE}">"#
    ));
    for block in content {
        match block {
            Block::Paragraph(paragraph) => emit::paragraph(&mut out, paragraph, styles),
            Block::Table(table) => emit::table(&mut out, table, styles),
            _ => {}
        }
    }
    out.push_str(&format!("</w:{root}>"));
    out.into_bytes()
}

/// The first `headerN.xml` (or `footerN.xml`) the package does not hold,
/// beside the document part, and the relationship target that names it.
fn free_name(package: &Package, beside: &PartName, footer: bool) -> Result<(PartName, String)> {
    let raw = beside.as_str();
    let dir = &raw[..raw.rfind('/').map_or(0, |at| at + 1)];
    let stem = if footer { "footer" } else { "header" };
    for n in 1u32.. {
        let file = format!("{stem}{n}.xml");
        let name = PartName::new(&format!("{dir}{file}")).map_err(Error::Package)?;
        if package.part(&name).is_none() {
            return Ok((name, file));
        }
    }
    unreachable!("u32 part numbers do not run out");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::package_with;
    use wp_model::doc::{HeaderFooter, Paragraph};
    use wp_model::section::{HeaderId, HeaderKind, HeaderRef};

    const DOC: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p/><w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:cols w:space="720"/></w:sectPr></w:body></w:document>"#;

    #[test]
    fn a_header_born_in_the_app_gets_a_part_a_relationship_and_a_page() {
        let mut package = package_with(DOC);
        let mut document = crate::read(&package).expect("it reads");
        document.headers.push(HeaderFooter {
            id: HeaderId(1),
            part: None,
            rel: None,
            footer: false,
            content: vec![Block::Paragraph(Paragraph::of("RESUME / CV"))],
        });
        document.section.headers.push(HeaderRef {
            kind: HeaderKind::Default,
            body: HeaderId(1),
            rel: None,
        });

        super::super::flush(&mut document, &mut package).expect("it flushes");

        // The model now knows the identities the save assigned…
        let header = &document.headers[0];
        assert!(header.part.is_some(), "a part name");
        assert!(header.rel.is_some(), "and a relationship id");
        assert_eq!(
            document.section.headers[0].rel, header.rel,
            "which the section reference names"
        );
        // The `r:id` the sectPr writes must carry its namespace binding:
        // quick_xml reads an unbound prefix without complaint, Word refuses
        // the whole file — which is exactly how this slipped past the suite.
        let main = crate::parts::locate(&package).expect("locates").document;
        let xml = package
            .part(&main)
            .expect("the document part")
            .data()
            .to_vec();
        let xml = std::str::from_utf8(&xml).expect("utf-8");
        assert!(
            !xml.contains("r:id") || xml.contains("xmlns:r"),
            "an r:id with no xmlns:r is a file Word will not open"
        );
        // …and a fresh read sees the header on the page.
        let back = crate::read(&package).expect("it reads back");
        let read = back.headers.iter().find(|h| !h.footer).expect("a header");
        assert_eq!(wp_model::doc::text_of(&read.content), "RESUME / CV");
        assert!(
            back.section.header_for_page(1, false).is_some(),
            "and the first page uses it"
        );
    }

    #[test]
    fn a_header_a_mid_document_section_names_gets_its_relationship_too() {
        // What breaking "Link to Previous" makes: a header named from a
        // paragraph's own `<w:sectPr>` rather than from the body's last one.
        // Patching only `Document::section` wrote the part into the package
        // and left nothing pointing at it, so the page came back bare.
        let mut package = package_with(DOC);
        let mut document = crate::read(&package).expect("it reads");
        let mut own = wp_model::SectionProps::new();
        own.headers.push(HeaderRef {
            kind: HeaderKind::Default,
            body: HeaderId(1),
            rel: None,
        });
        let mut opening = Paragraph::of("first section");
        opening.section = Some(Box::new(own));
        document.body.insert(0, Block::Paragraph(opening));
        document.headers.push(HeaderFooter {
            id: HeaderId(1),
            part: None,
            rel: None,
            footer: false,
            content: vec![Block::Paragraph(Paragraph::of("SECTION ONE"))],
        });

        super::super::flush(&mut document, &mut package).expect("it flushes");

        let assigned = document.headers[0].rel.clone().expect("a relationship");
        let sections = document.section_props();
        assert_eq!(
            sections[0].headers[0].rel.as_deref(),
            Some(assigned.as_ref()),
            "the paragraph's own section names it"
        );

        let back = crate::read(&package).expect("it reads back");
        let shown = back.bands();
        let body = shown[0]
            .header(HeaderKind::Default)
            .expect("the first section shows it");
        assert_eq!(
            wp_model::doc::text_of(&back.header(body).expect("read back").content),
            "SECTION ONE"
        );
    }

    #[test]
    fn an_untouched_header_keeps_its_bytes_and_an_edited_one_does_not() {
        // The `w:rsidR` decoration is what the model does not keep; identical
        // bytes after an untouched save is the proof nothing was reprinted.
        let header_part = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p w:rsidR="00AA1122"><w:r><w:t>Draft</w:t></w:r></w:p></w:hdr>"#;
        let doc = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p/><w:sectPr><w:headerReference w:type="default" r:id="rId9"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:cols w:space="720"/></w:sectPr></w:body></w:document>"#;
        let mut package = package_with(doc);
        let name = PartName::new("/word/header1.xml").expect("a name");
        package.put_part(name.clone(), HEADER_TYPE, header_part.to_vec());
        let rels = PartName::new("/word/_rels/document.xml.rels").expect("a name");
        package.put_part(
            rels,
            RELS_TYPE,
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/></Relationships>"#
                .to_vec(),
        );

        let mut document = crate::read(&package).expect("it reads");
        super::super::flush(&mut document, &mut package).expect("an untouched save");
        assert_eq!(
            package.part(&name).expect("the part").data(),
            &header_part[..],
            "nothing changed, nothing reprinted"
        );

        let header = document.headers.iter_mut().find(|h| !h.footer).expect("it");
        header.content = vec![Block::Paragraph(Paragraph::of("Final"))];
        super::super::flush(&mut document, &mut package).expect("an edited save");
        let back = crate::read(&package).expect("it reads back");
        let read = back.headers.iter().find(|h| !h.footer).expect("a header");
        assert_eq!(wp_model::doc::text_of(&read.content), "Final");
    }
}
