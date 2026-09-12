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
    put(
        &mut package,
        "/word/styles.xml",
        STYLES_TYPE,
        styles(document),
    )?;
    put(
        &mut package,
        "/word/settings.xml",
        SETTINGS_TYPE,
        settings(document),
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

/// The document defaults, and the styles the model brought with it.
///
/// A document authored here holds only the styles its app seeded — or the ones
/// a legacy `.doc` or an `.odt` gave up — and this is their one road into the
/// file: without it, the first save would forget every heading was a heading.
///
/// **Everything the model holds is written, and nothing it does not.** The
/// defaults were once Word 2013's Normal template, written whatever the
/// document said — eight points after every paragraph and a line of 1.08 —
/// and a style was written as its chain and a face, a weight and a size.
/// A new document drawn single-spaced came back from its own first save a
/// third taller, and a Word 97 specification of sixteen pages came back as
/// twenty-six, its headings unnumbered and its contents without their leaders:
/// the numbering, the tab stops and the spacing were in the model and never
/// reached the file. The defaults are now the document's own — empty, when it
/// states none, which Word reads as no space after and single spacing, the
/// same thing the layout reads it as — and a style's paragraph and run
/// properties go out through the same emitter a paragraph's do, which is the
/// reader's own vocabulary.
fn styles(document: &Document) -> Vec<u8> {
    use super::emit;
    use super::splice::escape_attr;
    use std::fmt::Write as _;

    let mut out = String::from(DECL);
    out.push_str(&format!("<w:styles xmlns:w=\"{WML}\">"));
    let defaults = document.styles.doc_defaults();
    out.push_str("<w:docDefaults>");
    if !defaults.run.is_empty() {
        out.push_str("<w:rPrDefault>");
        emit::run_props(&mut out, &defaults.run, &document.styles);
        out.push_str("</w:rPrDefault>");
    }
    let mut para = String::new();
    emit::para_props_alone(&mut para, &defaults.para, &document.styles);
    if !para.is_empty() {
        let _ = write!(out, "<w:pPrDefault>{para}</w:pPrDefault>");
    }
    out.push_str("</w:docDefaults>");
    let id_of = |target: Option<wp_model::StyleId>| {
        target
            .and_then(|id| document.styles.get(id))
            .map(|style| style.id.to_string())
    };
    for (_, style) in document.styles.iter() {
        let kind = match style.kind {
            wp_model::StyleKind::Paragraph => "paragraph",
            wp_model::StyleKind::Character => "character",
            // A table or numbering style speaks a vocabulary this writer does
            // not; half of one would be worse than none.
            _ => continue,
        };
        let _ = write!(out, r#"<w:style w:type="{kind}""#);
        if style.default {
            out.push_str(r#" w:default="1""#);
        }
        if style.custom {
            out.push_str(r#" w:customStyle="1""#);
        }
        let _ = write!(out, r#" w:styleId="{}">"#, escape_attr(&style.id));
        if let Some(name) = &style.name {
            let _ = write!(out, r#"<w:name w:val="{}"/>"#, escape_attr(name));
        }
        if let Some(based) = id_of(style.based_on) {
            let _ = write!(out, r#"<w:basedOn w:val="{}"/>"#, escape_attr(&based));
        }
        if let Some(next) = id_of(style.next) {
            let _ = write!(out, r#"<w:next w:val="{}"/>"#, escape_attr(&next));
        }
        if let Some(link) = id_of(style.link) {
            let _ = write!(out, r#"<w:link w:val="{}"/>"#, escape_attr(&link));
        }
        if let Some(priority) = style.priority {
            let _ = write!(out, r#"<w:uiPriority w:val="{priority}"/>"#);
        }
        if style.semi_hidden {
            out.push_str("<w:semiHidden/>");
        }
        if style.unhide_when_used {
            out.push_str("<w:unhideWhenUsed/>");
        }
        if style.quick {
            out.push_str("<w:qFormat/>");
        }
        emit::para_props_alone(&mut out, &style.para, &document.styles);
        // A character style reference inside a style is not a thing a style
        // can say: the run half is the formatting and nothing else.
        let run = wp_model::prop::RunProps {
            style: None,
            ..style.run.clone()
        };
        emit::run_props(&mut out, &run, &document.styles);
        out.push_str("</w:style>");
    }
    out.push_str("</w:styles>");
    out.into_bytes()
}

/// The settings that change what is drawn, as the document states them.
///
/// A Word 97 document's default tab stop is its own, and so is its
/// `noLeading`, which takes a quarter of a point off every line — half an
/// inch down a page of fifty, and pages by the end of a long document. Written
/// in the schema's order, because Word validates it.
fn settings(document: &Document) -> Vec<u8> {
    use std::fmt::Write as _;
    let settings = &document.settings;
    let mut out = String::from(DECL);
    let _ = write!(out, "<w:settings xmlns:w=\"{WML}\">");
    if let Some(percent) = settings.zoom {
        let _ = write!(out, r#"<w:zoom w:percent="{percent}"/>"#);
    }
    if settings.mirror_margins {
        out.push_str("<w:mirrorMargins/>");
    }
    let _ = write!(
        out,
        r#"<w:defaultTabStop w:val="{}"/>"#,
        settings.default_tab_stop.0
    );
    if settings.hyphenate {
        out.push_str("<w:autoHyphenation/>");
    }
    if settings.hyphen_limit > 0 {
        let _ = write!(
            out,
            r#"<w:consecutiveHyphenLimit w:val="{}"/>"#,
            settings.hyphen_limit
        );
    }
    if settings.even_and_odd_headers {
        out.push_str("<w:evenAndOddHeaders/>");
    }
    if settings.no_leading || settings.no_tab_for_hanging_indent || settings.compatibility_mode > 0
    {
        out.push_str("<w:compat>");
        if settings.no_leading {
            out.push_str("<w:noLeading/>");
        }
        if settings.no_tab_for_hanging_indent {
            out.push_str("<w:doNotUseIndentAsNumberingTabStop/>");
        }
        if settings.compatibility_mode > 0 {
            let _ = write!(
                out,
                r#"<w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="{}"/>"#,
                settings.compatibility_mode
            );
        }
        out.push_str("</w:compat>");
    }
    out.push_str("</w:settings>");
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::doc::{Block, Inline, Paragraph, Run};

    /// A `.doc` saved as `.docx` came back as a different document: the
    /// defaults were Word 2013's rather than its own, and every style was
    /// written as its chain and a face — its numbering, its tab stops, its
    /// spacing and indents left behind in the model. Everything the model
    /// holds goes out now, and reads back as it went.
    #[test]
    fn the_defaults_the_styles_and_the_settings_read_back_as_they_went() {
        use wp_model::prop::NumRef;
        use wp_model::prop::{Indent, LineSpacing, Spacing, TabKind, TabLeader, TabStop};
        use wp_model::units::{HalfPoint, Line240, Twips};
        use wp_model::{Style, StyleKind};

        let mut document = Document::new();
        document.body = vec![Block::Paragraph(Paragraph::of("text"))];
        let mut normal = Style::new("Normal", StyleKind::Paragraph);
        normal.name = Some("Normal".into());
        normal.run.fonts.ascii = Some("Times New Roman".into());
        normal.run.fonts.high_ansi = Some("Times New Roman".into());
        let normal = document.styles.insert(normal);
        let mut heading = Style::new("Heading1", StyleKind::Paragraph);
        heading.name = Some("heading 1".into());
        heading.based_on = Some(normal);
        heading.para.numbering = Some(NumRef {
            num_id: 1,
            level: 0,
        });
        heading.para.keep_next = Some(true);
        heading.para.spacing = Spacing {
            before: Some(Twips(240)),
            after: Some(Twips(60)),
            ..Spacing::default()
        };
        heading.para.indent = Indent {
            start: Some(Twips(360)),
            hanging: Some(Twips(360)),
            ..Indent::default()
        };
        heading.run.size = Some(HalfPoint(28));
        heading.run.toggles.set(wp_model::prop::Toggle::Bold, true);
        heading
            .run
            .toggles
            .set(wp_model::prop::Toggle::Italic, true);
        document.styles.insert(heading);
        let mut toc = Style::new("TOC2", StyleKind::Paragraph);
        toc.name = Some("toc 2".into());
        toc.based_on = Some(normal);
        toc.para.tabs = Some(vec![TabStop {
            position: Twips(8630),
            kind: TabKind::End,
            leader: TabLeader::Dot,
        }]);
        toc.para.spacing.line = Some(LineSpacing::Multiple(Line240::SINGLE));
        document.styles.insert(toc);
        document.settings.default_tab_stop = Twips(1440);
        document.settings.no_leading = true;

        let mut package = package_for(&document).expect("authored");
        crate::write::flush(&mut document, &mut package).expect("the paragraphs go in");
        let dir = std::env::temp_dir().join("wp-docx-authored-styles");
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let path = dir.join("styles.docx");
        package.save(&path).expect("saved");
        let (read, _) = crate::open(&path).expect("it opens");

        assert_eq!(
            read.styles.doc_defaults(),
            document.styles.doc_defaults(),
            "no defaults stated, and none read back — not Word 2013's"
        );
        for id in ["Normal", "Heading1", "TOC2"] {
            let wrote = document
                .styles
                .lookup(id)
                .and_then(|s| document.styles.get(s));
            let came = read.styles.lookup(id).and_then(|s| read.styles.get(s));
            let (wrote, came) = (wrote.expect("written"), came.expect("read back"));
            assert_eq!(came.para, wrote.para, "{id}: its paragraph half");
            assert_eq!(came.run, wrote.run, "{id}: its run half");
            assert_eq!(came.name, wrote.name);
        }
        assert_eq!(read.settings.default_tab_stop, Twips(1440));
        assert!(read.settings.no_leading, "and the Word 97 line rule");
    }

    #[test]
    fn a_document_authored_from_nothing_reads_back_as_itself() {
        // The whole claim of this module: the parts it writes are a document
        // this project's own reader accepts, and the paragraphs come out again.
        let mut document = Document::new();
        let mut paragraph = Paragraph::new();
        paragraph.content = vec![Inline::Run(Run::of("Written from nothing."))];
        document.body = vec![Block::Paragraph(paragraph)];

        let mut package = package_for(&document).expect("a package");
        crate::write::flush(&mut document, &mut package).expect("it writes");
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
