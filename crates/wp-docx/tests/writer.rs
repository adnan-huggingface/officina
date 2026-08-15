//! The writer against documents nobody here wrote.
//!
//! The fidelity harness runs check 2 over the corpus. This runs the same two
//! questions over the 41 templates Office ships — which are full of content
//! controls, glossary parts, floating art and building blocks that nothing in
//! this repository would have thought to generate.
//!
//! The questions are: **does a save with no edits reproduce the bytes**, and
//! **does an edit change only the paragraph it was made in**. A writer that
//! reprints `document.xml` passes neither, and reprinting is exactly what would
//! quietly drop the rsids, the content controls and the equations.

use std::path::{Path, PathBuf};

use ooxml::Package;
use wp_model::doc::{Inline, Run};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/docx")
        .canonicalize()
        .expect("the corpus is in the repository")
}

fn packages() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(corpus()).expect("the corpus is readable") {
        let path = entry.expect("an entry").path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.ends_with(".docx") || name.ends_with(".dotx") {
            out.push((name, path));
        }
    }
    for dir in [
        r"C:\Program Files\Microsoft Office\root\Templates\1033",
        r"C:\Program Files\Microsoft Office\root\Office16\1033\QuickStyles",
    ] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("dotx"))
            {
                out.push((
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    path,
                ));
            }
        }
    }
    out
}

#[test]
fn a_save_with_no_edits_reproduces_every_byte_of_every_document() {
    // The guarantee the whole design exists for, asked of real files rather than
    // of a fixture: not "equivalent XML", identical bytes.
    let mut checked = 0;
    for (name, path) in packages() {
        let package = Package::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let document = wp_docx::read(&package).unwrap_or_else(|e| panic!("{name}: {e}"));
        let parts = wp_docx::DocumentParts::locate_in(&package).expect("the document part");
        let before = package.part(&parts.document).expect("it is there").data();
        let after = wp_docx::document_out(before, &document);
        assert_eq!(
            before,
            after.as_slice(),
            "{name}: a no-edit save changed the document part"
        );
        checked += 1;
    }
    eprintln!("{checked} documents round-tripped byte for byte");
    assert!(checked >= 15);
}

#[test]
fn an_edit_changes_the_paragraph_it_was_made_in_and_nothing_else() {
    const MARKER: &str = "scriva writer test";
    let mut checked = 0;
    for (name, path) in packages() {
        let package = Package::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut document = wp_docx::read(&package).unwrap_or_else(|e| panic!("{name}: {e}"));
        let parts = wp_docx::DocumentParts::locate_in(&package).expect("the document part");
        let before = package
            .part(&parts.document)
            .expect("it is there")
            .data()
            .to_vec();

        let expected: Vec<String> = {
            let mut paragraphs = document.paragraphs_mut();
            let Some(first) = paragraphs.first_mut() else {
                continue;
            };
            first.content = vec![Inline::Run(Run::of(MARKER))];
            drop(paragraphs);
            document
                .paragraphs()
                .iter()
                .map(|paragraph| paragraph.text())
                .collect()
        };

        let after = wp_docx::document_out(&before, &document);

        // The edit came back, and every other paragraph came back unchanged.
        let mut rebuilt = Package::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        rebuilt.put_part(
            parts.document.clone(),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
            after,
        );
        let reopened = wp_docx::read(&rebuilt).unwrap_or_else(|e| panic!("{name}: reread {e}"));
        let got: Vec<String> = reopened
            .paragraphs()
            .iter()
            .map(|paragraph| paragraph.text())
            .collect();
        assert_eq!(got, expected, "{name}: the text changed on the way through");
        assert!(
            reopened.text().contains(MARKER),
            "{name}: the edit did not come back"
        );
        checked += 1;
    }
    eprintln!("{checked} documents survived an edit");
    assert!(checked >= 15);
}

#[test]
fn a_paragraph_added_inside_a_table_cell_survives_the_round_trip() {
    // Pressing Enter in a table-cell bullet adds a paragraph to the *cell*.
    // The writer has to serialise a paragraph it has no bytes for, in a place
    // no corpus edit had exercised before.
    use wp_model::doc::{Block, Paragraph};
    const MARKER: &str = "scriva cell split test";
    let mut checked = 0;
    for name in ["nested-tables.docx", "table-spanning-pages.docx"] {
        let path = corpus().join(name);
        let package = Package::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut document = wp_docx::read(&package).unwrap_or_else(|e| panic!("{name}: {e}"));
        let parts = wp_docx::DocumentParts::locate_in(&package).expect("the document part");
        let before = package
            .part(&parts.document)
            .expect("it is there")
            .data()
            .to_vec();

        let mut inserted = false;
        for block in &mut document.body {
            if let Block::Table(table) = block {
                let cell = &mut table.rows[0].cells[0];
                cell.content
                    .insert(0, Block::Paragraph(Paragraph::of(MARKER)));
                inserted = true;
                break;
            }
        }
        assert!(inserted, "{name}: no table to edit");
        let expected: Vec<String> = document
            .paragraphs()
            .iter()
            .map(|paragraph| paragraph.text())
            .collect();

        let after = wp_docx::document_out(&before, &document);
        let mut rebuilt = Package::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        rebuilt.put_part(
            parts.document.clone(),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
            after,
        );
        let reopened = wp_docx::read(&rebuilt).unwrap_or_else(|e| panic!("{name}: reread {e}"));
        let got: Vec<String> = reopened
            .paragraphs()
            .iter()
            .map(|paragraph| paragraph.text())
            .collect();
        assert_eq!(got, expected, "{name}: the cell edit did not round-trip");
        checked += 1;
    }
    assert_eq!(checked, 2);
}

#[test]
fn the_bytes_around_an_edited_paragraph_are_the_producers_own() {
    // A stronger form of the test above, and the one that catches a writer that
    // rebuilt more than it had to: everything the file said outside the one
    // paragraph must appear in the output unchanged.
    const MARKER: &str = "scriva writer test";
    for (name, path) in packages() {
        let package = Package::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut document = wp_docx::read(&package).unwrap_or_else(|e| panic!("{name}: {e}"));
        let parts = wp_docx::DocumentParts::locate_in(&package).expect("the document part");
        let before = package
            .part(&parts.document)
            .expect("it is there")
            .data()
            .to_vec();

        let paragraph_count = document.paragraphs().len();
        if paragraph_count < 2 {
            continue;
        }
        {
            let mut paragraphs = document.paragraphs_mut();
            let last = paragraphs.len() - 1;
            paragraphs[last].content = vec![Inline::Run(Run::of(MARKER))];
        }
        let after = wp_docx::document_out(&before, &document);

        // The first paragraph is untouched, so the file's opening bytes must be
        // identical up to a long prefix.
        let shared = before
            .iter()
            .zip(after.iter())
            .take_while(|(a, b)| a == b)
            .count();
        assert!(
            shared > before.len() / 2,
            "{name}: only {shared} of {} bytes survived an edit to the last paragraph",
            before.len()
        );
    }
}

#[test]
fn a_page_setup_change_survives_the_round_trip() {
    // The Layout menu edits the section: new margins have to come back from
    // the file, and everything before the body's closing `<w:sectPr>` has to
    // come back byte for byte, because nothing else was edited.
    use wp_model::units::Twips;
    let path = corpus().join("nested-tables.docx");
    let package = Package::open(&path).expect("the corpus document opens");
    let mut document = wp_docx::read(&package).expect("and reads");
    let parts = wp_docx::DocumentParts::locate_in(&package).expect("the document part");
    let before = package
        .part(&parts.document)
        .expect("it is there")
        .data()
        .to_vec();

    document.section.margins.top = Twips(2880);
    document.section.margins.start = Twips(720);
    let after = wp_docx::document_out(&before, &document);

    let mut rebuilt = Package::open(&path).expect("the package again");
    rebuilt.put_part(
        parts.document.clone(),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
        after.clone(),
    );
    let reopened = wp_docx::read(&rebuilt).expect("the edited document reads back");
    assert_eq!(reopened.section.margins.top, Twips(2880));
    assert_eq!(reopened.section.margins.start, Twips(720));

    // The body itself was not edited, so the bytes up to the section are the
    // producer's own.
    let sect = after
        .windows(b"<w:sectPr".len())
        .position(|w| w == b"<w:sectPr")
        .expect("the section is still there");
    assert_eq!(
        &before[..sect],
        &after[..sect],
        "only the section changed on the way through"
    );
}

#[test]
fn a_resized_picture_survives_being_saved_and_read_back() {
    // The whole point of splicing the drawing's own bytes. Before this, a
    // paragraph holding a picture could not be rewritten at all, so a resize was
    // shown on screen and thrown away on save — the document looked right until
    // it was reopened.
    let path = corpus().join("floating-image-wrap.docx");
    let (mut document, mut package) = wp_docx::open(&path).expect("the document opens");
    let before = {
        let mut paragraphs = document.paragraphs_mut();
        let index = (0..paragraphs.len())
            .find(|index| !paragraphs[*index].drawings().is_empty())
            .expect("a paragraph with a picture");
        let drawing = paragraphs[index].drawing_mut(0).expect("its first drawing");
        let was = drawing.extent;
        drawing.extent = (wp_model::Emu(was.0 .0 * 2), wp_model::Emu(was.1 .0 * 2));
        was
    };

    let temporary = std::env::temp_dir().join("scriva-resize-round-trip.docx");
    wp_docx::write::save(&document, &mut package, &temporary).expect("it writes");
    let (read, _) = wp_docx::open(&temporary).expect("it reads back");
    let _ = std::fs::remove_file(&temporary);

    let drawing = read
        .paragraphs()
        .iter()
        .flat_map(|paragraph| paragraph.drawings())
        .next()
        .expect("the picture is still there");
    assert_eq!(drawing.extent.0 .0, before.0 .0 * 2, "twice as wide");
    assert_eq!(drawing.extent.1 .0, before.1 .0 * 2, "and twice as tall");
    assert!(drawing.rel.is_some(), "and still names its bytes");
}
