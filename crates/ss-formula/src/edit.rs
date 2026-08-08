//! Editing a workbook, and being able to take it back.
//!
//! Every change here is a value, not a method call, and applying one returns the
//! change that undoes it. Undo is then not a second implementation of every
//! operation — the thing that got applied *is* the thing that gets un-applied,
//! so the two cannot drift apart. Redo falls out for free: applying the undo
//! returns the redo.
//!
//! Structural changes are why this lives in `ss-formula` rather than `ss-model`.
//! Inserting a row moves cells, which the model can do, and rewrites every
//! formula that pointed at them, which needs the lexer. Splitting those two
//! across a chunk boundary would leave a state where the data moved and the
//! formulas did not.
//!
//! What an undo entry costs is bounded by what actually changed: the cells a
//! deletion destroyed, the formulas whose text was rewritten, and one small
//! snapshot of merges, sizes, and the freeze. Nothing here clones a sheet.

use std::borrow::Cow;
use std::collections::BTreeMap;

use ss_model::{
    Axis, Cell, CellRange, CellRef, CellValue, Formula, FormulaId, Shift, StyleId, Workbook,
};

use crate::translate::translate;

/// Merges, sizes, and the freeze — everything positional a sheet holds that is
/// not a cell.
///
/// Snapshotted whole rather than patched, because it is small and because
/// getting a partial merge restore wrong is silent.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Geometry {
    pub merges: Vec<CellRange>,
    pub column_widths: BTreeMap<u32, f64>,
    pub row_heights: BTreeMap<u32, f64>,
    pub frozen: Option<CellRef>,
}

impl Geometry {
    pub fn of(sheet: &ss_model::Sheet) -> Self {
        Geometry {
            merges: sheet.merges.clone(),
            column_widths: sheet.column_widths.clone(),
            row_heights: sheet.row_heights.clone(),
            frozen: sheet.frozen,
        }
    }

    fn restore(self, sheet: &mut ss_model::Sheet) -> Geometry {
        let previous = Geometry::of(sheet);
        sheet.merges = self.merges;
        sheet.column_widths = self.column_widths;
        sheet.row_heights = self.row_heights;
        sheet.frozen = self.frozen;
        previous
    }
}

/// One indivisible piece of a change.
#[derive(Debug, Clone, PartialEq)]
pub enum Patch {
    /// Cells written to the given contents. `None` means "make it empty".
    Cells {
        sheet: usize,
        cells: Vec<(CellRef, Option<Cell>)>,
    },
    /// Formula text replaced in a sheet's arena.
    ///
    /// Text only: a structural shift moves an array formula's *range* itself,
    /// and writing back a whole `Formula` here would undo that.
    Formulas {
        sheet: usize,
        texts: Vec<(FormulaId, String)>,
    },
    Geometry {
        sheet: usize,
        geometry: Geometry,
    },
    /// Rows or columns inserted or deleted, moving everything positional.
    Shift {
        sheet: usize,
        shift: Shift,
    },
}

/// One user-visible action, as a list of patches applied in order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Change {
    pub label: String,
    pub patches: Vec<Patch>,
}

impl Change {
    pub fn new(label: impl Into<String>, patches: Vec<Patch>) -> Self {
        Change {
            label: label.into(),
            patches,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }
}

/// Applies `change` and returns the change that undoes it.
pub fn apply(book: &mut Workbook, change: Change) -> Change {
    let mut inverse: Vec<Patch> = Vec::new();
    for patch in change.patches {
        let undo = apply_patch(book, patch);
        // Each patch's undo goes in front of everything already collected, so
        // undo runs the patches backwards while keeping each group's own order.
        inverse.splice(0..0, undo);
    }
    Change {
        label: change.label,
        patches: inverse,
    }
}

fn apply_patch(book: &mut Workbook, patch: Patch) -> Vec<Patch> {
    match patch {
        Patch::Cells { sheet, cells } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            let mut before = Vec::with_capacity(cells.len());
            for (at, cell) in cells {
                before.push((at, target.get(at).copied()));
                target.set(at, cell.unwrap_or_default());
            }
            vec![Patch::Cells {
                sheet,
                cells: before,
            }]
        }

        Patch::Formulas { sheet, texts } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            let mut before = Vec::with_capacity(texts.len());
            for (id, text) in texts {
                let Some(formula) = target.formulas.get_mut(id.index() as usize) else {
                    continue;
                };
                before.push((id, std::mem::replace(&mut formula.text, text)));
            }
            vec![Patch::Formulas {
                sheet,
                texts: before,
            }]
        }

        Patch::Geometry { sheet, geometry } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            vec![Patch::Geometry {
                sheet,
                geometry: geometry.restore(target),
            }]
        }

        Patch::Shift { sheet, shift } => {
            let Some(target) = book.sheet_mut(sheet) else {
                return Vec::new();
            };
            let geometry = Geometry::of(target);
            let destroyed = target.shift(shift);
            // Undoing: move the grid back, put the destroyed cells where they
            // were, then restore the geometry exactly rather than trusting the
            // reverse shift to have reconstructed it.
            vec![
                Patch::Shift {
                    sheet,
                    shift: shift.inverse(),
                },
                Patch::Cells {
                    sheet,
                    cells: destroyed
                        .into_iter()
                        .map(|(at, cell)| (at, Some(cell)))
                        .collect(),
                },
                Patch::Geometry { sheet, geometry },
            ]
        }
    }
}

/// Inserting or deleting rows or columns, with every formula in the workbook
/// rewritten to keep meaning what it meant.
pub fn structural(book: &Workbook, sheet: usize, shift: Shift) -> Change {
    let Some(moved) = book.sheet(sheet) else {
        return Change::default();
    };
    let mut patches = vec![Patch::Shift { sheet, shift }];

    // Every sheet, not just the one that moved: a formula on Sheet2 can name
    // Sheet1!A1, and it is just as wrong afterwards.
    for (index, other) in book.sheets.iter().enumerate() {
        let texts: Vec<(FormulaId, String)> = other
            .formulas
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.text.is_empty())
            .filter_map(|(i, f)| {
                let rewritten = translate(&f.text, &other.name, &moved.name, shift)?;
                Some((FormulaId::from_index(i as u32), rewritten))
            })
            .collect();
        if !texts.is_empty() {
            patches.push(Patch::Formulas {
                sheet: index,
                texts,
            });
        }
    }

    Change::new(label_for(shift), patches)
}

fn label_for(shift: Shift) -> &'static str {
    match (shift.axis, shift.delete) {
        (Axis::Rows, false) => "Insert rows",
        (Axis::Rows, true) => "Delete rows",
        (Axis::Columns, false) => "Insert columns",
        (Axis::Columns, true) => "Delete columns",
    }
}

/// Emptying cells, keeping their style — Excel's Delete key.
pub fn clear_contents(book: &Workbook, sheet: usize, ranges: &[CellRange]) -> Change {
    let Some(target) = book.sheet(sheet) else {
        return Change::default();
    };
    let mut cells = Vec::new();
    for range in ranges {
        for row in range.start.row..=range.end.row {
            for col in range.start.col..=range.end.col {
                let at = CellRef::new(row, col);
                let Some(existing) = target.get(at) else {
                    continue;
                };
                cells.push((
                    at,
                    Some(Cell {
                        value: CellValue::Blank,
                        style: existing.style,
                        formula: None,
                    }),
                ));
            }
        }
    }
    Change::new("Clear", vec![Patch::Cells { sheet, cells }])
}

/// What the user typed, turned into what the cell should hold.
///
/// Takes the workbook mutably because a formula needs an arena slot and text
/// needs interning, both of which only ever grow. The cell write itself is in
/// the returned change, so it is still undo that decides when it happens.
pub fn input(book: &mut Workbook, sheet: usize, at: CellRef, typed: &str) -> Change {
    let style = book
        .sheet(sheet)
        .and_then(|s| s.get(at))
        .map_or(StyleId::DEFAULT, |c| c.style);
    let cell = typed_cell(book, sheet, style, typed);
    Change::new(
        "Edit cell",
        vec![Patch::Cells {
            sheet,
            cells: vec![(at, Some(cell))],
        }],
    )
}

/// The cell that `typed` produces, keeping `style` unless the input calls for
/// one of its own.
///
/// Shared with paste, because text arriving from another program has to be read
/// exactly the way text arriving from the keyboard is.
pub(crate) fn typed_cell(book: &mut Workbook, sheet: usize, style: StyleId, typed: &str) -> Cell {
    match classify(typed) {
        Typed::Empty => Cell {
            value: CellValue::Blank,
            style,
            formula: None,
        },
        Typed::Formula(text) => {
            let id = book
                .sheet_mut(sheet)
                .map(|s| s.push_formula(Formula::normal(text)));
            Cell {
                // A placeholder until recalculation runs.
                value: CellValue::Blank,
                style,
                formula: id,
            }
        }
        Typed::Number(n, format) => Cell {
            value: CellValue::Number(n),
            // A typed date is a number plus the format that makes it read as a
            // date. Without the second half the user sees 45306.
            style: match format {
                Some(code) if style == StyleId::DEFAULT => book.styles.style_for_format(&code),
                _ => style,
            },
            formula: None,
        },
        Typed::Bool(b) => Cell {
            value: CellValue::Bool(b),
            style,
            formula: None,
        },
        Typed::Error(e) => Cell {
            value: CellValue::Error(e),
            style,
            formula: None,
        },
        Typed::Text(text) => {
            let id = book.strings.intern(&text);
            Cell {
                value: CellValue::Text(id),
                style,
                formula: None,
            }
        }
    }
}

enum Typed {
    Empty,
    Formula(String),
    /// A number, and the format code it should be displayed through if the cell
    /// does not already have one of its own.
    ///
    /// Owned rather than `&'static str` because a typed percentage carries as
    /// many decimal places as the user typed, and there is no fixed set of them.
    Number(f64, Option<Cow<'static, str>>),
    Bool(bool),
    Error(ss_model::CellError),
    Text(String),
}

/// Excel's order of interpretation for typed input.
///
/// The order is the whole content of this function. `TRUE` is a boolean before
/// it is text; `1/2/2024` is a date before it is a division; `+5` is a number
/// even though `+` starts a formula in some spreadsheets.
fn classify(typed: &str) -> Typed {
    let trimmed = typed.trim();
    if trimmed.is_empty() {
        return Typed::Empty;
    }
    if let Some(rest) = trimmed.strip_prefix('=') {
        return Typed::Formula(rest.to_string());
    }
    // A leading apostrophe forces text, and is not part of the value.
    if let Some(rest) = typed.strip_prefix('\'') {
        return Typed::Text(rest.to_string());
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return Typed::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Typed::Bool(false);
    }
    if let Some(error) = ss_model::CellError::from_code(trimmed) {
        return Typed::Error(error);
    }
    if let Some(number) = parse_number(trimmed) {
        return number;
    }
    if let Some((serial, format)) = crate::functions::date::typed_datetime(trimmed) {
        return Typed::Number(serial, Some(Cow::Borrowed(format)));
    }
    Typed::Text(typed.to_string())
}

/// Plain numbers, percentages, and grouped thousands.
fn parse_number(text: &str) -> Option<Typed> {
    if let Some(body) = text.strip_suffix('%') {
        let body = body.trim();
        let value: f64 = degroup(body)?.parse().ok()?;
        // The format has to keep the digits that were typed. `42.5%` under `0%`
        // displays as `43%`, which is the cell disagreeing with what the user
        // just put in it.
        let decimals = body.split_once('.').map_or(0, |(_, d)| d.len());
        let format = match decimals {
            0 => "0%".to_string(),
            n => format!("0.{}%", "0".repeat(n)),
        };
        return Some(Typed::Number(value / 100.0, Some(Cow::Owned(format))));
    }
    let value: f64 = degroup(text)?.parse().ok()?;
    // `#,##0` here would be a guess about the user's intent; Excel does apply a
    // grouped format when the typing was grouped, and so do we.
    let grouped = text.contains(',');
    Some(Typed::Number(
        value,
        grouped.then(|| {
            Cow::Borrowed(if text.contains('.') {
                "#,##0.00"
            } else {
                "#,##0"
            })
        }),
    ))
}

/// Removes thousands separators, refusing anything that is not a number shape.
///
/// `f64::from_str` would accept `inf` and `NaN`, which are not values a
/// spreadsheet cell can hold, so the character set is checked first.
fn degroup(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    if !text
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b',' | b'-' | b'+' | b'e' | b'E'))
    {
        return None;
    }
    Some(text.replace(',', ""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::{CellValue, Sheet};

    fn book() -> Workbook {
        let mut book = Workbook::blank();
        book.sheets.push(Sheet::new("Data"));
        book
    }

    fn number(book: &Workbook, sheet: usize, a1: &str) -> Option<f64> {
        let at = CellRef::from_a1(a1)?;
        match book.sheet(sheet)?.get(at)?.value {
            CellValue::Number(n) => Some(n),
            _ => None,
        }
    }

    fn text_of(book: &mut Workbook, sheet: usize, a1: &str, typed: &str) -> Change {
        let at = CellRef::from_a1(a1).expect("valid address");
        input(book, sheet, at, typed)
    }

    #[test]
    fn undo_puts_back_exactly_what_was_there() {
        let mut book = book();
        let change = text_of(&mut book, 0, "A1", "5");
        let undo = apply(&mut book, change);
        assert_eq!(number(&book, 0, "A1"), Some(5.0));

        let redo = apply(&mut book, undo);
        assert_eq!(book.sheet(0).unwrap().get(CellRef::new(0, 0)), None);

        apply(&mut book, redo);
        assert_eq!(
            number(&book, 0, "A1"),
            Some(5.0),
            "redo is the undo of undo"
        );
    }

    #[test]
    fn a_percentage_keeps_the_decimals_it_was_typed_with() {
        // Under a plain `0%` the cell would answer `43%` to someone who just
        // typed `42.5%` — the one number they are certain about.
        // A cell each: typing into one that already has a format keeps that
        // format, which is a different rule and is tested by its own name.
        let mut book = book();
        for (row, (typed, shown)) in [("42.5%", "42.5%"), ("50%", "50%"), ("1.25%", "1.25%")]
            .into_iter()
            .enumerate()
        {
            let cell = format!("A{}", row + 1);
            let change = text_of(&mut book, 0, &cell, typed);
            apply(&mut book, change);
            let at = CellRef::new(row as u32, 0);
            let style = book.sheet(0).unwrap().get(at).unwrap().style;
            let value = number(&book, 0, &cell).expect("a number");
            assert_eq!(
                book.styles
                    .number_format(style)
                    .format(ss_model::numfmt::FormatValue::Number(value))
                    .text,
                shown,
                "typing {typed}"
            );
        }
    }

    #[test]
    fn typing_is_read_the_way_excel_reads_it() {
        let mut book = book();
        for (typed, expected) in [("5", 5.0), ("-2.5", -2.5), ("50%", 0.5), ("1,234", 1234.0)] {
            let change = text_of(&mut book, 0, "A1", typed);
            apply(&mut book, change);
            assert_eq!(number(&book, 0, "A1"), Some(expected), "typing {typed}");
        }

        let change = text_of(&mut book, 0, "A1", "TRUE");
        apply(&mut book, change);
        assert_eq!(
            book.sheet(0)
                .unwrap()
                .get(CellRef::new(0, 0))
                .unwrap()
                .value,
            CellValue::Bool(true)
        );

        // A leading apostrophe is Excel's escape hatch, and is not stored.
        let change = text_of(&mut book, 0, "A1", "'5");
        apply(&mut book, change);
        let cell = book.sheet(0).unwrap().get(CellRef::new(0, 0)).unwrap();
        match cell.value {
            CellValue::Text(id) => assert_eq!(book.strings.resolve(id), "5"),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn a_typed_date_gets_a_format_that_makes_it_read_as_one() {
        let mut book = book();
        let change = text_of(&mut book, 0, "A1", "2024-01-15");
        apply(&mut book, change);

        let cell = *book.sheet(0).unwrap().get(CellRef::new(0, 0)).unwrap();
        assert_eq!(cell.value, CellValue::Number(45306.0));
        assert_eq!(
            book.styles
                .number_format(cell.style)
                .format(ss_model::FormatValue::Number(45306.0))
                .text,
            "01-15-24",
            "the serial alone would show as 45306"
        );
    }

    #[test]
    fn inserting_rows_rewrites_formulas_on_every_sheet() {
        let mut book = book();
        let id = book.sheets[0].push_formula(Formula::normal("SUM(A1:A10)"));
        book.sheets[0].set(
            CellRef::new(20, 0),
            Cell {
                value: CellValue::Blank,
                style: StyleId::DEFAULT,
                formula: Some(id),
            },
        );
        let elsewhere = book.sheets[1].push_formula(Formula::normal("Sheet1!A5*2"));
        book.sheets[1].set(
            CellRef::new(0, 0),
            Cell {
                value: CellValue::Blank,
                style: StyleId::DEFAULT,
                formula: Some(elsewhere),
            },
        );

        let change = structural(&book, 0, Shift::insert(Axis::Rows, 0, 2));
        let undo = apply(&mut book, change);

        assert_eq!(book.sheets[0].formulas[0].text, "SUM(A3:A12)");
        assert_eq!(
            book.sheets[1].formulas[0].text, "Sheet1!A7*2",
            "a formula on another sheet pointed at the rows that moved"
        );

        apply(&mut book, undo);
        assert_eq!(book.sheets[0].formulas[0].text, "SUM(A1:A10)");
        assert_eq!(book.sheets[1].formulas[0].text, "Sheet1!A5*2");
    }

    #[test]
    fn undoing_a_delete_brings_the_data_back() {
        let mut book = book();
        for row in 0..5u32 {
            book.sheets[0].set(
                CellRef::new(row, 0),
                Cell {
                    value: CellValue::Number(f64::from(row)),
                    ..Default::default()
                },
            );
        }
        book.sheets[0]
            .merges
            .push(CellRange::new(CellRef::new(1, 0), CellRef::new(1, 3)));
        book.sheets[0].row_heights.insert(1, 40.0);

        let change = structural(&book, 0, Shift::delete(Axis::Rows, 1, 2));
        let undo = apply(&mut book, change);
        assert_eq!(number(&book, 0, "A2"), Some(3.0));
        assert!(book.sheets[0].merges.is_empty());

        apply(&mut book, undo);
        for row in 0..5u32 {
            assert_eq!(
                number(&book, 0, &format!("A{}", row + 1)),
                Some(f64::from(row)),
                "row {row} came back"
            );
        }
        assert_eq!(book.sheets[0].merges.len(), 1, "the merge came back too");
        assert_eq!(book.sheets[0].row_heights.get(&1), Some(&40.0));
    }

    #[test]
    fn clearing_keeps_the_formatting_and_removes_the_content() {
        let mut book = book();
        book.sheets[0].set(
            CellRef::new(0, 0),
            Cell {
                value: CellValue::Number(7.0),
                style: StyleId(3),
                formula: None,
            },
        );
        let range = CellRange::new(CellRef::new(0, 0), CellRef::new(0, 0));
        let change = clear_contents(&book, 0, &[range]);
        apply(&mut book, change);

        let cell = book.sheets[0].get(CellRef::new(0, 0)).expect("still there");
        assert_eq!(cell.value, CellValue::Blank);
        assert_eq!(cell.style, StyleId(3), "Delete clears contents, not format");
    }
}
