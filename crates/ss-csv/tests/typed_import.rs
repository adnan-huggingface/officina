//! Importing a delimited file the way the application does it.
//!
//! `ss-csv` deliberately does not know how to interpret a field — that lives in
//! `ss-formula`, and arrives as a callback. This is the two halves put together,
//! which is the only place the *result* of an import can be checked: a column
//! of dates has to come back as dates rather than as strings, and a field
//! beginning with `=` has to come back as a formula.

use std::io::Cursor;

use ss_csv::{Dialect, Encoding, Reader};
use ss_formula::edit;
use ss_model::{CellRef, CellValue, Sheet, StyleId, Workbook};

fn import(text: &str) -> Workbook {
    let (encoding, dialect) = ss_csv::sniff(text.as_bytes());
    import_with(text, encoding, dialect)
}

fn import_with(text: &str, encoding: Encoding, dialect: Dialect) -> Workbook {
    let mut book = Workbook::blank();
    let mut reader = Reader::new(Cursor::new(text.as_bytes()), encoding, dialect);
    ss_csv::read_into(&mut reader, |row, fields| {
        for (col, field) in fields.iter().enumerate() {
            if field.is_empty() {
                continue;
            }
            let cell = edit::typed_cell(&mut book, 0, StyleId::DEFAULT, field);
            book.sheets[0].set(CellRef::new(row, col as u32), cell);
        }
    })
    .expect("reads");
    ss_formula::recalculate(&mut book);
    book
}

fn at(a1: &str) -> CellRef {
    CellRef::from_a1(a1).expect("valid address")
}

fn value(sheet: &Sheet, a1: &str) -> CellValue {
    sheet
        .get(at(a1))
        .map(|c| c.value)
        .unwrap_or(CellValue::Blank)
}

#[test]
fn fields_are_interpreted_the_same_way_typing_them_would_be() {
    let book = import("Item,Qty,Price,When\nWidget,3,4.50,2024-01-15\n");
    let sheet = &book.sheets[0];

    assert_eq!(
        book.strings.resolve(match value(sheet, "A1") {
            CellValue::Text(id) => id,
            other => panic!("{other:?}"),
        }),
        "Item"
    );
    assert_eq!(value(sheet, "B2"), CellValue::Number(3.0));
    assert_eq!(value(sheet, "C2"), CellValue::Number(4.5));
    assert_eq!(
        value(sheet, "D2"),
        CellValue::Number(45306.0),
        "a date is a serial"
    );

    // And the serial comes with the format that makes it read as a date, or the
    // user sees 45306.
    let shown = book
        .styles
        .number_format(sheet.style_at(at("D2")))
        .format(ss_model::FormatValue::Number(45306.0))
        .text;
    assert_ne!(shown, "45306", "{shown}");
}

#[test]
fn a_leading_zero_is_dropped_exactly_as_excel_drops_it() {
    // Product codes, postcodes, and phone numbers, and the single most
    // complained-about behaviour of any spreadsheet. It is *Excel's* behaviour,
    // and matching Excel is the contract: the same file opened here and there
    // has to hold the same values, or a formula written against one is wrong
    // against the other. Quoting the field does not change it — quotes are
    // about the delimiter, not about the type — and only a leading apostrophe
    // does, in Excel and here.
    let book = import("Code\n007\n\"0123\"\n'0456\n");
    let sheet = &book.sheets[0];
    assert_eq!(value(sheet, "A2"), CellValue::Number(7.0));
    assert_eq!(value(sheet, "A3"), CellValue::Number(123.0));
    assert!(
        matches!(value(sheet, "A4"), CellValue::Text(_)),
        "an apostrophe forces text: {:?}",
        value(sheet, "A4")
    );
}

#[test]
fn a_formula_in_a_field_is_a_formula() {
    let book = import("a,b,c\n2,3,=A2+B2\n");
    let sheet = &book.sheets[0];
    assert_eq!(
        sheet.formula_at(at("C2")).map(|f| f.text.as_str()),
        Some("A2+B2")
    );
    assert_eq!(value(sheet, "C2"), CellValue::Number(5.0), "and it ran");
}

#[test]
fn a_semicolon_file_from_a_european_export_opens_as_columns() {
    // Read as comma-separated this is one column of nonsense, and it opens
    // rather than failing — which is why the sniffer exists.
    let book = import("Name;Menge\nSchraube;12\nMutter;8\n");
    let sheet = &book.sheets[0];
    assert_eq!(value(sheet, "B2"), CellValue::Number(12.0));
    assert_eq!(value(sheet, "B3"), CellValue::Number(8.0));
}

#[test]
fn a_windows_1252_export_keeps_its_accents() {
    let bytes = b"Ville,Population\nGen\xE8ve,203856\n";
    let (encoding, dialect) = ss_csv::sniff(bytes);
    assert_eq!(encoding, Encoding::Windows1252);
    let text = encoding.decode(bytes);
    let book = import_with(&text, Encoding::Utf8, dialect);
    let sheet = &book.sheets[0];
    let name = match value(sheet, "A2") {
        CellValue::Text(id) => book.strings.resolve(id).to_string(),
        other => panic!("{other:?}"),
    };
    assert_eq!(name, "Genève");
}

#[test]
fn what_is_exported_imports_back_as_the_same_values() {
    let original = import("Item,Qty\nWidget,3\n\"Smith, John\",12\n\"say \"\"hi\"\"\",1\n");
    let sheet = &original.sheets[0];

    let mut out = Vec::new();
    ss_csv::write_sheet(&mut out, sheet, Dialect::default(), |cell| {
        let Some(found) = sheet.get(cell) else {
            return String::new();
        };
        match found.value {
            CellValue::Blank => String::new(),
            CellValue::Number(n) => ss_model::format_general(n),
            CellValue::Bool(b) => if b { "TRUE" } else { "FALSE" }.to_string(),
            CellValue::Error(e) => e.as_str().to_string(),
            CellValue::Text(id) => original.strings.resolve(id).to_string(),
        }
    })
    .expect("writes");

    let text = String::from_utf8(out).expect("utf-8");
    let again = import(&text);
    let round = &again.sheets[0];
    assert_eq!(value(round, "B2"), CellValue::Number(3.0));
    for a1 in ["A3", "A4"] {
        let before = match value(sheet, a1) {
            CellValue::Text(id) => original.strings.resolve(id).to_string(),
            other => panic!("{other:?}"),
        };
        let after = match value(round, a1) {
            CellValue::Text(id) => again.strings.resolve(id).to_string(),
            other => panic!("{other:?}"),
        };
        assert_eq!(before, after, "{a1}");
    }
}
