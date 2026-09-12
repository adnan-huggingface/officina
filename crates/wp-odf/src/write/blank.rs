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
    let has_header = document.headers.iter().any(|band| !band.footer);
    let has_footer = document.headers.iter().any(|band| band.footer);
    page_layout(&mut out, &document.section, has_header, has_footer);
    out.push_str("</office:automatic-styles>");

    // The bands are written empty and filled by the splice writer, exactly as
    // the body is: an element that is not there is one it cannot fill, and a
    // header typed in the application would be on the screen and absent from
    // the file. Header before footer, which is the order ODF 1.4 part 3 §16.9
    // states and not the order the model keeps them in.
    out.push_str(r#"<office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1">"#);
    if has_header {
        out.push_str("<style:header></style:header>");
    }
    if has_footer {
        out.push_str("<style:footer></style:footer>");
    }
    out.push_str("</style:master-page></office:master-styles>");
    out.push_str("</office:document-styles>");
    out.into_bytes()
}

/// `<style:page-layout>` — the paper.
///
/// The margins are converted back the way `page.rs` converts them forward: ODF
/// measures the top margin to the *header* and the model keeps the distance to
/// the header separately, so where there is a header it is that distance and
/// not the body's that goes out.
///
/// **Where there is none, the body's own margin goes out**, and the two are not
/// interchangeable however alike they look. A document that has never had a
/// header still states where one would go — a Word document says half an inch,
/// against an inch of top margin — and writing that number for a page with no
/// header to fill the space moves every line of the document half an inch up
/// the page. `page::section` draws the same distinction reading, which is what
/// makes the round trip close: with no header the margin it reads is the body's.
fn page_layout(
    out: &mut String,
    section: &wp_model::section::SectionProps,
    has_header: bool,
    has_footer: bool,
) {
    let page = &section.page;
    let margins = &section.margins;
    let top = match has_header && margins.header.0 > 0 && margins.header.0 < margins.top.0 {
        true => margins.header,
        false => margins.top,
    };
    let bottom = match has_footer && margins.footer.0 > 0 && margins.footer.0 < margins.bottom.0 {
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
    // What the band takes out of the page, which is the other half of the same
    // arithmetic: ODF's margin reaches the band and `page::section` puts the
    // body below it by the band's own least height, so that height is the
    // distance between the two margins. Without it a document with a header
    // would come back with its body where the header is.
    if has_header {
        band_style(
            out,
            "header",
            margins.top,
            margins.header,
            section.header_gap,
        );
    }
    if has_footer {
        band_style(
            out,
            "footer",
            margins.bottom,
            margins.footer,
            section.footer_gap,
        );
    }
    out.push_str("</style:page-layout>");
}

/// `<style:header-style>` or `<style:footer-style>`.
///
/// The gap is stated as the band's own margin facing the text — the bottom of a
/// header, the top of a footer — because the two face it from opposite sides.
fn band_style(
    out: &mut String,
    which: &str,
    body: wp_model::Twips,
    band: wp_model::Twips,
    gap: wp_model::Twips,
) {
    let height = wp_model::Twips((body.0 - band.0).max(0));
    let facing = match which {
        "header" => "margin-bottom",
        _ => "margin-top",
    };
    let _ = write!(
        out,
        r#"<style:{which}-style><style:header-footer-properties fo:min-height="{}" fo:{facing}="{}"/></style:{which}-style>"#,
        length(height),
        length(gap)
    );
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

    /// The keystroke drive's own document: a new one, a ruled two-by-two
    /// table, and page breaks. Saved as `.odt`, the table came back as
    /// unruled text with no column widths — the element was written, and
    /// nothing that says what it looks like — and every page break was gone.
    #[test]
    fn a_new_documents_ruled_table_and_its_page_breaks_come_back() {
        use wp_model::doc::{Break, Inline, Piece, Run};
        use wp_model::prop::{Border, BorderStyle};
        use wp_model::table::{Cell, Row, Table, TableBorders, TableProps};
        use wp_model::units::{Eighth, Twips};

        let rule = Border {
            style: BorderStyle::Single,
            size: Some(Eighth(4)),
            ..Border::default()
        };
        let cell = |text: &str| Cell {
            content: vec![Block::Paragraph(Paragraph::of(text))],
            ..Cell::new()
        };
        let table = Table {
            props: TableProps {
                borders: TableBorders {
                    top: Some(rule),
                    start: Some(rule),
                    bottom: Some(rule),
                    end: Some(rule),
                    inside_h: Some(rule),
                    inside_v: Some(rule),
                },
                ..TableProps::default()
            },
            grid: vec![Twips(4680), Twips(2340)],
            rows: vec![
                Row {
                    cells: vec![cell("A1"), cell("B1")],
                    ..Row::new()
                },
                Row {
                    cells: vec![cell("A2"), cell("B2")],
                    ..Row::new()
                },
            ],
        };
        let ending = |text: &str| {
            let mut run = Run::of(text);
            run.content.push(Piece::Break(Break::Page));
            Paragraph {
                content: vec![Inline::Run(run)],
                ..Paragraph::default()
            }
        };
        let mut split = Run::of("left of it");
        split.content.push(Piece::Break(Break::Page));
        split.content.push(Piece::Text("right of it".into()));
        let mut document = Document::new();
        document.body = vec![
            Block::Paragraph(Paragraph::of("Before the table.")),
            Block::Table(table.clone()),
            Block::Paragraph(ending("Ends its page.")),
            Block::Paragraph(Paragraph::of("On the next.")),
            Block::Paragraph(Paragraph {
                content: vec![Inline::Run(split)],
                ..Paragraph::default()
            }),
        ];

        let mut container = container_for(&document).expect("a package");
        super::super::flush(&mut document, &mut container).expect("it writes");
        let (read, _) = crate::read(&container).expect("it reads");

        let Some(Block::Table(back)) = read.body.get(1) else {
            panic!("the table is a table: {:?}", read.body.get(1));
        };
        assert_eq!(
            back.grid, table.grid,
            "its columns are as wide as they were"
        );
        assert_eq!(back.rows.len(), 2);
        for (r, row) in back.rows.iter().enumerate() {
            for (c, cell) in row.cells.iter().enumerate() {
                let edges = &cell.props.borders;
                assert!(
                    edges.top.is_some()
                        && edges.bottom.is_some()
                        && edges.start.is_some()
                        && edges.end.is_some(),
                    "cell {r},{c} is ruled on every side: {edges:?}"
                );
            }
        }
        assert_eq!(back.rows[1].cells[1].text(), "B2");

        let paragraphs = read.paragraphs();
        let ends_with_break = |at: usize| {
            paragraphs[at]
                .runs()
                .last()
                .and_then(|run| run.content.last())
                .is_some_and(|piece| matches!(piece, Piece::Break(Break::Page)))
        };
        let texts: Vec<String> = paragraphs.iter().map(|p| p.text()).collect();
        let at = texts
            .iter()
            .position(|t| t == "Ends its page.")
            .expect("there");
        assert!(
            ends_with_break(at),
            "the break at a paragraph's end is back where it was"
        );
        assert_eq!(texts[at + 1], "On the next.");
        assert_eq!(texts[at + 2], "left of it");
        assert_eq!(
            texts[at + 3],
            "right of it",
            "a break inside a paragraph cuts it"
        );
        let starts_page = read
            .styles
            .resolve_paragraph(&paragraphs[at + 3].props, None)
            .para
            .page_break_before;
        assert_eq!(starts_page, Some(true), "and what follows it starts a page");
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

    /// Found by saving a document through the application and opening it
    /// again: every line of it had moved half an inch up the page.
    ///
    /// A document with no header still says where a header would go, and a
    /// Word one says half an inch against an inch of top margin. Writing that
    /// number as the page's own margin is only right when there is a header
    /// standing in the space it leaves.
    #[test]
    fn a_page_with_no_header_is_written_at_the_margin_the_body_has() {
        let document = Document::new();
        assert_eq!(document.section.margins.top, wp_model::Twips::INCH);
        assert_eq!(document.section.margins.header, wp_model::Twips(720));

        let container = container_for(&document).expect("a package");
        let text = String::from_utf8(container.data("styles.xml").unwrap().to_vec()).unwrap();
        assert!(text.contains(r#"fo:margin-top="72pt""#), "{text}");
        assert!(text.contains(r#"fo:margin-bottom="72pt""#), "{text}");

        let (read, _) = crate::read(&container).expect("it reads back");
        assert_eq!(read.section.margins.top, wp_model::Twips::INCH);
        assert_eq!(read.section.margins.bottom, wp_model::Twips::INCH);
    }

    /// Found by driving the application: a header typed into a new document
    /// and saved as `.odt` was on the screen and nowhere in the file.
    ///
    /// The splice writer fills the band elements the master page has, and an
    /// authored master page had none — so there was nothing to fill, no error,
    /// and a document that looked saved. The page it comes back on is checked
    /// as well as the words, because the two are one arithmetic: what the band
    /// reserves is what the body is pushed down by.
    #[test]
    fn a_header_made_in_the_application_is_in_the_package_it_authors() {
        let mut document = Document::blank();
        document.headers.push(wp_model::doc::HeaderFooter {
            id: wp_model::section::HeaderId(0),
            part: None,
            rel: None,
            footer: false,
            content: vec![Block::Paragraph(Paragraph::of("Every page says this."))],
        });

        let mut container = container_for(&document).expect("a package");
        super::super::flush(&mut document, &mut container).expect("it writes");
        let text = String::from_utf8(container.data("styles.xml").unwrap().to_vec()).unwrap();
        assert!(text.contains("Every page says this."), "{text}");

        let (read, _) = crate::read(&container).expect("it reads back");
        let band = read.headers.first().expect("the header came back");
        assert!(!band.footer);
        assert_eq!(
            band.content
                .iter()
                .map(|block| match block {
                    Block::Paragraph(paragraph) => paragraph.text(),
                    _ => String::new(),
                })
                .collect::<String>(),
            "Every page says this."
        );
        assert_eq!(
            read.section.margins.top,
            wp_model::Twips::INCH,
            "the body is still an inch down the page"
        );
        assert_eq!(read.section.margins.header, wp_model::Twips(720));
    }

    /// The second half of the same finding: the header came back, and it came
    /// back in a face nobody had chosen.
    ///
    /// A header's paragraph carries its spacing and its tab stops as direct
    /// formatting, so a style is minted for it — and a minted style with no
    /// parent inherits nothing, which took the paragraph out of the document's
    /// default style while the body beside it, naming no style at all, kept it.
    #[test]
    fn a_paragraph_whose_formatting_is_minted_keeps_the_default_style_under_it() {
        let mut document = Document::new();
        let mut normal = wp_model::style::Style::new("Normal", wp_model::StyleKind::Paragraph);
        normal.run.fonts.ascii = Some("Calibri".into());
        document.styles.insert(normal);
        document.body = vec![Block::Paragraph(Paragraph {
            props: wp_model::prop::ParaProps {
                spacing: wp_model::prop::Spacing {
                    after: Some(wp_model::Twips(0)),
                    ..Default::default()
                },
                ..Default::default()
            },
            content: vec![wp_model::doc::Inline::Run(wp_model::doc::Run::of("Set in"))],
            ..Paragraph::default()
        })];

        let mut container = container_for(&document).expect("a package");
        super::super::flush(&mut document, &mut container).expect("it writes");
        let (read, _) = crate::read(&container).expect("it reads back");

        let paragraphs = read.paragraphs();
        let first = paragraphs.first().expect("the paragraph came back");
        let resolved = read.styles.resolve_paragraph(&first.props, None);
        assert_eq!(
            resolved.run.fonts.ascii.as_deref(),
            Some("Calibri"),
            "the minted style stands on the default one rather than on nothing"
        );
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
