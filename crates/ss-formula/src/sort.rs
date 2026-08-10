//! Sorting a range, and finding the range a user meant.
//!
//! Two things here are less obvious than sorting normally is.
//!
//! **Excel does not order values, it orders *kinds* first.** Ascending, every
//! number comes before every piece of text, which comes before `FALSE`, then
//! `TRUE`, then errors. Comparing a number against text by coercing one to the
//! other — the obvious implementation — puts `"10"` next to `10` and scatters a
//! column's text headers through the middle of its data. Blanks are separate
//! again: they sort last *in both directions*, because a blank is the absence
//! of a value rather than a value smaller than all the others.
//!
//! **A formula that moves is rewritten.** Sorting is defined as moving the rows,
//! and Excel adjusts the relative references in a formula that travels with
//! them — `=B5*2` in row 5, landing in row 8, becomes `=B8*2`. That is the same
//! translation copying uses, dollar signs and all, so [`translate::offset`] does
//! it. The rewritten text goes into a *new* arena entry rather than over the old
//! one: an entry can be shared, and overwriting a shared master would rewrite a
//! formula that never moved.
//!
//! Which is also why there are two shapes of sort. Rows carrying no formulas
//! come out of a sort as the same rows in a different order — a permutation —
//! so the change is a list of row numbers and its undo is the inverse
//! permutation. Rows whose formulas are rewritten on the way are *not* the rows
//! they were, so those go through a cell-by-cell change instead. On a hundred
//! thousand rows the difference is half a megabyte of undo against a hundred.

use std::cmp::Ordering;

use ss_model::{Cell, CellRange, CellRef, CellValue, Formula, Sheet, Workbook};

use crate::edit::{Change, Patch};
use crate::translate::offset;

/// One level of a sort: which column, and which way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortKey {
    /// An absolute sheet column, not an offset into the range.
    pub col: u32,
    pub descending: bool,
}

/// The most rows a single sort will move.
///
/// Excel's own limit is the sheet; ours is the point past which the operation
/// is certainly a mistake, and it is cheap insurance against an accidental
/// select-all asking for a comparison key on a million empty rows.
pub const MAX_SORT_ROWS: u32 = 1 << 20;

/// Sorts the rows of `range` by `keys`.
///
/// `header` keeps the first row where it is, which is what "my data has
/// headers" means. Refused rather than half-done when the range holds a merge:
/// a merged block cannot move as a row, and Excel refuses the same case.
pub fn sort(
    book: &mut Workbook,
    sheet: usize,
    range: CellRange,
    keys: &[SortKey],
    header: bool,
) -> Result<Change, String> {
    let Some(model) = book.sheet(sheet) else {
        return Ok(Change::default());
    };
    if keys.is_empty() {
        return Ok(Change::default());
    }
    if model.merges.iter().any(|m| overlaps(*m, range)) {
        return Err("That range holds merged cells, which cannot be sorted".to_string());
    }
    if range.rows() > MAX_SORT_ROWS {
        return Err("That is too many rows to sort at once".to_string());
    }

    let first = range.start.row + u32::from(header);
    if first >= range.end.row {
        return Ok(Change::default());
    }

    // Decorate, sort, and keep the original position as the tie-break so the
    // sort is stable: a second sort on a different column has to leave rows
    // that tie in the order the first one put them.
    let mut order: Vec<(u32, Vec<Key>)> = (first..=range.end.row)
        .map(|row| {
            let ranked = keys
                .iter()
                .map(|k| key_of(model, book, CellRef::new(row, k.col)))
                .collect();
            (row, ranked)
        })
        .collect();
    order.sort_by(|(a_row, a), (b_row, b)| {
        for (i, key) in keys.iter().enumerate() {
            let ordering = compare(&a[i], &b[i]);
            if ordering != Ordering::Equal {
                // Blanks stay at the bottom whichever way the column is sorted;
                // reversing them would push every empty row to the top, which
                // is not what anyone means by "Z to A".
                return if key.descending && !a[i].is_blank() && !b[i].is_blank() {
                    ordering.reverse()
                } else {
                    ordering
                };
            }
        }
        a_row.cmp(b_row)
    });

    if order
        .iter()
        .enumerate()
        .all(|(i, (row, _))| *row == first + i as u32)
    {
        return Ok(Change::default()); // already in this order
    }

    let cols = (range.start.col, range.end.col);
    let rows = (first, range.end.row);

    // Nothing in the way of treating this as what it is — a reordering of rows
    // — so say so, and let undo be the inverse permutation rather than a copy
    // of every cell that moved.
    if !carries_formulas(model, rows, cols) {
        let order: Vec<u32> = order.into_iter().map(|(row, _)| row).collect();
        return Ok(Change::new(
            "Sort",
            vec![Patch::Permute {
                sheet,
                first,
                cols,
                order,
            }],
        ));
    }

    // Read every cell that will move before anything is written: the source of
    // one row is the destination of another.
    let taken: Vec<Vec<Option<Cell>>> = order
        .iter()
        .map(|(row, _)| {
            (range.start.col..=range.end.col)
                .map(|col| book.sheets[sheet].get(CellRef::new(*row, col)).copied())
                .collect()
        })
        .collect();

    let mut cells: Vec<(CellRef, Option<Cell>)> = Vec::new();
    for (index, row_cells) in taken.into_iter().enumerate() {
        let to = first + index as u32;
        let from = order[index].0;
        let delta = i64::from(to) - i64::from(from);
        for (offset_col, cell) in row_cells.into_iter().enumerate() {
            let at = CellRef::new(to, range.start.col + offset_col as u32);
            let cell = cell.map(|cell| moved(book, sheet, cell, delta));
            cells.push((at, cell));
        }
    }

    Ok(Change::new("Sort", vec![Patch::Cells { sheet, cells }]))
}

/// Whether any row about to move carries a formula.
///
/// A sheet with an empty arena cannot, and answering from the arena costs
/// nothing — which matters, because the rectangle scan behind it is the one
/// thing here that has to look at every cell whether or not the answer is no.
fn carries_formulas(sheet: &Sheet, rows: (u32, u32), cols: (u32, u32)) -> bool {
    sheet.formulas.iter().any(|f| !f.text.is_empty()) && sheet.cells.any_formula(rows, cols)
}

/// A cell as it should look `delta` rows further down, formula and all.
fn moved(book: &mut Workbook, sheet: usize, cell: Cell, delta: i64) -> Cell {
    let Some(id) = cell.formula else {
        return cell;
    };
    if delta == 0 {
        return cell;
    }
    let Some(existing) = book
        .sheet(sheet)
        .and_then(|s| s.formula(id))
        .map(|f| f.text.clone())
    else {
        return cell;
    };
    let Some(rewritten) = offset(&existing, delta, 0) else {
        return cell; // nothing relative in it, so it is already right
    };
    let new_id = book
        .sheet_mut(sheet)
        .map(|s| s.push_formula(Formula::normal(rewritten)));
    Cell {
        formula: new_id,
        ..cell
    }
}

/// What a cell contributes to the ordering.
#[derive(Debug, Clone, PartialEq)]
enum Key {
    Number(f64),
    Text(String),
    Bool(bool),
    Error(String),
    Blank,
}

impl Key {
    const fn is_blank(&self) -> bool {
        matches!(self, Key::Blank)
    }

    /// Excel's order between *kinds*: numbers, text, logicals, errors, blanks.
    const fn rank(&self) -> u8 {
        match self {
            Key::Number(_) => 0,
            Key::Text(_) => 1,
            Key::Bool(_) => 2,
            Key::Error(_) => 3,
            Key::Blank => 4,
        }
    }
}

fn key_of(sheet: &Sheet, book: &Workbook, at: CellRef) -> Key {
    match sheet.get(at).map(|c| c.value) {
        None | Some(CellValue::Blank) => Key::Blank,
        Some(CellValue::Number(n)) => Key::Number(n),
        Some(CellValue::Text(id)) => {
            let text = book.strings.resolve(id);
            if text.is_empty() {
                Key::Blank
            } else {
                // Folded once here rather than on every comparison. A sort of a
                // hundred thousand rows asks two million times, and lowercasing
                // a string inside the comparator is two allocations each time.
                Key::Text(fold(text))
            }
        }
        Some(CellValue::Bool(b)) => Key::Bool(b),
        Some(CellValue::Error(e)) => Key::Error(e.as_str().to_string()),
    }
}

/// A string as it is compared: case folded, since "apple" and "Apple" are the
/// same word to a person sorting a list.
///
/// The ASCII path is not an optimization for its own sake — it is the whole
/// column, on every spreadsheet anybody sorts by an identifier or a timestamp,
/// and `str::to_lowercase` walks it through the full Unicode tables regardless.
fn fold(text: &str) -> String {
    if text.is_ascii() {
        text.to_ascii_lowercase()
    } else {
        text.to_lowercase()
    }
}

fn compare(a: &Key, b: &Key) -> Ordering {
    match (a, b) {
        (Key::Number(a), Key::Number(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
        // Already folded. Two spellings of the same word tie here and fall
        // through to the row number, which is what keeps a case-insensitive
        // sort stable — Excel leaves "Apple" and "apple" in the order it found
        // them rather than deciding one of them sorts first.
        (Key::Text(a), Key::Text(b)) => a.cmp(b),
        (Key::Bool(a), Key::Bool(b)) => a.cmp(b),
        (Key::Error(a), Key::Error(b)) => a.cmp(b),
        _ => a.rank().cmp(&b.rank()),
    }
}

fn overlaps(a: CellRange, b: CellRange) -> bool {
    a.start.row <= b.end.row
        && b.start.row <= a.end.row
        && a.start.col <= b.end.col
        && b.start.col <= a.end.col
}

/// The block of data around `at` — what Excel calls the current region.
///
/// Grown outwards from the cell until a fully empty row or column stops it,
/// because that is how a user reads "the table I am standing in". A single
/// selected cell means the whole table; a dragged selection means itself, and
/// the caller decides which it has.
pub fn region(sheet: &Sheet, at: CellRef) -> CellRange {
    let (mut top, mut bottom) = (at.row, at.row);
    let (mut left, mut right) = (at.col, at.col);

    let occupied = |row: u32, col: u32| {
        sheet
            .get(CellRef::new(row, col))
            .is_some_and(|c| !c.value.is_blank() || c.formula.is_some())
    };
    let row_has = |row: u32, left: u32, right: u32| (left..=right).any(|c| occupied(row, c));
    let col_has = |col: u32, top: u32, bottom: u32| (top..=bottom).any(|r| occupied(r, col));

    // Alternating passes: widening the block can reveal rows that were empty
    // within the narrower one, and a single pass each way would stop short of a
    // table whose first column has a gap.
    loop {
        let before = (top, bottom, left, right);
        while top > 0 && row_has(top - 1, left, right) {
            top -= 1;
        }
        while bottom + 1 < ss_model::cell::MAX_ROWS && row_has(bottom + 1, left, right) {
            bottom += 1;
        }
        while left > 0 && col_has(left - 1, top, bottom) {
            left -= 1;
        }
        while right + 1 < ss_model::cell::MAX_COLS && col_has(right + 1, top, bottom) {
            right += 1;
        }
        if (top, bottom, left, right) == before {
            break;
        }
    }

    CellRange::new(CellRef::new(top, left), CellRef::new(bottom, right))
}

/// Whether the first row of `range` looks like headings rather than data.
///
/// The signal is what is *at the top*, not how it compares to what is below.
/// A row of text over a column of numbers is the easy case and every guess
/// gets it right; the case that decides the rule is a text column under a text
/// heading, which is what most exported data looks like — timestamps, ids,
/// statuses, all of them strings. There is nothing in the values to tell the
/// heading apart, and both answers are wrong sometimes:
///
/// - Call it data, and a quick A→Z files the word `topic` in among the topics.
/// - Call it a heading, and a list that genuinely has none loses its first row
///   from the sort.
///
/// Excel takes the second, and so do we, because the two mistakes are not
/// equally visible: a heading sitting in the middle of the data is obvious
/// from the screen, and a row quietly left out of the sort is not. The Sort
/// dialog's checkbox is there for the times the guess is wrong, and a quick
/// sort says in the status bar when it kept a row back.
///
/// A number, a date or a boolean at the top is still data, and vetoes the
/// whole guess: nobody heads a column with `2024`.
pub fn looks_like_headers(sheet: &Sheet, range: CellRange) -> bool {
    if range.rows() < 3 {
        return false;
    }
    let kind = |at: CellRef| sheet.get(at).map(|c| c.value);
    let mut any = false;
    for col in range.start.col..=range.end.col {
        match kind(CellRef::new(range.start.row, col)) {
            Some(CellValue::Text(_)) => any = true,
            // A column with nothing at the top says nothing either way — a
            // calculated column with no heading is an ordinary shape, and
            // letting it veto the guess would mean every such table sorted its
            // own headings into the data.
            None | Some(CellValue::Blank) => {}
            _ => return false,
        }
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::apply;
    use ss_model::{CellError, StyleId};

    fn book() -> Workbook {
        Workbook::blank()
    }

    fn put(book: &mut Workbook, a1: &str, value: CellValue) {
        let at = CellRef::from_a1(a1).expect("a1");
        book.sheets[0].set(
            at,
            Cell {
                value,
                ..Default::default()
            },
        );
    }

    fn text(book: &mut Workbook, a1: &str, s: &str) {
        let id = book.strings.intern(s);
        put(book, a1, CellValue::Text(id));
    }

    fn shown(book: &Workbook, a1: &str) -> String {
        let at = CellRef::from_a1(a1).expect("a1");
        match book.sheets[0].get(at).map(|c| c.value) {
            None | Some(CellValue::Blank) => String::new(),
            Some(CellValue::Number(n)) => ss_model::format_general(n),
            Some(CellValue::Text(id)) => book.strings.resolve(id).to_string(),
            Some(CellValue::Bool(b)) => b.to_string().to_uppercase(),
            Some(CellValue::Error(e)) => e.as_str().to_string(),
        }
    }

    fn range(a: &str, b: &str) -> CellRange {
        CellRange::new(
            CellRef::from_a1(a).expect("a1"),
            CellRef::from_a1(b).expect("a1"),
        )
    }

    fn ascending(col: u32) -> Vec<SortKey> {
        vec![SortKey {
            col,
            descending: false,
        }]
    }

    #[test]
    fn a_column_sorts_and_takes_its_row_with_it() {
        let mut book = book();
        for (row, (name, n)) in [("carol", 3.0), ("alice", 1.0), ("bob", 2.0)]
            .into_iter()
            .enumerate()
        {
            text(&mut book, &format!("A{}", row + 1), name);
            put(&mut book, &format!("B{}", row + 1), CellValue::Number(n));
        }

        let change = sort(&mut book, 0, range("A1", "B3"), &ascending(0), false).expect("sortable");
        let undo = apply(&mut book, change);

        assert_eq!(shown(&book, "A1"), "alice");
        assert_eq!(shown(&book, "B1"), "1", "the whole row moved, not one cell");
        assert_eq!(shown(&book, "A3"), "carol");
        assert_eq!(shown(&book, "B3"), "3");

        apply(&mut book, undo);
        assert_eq!(shown(&book, "A1"), "carol", "undone exactly");
        assert_eq!(shown(&book, "B1"), "3");
    }

    #[test]
    fn kinds_sort_before_values_and_blanks_are_always_last() {
        // Coercing text to a number would put "10" beside 10; coercing the other
        // way would sort 2 after 10. Excel does neither.
        let mut book = book();
        put(&mut book, "A1", CellValue::Number(10.0));
        text(&mut book, "A2", "10");
        put(&mut book, "A3", CellValue::Number(2.0));
        put(&mut book, "A4", CellValue::Blank);
        put(&mut book, "A5", CellValue::Bool(false));
        put(&mut book, "A6", CellValue::Error(CellError::Value));

        let change = sort(&mut book, 0, range("A1", "A6"), &ascending(0), false).expect("sortable");
        apply(&mut book, change);
        let column: Vec<String> = (1..=6).map(|r| shown(&book, &format!("A{r}"))).collect();
        assert_eq!(column, ["2", "10", "10", "FALSE", "#VALUE!", ""]);

        let change = sort(
            &mut book,
            0,
            range("A1", "A6"),
            &[SortKey {
                col: 0,
                descending: true,
            }],
            false,
        )
        .expect("sortable");
        apply(&mut book, change);
        let column: Vec<String> = (1..=6).map(|r| shown(&book, &format!("A{r}"))).collect();
        assert_eq!(
            column,
            ["#VALUE!", "FALSE", "10", "10", "2", ""],
            "descending reverses everything except where the blanks sit"
        );
    }

    #[test]
    fn a_second_key_only_decides_where_the_first_ties() {
        let mut book = book();
        for (row, (dept, n)) in [("b", 2.0), ("a", 9.0), ("b", 1.0), ("a", 4.0)]
            .into_iter()
            .enumerate()
        {
            text(&mut book, &format!("A{}", row + 1), dept);
            put(&mut book, &format!("B{}", row + 1), CellValue::Number(n));
        }

        let keys = [
            SortKey {
                col: 0,
                descending: false,
            },
            SortKey {
                col: 1,
                descending: true,
            },
        ];
        let change = sort(&mut book, 0, range("A1", "B4"), &keys, false).expect("sortable");
        apply(&mut book, change);

        let rows: Vec<(String, String)> = (1..=4)
            .map(|r| {
                (
                    shown(&book, &format!("A{r}")),
                    shown(&book, &format!("B{r}")),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("a".into(), "9".into()),
                ("a".into(), "4".into()),
                ("b".into(), "2".into()),
                ("b".into(), "1".into()),
            ]
        );
    }

    #[test]
    fn a_header_row_stays_where_it_is() {
        let mut book = book();
        text(&mut book, "A1", "Name");
        text(&mut book, "A2", "carol");
        text(&mut book, "A3", "alice");

        let change = sort(&mut book, 0, range("A1", "A3"), &ascending(0), true).expect("sortable");
        apply(&mut book, change);
        assert_eq!(shown(&book, "A1"), "Name");
        assert_eq!(shown(&book, "A2"), "alice");
    }

    #[test]
    fn a_formula_that_moves_is_rewritten_the_way_a_copied_one_is() {
        // Row 1 holds =B1*2 and sorts down to row 2. Left alone it would still
        // say B1 and quietly total someone else's row.
        let mut book = book();
        put(&mut book, "B1", CellValue::Number(5.0));
        put(&mut book, "B2", CellValue::Number(1.0));
        let id = book.sheets[0].push_formula(Formula::normal("B1*2"));
        book.sheets[0].set(
            CellRef::from_a1("C1").expect("a1"),
            Cell {
                value: CellValue::Number(10.0),
                style: StyleId::DEFAULT,
                formula: Some(id),
            },
        );

        let change = sort(&mut book, 0, range("B1", "C2"), &ascending(1), false).expect("sortable");
        apply(&mut book, change);

        let moved = book.sheets[0]
            .formula_at(CellRef::from_a1("C2").expect("a1"))
            .expect("the formula travelled with its row");
        assert_eq!(moved.text, "B2*2");
        assert_eq!(
            book.sheets[0].formulas[id.index() as usize].text,
            "B1*2",
            "the original arena entry is untouched, in case it was shared"
        );
    }

    #[test]
    fn a_sort_of_plain_data_is_a_permutation_and_costs_what_one_costs() {
        // The point of the permutation patch: undo is the rows read backwards,
        // not a copy of every cell that moved.
        let mut book = book();
        for (row, name) in ["delta", "alpha", "charlie", "bravo"]
            .into_iter()
            .enumerate()
        {
            text(&mut book, &format!("A{}", row + 1), name);
            put(
                &mut book,
                &format!("B{}", row + 1),
                CellValue::Number(row as f64),
            );
        }

        let change = sort(&mut book, 0, range("A1", "B4"), &ascending(0), false).expect("sortable");
        match change.patches.as_slice() {
            [Patch::Permute { order, cols, .. }] => {
                assert_eq!(order, &[1, 3, 2, 0]);
                assert_eq!(*cols, (0, 1));
            }
            other => panic!("expected one permutation, got {other:?}"),
        }

        let undo = apply(&mut book, change);
        let column: Vec<String> = (1..=4).map(|r| shown(&book, &format!("A{r}"))).collect();
        assert_eq!(column, ["alpha", "bravo", "charlie", "delta"]);
        assert_eq!(shown(&book, "B1"), "1", "the whole row travelled");

        apply(&mut book, undo);
        let column: Vec<String> = (1..=4).map(|r| shown(&book, &format!("A{r}"))).collect();
        assert_eq!(column, ["delta", "alpha", "charlie", "bravo"]);
        assert_eq!(shown(&book, "B1"), "0");
    }

    #[test]
    fn a_sort_that_moves_a_formula_writes_cells_instead() {
        // A formula is rewritten as it travels, so the rows afterwards are not
        // the rows from before in a different order, and a permutation would
        // have no inverse.
        let mut book = book();
        put(&mut book, "A1", CellValue::Number(2.0));
        put(&mut book, "A2", CellValue::Number(1.0));
        let id = book.sheets[0].push_formula(Formula::normal("A1*2"));
        book.sheets[0].set(
            CellRef::from_a1("B1").expect("a1"),
            Cell {
                value: CellValue::Number(4.0),
                style: StyleId::DEFAULT,
                formula: Some(id),
            },
        );

        let change = sort(&mut book, 0, range("A1", "B2"), &ascending(0), false).expect("sortable");
        assert!(
            matches!(change.patches.as_slice(), [Patch::Cells { .. }]),
            "a formula in the band rules the shortcut out"
        );
    }

    #[test]
    fn an_empty_row_inside_the_range_sorts_to_the_bottom_and_takes_nothing_with_it() {
        // Under a permutation, a blank row is a real row that has to arrive
        // somewhere: getting this wrong duplicates whatever was in its place.
        let mut book = book();
        text(&mut book, "A1", "b");
        text(&mut book, "A3", "a");

        let change = sort(&mut book, 0, range("A1", "A3"), &ascending(0), false).expect("sortable");
        apply(&mut book, change);
        let column: Vec<String> = (1..=3).map(|r| shown(&book, &format!("A{r}"))).collect();
        assert_eq!(column, ["a", "b", ""]);
        assert_eq!(book.sheets[0].cells.len(), 2, "nothing was duplicated");
    }

    #[test]
    fn a_range_with_merged_cells_is_refused_rather_than_half_sorted() {
        let mut book = book();
        put(&mut book, "A1", CellValue::Number(2.0));
        put(&mut book, "A2", CellValue::Number(1.0));
        book.sheets[0].merges.push(range("A1", "B1"));
        assert!(sort(&mut book, 0, range("A1", "B2"), &ascending(0), false).is_err());
    }

    #[test]
    fn the_current_region_stops_at_empty_rows_and_columns() {
        let mut book = book();
        for a1 in ["B2", "C2", "B3", "C3", "B4"] {
            put(&mut book, a1, CellValue::Number(1.0));
        }
        put(&mut book, "E9", CellValue::Number(1.0));

        let found = region(&book.sheets[0], CellRef::from_a1("B3").expect("a1"));
        assert_eq!(found, range("B2", "C4"));
        assert!(
            !found.contains(CellRef::from_a1("E9").expect("a1")),
            "an island across a gap is a different table"
        );
    }

    #[test]
    fn headers_are_guessed_only_where_the_guess_is_safe() {
        let mut book = book();
        text(&mut book, "A1", "Name");
        text(&mut book, "B1", "Score");
        text(&mut book, "A2", "alice");
        put(&mut book, "B2", CellValue::Number(1.0));
        text(&mut book, "A3", "bob");
        put(&mut book, "B3", CellValue::Number(2.0));
        assert!(looks_like_headers(&book.sheets[0], range("A1", "B3")));
        assert!(
            looks_like_headers(&book.sheets[0], range("A1", "C3")),
            "an empty column C does not veto the guess"
        );
        // A number at the top of a column is data, whatever its neighbours say.
        put(&mut book, "C1", CellValue::Number(2024.0));
        assert!(!looks_like_headers(&book.sheets[0], range("A1", "C3")));

        // Text over text — what most exported data looks like, and the case
        // the rule exists to decide. Excel calls it a heading; so do we.
        let mut all_text = book_of_text();
        assert!(looks_like_headers(&all_text.sheets[0], range("A1", "A3")));
        put(&mut all_text, "A1", CellValue::Number(1.0));
        assert!(
            !looks_like_headers(&all_text.sheets[0], range("A1", "A3")),
            "nobody heads a column with a number"
        );
    }

    #[test]
    fn a_column_of_strings_under_a_string_is_a_heading() {
        // The workbook this came from: 140k rows of an export, every column a
        // timestamp or an id or a status, all of them text. Guessing "no" here
        // filed the word `topic` in among the topics.
        let mut book = book();
        for (row, value) in ["topic", "beta", "alpha", "gamma"].into_iter().enumerate() {
            text(&mut book, &format!("A{}", row + 1), value);
        }
        assert!(looks_like_headers(&book.sheets[0], range("A1", "A4")));

        let change = sort(&mut book, 0, range("A1", "A4"), &ascending(0), true).expect("sortable");
        apply(&mut book, change);
        let column: Vec<String> = (1..=4).map(|r| shown(&book, &format!("A{r}"))).collect();
        assert_eq!(column, ["topic", "alpha", "beta", "gamma"]);
    }

    fn book_of_text() -> Workbook {
        let mut book = book();
        for row in 1..=3 {
            text(&mut book, &format!("A{row}"), "word");
        }
        book
    }
}
