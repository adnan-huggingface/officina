//! The numbering part: new definitions appended, existing bytes untouched.
//!
//! The reader keeps a *typed* slice of each definition, not the whole
//! element — so regenerating an existing `numbering.xml` from the model
//! would lose whatever vocabulary this crate does not model: picture
//! bullets, `w:legacy`, a vendor extension inside a level. Appending is the
//! one edit that cannot lose anything. Definitions the file already has
//! pass through byte for byte, and only definitions born in the model — a
//! list made in the app — are written, each in the schema's child order,
//! because Word validates that order and calls a violation damage.
//!
//! A document that has no numbering part yet gets one authored whole, with
//! its content type and its relationship from the document part — the same
//! road a pasted picture's part takes.

use std::collections::HashSet;
use std::fmt::Write as _;

use ooxml::{Package, PartName, Relationship, TargetMode};
use quick_xml::events::Event;
use quick_xml::Reader;
use wp_model::numbering::{AbstractNum, Level, MultiLevel, Num, Suffix};
use wp_model::prop::Justify;
use wp_model::style::StyleTable;
use wp_model::Document;

use crate::error::{Error, Result};
use crate::parts::DocumentParts;
use crate::xml::{attr_u32, end_local_name, local_name};

use super::emit;
use super::splice::escape_attr;

const NUMBERING_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";
const REL_BASE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const RELS_TYPE: &str = "application/vnd.openxmlformats-package.relationships+xml";
const WML: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";

/// Brings the package's numbering part up to date with the model.
pub(crate) fn flush(
    document: &Document,
    package: &mut Package,
    located: &DocumentParts,
) -> Result<()> {
    if document.numbering.is_empty() {
        return Ok(());
    }
    match &located.numbering {
        Some(name) => {
            let part = package.part(name).ok_or_else(|| Error::MissingPart {
                referenced_by: "word/_rels/document.xml.rels".to_owned(),
                rel_id: "numbering".to_owned(),
            })?;
            let content_type = part.content_type.clone();
            let Some(rewritten) = appended(part.data(), document) else {
                return Ok(());
            };
            package.put_part(name.clone(), &content_type, rewritten);
        }
        None => {
            let mut out = String::from(DECL);
            let _ = write!(out, r#"<w:numbering xmlns:w="{WML}">"#);
            definitions(&mut out, document, &HashSet::new(), &HashSet::new());
            out.push_str("</w:numbering>");

            // Beside the document part, which is where Word keeps it and what
            // keeps the relationship's relative target one word long.
            let name = sibling(&located.document, "numbering.xml")?;
            package.put_part(name.clone(), NUMBERING_TYPE, out.into_bytes());
            let mut rels = package.relationships(&located.document)?;
            rels.insert(Relationship {
                id: rels.next_id(),
                rel_type: format!("{REL_BASE}/numbering"),
                target: "numbering.xml".to_owned(),
                mode: TargetMode::Internal,
            });
            package.put_part(located.document.rels_part(), RELS_TYPE, rels.to_xml());
        }
    }
    Ok(())
}

/// The part with the model's new definitions spliced in, or `None` when the
/// file already holds every definition the model does.
fn appended(xml: &[u8], document: &Document) -> Option<Vec<u8>> {
    let survey = survey(xml);
    let mut fresh = String::new();
    definitions(&mut fresh, document, &survey.abstracts, &survey.nums);
    if fresh.is_empty() {
        return None;
    }
    // Everything new goes in one block before the first `<w:num>` — new
    // abstracts first, new instances after them. The schema wants abstracts
    // before instances, and a file with no instances yet satisfies that
    // trivially at the root's end.
    let at = survey.first_num.or(survey.root_end)?;
    let mut out = Vec::with_capacity(xml.len() + fresh.len());
    out.extend_from_slice(&xml[..at]);
    out.extend_from_slice(fresh.as_bytes());
    out.extend_from_slice(&xml[at..]);
    Some(out)
}

/// Every model definition the file does not hold, abstracts before instances.
fn definitions(
    out: &mut String,
    document: &Document,
    skip_abstract: &HashSet<u32>,
    skip_num: &HashSet<u32>,
) {
    let styles = &document.styles;
    let mut abstracts: Vec<&AbstractNum> = document
        .numbering
        .abstracts()
        .filter(|d| !skip_abstract.contains(&d.id))
        .collect();
    abstracts.sort_by_key(|d| d.id);
    for definition in abstracts {
        abstract_out(out, definition, styles);
    }
    let mut nums: Vec<&Num> = document
        .numbering
        .nums()
        .filter(|n| !skip_num.contains(&n.id))
        .collect();
    nums.sort_by_key(|n| n.id);
    for instance in nums {
        num_out(out, instance, styles);
    }
}

/// What the existing part holds and where its seams are, in one pass.
struct Survey {
    abstracts: HashSet<u32>,
    nums: HashSet<u32>,
    /// Byte position of the first root-level `<w:num>`, where new content goes.
    first_num: Option<usize>,
    /// Byte position of `</w:numbering>`, the fallback seam.
    root_end: Option<usize>,
}

fn survey(xml: &[u8]) -> Survey {
    let mut found = Survey {
        abstracts: HashSet::new(),
        nums: HashSet::new(),
        first_num: None,
        root_end: None,
    };
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut depth = 0usize;
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                if depth == 1 {
                    note(&mut found, &e, start);
                }
                depth += 1;
            }
            Ok(Event::Empty(e)) => {
                if depth == 1 {
                    note(&mut found, &e, start);
                }
            }
            Ok(Event::End(e)) => {
                depth = depth.saturating_sub(1);
                if depth == 0 && end_local_name(&e) == b"numbering" {
                    found.root_end = Some(start);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    found
}

fn note(found: &mut Survey, e: &quick_xml::events::BytesStart<'_>, start: usize) {
    match local_name(e) {
        b"abstractNum" => {
            if let Some(id) = attr_u32(e, b"abstractNumId") {
                found.abstracts.insert(id);
            }
        }
        b"num" => {
            if let Some(id) = attr_u32(e, b"numId") {
                found.nums.insert(id);
            }
            found.first_num.get_or_insert(start);
        }
        _ => {}
    }
}

/// One `<w:abstractNum>`, in the schema's child order.
///
/// The contract is the one every emitter here keeps: reading this back must
/// produce an equal [`AbstractNum`], because a test does exactly that.
fn abstract_out(out: &mut String, definition: &AbstractNum, styles: &StyleTable) {
    let _ = write!(
        out,
        r#"<w:abstractNum w:abstractNumId="{}">"#,
        definition.id
    );
    if let Some(nsid) = &definition.nsid {
        let _ = write!(out, r#"<w:nsid w:val="{}"/>"#, escape_attr(nsid));
    }
    let kind = match definition.multi_level {
        MultiLevel::Single => "singleLevel",
        MultiLevel::Multi => "multilevel",
        MultiLevel::Hybrid => "hybridMultilevel",
    };
    let _ = write!(out, r#"<w:multiLevelType w:val="{kind}"/>"#);
    if let Some(template) = &definition.template {
        let _ = write!(out, r#"<w:tmpl w:val="{}"/>"#, escape_attr(template));
    }
    if let Some(name) = &definition.name {
        let _ = write!(out, r#"<w:name w:val="{}"/>"#, escape_attr(name));
    }
    if let Some(link) = &definition.style_link {
        let _ = write!(out, r#"<w:styleLink w:val="{}"/>"#, escape_attr(link));
    }
    if let Some(link) = &definition.num_style_link {
        let _ = write!(out, r#"<w:numStyleLink w:val="{}"/>"#, escape_attr(link));
    }
    for level in definition.levels.iter().flatten() {
        level_out(out, level, styles);
    }
    out.push_str("</w:abstractNum>");
}

fn level_out(out: &mut String, level: &Level, styles: &StyleTable) {
    let _ = write!(out, r#"<w:lvl w:ilvl="{}">"#, level.index);
    let _ = write!(out, r#"<w:start w:val="{}"/>"#, level.start);
    let _ = write!(out, r#"<w:numFmt w:val="{}"/>"#, level.format.name());
    if let Some(restart) = level.restart {
        let _ = write!(out, r#"<w:lvlRestart w:val="{restart}"/>"#);
    }
    if level.legal {
        out.push_str("<w:isLgl/>");
    }
    if level.suffix != Suffix::Tab {
        let suffix = match level.suffix {
            Suffix::Tab => unreachable!(),
            Suffix::Space => "space",
            Suffix::Nothing => "nothing",
        };
        let _ = write!(out, r#"<w:suff w:val="{suffix}"/>"#);
    }
    let _ = write!(out, r#"<w:lvlText w:val="{}"/>"#, escape_attr(&level.text));
    if let Some(id) = level.picture_bullet {
        let _ = write!(out, r#"<w:lvlPicBulletId w:val="{id}"/>"#);
    }
    let justify = match level.justify {
        Justify::Start => "left",
        Justify::Center => "center",
        Justify::End => "right",
        Justify::Both => "both",
        Justify::Distribute => "distribute",
    };
    let _ = write!(out, r#"<w:lvlJc w:val="{justify}"/>"#);
    // Only the indent: it is the whole of what a level this crate authors
    // states, and the reader reads the pPr back through the same properties
    // parser paragraphs use.
    let indent = &level.para.indent;
    if !indent.is_empty() {
        out.push_str("<w:pPr><w:ind");
        if let Some(start) = indent.start {
            let _ = write!(out, r#" w:left="{}""#, start.0);
        }
        if let Some(end) = indent.end {
            let _ = write!(out, r#" w:right="{}""#, end.0);
        }
        if let Some(hanging) = indent.hanging {
            let _ = write!(out, r#" w:hanging="{}""#, hanging.0);
        } else if let Some(first) = indent.first_line {
            let _ = write!(out, r#" w:firstLine="{}""#, first.0);
        }
        out.push_str("/></w:pPr>");
    }
    emit::run_props(out, &level.run, styles);
    out.push_str("</w:lvl>");
}

fn num_out(out: &mut String, instance: &Num, styles: &StyleTable) {
    let _ = write!(out, r#"<w:num w:numId="{}">"#, instance.id);
    let _ = write!(
        out,
        r#"<w:abstractNumId w:val="{}"/>"#,
        instance.abstract_id
    );
    for over in &instance.overrides {
        let _ = write!(out, r#"<w:lvlOverride w:ilvl="{}">"#, over.index);
        if let Some(start) = over.start {
            let _ = write!(out, r#"<w:startOverride w:val="{start}"/>"#);
        }
        if let Some(level) = &over.level {
            level_out(out, level, styles);
        }
        out.push_str("</w:lvlOverride>");
    }
    out.push_str("</w:num>");
}

/// A part name beside another part — `word/numbering.xml` for
/// `word/document.xml`.
fn sibling(beside: &PartName, file: &str) -> Result<PartName> {
    let raw = beside.as_str();
    let dir = &raw[..raw.rfind('/').map_or(0, |at| at + 1)];
    PartName::new(&format!("{dir}{file}")).map_err(Error::Package)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::package_with;
    use wp_model::numbering::NumFormat;
    use wp_model::units::Twips;

    const DOC: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p/></w:body></w:document>"#;

    /// The definition the app's bullet button makes, one level deep.
    fn listed_document() -> Document {
        let mut document = Document::new();
        let mut definition = AbstractNum::new(1);
        definition.multi_level = MultiLevel::Hybrid;
        let mut level = Level::new(0);
        level.format = NumFormat::Bullet;
        level.text = "\u{F0B7}".into();
        level.para.indent.start = Some(Twips(720));
        level.para.indent.hanging = Some(Twips(360));
        level.run.fonts.ascii = Some("Symbol".into());
        level.run.fonts.high_ansi = Some("Symbol".into());
        definition.set_level(level);
        document.numbering.insert_abstract(definition);
        document.numbering.insert_num(Num::new(1, 1));
        document
    }

    fn package_with_numbering(numbering: &[u8]) -> Package {
        let mut package = package_with(DOC);
        let name = PartName::new("/word/numbering.xml").expect("a name");
        package.put_part(name, NUMBERING_TYPE, numbering.to_vec());
        let rels = PartName::new("/word/_rels/document.xml.rels").expect("a name");
        package.put_part(
            rels,
            RELS_TYPE,
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/></Relationships>"#
                .to_vec(),
        );
        package
    }

    #[test]
    fn a_fresh_list_gains_the_part_the_relationship_and_a_way_back() {
        let mut package = package_with(DOC);
        let document = listed_document();
        let located = crate::parts::locate(&package).expect("it locates");
        flush(&document, &mut package, &located).expect("it flushes");

        let located = crate::parts::locate(&package).expect("it locates again");
        assert!(located.numbering.is_some(), "the relationship is there");
        let back = crate::read(&package).expect("and the package still reads");
        let level = back.numbering.level(1, 0).expect("the instance resolves");
        assert_eq!(level.format, NumFormat::Bullet);
        assert_eq!(&*level.text, "\u{F0B7}");
        assert_eq!(level.run.fonts.ascii.as_deref(), Some("Symbol"));
        assert_eq!(level.para.indent.start, Some(Twips(720)));
        assert_eq!(level.para.indent.hanging, Some(Twips(360)));
    }

    #[test]
    fn definitions_the_file_already_has_pass_through_byte_for_byte() {
        // `w:legacy` and the vendor attribute are vocabulary the model does
        // not keep. Appending must carry them anyway, or an edit three lists
        // away rewrites this one.
        let existing = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1" custom:mark="kept"><w:multiLevelType w:val="multilevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:legacy w:legacy="1"/><w:lvlJc w:val="left"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#;
        let mut package = package_with_numbering(existing);
        let mut document = crate::read(&package).expect("it reads");

        // What the app's button would add beside it.
        let mut definition = AbstractNum::new(2);
        definition.multi_level = MultiLevel::Hybrid;
        let mut level = Level::new(0);
        level.format = NumFormat::Bullet;
        level.text = "\u{F0B7}".into();
        definition.set_level(level);
        document.numbering.insert_abstract(definition);
        document.numbering.insert_num(Num::new(2, 2));

        let located = crate::parts::locate(&package).expect("it locates");
        flush(&document, &mut package, &located).expect("it flushes");

        let name = PartName::new("/word/numbering.xml").expect("a name");
        let bytes = package.part(&name).expect("the part").data().to_vec();
        let kept = br#"<w:abstractNum w:abstractNumId="1" custom:mark="kept"><w:multiLevelType w:val="multilevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:legacy w:legacy="1"/><w:lvlJc w:val="left"/></w:lvl></w:abstractNum>"#;
        assert!(
            bytes.windows(kept.len()).any(|w| w == *kept),
            "the existing definition is still its exact bytes"
        );
        let back = crate::read(&package).expect("and it reads");
        assert!(
            back.numbering.level(1, 0).is_some(),
            "the old list resolves"
        );
        let bullet = back.numbering.level(2, 0).expect("and so does the new");
        assert_eq!(bullet.format, NumFormat::Bullet);
    }

    #[test]
    fn a_model_matching_the_file_leaves_the_part_alone() {
        let existing = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:multiLevelType w:val="multilevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#;
        let mut package = package_with_numbering(existing);
        let document = crate::read(&package).expect("it reads");
        let located = crate::parts::locate(&package).expect("it locates");
        flush(&document, &mut package, &located).expect("it flushes");
        let name = PartName::new("/word/numbering.xml").expect("a name");
        assert_eq!(
            package.part(&name).expect("the part").data(),
            &existing[..],
            "nothing to add means nothing rewritten"
        );
    }
}
