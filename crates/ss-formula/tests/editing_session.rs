//! A spreadsheet session, start to finish.
//!
//! The unit tests each check one verb. This checks that the verbs compose the
//! way a person uses them: type some numbers, total them, fill the total
//! sideways, insert a row in the middle, undo it, cut a block and paste it
//! somewhere else — with a recalculation after every step, because that is what
//! the application does and it is where stale values would show up.

use ss_formula::clip::{self, Clip};
use ss_formula::edit::{self, Change};
use ss_model::{Axis, CellRange, CellRef, CellValue, Shift, Workbook};

struct Session {
    book: Workbook,
    undo: Vec<Change>,
}

impl Session {
    fn new() -> Self {
        Session {
            book: Workbook::blank(),
            undo: Vec::new(),
        }
    }

    fn perform(&mut self, change: Change) {
        let undo = edit::apply(&mut self.book, change);
        self.undo.push(undo);
        ss_formula::recalculate(&mut self.book);
    }

    fn type_into(&mut self, a1: &str, text: &str) {
        let at = at(a1);
        let change = edit::input(&mut self.book, 0, at, text);
        self.perform(change);
    }

    fn undo(&mut self) {
        if let Some(change) = self.undo.pop() {
            edit::apply(&mut self.book, change);
            ss_formula::recalculate(&mut self.book);
        }
    }

    /// What the cell would show, as a string.
    fn shown(&self, a1: &str) -> String {
        match self.book.sheets[0].get(at(a1)).map(|c| c.value) {
            Some(CellValue::Number(n)) => ss_model::format_general(n),
            Some(CellValue::Text(id)) => self.book.strings.resolve(id).to_string(),
            Some(CellValue::Bool(b)) => if b { "TRUE" } else { "FALSE" }.to_string(),
            Some(CellValue::Error(e)) => e.as_str().to_string(),
            _ => String::new(),
        }
    }

    fn formula(&self, a1: &str) -> String {
        self.book.sheets[0]
            .formula_at(at(a1))
            .map(|f| format!("={}", f.text))
            .unwrap_or_default()
    }
}

fn at(a1: &str) -> CellRef {
    CellRef::from_a1(a1).expect("valid address")
}

fn range(a: &str, b: &str) -> CellRange {
    CellRange::new(at(a), at(b))
}

#[test]
fn an_afternoon_of_spreadsheet_work() {
    let mut s = Session::new();

    // Three quarters of revenue, and a total under each.
    for (cell, value) in [("A1", "100"), ("A2", "200"), ("A3", "300")] {
        s.type_into(cell, value);
    }
    s.type_into("A4", "=SUM(A1:A3)");
    assert_eq!(s.shown("A4"), "600");

    // Two more columns of the same shape, by filling the formula sideways.
    for (cell, value) in [
        ("B1", "110"),
        ("B2", "210"),
        ("B3", "310"),
        ("C1", "120"),
        ("C2", "220"),
        ("C3", "320"),
    ] {
        s.type_into(cell, value);
    }
    let change = clip::fill(&mut s.book, 0, range("A4", "A4"), range("A4", "C4"));
    s.perform(change);
    assert_eq!(s.formula("C4"), "=SUM(C1:C3)");
    assert_eq!(s.shown("C4"), "660");

    // A row inserted in the middle has to widen the totals, not break them.
    let change = edit::structural(&s.book, 0, Shift::insert(Axis::Rows, 1, 1));
    s.perform(change);
    assert_eq!(s.formula("A5"), "=SUM(A1:A4)");
    assert_eq!(
        s.shown("A5"),
        "600",
        "the new row is empty, so nothing moved"
    );

    s.type_into("A2", "50");
    assert_eq!(s.shown("A5"), "650");

    // ...and undoing it puts the sheet back exactly, formulas included.
    s.undo(); // the typed 50
    s.undo(); // the inserted row
    assert_eq!(s.formula("A4"), "=SUM(A1:A3)");
    assert_eq!(s.shown("A4"), "600");
    assert_eq!(s.shown("A2"), "200");

    // Cut the whole block and paste it three rows down. The formulas move with
    // it and keep pointing at the numbers they were always about.
    let taken = clip::copy(&s.book, 0, range("A1", "C4")).expect("copied");
    let mut change = clip::paste(&mut s.book, 0, range("A7", "A7"), &taken);
    let cleared = edit::clear_contents(&s.book, 0, &[range("A1", "C4")]);
    change.patches.splice(0..0, cleared.patches);
    s.perform(change);

    assert_eq!(s.shown("A1"), "", "the cut emptied where it came from");
    assert_eq!(s.formula("A10"), "=SUM(A7:A9)");
    assert_eq!(s.shown("A10"), "600");

    // One undo takes the move back as a whole, both halves of it.
    s.undo();
    assert_eq!(s.shown("A1"), "100");
    assert_eq!(s.shown("A10"), "");
    assert_eq!(s.shown("A4"), "600");
}

#[test]
fn deleting_the_rows_a_formula_reads_leaves_a_ref_error_not_a_wrong_number() {
    let mut s = Session::new();
    s.type_into("A1", "10");
    s.type_into("A2", "20");
    s.type_into("B4", "=A1+A2");
    assert_eq!(s.shown("B4"), "30");

    let change = edit::structural(&s.book, 0, Shift::delete(Axis::Rows, 0, 1));
    s.perform(change);

    // A1 is gone, and the formula moved up to B3 with everything else. It says
    // so rather than quietly totalling whatever slid into A1's place.
    assert_eq!(s.formula("B3"), "=#REF!+A1");
    assert_eq!(s.shown("B3"), "#REF!");

    s.undo();
    assert_eq!(s.formula("B4"), "=A1+A2");
    assert_eq!(s.shown("B4"), "30");
}

#[test]
fn text_pasted_from_another_program_is_read_the_way_it_would_be_typed() {
    let mut s = Session::new();
    // What Excel puts on the clipboard for a 2x2 block.
    let clip = Clip::from_tsv("5\t2024-01-15\n=1+1\tTRUE\n", at("A1"));
    let change = clip::paste(&mut s.book, 0, range("A1", "A1"), &clip);
    s.perform(change);

    assert_eq!(
        s.book.sheets[0].get(at("A1")).map(|c| c.value),
        Some(CellValue::Number(5.0)),
        "a number, not the text \"5\""
    );
    assert_eq!(s.shown("B1"), "45306", "a date, stored as its serial");
    assert_eq!(s.shown("A2"), "2", "a formula, evaluated");
    assert_eq!(
        s.book.sheets[0].get(at("B2")).map(|c| c.value),
        Some(CellValue::Bool(true))
    );
}
