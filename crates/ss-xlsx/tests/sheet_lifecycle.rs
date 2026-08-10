//! Adding, removing, renaming and reordering sheets, through the file.
//!
//! Every assertion here is made against a package that has been written out and
//! read back. That is deliberate: the model round-trips through itself
//! perfectly whether or not a single byte reached the file, and this is exactly
//! the class of bug — a writer that was never written — that a green model test
//! suite hid for five chunks.

use std::io::Cursor;

use ss_formula::{edit, sheets};
use ss_model::{Cell, CellRef, CellValue, StyleId, Workbook};
use ss_xlsx::XlsxDocument;

fn at(a1: &str) -> CellRef {
    CellRef::from_a1(a1).expect("valid address")
}

fn new_doc() -> XlsxDocument {
    XlsxDocument::new(Workbook::blank()).expect("authors a package")
}

fn put(doc: &mut XlsxDocument, sheet: usize, a1: &str, n: f64) {
    doc.workbook.sheets[sheet].set(
        at(a1),
        Cell {
            value: CellValue::Number(n),
            style: StyleId::DEFAULT,
            formula: None,
        },
    );
}

fn reopen(doc: &mut XlsxDocument) -> XlsxDocument {
    let mut bytes = Vec::new();
    doc.write_to(Cursor::new(&mut bytes)).expect("writes");
    XlsxDocument::read(Cursor::new(bytes)).expect("reads back")
}

fn names(doc: &XlsxDocument) -> Vec<String> {
    doc.workbook.sheets.iter().map(|s| s.name.clone()).collect()
}

fn number(doc: &XlsxDocument, sheet: usize, a1: &str) -> Option<f64> {
    match doc.workbook.sheets.get(sheet)?.get(at(a1))?.value {
        CellValue::Number(n) => Some(n),
        _ => None,
    }
}

#[test]
fn a_sheet_added_to_the_model_reaches_the_file_with_its_cells() {
    let mut doc = new_doc();
    put(&mut doc, 0, "A1", 1.0);

    let change = sheets::insert(&doc.workbook, 1, "Added");
    edit::apply(&mut doc.workbook, change);
    put(&mut doc, 1, "B2", 42.0);

    let reopened = reopen(&mut doc);
    assert_eq!(names(&reopened), ["Sheet1", "Added"]);
    assert_eq!(number(&reopened, 0, "A1"), Some(1.0));
    assert_eq!(
        number(&reopened, 1, "B2"),
        Some(42.0),
        "the authored part got the new sheet's cells"
    );
}

#[test]
fn a_sheet_removed_from_the_model_takes_its_part_with_it() {
    let mut doc = new_doc();
    for (index, name) in [(1usize, "Two"), (2, "Three")] {
        let change = sheets::insert(&doc.workbook, index, name);
        edit::apply(&mut doc.workbook, change);
    }
    put(&mut doc, 2, "A1", 7.0);
    let mut doc = reopen(&mut doc);
    assert_eq!(names(&doc), ["Sheet1", "Two", "Three"]);
    let gone = doc.workbook.sheets[1]
        .part
        .clone()
        .expect("read from a part");

    let change = sheets::remove(&doc.workbook, 1);
    edit::apply(&mut doc.workbook, change);
    let reopened = reopen(&mut doc);

    assert_eq!(names(&reopened), ["Sheet1", "Three"]);
    assert_eq!(
        number(&reopened, 1, "A1"),
        Some(7.0),
        "the sheet after it kept its own cells"
    );
    assert!(
        reopened
            .package
            .parts()
            .all(|p| p.name.as_str() != gone.as_str()),
        "the part is still in the package: {gone}"
    );
    // An *override* naming a part that is not there is an invalid package, and
    // Excel reports it as damage rather than as a missing sheet. The extension
    // default is a different thing and is shared with every other part.
    assert!(
        reopened
            .package
            .content_types()
            .overrides()
            .all(|(name, _)| name.as_str() != gone.as_str()),
        "an override still names {gone}"
    );
}

#[test]
fn a_rename_reaches_the_file_and_the_formulas_that_named_it() {
    let mut doc = new_doc();
    let change = sheets::insert(&doc.workbook, 1, "Data");
    edit::apply(&mut doc.workbook, change);
    put(&mut doc, 1, "A1", 5.0);
    let change = edit::input(&mut doc.workbook, 0, at("A1"), "=Data!A1*2");
    edit::apply(&mut doc.workbook, change);

    let mut doc = reopen(&mut doc);
    let change = sheets::rename(&doc.workbook, 1, "Q1 Results");
    edit::apply(&mut doc.workbook, change);
    let reopened = reopen(&mut doc);

    assert_eq!(names(&reopened), ["Sheet1", "Q1 Results"]);
    assert_eq!(
        reopened.workbook.sheets[0]
            .formula_at(at("A1"))
            .map(|f| f.text.as_str()),
        Some("'Q1 Results'!A1*2"),
        "a name with a space is quoted, and it survived the file"
    );
}

#[test]
fn reordering_tabs_moves_the_entries_and_not_the_contents() {
    let mut doc = new_doc();
    for (index, name) in [(1usize, "Two"), (2, "Three")] {
        let change = sheets::insert(&doc.workbook, index, name);
        edit::apply(&mut doc.workbook, change);
    }
    put(&mut doc, 0, "A1", 1.0);
    put(&mut doc, 1, "A1", 2.0);
    put(&mut doc, 2, "A1", 3.0);
    let mut doc = reopen(&mut doc);

    let change = sheets::reorder(&doc.workbook, 2, 0);
    edit::apply(&mut doc.workbook, change);
    let reopened = reopen(&mut doc);

    assert_eq!(names(&reopened), ["Three", "Sheet1", "Two"]);
    assert_eq!(number(&reopened, 0, "A1"), Some(3.0));
    assert_eq!(number(&reopened, 1, "A1"), Some(1.0));
    assert_eq!(number(&reopened, 2, "A1"), Some(2.0));
}

#[test]
fn hiding_a_sheet_writes_the_state_attribute() {
    let mut doc = new_doc();
    let change = sheets::insert(&doc.workbook, 1, "Working");
    edit::apply(&mut doc.workbook, change);
    let mut doc = reopen(&mut doc);

    let change = sheets::set_hidden(1, true);
    edit::apply(&mut doc.workbook, change);
    let reopened = reopen(&mut doc);

    assert!(reopened.workbook.sheets[1].hidden);
    assert!(!reopened.workbook.sheets[0].hidden);
}

#[test]
fn a_copied_sheet_gets_a_part_of_its_own() {
    // Two sheets sharing a part would have the writer put both their cells in
    // one file and lose whichever it wrote first.
    let mut doc = new_doc();
    put(&mut doc, 0, "A1", 11.0);
    let mut doc = reopen(&mut doc);

    let change = sheets::duplicate(&doc.workbook, 0, 1, "Sheet1 (2)");
    edit::apply(&mut doc.workbook, change);
    put(&mut doc, 1, "A2", 22.0);
    let reopened = reopen(&mut doc);

    assert_eq!(names(&reopened), ["Sheet1", "Sheet1 (2)"]);
    assert_eq!(number(&reopened, 0, "A1"), Some(11.0));
    assert_eq!(
        number(&reopened, 0, "A2"),
        None,
        "the original is unchanged"
    );
    assert_eq!(number(&reopened, 1, "A1"), Some(11.0), "the copy has both");
    assert_eq!(number(&reopened, 1, "A2"), Some(22.0));

    let parts: Vec<Option<String>> = reopened
        .workbook
        .sheets
        .iter()
        .map(|s| s.part.clone())
        .collect();
    assert_ne!(parts[0], parts[1]);
}

#[test]
fn a_save_that_changes_no_sheet_leaves_the_workbook_part_alone() {
    // The whole reconciliation is skipped when nothing structural differs, so a
    // save that only touched a cell must not rewrite workbook.xml or the
    // relationship part. This is the guarantee the fidelity harness rests on.
    let mut doc = new_doc();
    let change = sheets::insert(&doc.workbook, 1, "Data");
    edit::apply(&mut doc.workbook, change);
    let mut doc = reopen(&mut doc);

    let before = |doc: &XlsxDocument, path: &str| {
        let name = ooxml::PartName::new(path).expect("valid");
        doc.package.part(&name).expect("present").data().to_vec()
    };
    let workbook_before = before(&doc, "/xl/workbook.xml");
    let rels_before = before(&doc, "/xl/_rels/workbook.xml.rels");

    put(&mut doc, 0, "C3", 9.0);
    doc.flush().expect("flushes");

    assert_eq!(before(&doc, "/xl/workbook.xml"), workbook_before);
    assert_eq!(before(&doc, "/xl/_rels/workbook.xml.rels"), rels_before);
}

#[test]
fn defined_names_follow_their_sheets_through_the_file() {
    let mut doc = new_doc();
    let change = sheets::insert(&doc.workbook, 1, "Data");
    edit::apply(&mut doc.workbook, change);
    doc.workbook.defined_names.push(ss_model::DefinedName {
        name: "Local".into(),
        refers_to: "Data!$A$1".into(),
        scope: Some(1),
    });
    let mut doc = reopen(&mut doc);
    assert_eq!(doc.workbook.defined_names[0].scope, Some(1));

    // A sheet inserted in front of it re-points the scope, and that has to
    // reach the file or the name comes back attached to the wrong tab.
    let change = sheets::insert(&doc.workbook, 0, "First");
    edit::apply(&mut doc.workbook, change);
    let reopened = reopen(&mut doc);

    assert_eq!(names(&reopened), ["First", "Sheet1", "Data"]);
    assert_eq!(reopened.workbook.defined_names[0].scope, Some(2));
    assert_eq!(reopened.workbook.defined_names[0].refers_to, "Data!$A$1");
}
