//! A workbook that was never a file, saved and read back.
//!
//! The fidelity harness only ever sees documents Excel produced, so it says
//! nothing about the package we author from nothing. This covers the other
//! direction: File → New, type into it, save, and get all of it back.
//!
//! What is actually being checked is that the *authored* package and the
//! *spliced* writer meet in the middle. The skeleton written at creation is
//! deliberately empty — `<sheetData/>`, an empty string table, one style — so
//! everything the user types has to arrive through the same code that edits a
//! real Excel file. If those two disagreed, this is where it would show.

use std::io::Cursor;

use ss_formula::edit;
use ss_model::{CellRef, CellValue, Workbook};
use ss_xlsx::XlsxDocument;

fn at(a1: &str) -> CellRef {
    CellRef::from_a1(a1).expect("valid address")
}

/// Types into a new workbook, saves it, and reads it back.
fn round_trip(typed: &[(&str, &str)]) -> XlsxDocument {
    let mut doc = XlsxDocument::new(Workbook::blank()).expect("authors a package");
    for (cell, text) in typed {
        let change = edit::input(&mut doc.workbook, 0, at(cell), text);
        edit::apply(&mut doc.workbook, change);
    }
    ss_formula::recalculate(&mut doc.workbook);

    let mut bytes = Vec::new();
    doc.write_to(Cursor::new(&mut bytes)).expect("writes");
    XlsxDocument::read(Cursor::new(bytes)).expect("reads back")
}

#[test]
fn a_workbook_typed_from_nothing_survives_a_save() {
    let reopened = round_trip(&[
        ("A1", "Revenue"),
        ("A2", "100"),
        ("A3", "250.5"),
        ("A4", "=SUM(A2:A3)"),
        ("B1", "TRUE"),
        ("B2", "=1/0"),
    ]);

    let sheet = &reopened.workbook.sheets[0];
    let text = |a1: &str| match sheet.get(at(a1)).map(|c| c.value) {
        Some(CellValue::Text(id)) => reopened.workbook.strings.resolve(id).to_string(),
        other => panic!("{a1} is {other:?}, not text"),
    };
    let number = |a1: &str| match sheet.get(at(a1)).map(|c| c.value) {
        Some(CellValue::Number(n)) => n,
        other => panic!("{a1} is {other:?}, not a number"),
    };

    assert_eq!(text("A1"), "Revenue");
    assert_eq!(number("A2"), 100.0);
    assert_eq!(number("A3"), 250.5);
    assert_eq!(number("A4"), 350.5, "the cached total came back");
    assert_eq!(
        sheet.formula_at(at("A4")).map(|f| f.text.as_str()),
        Some("SUM(A2:A3)"),
        "and so did the formula behind it"
    );
    assert_eq!(
        sheet.get(at("B1")).map(|c| c.value),
        Some(CellValue::Bool(true))
    );
    assert!(matches!(
        sheet.get(at("B2")).map(|c| c.value),
        Some(CellValue::Error(_))
    ));
}

#[test]
fn a_typed_date_comes_back_as_a_date_and_not_as_a_serial() {
    // The value stored is the serial. What makes the cell read as a date is a
    // style, which a new workbook does not have until typing one creates it —
    // and the style has to reach the file or the user opens it to `45306`.
    let reopened = round_trip(&[("A1", "2024-01-15")]);
    let sheet = &reopened.workbook.sheets[0];

    assert_eq!(
        sheet.get(at("A1")).map(|c| c.value),
        Some(CellValue::Number(45306.0))
    );
    let style = sheet.get(at("A1")).expect("cell").style;
    let shown = reopened
        .workbook
        .styles
        .number_format(style)
        .format(ss_model::numfmt::FormatValue::Number(45306.0))
        .text;
    assert_ne!(shown, "45306", "the format survived the save");
    assert!(shown.contains("24") || shown.contains("2024"), "{shown}");
}

#[test]
fn the_authored_package_has_what_a_reader_looks_for() {
    let doc = round_trip(&[("A1", "1")]);
    let names: Vec<String> = doc
        .package
        .parts()
        .map(|p| p.name.as_str().to_string())
        .collect();

    for wanted in [
        "/_rels/.rels",
        "/xl/workbook.xml",
        "/xl/_rels/workbook.xml.rels",
        "/xl/worksheets/sheet1.xml",
        "/xl/styles.xml",
        "/xl/sharedStrings.xml",
    ] {
        assert!(
            names.iter().any(|n| n == wanted),
            "{wanted} missing: {names:?}"
        );
    }

    // Content types are what tell a consumer this is a workbook rather than a
    // zip of XML, and getting one wrong is the difference between opening and
    // "we found a problem with some content".
    let main = ooxml::PartName::new("/xl/workbook.xml").expect("valid");
    assert_eq!(
        doc.package.content_types().get(&main),
        Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml")
    );
}

#[test]
fn saving_twice_is_the_same_as_saving_once() {
    // The second save re-reads the string and style tables out of the parts the
    // first one wrote. If it read them differently, text would be appended to
    // the table twice and every cell after the first duplicate would shift.
    let mut doc = XlsxDocument::new(Workbook::blank()).expect("authors a package");
    let change = edit::input(&mut doc.workbook, 0, at("A1"), "Total");
    edit::apply(&mut doc.workbook, change);

    let mut first = Vec::new();
    doc.write_to(Cursor::new(&mut first)).expect("writes");
    let mut second = Vec::new();
    doc.write_to(Cursor::new(&mut second))
        .expect("writes again");

    assert_eq!(first, second);
}
