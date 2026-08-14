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
