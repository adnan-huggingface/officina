//! Dynamic arrays, which are about *where* the answer goes.
//!
//! A formula whose result is a rectangle writes into the cells around it. That
//! is a property of the recalculation loop rather than of any function, so it
//! is tested against a real workbook rather than through the evaluator.

use ss_formula::{edit, recalculate};
use ss_model::{Cell, CellError, CellRef, CellValue, Workbook};

fn at(a1: &str) -> CellRef {
    CellRef::from_a1(a1).expect("valid address")
}

fn book_with(values: &[(&str, f64)]) -> Workbook {
    let mut book = Workbook::blank();
    for (a1, value) in values {
        book.sheets[0].set(
            at(a1),
            Cell {
                value: CellValue::Number(*value),
                ..Default::default()
            },
        );
    }
    book
}

fn type_in(book: &mut Workbook, a1: &str, text: &str) {
    let change = edit::input(book, 0, at(a1), text);
    edit::apply(book, change);
    recalculate(book);
}

fn value(book: &Workbook, a1: &str) -> CellValue {
    book.sheets[0]
        .get(at(a1))
        .map(|c| c.value)
        .unwrap_or(CellValue::Blank)
}

#[test]
fn a_dynamic_array_writes_into_the_cells_below_it() {
    let mut book = book_with(&[("A1", 3.0), ("A2", 1.0), ("A3", 3.0), ("A4", 2.0)]);
    type_in(&mut book, "C1", "=UNIQUE(A1:A4)");

    assert_eq!(value(&book, "C1"), CellValue::Number(3.0));
    assert_eq!(value(&book, "C2"), CellValue::Number(1.0));
    assert_eq!(value(&book, "C3"), CellValue::Number(2.0));
    assert_eq!(
        value(&book, "C4"),
        CellValue::Blank,
        "three distinct values"
    );
    assert!(
        book.sheets[0]
            .get(at("C2"))
            .is_none_or(|c| c.formula.is_none()),
        "a spilled cell holds a value, not a copy of the formula"
    );
}

#[test]
fn a_legacy_range_formula_still_intersects_rather_than_spilling() {
    // `=A1:A4` in row 2 is A2, not a spill. Every formula written before 2019
    // depends on that, and treating any array result as a spill would rewrite
    // what they all mean.
    let mut book = book_with(&[("A1", 10.0), ("A2", 20.0), ("A3", 30.0), ("A4", 40.0)]);
    type_in(&mut book, "C2", "=A1:A4");

    assert_eq!(value(&book, "C2"), CellValue::Number(20.0));
    assert_eq!(value(&book, "C3"), CellValue::Blank, "nothing spilled");
}

#[test]
fn a_result_that_shrinks_clears_what_it_used_to_fill() {
    // The reason the sheet remembers where a spill went. A FILTER that goes
    // from four matches to two would otherwise leave two stale rows behind,
    // which reads as data rather than as a leftover.
    let mut book = book_with(&[("A1", 5.0), ("A2", 6.0), ("A3", 7.0), ("A4", 8.0)]);
    book.sheets[0].set(
        at("B1"),
        Cell {
            value: CellValue::Number(0.0),
            ..Default::default()
        },
    );
    type_in(&mut book, "C1", "=FILTER(A1:A4,A1:A4>B1)");
    assert_eq!(value(&book, "C4"), CellValue::Number(8.0));

    type_in(&mut book, "B1", "6");
    assert_eq!(value(&book, "C1"), CellValue::Number(7.0));
    assert_eq!(value(&book, "C2"), CellValue::Number(8.0));
    assert_eq!(value(&book, "C3"), CellValue::Blank);
    assert_eq!(value(&book, "C4"), CellValue::Blank, "the tail was cleared");
}

#[test]
fn something_in_the_way_is_reported_and_not_overwritten() {
    let mut book = book_with(&[("A1", 3.0), ("A2", 1.0), ("A3", 2.0)]);
    let obstruction = book.strings.intern("mine");
    book.sheets[0].set(
        at("C2"),
        Cell {
            value: CellValue::Text(obstruction),
            ..Default::default()
        },
    );
    type_in(&mut book, "C1", "=SORT(A1:A3)");

    assert_eq!(
        value(&book, "C1"),
        CellValue::Error(CellError::Spill),
        "the anchor says why"
    );
    assert_eq!(
        value(&book, "C2"),
        CellValue::Text(obstruction),
        "and the obstruction is untouched, which is the whole point"
    );
}

#[test]
fn a_spill_can_be_summed_from_somewhere_else() {
    let mut book = book_with(&[("A1", 1.0), ("A2", 2.0), ("A3", 3.0)]);
    type_in(&mut book, "C1", "=SEQUENCE(3,1,10,10)");
    type_in(&mut book, "E1", "=SUM(C1:C3)");
    assert_eq!(value(&book, "E1"), CellValue::Number(60.0));
}

#[test]
fn a_spilled_cell_keeps_the_formatting_that_was_there() {
    // A report spilling into a shaded block must not strip the shading: the
    // cell's value is ours to write and its style is not.
    let mut book = book_with(&[("A1", 2.0), ("A2", 1.0)]);
    let shaded = book.styles.restyle(ss_model::StyleId::DEFAULT, |look| {
        look.fill = ss_model::Fill::solid(ss_model::Color::rgb(0xFF, 0xEB, 0x9C))
    });
    book.sheets[0].set(
        at("C2"),
        Cell {
            value: CellValue::Blank,
            style: shaded,
            formula: None,
        },
    );
    type_in(&mut book, "C1", "=SORT(A1:A2)");

    assert_eq!(value(&book, "C2"), CellValue::Number(2.0));
    assert_eq!(
        book.sheets[0].get(at("C2")).map(|c| c.style),
        Some(shaded),
        "a formatted-but-empty cell is not an obstruction, and keeps its fill"
    );
}
