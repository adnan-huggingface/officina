//! Row and column grouping, read, changed, and written back.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use ss_xlsx::XlsxDocument;

fn corpus() -> Option<PathBuf> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xlsx/merged-frozen-grouped.xlsx");
    path.exists().then_some(path)
}

#[test]
fn a_grouped_workbook_reads_its_levels_and_keeps_them() {
    let Some(path) = corpus() else { return };
    let doc = XlsxDocument::open(&path).expect("opens");
    let sheet = &doc.workbook.sheets[0];
    assert!(
        !sheet.row_outlines.is_empty(),
        "the corpus file has grouped rows"
    );
    assert!(sheet.row_outlines.values().all(|l| *l == 1));
    assert!(
        !sheet.column_outlines.is_empty(),
        "and grouped columns (B:C)"
    );
}

#[test]
fn grouping_more_rows_survives_a_save_and_ungrouping_empties_it() {
    let Some(path) = corpus() else { return };
    let mut doc = XlsxDocument::open(&path).expect("opens");

    // Deepen: a second level inside the existing group, and a brand-new
    // group further down, collapsed with its summary row marked.
    {
        let sheet = &mut doc.workbook.sheets[0];
        sheet.row_outlines.insert(5, 2);
        sheet.row_outlines.insert(6, 2);
        sheet.row_outlines.insert(20, 1);
        sheet.row_outlines.insert(21, 1);
        sheet.row_collapsed.insert(22);
        sheet.row_heights.insert(20, 0.0);
        sheet.row_heights.insert(21, 0.0);
    }
    let rows = doc.workbook.sheets[0].row_outlines.clone();
    let collapsed = doc.workbook.sheets[0].row_collapsed.clone();

    let mut bytes = Vec::new();
    doc.write_to(Cursor::new(&mut bytes)).expect("writes");
    let reopened = XlsxDocument::read(Cursor::new(bytes)).expect("reads back");
    let sheet = &reopened.workbook.sheets[0];
    assert_eq!(sheet.row_outlines, rows);
    assert_eq!(sheet.row_collapsed, collapsed);
    assert_eq!(
        sheet.row_heights.get(&20).copied(),
        Some(0.0),
        "the collapsed rows stayed hidden"
    );

    // Ungroup everything; the attributes must actually leave the file.
    let mut doc = reopened;
    {
        let sheet = &mut doc.workbook.sheets[0];
        sheet.row_outlines.clear();
        sheet.row_collapsed.clear();
        sheet.column_outlines.clear();
        sheet.column_collapsed.clear();
    }
    let mut bytes = Vec::new();
    doc.write_to(Cursor::new(&mut bytes)).expect("writes");
    let reopened = XlsxDocument::read(Cursor::new(bytes)).expect("reads back");
    let sheet = &reopened.workbook.sheets[0];
    assert!(sheet.row_outlines.is_empty(), "ungrouped rows stay so");
    assert!(sheet.column_outlines.is_empty(), "and columns");
    assert!(sheet.row_collapsed.is_empty());
}
