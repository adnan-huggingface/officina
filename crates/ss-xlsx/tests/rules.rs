//! Data-validation and conditional-formatting rules, authored and edited.
//!
//! The writer's contract has two halves and both are under test here: a
//! sheet whose rules the model still agrees with goes back byte-for-byte
//! (the fidelity harness owns that half), and a sheet whose rules were
//! *changed* is rewritten from the model — created, edited, or deleted —
//! and read back to the same model.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use ss_model::cond::{
    CfKind, CfOperator, CfRule, ConditionalFormat, DataValidation, DvKind, DvOperator,
};
use ss_model::{CellRange, CellRef, Color};
use ss_xlsx::XlsxDocument;

fn corpus(name: &str) -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../corpus/xlsx/{name}"));
    path.exists().then_some(path)
}

fn range(a: &str, b: &str) -> CellRange {
    CellRange::new(
        CellRef::from_a1(a).expect("address"),
        CellRef::from_a1(b).expect("address"),
    )
}

#[test]
fn authored_rules_survive_a_save_and_come_back_the_same() {
    let Some(path) = corpus("minimal.xlsx") else {
        return;
    };
    let mut doc = XlsxDocument::open(&path).expect("opens");

    let dxf_id = doc.workbook.styles.add_dxf(ss_model::style::Dxf {
        bold: Some(true),
        fill: Some(ss_model::style::Fill::solid(Color::rgb(0xFF, 0xC7, 0xCE))),
        ..Default::default()
    });
    doc.workbook.sheets[0]
        .conditional_formats
        .push(ConditionalFormat {
            ranges: vec![range("A1", "A10")],
            rules: vec![CfRule {
                kind: CfKind::CellIs {
                    operator: CfOperator::GreaterThan,
                    formulas: vec!["100".into()],
                },
                dxf: Some(dxf_id),
                priority: 1,
                stop_if_true: false,
            }],
        });
    doc.workbook.sheets[0].validations.push(DataValidation {
        ranges: vec![range("B1", "B10")],
        kind: DvKind::Whole,
        operator: DvOperator::Between,
        formula1: "1".into(),
        formula2: "200".into(),
        allow_blank: true,
        error_title: "Out of range".into(),
        error_message: "1 to 200 only.".into(),
        ..Default::default()
    });
    let cf = doc.workbook.sheets[0].conditional_formats.clone();
    let dv = doc.workbook.sheets[0].validations.clone();

    let mut bytes = Vec::new();
    doc.write_to(Cursor::new(&mut bytes)).expect("writes");
    let reopened = XlsxDocument::read(Cursor::new(bytes)).expect("reads back");

    assert_eq!(reopened.workbook.sheets[0].conditional_formats, cf);
    assert_eq!(reopened.workbook.sheets[0].validations, dv);
    let dxf = reopened
        .workbook
        .styles
        .dxf(dxf_id)
        .expect("the dxf landed");
    assert_eq!(dxf.bold, Some(true));
    assert_eq!(
        dxf.fill
            .as_ref()
            .and_then(|f| f.shade(reopened.workbook.styles.theme())),
        Some([0xFF, 0xC7, 0xCE]),
        "the dxf fill is read back off bgColor"
    );
}

#[test]
fn deleting_every_rule_actually_empties_the_file() {
    // The old writer preserved these elements by never touching them, which
    // meant a deletion in the model changed nothing on disk.
    for (name, which) in [
        ("conditional-formatting.xlsx", "cf"),
        ("data-validation.xlsx", "dv"),
    ] {
        let Some(path) = corpus(name) else { continue };
        let mut doc = XlsxDocument::open(&path).expect("opens");
        let sheet = &mut doc.workbook.sheets[0];
        let had = match which {
            "cf" => !sheet.conditional_formats.is_empty(),
            _ => !sheet.validations.is_empty(),
        };
        assert!(had, "{name} carries rules to delete");
        sheet.conditional_formats.clear();
        sheet.validations.clear();

        let mut bytes = Vec::new();
        doc.write_to(Cursor::new(&mut bytes)).expect("writes");
        let reopened = XlsxDocument::read(Cursor::new(bytes)).expect("reads back");
        assert!(reopened.workbook.sheets[0].conditional_formats.is_empty());
        assert!(reopened.workbook.sheets[0].validations.is_empty());
    }
}

#[test]
fn an_edited_rule_is_rewritten_and_reads_back_edited() {
    let Some(path) = corpus("data-validation.xlsx") else {
        return;
    };
    let mut doc = XlsxDocument::open(&path).expect("opens");
    let sheet = &mut doc.workbook.sheets[0];
    assert!(!sheet.validations.is_empty());
    sheet.validations[0].error_title = "Changed".into();
    sheet.validations[0].error_message = "By the test.".into();
    let dv = sheet.validations.clone();

    let mut bytes = Vec::new();
    doc.write_to(Cursor::new(&mut bytes)).expect("writes");
    let reopened = XlsxDocument::read(Cursor::new(bytes)).expect("reads back");
    assert_eq!(reopened.workbook.sheets[0].validations, dv);
}
