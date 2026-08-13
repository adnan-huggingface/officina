//! Notes on cells, through a save and back.
//!
//! Built in memory rather than taken from the corpus: none of the corpus files
//! carry a note, and what is under test is package plumbing — a comments part,
//! a VML part beside it, two relationships and a `<legacyDrawing>` — which is
//! exactly what a fixture can express.

use std::io::Cursor;

use ss_model::{CellRef, Comment, Workbook};
use ss_xlsx::XlsxDocument;

fn with_notes(notes: Vec<Comment>) -> XlsxDocument {
    let mut book = Workbook::blank();
    book.sheets[0].comments = notes;
    let mut doc = XlsxDocument::new(book).expect("authors a package");
    let mut buffer = Cursor::new(Vec::new());
    doc.write_to(&mut buffer).expect("writes");
    XlsxDocument::read(Cursor::new(buffer.into_inner())).expect("reads back")
}

#[test]
fn a_note_written_into_a_blank_workbook_comes_back_on_its_cell() {
    let notes = vec![
        Comment::new(CellRef::new(1, 1), "Ada", "Ada:\ncheck against the ledger"),
        Comment::new(CellRef::new(7, 3), "Grace", "Grace:\nrounded up"),
    ];
    let reopened = with_notes(notes.clone());
    assert_eq!(reopened.workbook.sheets[0].comments, notes);
}

#[test]
fn the_worksheet_names_the_shape_that_draws_the_boxes() {
    // Excel offers to repair a file whose comments part has no VML beside it,
    // which is a worse outcome than the note not being there at all.
    let mut doc = with_notes(vec![Comment::new(CellRef::new(0, 0), "Ada", "hello")]);
    doc.flush().expect("flushes");
    let mut buffer = Cursor::new(Vec::new());
    doc.write_to(&mut buffer).expect("writes");

    let bytes = buffer.into_inner();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("a zip");
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).expect("entry").name().to_string())
        .collect();
    assert!(
        names.iter().any(|n| n.ends_with("comments1.xml")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("vmlDrawing1.vml")),
        "{names:?}"
    );

    let mut sheet = String::new();
    {
        use std::io::Read;
        zip.by_name("xl/worksheets/sheet1.xml")
            .expect("the sheet")
            .read_to_string(&mut sheet)
            .expect("utf-8");
    }
    assert!(sheet.contains("<legacyDrawing r:id="), "{sheet}");
}

#[test]
fn deleting_every_note_leaves_the_sheet_with_none() {
    let mut doc = with_notes(vec![Comment::new(CellRef::new(2, 2), "Ada", "temporary")]);
    doc.workbook.sheets[0].comments.clear();
    let mut buffer = Cursor::new(Vec::new());
    doc.write_to(&mut buffer).expect("writes");

    let reopened = XlsxDocument::read(Cursor::new(buffer.into_inner())).expect("reads back");
    assert!(reopened.workbook.sheets[0].comments.is_empty());
}

#[test]
fn a_note_moves_down_with_the_row_it_is_on() {
    let mut book = Workbook::blank();
    book.sheets[0]
        .comments
        .push(Comment::new(CellRef::new(4, 1), "Ada", "on row five"));
    book.sheets[0].insert_rows(1, 2);
    assert_eq!(book.sheets[0].comments[0].at, CellRef::new(6, 1));

    // And a note on a row that is deleted goes with it.
    book.sheets[0].delete_rows(6, 1);
    assert!(book.sheets[0].comments.is_empty());
}
