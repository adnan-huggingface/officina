//! Authoring a package for a document that has never been in one.
//!
//! "Never write a part we did not author or retain" cuts both ways: there is no
//! original here to preserve, so these parts are ours to write — and being ours,
//! they are written as small as a consumer will accept rather than as a copy of
//! whatever some template happened to hold.
//!
//! Only the skeleton is built. `content.xml` goes out with an empty
//! `<office:text>`, and the paragraphs are then put in by the same splice writer
//! that edits a real file. One code path writes paragraphs, whether the document
//! came from a `.odt`, from a `.docx` or from nothing — which is the only way
//! the Save As path can be trusted, because it is the path everything else
//! already uses.
//!
//! The namespaces are declared on the root of each part, all of them, because a
//! splice cannot add one later: an automatic style minted at the end of a save
//! names `style:` and `fo:`, and a paragraph that gains a picture names `draw:`,
//! `svg:` and `xlink:`. A prefix that is used and not bound is not a document
//! any consumer will open.

use std::fmt::Write as _;

use wp_model::doc::Document;
use wp_model::section::Orientation;

use crate::container::{Container, TEXT_MIMETYPE};
use crate::Result;

const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>";

/// Every namespace either part may need, bound once on the root.
const NAMESPACES: &str = concat!(
    r#" xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0""#,
    r#" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0""#,
    r#" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0""#,
    r#" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0""#,
    r#" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0""#,
    r#" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0""#,
    r#" xmlns:xlink="http://www.w3.org/1999/xlink""#,
    r#" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#,
);

/// The media type both modelled parts are declared as in the manifest.
const XML: &str = "text/xml";

/// Builds a package holding the document's shape but none of its paragraphs.
pub fn container_for(document: &Document) -> Result<Container> {
    let mut container = Container::empty(TEXT_MIMETYPE);
    container.put_part("content.xml", XML, content())?;
    container.put_part("styles.xml", XML, styles(document))?;
    Ok(container)
}

/// An empty body.
///
/// Empty on purpose: [`super::flush`] then appends every block the model holds,
/// exactly as it would for a document read from a file.
fn content() -> Vec<u8> {
    format!(
        "{DECL}<office:document-content{NAMESPACES} office:version=\"{}\">\
         <office:automatic-styles/>\
         <office:body><office:text></office:text></office:body>\
         </office:document-content>",
        crate::ODF_VERSION
    )
    .into_bytes()
}

/// The named styles the model brought with it, the paper it is set on, and the
/// one master page that carries them.
///
/// A document authored here holds only the styles its application seeded — or
/// the ones another format gave up — and this is their one road into the file:
/// without it, the first save would forget every heading was a heading. Only
/// the slice of a style this project puts into a model is written, which is
/// also the slice the reader reads back.
fn styles(document: &Document) -> Vec<u8> {
    let mut out = String::from(DECL);
    let _ = write!(
        out,
        "<office:document-styles{NAMESPACES} office:version=\"{}\">",
        crate::ODF_VERSION
    );

    out.push_str("<office:styles>");
    let name_of = |id: Option<wp_model::StyleId>| {
        id.and_then(|id| document.styles.get(id))
            .map(|style| style.id.to_string())
    };
    for (_, style) in document.styles.iter() {
        let family = match style.kind {
            wp_model::StyleKind::Paragraph => "paragraph",
            wp_model::StyleKind::Character => "text",
            // A table or numbering style speaks a vocabulary this writer does
            // not; half of one would be worse than none.
            _ => continue,
        };
        let _ = write!(
            out,
            r#"<style:style style:name="{}" style:family="{family}""#,
            super::splice::escape_attr(&style.id)
        );
        if let Some(name) = &style.name {
            let _ = write!(
                out,
                r#" style:display-name="{}""#,
                super::splice::escape_attr(name)
            );
        }
        if let Some(parent) = name_of(style.based_on) {
            let _ = write!(
                out,
                r#" style:parent-style-name="{}""#,
                super::splice::escape_attr(&parent)
            );
        }
        if let Some(next) = name_of(style.next) {
            let _ = write!(
                out,
                r#" style:next-style-name="{}""#,
                super::splice::escape_attr(&next)
            );
        }
        out.push('>');
        super::auto::paragraph_properties(&mut out, &style.para);
        super::auto::text_properties(&mut out, &style.run);
        out.push_str("</style:style>");
    }
    out.push_str("</office:styles>");

    out.push_str("<office:automatic-styles>");
    page_layout(&mut out, &document.section);
    out.push_str("</office:automatic-styles>");
    out.push_str(
        r#"<office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles>"#,
    );
    out.push_str("</office:document-styles>");
    out.into_bytes()
}

/// `<style:page-layout>` — the paper.
///
/// The margins are converted back the way `page.rs` converts them forward: ODF
/// measures the top margin to the *header* and the model keeps the distance to
/// the header separately, so it is that distance and not the body's that goes
/// out. A page authored here carries no header, so the two are the same number
/// unless the document came from a format where they were not.
fn page_layout(out: &mut String, section: &wp_model::section::SectionProps) {
    let page = &section.page;
    let margins = &section.margins;
    let top = match margins.header.0 > 0 && margins.header.0 < margins.top.0 {
        true => margins.header,
        false => margins.top,
    };
    let bottom = match margins.footer.0 > 0 && margins.footer.0 < margins.bottom.0 {
        true => margins.footer,
        false => margins.bottom,
    };
    let _ = write!(
        out,
        r#"<style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="{}" fo:page-height="{}" style:print-orientation="{}" fo:margin-top="{}" fo:margin-bottom="{}" fo:margin-left="{}" fo:margin-right="{}""#,
        length(page.width),
        length(page.height),
        match page.orientation {
            Orientation::Landscape => "landscape",
            Orientation::Portrait => "portrait",
        },
        length(top),
        length(bottom),
        length(margins.start),
        length(margins.end),
    );
    if section.columns.count() > 1 {
        let _ = write!(
            out,
            r#"><style:columns fo:column-count="{}" fo:column-gap="{}"/></style:page-layout-properties>"#,
            section.columns.count(),
            length(section.columns.space)
        );
    } else {
        out.push_str("/>");
    }
    out.push_str("</style:page-layout>");
}

fn length(value: wp_model::Twips) -> String {
    let points = value.0 as f64 / 20.0;
    let text = format!("{points:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{}pt", if text.is_empty() { "0" } else { text })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::doc::{Block, Paragraph};

    /// The whole claim of this module: the parts it writes are a document this
    /// project's own reader accepts, and the paragraphs come out again.
    #[test]
    fn a_document_authored_from_nothing_reads_back_as_itself() {
        let mut document = Document::new();
        document.body = vec![
            Block::Paragraph(Paragraph::of("Written from nothing.")),
            Block::Paragraph(Paragraph::of("And a second line.")),
        ];

        let mut container = container_for(&document).expect("a package");
        super::super::flush(&mut document, &mut container).expect("it writes");
        let (read, _) = crate::read(&container).expect("it reads");
        assert_eq!(read.text(), "Written from nothing.\nAnd a second line.");
    }

    #[test]
    fn a_blank_package_is_a_file_that_opens_again() {
        let mut document = Document::new();
        document.body = vec![Block::Paragraph(Paragraph::of("On disk."))];
        let mut container = container_for(&document).expect("a package");
        let path = std::env::temp_dir().join("wp-odf-blank.odt");
        super::super::save(&mut document, &mut container, &path).expect("saved");
        let (read, _, _) = crate::open(&path).expect("it opens");
        let _ = std::fs::remove_file(&path);
        assert_eq!(read.text(), "On disk.");
    }

    #[test]
    fn the_page_the_document_states_is_the_one_that_is_written() {
        // A document that came from another format has that file's page setup,
        // and losing it on the first save would resize every page.
        let mut document = Document::new();
        document.section.page.width = wp_model::Twips(11906);
        document.section.page.height = wp_model::Twips(16838);
        let container = container_for(&document).expect("a package");
        let text = String::from_utf8(container.data("styles.xml").unwrap().to_vec()).unwrap();
        assert!(text.contains(r#"fo:page-width="595.3pt""#), "{text}");

        let (read, _) = crate::read(&container).expect("it reads back");
        assert_eq!(read.section.page.height, wp_model::Twips(16838));
    }

    /// A style seeded by the application is a style the file has to carry, or
    /// the first save forgets every heading was a heading.
    #[test]
    fn the_styles_the_model_holds_are_written_and_read_back() {
        let mut document = Document::new();
        let mut heading =
            wp_model::style::Style::new("Heading_20_1", wp_model::StyleKind::Paragraph);
        heading.name = Some("Heading 1".into());
        heading.run.size = Some(wp_model::HalfPoint(32));
        let id = document.styles.insert(heading);
        document.body = vec![Block::Paragraph(Paragraph {
            props: wp_model::prop::ParaProps {
                style: Some(id),
                outline_level: Some(0),
                ..Default::default()
            },
            content: vec![wp_model::doc::Inline::Run(wp_model::doc::Run::of("Title"))],
            ..Paragraph::default()
        })];

        let mut container = container_for(&document).expect("a package");
        super::super::flush(&mut document, &mut container).expect("it writes");
        let (read, _) = crate::read(&container).expect("it reads");
        let style = read
            .styles
            .iter()
            .map(|(_, style)| style)
            .find(|style| style.id.as_ref() == "Heading_20_1")
            .expect("the style came back");
        assert_eq!(style.name.as_deref(), Some("Heading 1"));
        assert_eq!(style.run.size, Some(wp_model::HalfPoint(32)));
    }
}
