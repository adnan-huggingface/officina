//! The reader against real `.doc` files.
//!
//! Two sources, neither of them ours. The corpus in this repository was written
//! by Word itself with known content, so what the text *should* be is known. And
//! Office ships two legacy `.doc` files of its own — the same move as the Excel
//! sample workbook: a document nobody here designed, full of whatever a real
//! producer puts in a file.

use std::path::{Path, PathBuf};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/doc")
}

fn text_of(document: &wp_model::Document) -> String {
    document
        .paragraphs()
        .iter()
        .map(|paragraph| paragraph.text())
        .collect::<Vec<_>>()
        .join("\n")
}

fn open(name: &str) -> wp_model::Document {
    let (document, _) =
        wp_doc::open(corpus().join(name)).unwrap_or_else(|error| panic!("{name}: {error}"));
    document
}

#[test]
fn the_words_of_a_plain_document_come_out_in_order() {
    let text = text_of(&open("plain-paragraphs.doc"));
    assert!(text.contains("The first paragraph."), "{text}");
    assert!(text.contains("more than one line"), "{text}");
    assert!(text.contains("The third."), "{text}");
    // And in that order, which the piece table is what makes true.
    let first = text.find("first").expect("the first paragraph");
    let third = text.find("third").expect("the third paragraph");
    assert!(first < third);
}

#[test]
fn every_paragraph_mark_starts_a_new_paragraph() {
    let document = open("plain-paragraphs.doc");
    let text: Vec<String> = document
        .paragraphs()
        .iter()
        .map(|paragraph| paragraph.text())
        .filter(|text| !text.trim().is_empty())
        .collect();
    assert_eq!(text.len(), 3, "three paragraphs, not one: {text:?}");
}

#[test]
fn direct_character_formatting_survives() {
    // Bold in the middle of a sentence is the thing that tells a working bin
    // table from one that is being read at the wrong offset.
    let document = open("character-formatting.doc");
    let runs: Vec<_> = document
        .paragraphs()
        .iter()
        .flat_map(|paragraph| paragraph.runs())
        .map(|run| {
            (
                run.text(),
                run.props.toggles.get(wp_model::prop::Toggle::Bold),
                run.props.toggles.get(wp_model::prop::Toggle::Italic),
                run.props.underline.is_some(),
            )
        })
        .collect();
    assert!(
        runs.iter()
            .any(|(text, bold, _, _)| { text.contains("bold") && *bold == Some(true) }),
        "the bold word is bold: {runs:?}"
    );
    assert!(
        runs.iter()
            .any(|(text, _, italic, _)| text.contains("italic") && *italic == Some(true)),
        "the italic word is italic: {runs:?}"
    );
}

#[test]
fn a_heading_says_which_style_it_is_in() {
    let document = open("headings-and-list.doc");
    let heading = document
        .paragraphs()
        .into_iter()
        .find(|paragraph| paragraph.text().contains("A Heading"))
        .expect("the heading is there");
    let style = heading.props.style.expect("it names a style");
    let name = document
        .styles
        .get(style)
        .and_then(|style| style.name.clone())
        .expect("the style has a name");
    assert!(name.contains("eading"), "{name}");
}

#[test]
fn a_table_comes_out_as_a_table_rather_than_run_on_words() {
    let document = open("simple-table.doc");
    let tables: Vec<_> = document
        .body
        .iter()
        .filter_map(|block| match block {
            wp_model::doc::Block::Table(table) => Some(table),
            _ => None,
        })
        .collect();
    assert_eq!(tables.len(), 1, "one table");
    assert_eq!(tables[0].rows.len(), 2, "two rows");
    assert_eq!(tables[0].rows[0].cells.len(), 3, "three cells in the first");
    let first = tables[0].rows[0].cells[0]
        .content
        .iter()
        .filter_map(|block| match block {
            wp_model::doc::Block::Paragraph(paragraph) => Some(paragraph.text()),
            _ => None,
        })
        .collect::<String>();
    assert!(first.contains("one"), "the first cell says one: {first:?}");
}

#[test]
fn the_body_does_not_contain_the_header_the_footer_or_the_footnote() {
    // A `.doc` lays every part end to end in one coordinate space, so a reader
    // that ignores the counts puts the running header in the middle of the text.
    let document = open("header-footer-footnote.doc");
    let text = text_of(&document);
    assert!(text.contains("The body of the document."), "{text}");
    assert!(!text.contains("Running header"), "{text}");
    assert!(!text.contains("Running footer"), "{text}");
    assert!(!text.contains("A footnote about the body."), "{text}");
}

#[test]
fn the_other_parts_are_still_read_rather_than_thrown_away() {
    let doc = wp_doc::read(corpus().join("header-footer-footnote.doc")).expect("it opens");
    let headers = doc.part(wp_doc::Part::Headers);
    let footnotes = doc.part(wp_doc::Part::Footnotes);
    assert!(headers.contains("Running header"), "{headers:?}");
    assert!(footnotes.contains("A footnote"), "{footnotes:?}");
}

#[test]
fn the_two_legacy_documents_office_ships_open() {
    // Nobody here designed these, which is the point of them.
    let mut opened = 0;
    for name in ["PROTTPLN.DOC", "PROTTPLV.DOC"] {
        let path = Path::new(r"C:\Program Files\Microsoft Office\root\Office16\1033").join(name);
        if !path.exists() {
            continue;
        }
        let (document, _) = wp_doc::open(&path).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(!document.paragraphs().is_empty(), "{name} has paragraphs");
        opened += 1;
    }
    if opened == 0 {
        eprintln!("Office is not installed here; this test had nothing to read");
    }
}

#[test]
fn a_legacy_document_can_be_saved_as_a_modern_one() {
    // The escape hatch, end to end: read a format that cannot be written, and
    // write the words out in one that can. A reader with no way out of the old
    // format is a museum piece.
    let mut document = open("plain-paragraphs.doc");
    let before: Vec<String> = document
        .paragraphs()
        .iter()
        .map(|paragraph| paragraph.text())
        .filter(|text| !text.trim().is_empty())
        .collect();

    let mut package = wp_docx::write::blank::package_for(&document).expect("a package");
    let path = std::env::temp_dir().join("wp-doc-escape-hatch.docx");
    wp_docx::write::save(&mut document, &mut package, &path).expect("it writes");
    let (read, _) = wp_docx::open(&path).expect("and opens again");
    let _ = std::fs::remove_file(&path);

    let after: Vec<String> = read
        .paragraphs()
        .iter()
        .map(|paragraph| paragraph.text())
        .filter(|text| !text.trim().is_empty())
        .collect();
    assert_eq!(before, after);
}

#[test]
fn the_page_setup_comes_from_the_file_rather_than_from_a_default() {
    // A document written on A4 that opens as Letter reflows on its first line
    // and paginates differently for the whole of its length.
    let document = open("plain-paragraphs.doc");
    assert!(
        document.section.page.width.0 > 0,
        "the paper has a width: {:?}",
        document.section.page
    );
    assert!(
        document.section.margins.start.0 > 0,
        "and a left margin: {:?}",
        document.section.margins
    );
    // Word's own default here, which is what these were written with.
    assert_eq!(document.section.page.width, wp_model::Twips(12240));
    assert_eq!(document.section.page.height, wp_model::Twips(15840));
}
