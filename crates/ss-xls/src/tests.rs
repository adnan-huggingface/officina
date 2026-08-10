//! Reading a workbook laid out from the specification.
//!
//! There is no `.xls` in this repository and there will not be one: the
//! corpus is generated locally rather than downloaded, and Excel on this
//! machine will not run. So the fixtures here are built byte by byte from the
//! record layouts, and read back by code that shares nothing with the builder
//! but the layout itself. That catches a reader that misplaces a field. It
//! cannot catch a layout this file and the reader are both wrong about, and the
//! places where that risk is real are named in the module docs.

use ss_model::{CellRef, CellValue};

use super::*;

/// Builds a `Workbook` stream, and a container to put it in.
#[derive(Default)]
struct Book {
    globals: Vec<u8>,
    sheets: Vec<Tab>,
}

/// One sheet: its name, the `BOUNDSHEET` flags, and the records between its
/// substream's own BOF and EOF.
struct Tab {
    name: String,
    flags: u16,
    body: Vec<u8>,
}

impl Book {
    fn new() -> Book {
        let mut book = Book::default();
        // One font and sixteen formats, which is roughly what the smallest
        // workbook Excel writes has. Cells below use format 15.
        book.globals
            .extend(record(kind::FONT, &font("Arial", false)));
        for _ in 0..16 {
            book.globals.extend(record(kind::XF, &xf(0, 0)));
        }
        book
    }

    fn record(mut self, kind: u16, body: &[u8]) -> Book {
        self.globals.extend(record(kind, body));
        self
    }

    fn strings(mut self, strings: &[&str]) -> Book {
        let mut body = (strings.len() as u32).to_le_bytes().to_vec();
        body.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for text in strings {
            body.extend_from_slice(&(text.chars().count() as u16).to_le_bytes());
            body.push(0);
            body.extend(text.chars().map(|c| c as u8));
        }
        self.globals.extend(record(kind::SST, &body));
        self
    }

    fn sheet(self, name: &str, body: Vec<u8>) -> Book {
        self.tab(name, 0x0000, body)
    }

    fn hidden(self, name: &str, body: Vec<u8>) -> Book {
        self.tab(name, 0x0001, body)
    }

    fn chart(self, name: &str) -> Book {
        self.tab(name, 0x0200, Vec::new())
    }

    fn tab(mut self, name: &str, flags: u16, body: Vec<u8>) -> Book {
        self.sheets.push(Tab {
            name: name.to_string(),
            flags,
            body,
        });
        self
    }

    fn stream(self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(record(kind::BOF, &[0x00, 0x06, 0x05, 0x00, 0, 0, 0, 0]));
        out.extend_from_slice(&self.globals);

        // BOUNDSHEET carries the absolute offset of its sheet's substream, so
        // the entries go in with a placeholder and are patched once the
        // globals — and therefore the position of the first sheet — are known.
        let mut patch = Vec::new();
        for tab in &self.sheets {
            let mut body = vec![0u8; 4];
            body.extend_from_slice(&tab.flags.to_le_bytes());
            body.push(tab.name.chars().count() as u8);
            body.push(0);
            body.extend(tab.name.chars().map(|c| c as u8));
            patch.push(out.len() + 4);
            out.extend(record(kind::BOUNDSHEET, &body));
        }
        out.extend(record(kind::EOF, &[]));

        for (tab, at) in self.sheets.iter().zip(patch) {
            let start = out.len() as u32;
            out[at..at + 4].copy_from_slice(&start.to_le_bytes());
            let substream = if tab.flags & 0xFF00 == 0x0200 {
                0x0020
            } else {
                0x0010
            };
            out.extend(record(
                kind::BOF,
                &[0x00, 0x06, substream as u8, 0x00, 0, 0, 0, 0],
            ));
            out.extend_from_slice(&tab.body);
            out.extend(record(kind::EOF, &[]));
        }
        out
    }

    fn file(self) -> Vec<u8> {
        cfb_reader::fixture::Builder::new()
            .stream("Workbook", self.stream())
            .build()
    }

    fn read(self) -> XlsDocument {
        from_stream(&self.stream()).expect("the workbook reads")
    }
}

use crate::record::kind;

fn record(kind: u16, body: &[u8]) -> Vec<u8> {
    let mut out = kind.to_le_bytes().to_vec();
    out.extend_from_slice(&(body.len() as u16).to_le_bytes());
    out.extend_from_slice(body);
    out
}

fn font(name: &str, bold: bool) -> Vec<u8> {
    let mut out = 200u16.to_le_bytes().to_vec(); // ten points, in twips
    out.extend_from_slice(&0u16.to_le_bytes()); // flags
    out.extend_from_slice(&0x7FFFu16.to_le_bytes()); // automatic colour
    out.extend_from_slice(&if bold { 700u16 } else { 400 }.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // script, underline, family, charset, reserved
    out.push(name.len() as u8);
    out.push(0);
    out.extend(name.bytes());
    out
}

/// A minimal XF: a font, a number format, and nothing else set.
fn xf(font: u16, format: u16) -> Vec<u8> {
    let mut out = font.to_le_bytes().to_vec();
    out.extend_from_slice(&format.to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    out
}

fn cell(kind_of: u16, row: u16, col: u16, rest: &[u8]) -> Vec<u8> {
    let mut body = row.to_le_bytes().to_vec();
    body.extend_from_slice(&col.to_le_bytes());
    body.extend_from_slice(&15u16.to_le_bytes());
    body.extend_from_slice(rest);
    record(kind_of, &body)
}

fn number(row: u16, col: u16, value: f64) -> Vec<u8> {
    cell(kind::NUMBER, row, col, &value.to_le_bytes())
}

fn label(row: u16, col: u16, index: u32) -> Vec<u8> {
    cell(kind::LABELSST, row, col, &index.to_le_bytes())
}

/// A FORMULA record with a cached double and an expression.
fn formula(row: u16, col: u16, cached: f64, rgce: &[u8]) -> Vec<u8> {
    let mut rest = cached.to_le_bytes().to_vec();
    rest.extend_from_slice(&0u16.to_le_bytes()); // grbit
    rest.extend_from_slice(&0u32.to_le_bytes()); // chn
    rest.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
    rest.extend_from_slice(rgce);
    cell(kind::FORMULA, row, col, &rest)
}

fn value_at(doc: &XlsDocument, sheet: usize, a1: &str) -> CellValue {
    let at = CellRef::from_a1(a1).expect("an address");
    doc.workbook.sheets[sheet]
        .get(at)
        .map(|c| c.value)
        .unwrap_or(CellValue::Blank)
}

fn text_at(doc: &XlsDocument, sheet: usize, a1: &str) -> String {
    match value_at(doc, sheet, a1) {
        CellValue::Text(id) => doc.workbook.strings.resolve(id).to_string(),
        other => panic!("{a1} is {other:?}, not text"),
    }
}

#[test]
fn a_workbook_comes_back_with_its_sheets_in_order() {
    let doc = Book::new()
        .sheet("First", number(0, 0, 1.0))
        .sheet("Second", Vec::new())
        .sheet("Third", Vec::new())
        .read();
    let names: Vec<&str> = doc
        .workbook
        .sheets
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(names, vec!["First", "Second", "Third"]);
}

#[test]
fn numbers_and_strings_land_in_the_right_cells() {
    let mut body = number(0, 0, 42.5);
    body.extend(label(1, 2, 1));
    let doc = Book::new()
        .strings(&["unused", "hello"])
        .sheet("Sheet1", body)
        .read();
    assert_eq!(value_at(&doc, 0, "A1"), CellValue::Number(42.5));
    assert_eq!(text_at(&doc, 0, "C2"), "hello");
}

#[test]
fn an_rk_number_is_decoded_all_four_ways() {
    // The two flag bits: a truncated double or a 30-bit integer, each with or
    // without the divide by a hundred.
    let mut body = Vec::new();
    let cases: [(u32, f64); 4] = [
        (
            f64::to_bits(1234.5).wrapping_shr(32) as u32 & 0xFFFF_FFFC,
            1234.5,
        ),
        (
            (f64::to_bits(1234.5).wrapping_shr(32) as u32 & 0xFFFF_FFFC) | 1,
            12.345,
        ),
        ((100i32 << 2) as u32 | 2, 100.0),
        ((-25i32 << 2) as u32 | 2, -25.0),
    ];
    for (i, (bits, _)) in cases.iter().enumerate() {
        body.extend(cell(kind::RK, i as u16, 0, &bits.to_le_bytes()));
    }
    let doc = Book::new().sheet("Sheet1", body).read();
    for (i, (_, expected)) in cases.iter().enumerate() {
        let at = format!("A{}", i + 1);
        assert_eq!(
            value_at(&doc, 0, &at),
            CellValue::Number(*expected),
            "at {at}"
        );
    }
}

#[test]
fn a_negative_rk_integer_does_not_come_back_as_a_billion() {
    // The integer form is signed, and a logical shift turns every negative
    // number in a file into a large positive one.
    assert_eq!(crate::record::rk((-1i32 << 2) as u32 | 2), -1.0);
}

#[test]
fn one_mulrk_record_fills_a_run_of_cells() {
    let mut body = 4u16.to_le_bytes().to_vec(); // row 5
    body.extend_from_slice(&1u16.to_le_bytes()); // from column B
    for value in [10i32, 20, 30] {
        body.extend_from_slice(&15u16.to_le_bytes());
        body.extend_from_slice(&(((value << 2) as u32) | 2).to_le_bytes());
    }
    body.extend_from_slice(&3u16.to_le_bytes()); // to column D
    let doc = Book::new()
        .sheet("Sheet1", record(kind::MULRK, &body))
        .read();
    assert_eq!(value_at(&doc, 0, "B5"), CellValue::Number(10.0));
    assert_eq!(value_at(&doc, 0, "C5"), CellValue::Number(20.0));
    assert_eq!(value_at(&doc, 0, "D5"), CellValue::Number(30.0));
}

#[test]
fn a_formula_keeps_both_its_text_and_the_value_excel_cached() {
    // =SUM(A1:A2), cached as 3.
    let mut rgce = vec![0x25];
    rgce.extend_from_slice(&[0, 0, 1, 0, 0x00, 0xC0, 0x00, 0xC0]);
    rgce.extend_from_slice(&[0x22, 1, 4, 0]);
    let doc = Book::new()
        .sheet("Sheet1", formula(4, 0, 3.0, &rgce))
        .read();
    let at = CellRef::from_a1("A5").expect("A5");
    assert_eq!(value_at(&doc, 0, "A5"), CellValue::Number(3.0));
    assert_eq!(
        doc.workbook.sheets[0]
            .formula_at(at)
            .map(|f| f.text.as_str()),
        Some("SUM(A1:A2)")
    );
}

#[test]
fn a_formula_this_reader_cannot_decompile_still_shows_its_value() {
    // ptgArray, whose constant is not in the token stream at all.
    let doc = Book::new()
        .sheet("Sheet1", formula(0, 0, 7.0, &[0x20, 0, 0, 0, 0, 0, 0, 0]))
        .read();
    let at = CellRef::from_a1("A1").expect("A1");
    assert_eq!(value_at(&doc, 0, "A1"), CellValue::Number(7.0));
    assert!(
        doc.workbook.sheets[0].formula_at(at).is_none(),
        "no formula is better than the wrong formula"
    );
}

#[test]
fn a_formula_whose_answer_is_text_takes_it_from_the_record_that_follows() {
    // The cached "value" is a sentinel: the first byte says the answer is text
    // and the last two say the eight bytes are not a double.
    let mut rgce = vec![0x17, 2, 0];
    rgce.extend_from_slice(b"hi");
    let mut rest = vec![0u8, 0, 0, 0, 0, 0, 0xFF, 0xFF];
    rest.extend_from_slice(&0u16.to_le_bytes());
    rest.extend_from_slice(&0u32.to_le_bytes());
    rest.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
    rest.extend_from_slice(&rgce);
    let mut body = cell(kind::FORMULA, 0, 0, &rest);

    let mut string = 5u16.to_le_bytes().to_vec();
    string.push(0);
    string.extend_from_slice(b"there");
    body.extend(record(kind::STRING, &string));

    let doc = Book::new().sheet("Sheet1", body).read();
    assert_eq!(text_at(&doc, 0, "A1"), "there");
}

#[test]
fn booleans_and_errors_are_not_numbers() {
    let mut body = cell(kind::BOOLERR, 0, 0, &[1, 0]);
    body.extend(cell(kind::BOOLERR, 1, 0, &[0x07, 1]));
    let doc = Book::new().sheet("Sheet1", body).read();
    assert_eq!(value_at(&doc, 0, "A1"), CellValue::Bool(true));
    assert_eq!(
        value_at(&doc, 0, "A2"),
        CellValue::Error(ss_model::CellError::Div0)
    );
}

#[test]
fn a_shared_formula_reaches_every_cell_that_uses_it() {
    // Three cells whose FORMULA records point at one SHRFMLA holding =A1*2
    // with a relative reference.
    let mut body = Vec::new();
    for row in 0..3u16 {
        let mut rgce = vec![0x01];
        rgce.extend_from_slice(&0u16.to_le_bytes());
        rgce.extend_from_slice(&1u16.to_le_bytes());
        let mut rest = (row as f64 * 2.0).to_le_bytes().to_vec();
        rest.extend_from_slice(&0u16.to_le_bytes());
        rest.extend_from_slice(&0u32.to_le_bytes());
        rest.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
        rest.extend_from_slice(&rgce);
        body.extend(cell(kind::FORMULA, row, 1, &rest));
        if row == 0 {
            // ptgRefN at (0,-1) then ptgInt 2 then multiply.
            let mut shared = vec![0x2C];
            shared.extend_from_slice(&0u16.to_le_bytes());
            shared.extend_from_slice(&(0xC000u16 | 0x3FFF).to_le_bytes());
            shared.extend_from_slice(&[0x1E, 2, 0, 0x05]);
            let mut head = vec![0u8, 0, 2, 0, 1, 1, 0, 0];
            head.extend_from_slice(&(shared.len() as u16).to_le_bytes());
            head.extend_from_slice(&shared);
            body.extend(record(kind::SHRFMLA, &head));
        }
    }

    let doc = Book::new().sheet("Sheet1", body).read();
    let sheet = &doc.workbook.sheets[0];
    let master = sheet
        .formula_at(CellRef::from_a1("B1").expect("B1"))
        .expect("the master carries the text");
    assert_eq!(master.text, "A1*2");
    for a1 in ["B2", "B3"] {
        let follower = sheet
            .formula_at(CellRef::from_a1(a1).expect("an address"))
            .expect("a follower");
        assert!(
            follower.borrows_text(),
            "{a1} should take its text from the master rather than repeat it"
        );
    }
}

#[test]
fn merges_column_widths_row_heights_and_the_freeze_all_survive() {
    let mut body = Vec::new();

    let mut merged = 1u16.to_le_bytes().to_vec();
    merged.extend_from_slice(&[0, 0, 2, 0, 1, 0, 3, 0]); // A2:D3 in row/col order
    body.extend(record(kind::MERGEDCELLS, &merged));

    // COLINFO: columns B to C, 12.5 characters wide.
    let mut col = 1u16.to_le_bytes().to_vec();
    col.extend_from_slice(&2u16.to_le_bytes());
    col.extend_from_slice(&((12.5 * 256.0) as u16).to_le_bytes());
    col.extend_from_slice(&15u16.to_le_bytes());
    col.extend_from_slice(&[0, 0, 0, 0]);
    body.extend(record(kind::COLINFO, &col));

    // ROW 4 at 30 points, marked as a height the user set.
    let mut row = 3u16.to_le_bytes().to_vec();
    row.extend_from_slice(&[0, 0, 0, 0]);
    row.extend_from_slice(&600u16.to_le_bytes());
    row.extend_from_slice(&[0, 0, 0, 0]);
    row.extend_from_slice(&0x0040u32.to_le_bytes());
    body.extend(record(kind::ROW, &row));

    // WINDOW2 with the frozen bit, then PANE saying two rows and one column.
    let mut window = 0x0008u16.to_le_bytes().to_vec();
    window.extend_from_slice(&[0, 0, 0, 0]);
    window.extend_from_slice(&[0; 12]);
    body.extend(record(kind::WINDOW2, &window));
    let mut pane = 1u16.to_le_bytes().to_vec();
    pane.extend_from_slice(&2u16.to_le_bytes());
    pane.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    body.extend(record(kind::PANE, &pane));

    let doc = Book::new().sheet("Sheet1", body).read();
    let sheet = &doc.workbook.sheets[0];
    assert_eq!(sheet.merges.len(), 1);
    assert_eq!(sheet.merges[0].start, CellRef::from_a1("B1").expect("B1"));
    assert_eq!(sheet.merges[0].end, CellRef::from_a1("D3").expect("D3"));
    assert_eq!(sheet.column_widths.get(&1), Some(&12.5));
    assert_eq!(sheet.column_widths.get(&2), Some(&12.5));
    assert_eq!(sheet.row_heights.get(&3), Some(&30.0));
    assert_eq!(sheet.frozen, Some(CellRef::new(2, 1)));
}

#[test]
fn a_gridless_sheet_is_switched_off_rather_than_assumed_on() {
    let mut window = 0u16.to_le_bytes().to_vec(); // neither gridlines nor headings
    window.extend_from_slice(&[0; 16]);
    let doc = Book::new()
        .sheet("Sheet1", record(kind::WINDOW2, &window))
        .read();
    assert!(!doc.workbook.sheets[0].view.gridlines);
    assert!(!doc.workbook.sheets[0].view.headings);
}

#[test]
fn the_font_a_cell_uses_skips_the_index_that_does_not_exist() {
    // Fonts 0-3 then 5: an XF saying `ifnt = 5` means the fifth record, not the
    // sixth slot. Read as a plain index it lands on the one before.
    let mut book = Book::default();
    for i in 0..5 {
        book.globals
            .extend(record(kind::FONT, &font("Arial", i == 4)));
    }
    for _ in 0..15 {
        book.globals.extend(record(kind::XF, &xf(0, 0)));
    }
    book.globals.extend(record(kind::XF, &xf(5, 0))); // index 15
    let doc = book.sheet("Sheet1", number(0, 0, 1.0)).read();

    let style = doc.workbook.sheets[0].style_at(CellRef::new(0, 0));
    assert!(
        doc.workbook.styles.font(style).bold,
        "ifnt 5 is the fifth FONT record, which is the bold one"
    );
}

#[test]
fn a_palette_the_workbook_overrode_is_used_instead_of_the_standard_one() {
    let mut palette = 2u16.to_le_bytes().to_vec();
    palette.extend_from_slice(&[0x11, 0x22, 0x33, 0x00, 0x44, 0x55, 0x66, 0x00]);
    let mut book = Book::default();
    book.globals.extend(record(kind::PALETTE, &palette));
    book.globals
        .extend(record(kind::FONT, &font("Arial", false)));
    for _ in 0..15 {
        book.globals.extend(record(kind::XF, &xf(0, 0)));
    }
    // An XF whose foreground colour is index 9, the second palette entry.
    let mut styled = xf(0, 0);
    styled[18] = 9;
    styled[15] = 0x04; // a solid pattern, in the top six bits of the second word
    book.globals.extend(record(kind::XF, &styled));

    let doc = book.sheet("Sheet1", number(0, 0, 1.0)).read();
    let style = doc.workbook.sheets[0].style_at(CellRef::new(0, 0));
    let fill = doc.workbook.styles.fill(style);
    assert_eq!(fill.fg, ss_model::Color::rgb(0x44, 0x55, 0x66));
}

#[test]
fn a_whole_file_reads_through_its_container() {
    let file = Book::new()
        .strings(&["greetings"])
        .sheet("Sheet1", label(0, 0, 0))
        .file();
    let doc = read(file).expect("the file reads");
    assert_eq!(text_at(&doc, 0, "A1"), "greetings");
}

#[test]
fn an_xlsx_handed_to_this_reader_is_told_what_it_is() {
    let err = read(b"PK\x03\x04 this is a zip".to_vec()).expect_err("refused");
    assert!(matches!(err, Error::Container(_)), "{err}");
}

#[test]
fn an_excel_five_workbook_says_so_rather_than_reporting_damage() {
    let mut stream = record(kind::BOF, &[0x00, 0x05, 0x05, 0x00, 0, 0, 0, 0]);
    stream.extend(record(kind::EOF, &[]));
    let err = from_stream(&stream).expect_err("refused");
    assert!(matches!(err, Error::OldVersion(0x0500)), "{err}");
    assert!(err.to_string().contains("Excel 5"), "{err}");
}

#[test]
fn a_password_protected_workbook_says_so_rather_than_showing_noise() {
    let mut stream = record(kind::BOF, &[0x00, 0x06, 0x05, 0x00, 0, 0, 0, 0]);
    stream.extend(record(kind::FILEPASS, &[0, 0]));
    stream.extend(record(kind::EOF, &[]));
    let err = from_stream(&stream).expect_err("refused");
    assert!(matches!(err, Error::Encrypted), "{err}");
}

#[test]
fn a_hidden_sheet_keeps_its_place_in_the_list() {
    // Dropping it would be worse than hiding it wrongly: a sheet-scoped name
    // carries an index into this list, so every name after the gap would point
    // at the wrong sheet.
    let doc = Book::new()
        .sheet("Visible", Vec::new())
        .hidden("Hidden", Vec::new())
        .sheet("After", Vec::new())
        .read();
    let hidden: Vec<bool> = doc.workbook.sheets.iter().map(|s| s.hidden).collect();
    assert_eq!(hidden, vec![false, true, false]);
    assert_eq!(doc.workbook.sheets[2].name, "After");
}

#[test]
fn the_macintosh_date_system_is_reported_rather_than_assumed_away() {
    // Every serial in a 1904 workbook is 1462 days from the same date in a
    // 1900 one, so the flag has to travel with the values or every date in the
    // file is four years and a day out.
    let plain = Book::new().sheet("Sheet1", Vec::new()).read();
    assert!(!plain.date_1904);

    let mac = Book::new()
        .record(kind::DATEMODE, &[1, 0])
        .sheet("Sheet1", Vec::new())
        .read();
    assert!(mac.date_1904);
}

#[test]
fn a_chart_sheet_is_a_tab_with_no_grid() {
    let doc = Book::new()
        .sheet("Data", number(0, 0, 1.0))
        .chart("Chart1")
        .read();
    assert_eq!(doc.workbook.sheets[1].kind, ss_model::SheetKind::Chart);
    assert!(!doc.workbook.sheets[1].kind.has_grid());
}
