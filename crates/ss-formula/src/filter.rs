//! Applying an autofilter: which rows it leaves showing.
//!
//! The rule lives in the model and goes into the file as `<autoFilter>`. What
//! this does is turn it into the only thing the file records about the
//! *result* — hidden rows — so that a workbook filtered here and opened in a
//! program that has never heard of filters still shows the right rows.
//!
//! A filter matches on **displayed text**, not on the stored value. That is
//! what the file stores in `<filter val="…">`, and it is what the user ticked:
//! a date shown as `15/01/2024` is filtered by those characters, and matching
//! on the serial instead would make the checkbox list and the sheet disagree
//! about what the same cell is. A number filter is the exception and says so —
//! `>10` has to compare numerically or every value from 2 to 9 would sort above
//! 10 as text.

use std::collections::BTreeSet;

use ss_model::{AutoFilter, Cell, CellRef, CellValue, Compare, FilterKind, FormatValue, Workbook};

use crate::edit::{Change, Geometry, Patch};

/// The rows a filter hides, as an undoable change to the sheet's row heights.
///
/// Height zero is how a hidden row is stored everywhere else in this codebase
/// and how the writer spells `hidden="1"`, so filtering and the Hide command
/// produce the same thing and undo the same way.
pub fn apply(book: &Workbook, sheet: usize) -> Change {
    let Some(model) = book.sheet(sheet) else {
        return Change::default();
    };
    let Some(filter) = &model.filter else {
        return Change::default();
    };

    // Inside its own range the filter owns visibility outright: every data row
    // is shown or hidden by the criteria, and a row hidden by hand beforehand
    // is not remembered. Nothing in the file distinguishes the two — a hidden
    // row is a hidden row — so the alternative would be a rule the document
    // cannot record and the next reader could not reproduce.
    let mut geometry = Geometry::of(model);
    for row in filter.first_data_row()..=filter.range.end.row {
        if shows(book, sheet, filter, row) {
            if geometry.row_heights.get(&row) == Some(&0.0) {
                geometry.row_heights.remove(&row);
            }
        } else {
            geometry.row_heights.insert(row, 0.0);
        }
    }

    if geometry.row_heights == model.row_heights {
        return Change::default();
    }
    Change::new("Filter", vec![Patch::Geometry { sheet, geometry }])
}

/// Clears the filter's criteria and shows every row it had hidden.
///
/// The filter itself stays: Excel's "Clear" empties the criteria and leaves the
/// arrows, and removing them is a separate button.
pub fn clear(book: &Workbook, sheet: usize) -> Change {
    let Some(model) = book.sheet(sheet) else {
        return Change::default();
    };
    let Some(filter) = &model.filter else {
        return Change::default();
    };

    let mut geometry = Geometry::of(model);
    for row in filter.first_data_row()..=filter.range.end.row {
        if geometry.row_heights.get(&row) == Some(&0.0) {
            geometry.row_heights.remove(&row);
        }
    }
    Change::new(
        "Clear filter",
        vec![
            Patch::Filter {
                sheet,
                filter: Some(AutoFilter::over(filter.range)),
            },
            Patch::Geometry { sheet, geometry },
        ],
    )
}

/// Removes the filter altogether, arrows included, and shows every row.
pub fn remove(book: &Workbook, sheet: usize) -> Change {
    let mut change = clear(book, sheet);
    if let Some(Patch::Filter { filter, .. }) = change.patches.first_mut() {
        *filter = None;
    }
    change.label = "Remove filter".to_string();
    change
}

/// Whether a row survives every constrained column.
fn shows(book: &Workbook, sheet: usize, filter: &AutoFilter, row: u32) -> bool {
    filter.columns.iter().all(|column| {
        let at = CellRef::new(row, filter.range.start.col + column.col);
        let cell = book.sheet(sheet).and_then(|s| s.get(at));
        match &column.kind {
            FilterKind::Values { values, blanks } => {
                let text = shown(book, sheet, at, cell);
                if text.is_empty() {
                    return *blanks;
                }
                values.contains(&text)
            }
            FilterKind::Custom { first, second } => {
                let one = holds(book, sheet, at, cell, first);
                match second {
                    None => one,
                    Some((and, other)) => {
                        let two = holds(book, sheet, at, cell, other);
                        if *and {
                            one && two
                        } else {
                            one || two
                        }
                    }
                }
            }
        }
    })
}

/// One comparison against one cell.
fn holds(
    book: &Workbook,
    sheet: usize,
    at: CellRef,
    cell: Option<&Cell>,
    criterion: &ss_model::Criterion,
) -> bool {
    // A number on both sides compares as numbers. Anything else compares as
    // text, case-insensitively, with `*` and `?` as wildcards on an equality
    // test — which is what Excel's "begins with" and "contains" are underneath.
    let value = cell.map(|c| c.value);
    if let (Some(CellValue::Number(n)), Ok(rhs)) = (value, criterion.value.trim().parse::<f64>()) {
        return n
            .partial_cmp(&rhs)
            .is_some_and(|ordering| criterion.op.holds(ordering));
    }
    let text = shown(book, sheet, at, cell);
    if matches!(criterion.op, Compare::Equal | Compare::NotEqual)
        && criterion.value.contains(['*', '?'])
    {
        let matched = wildcard(&text.to_lowercase(), &criterion.value.to_lowercase());
        return matched == matches!(criterion.op, Compare::Equal);
    }
    criterion
        .op
        .holds(text.to_lowercase().cmp(&criterion.value.to_lowercase()))
}

/// A cell as the grid draws it.
fn shown(book: &Workbook, sheet: usize, at: CellRef, cell: Option<&Cell>) -> String {
    let Some(model) = book.sheet(sheet) else {
        return String::new();
    };
    let value = match cell.map(|c| c.value) {
        None | Some(CellValue::Blank) => return String::new(),
        Some(CellValue::Number(n)) => FormatValue::Number(n),
        Some(CellValue::Bool(b)) => FormatValue::Bool(b),
        Some(CellValue::Error(e)) => FormatValue::Error(e),
        Some(CellValue::Text(id)) => FormatValue::Text(book.strings.resolve(id)),
    };
    book.styles
        .number_format(model.style_at(at))
        .format(value)
        .text
}

/// `*` for any run and `?` for any one character, over already-lowercased text.
fn wildcard(text: &str, pattern: &str) -> bool {
    let text: Vec<char> = text.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    // The classic two-pointer walk with a remembered star, which is linear
    // rather than the exponential backtracking a naive recursion gives on a
    // pattern like `*a*a*a*`.
    let (mut t, mut p) = (0usize, 0usize);
    let (mut star, mut resume) = (None, 0usize);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            t += 1;
            p += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            resume = t;
            p += 1;
        } else if let Some(at) = star {
            p = at + 1;
            resume += 1;
            t = resume;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

/// The distinct displayed values in one column of the filter's range, for the
/// checkbox list, plus whether the column has any blanks in it.
///
/// Sorted and deduplicated by the `BTreeSet`, which is also the order Excel
/// shows them in. The header row is excluded: it is the column's name, not one
/// of its values.
pub fn distinct(book: &Workbook, sheet: usize, col: u32) -> (BTreeSet<String>, bool) {
    let mut values = BTreeSet::new();
    let mut blanks = false;
    let Some(model) = book.sheet(sheet) else {
        return (values, blanks);
    };
    let Some(filter) = &model.filter else {
        return (values, blanks);
    };
    for row in filter.first_data_row()..=filter.range.end.row {
        let at = CellRef::new(row, filter.range.start.col + col);
        let text = shown(book, sheet, at, model.get(at));
        if text.is_empty() {
            blanks = true;
        } else {
            values.insert(text);
        }
    }
    (values, blanks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::apply as apply_change;
    use ss_model::{CellRange, Criterion, Sheet, StyleId};

    fn book() -> Workbook {
        let mut book = Workbook::blank();
        let mut sheet = Sheet::new("Data");
        let header = book.strings.intern("Region");
        sheet.set(
            CellRef::new(0, 0),
            Cell {
                value: CellValue::Text(header),
                ..Default::default()
            },
        );
        for (row, (region, amount)) in [
            ("North", 10.0),
            ("South", 20.0),
            ("North", 30.0),
            ("East", 40.0),
        ]
        .into_iter()
        .enumerate()
        {
            let id = book.strings.intern(region);
            let row = row as u32 + 1;
            sheet.set(
                CellRef::new(row, 0),
                Cell {
                    value: CellValue::Text(id),
                    ..Default::default()
                },
            );
            sheet.set(
                CellRef::new(row, 1),
                Cell {
                    value: CellValue::Number(amount),
                    style: StyleId::DEFAULT,
                    formula: None,
                },
            );
        }
        sheet.filter = Some(AutoFilter::over(CellRange::new(
            CellRef::new(0, 0),
            CellRef::new(4, 1),
        )));
        book.sheets[0] = sheet;
        book
    }

    fn hidden(book: &Workbook) -> Vec<u32> {
        book.sheets[0]
            .row_heights
            .iter()
            .filter(|(_, h)| **h == 0.0)
            .map(|(r, _)| *r)
            .collect()
    }

    fn constrain(book: &mut Workbook, col: u32, kind: FilterKind) {
        book.sheets[0]
            .filter
            .as_mut()
            .expect("filtered")
            .set(col, Some(kind));
    }

    #[test]
    fn a_value_list_hides_the_rows_that_were_not_ticked() {
        let mut book = book();
        constrain(
            &mut book,
            0,
            FilterKind::Values {
                values: ["North".to_string()].into_iter().collect(),
                blanks: false,
            },
        );
        let change = apply(&book, 0);
        let undo = apply_change(&mut book, change);
        assert_eq!(hidden(&book), [2, 4], "South and East");

        apply_change(&mut book, undo);
        assert!(hidden(&book).is_empty(), "and undo shows them again");
    }

    #[test]
    fn the_header_row_is_never_hidden_by_its_own_filter() {
        let mut book = book();
        constrain(
            &mut book,
            0,
            FilterKind::Values {
                values: ["West".to_string()].into_iter().collect(),
                blanks: false,
            },
        );
        let change = apply(&book, 0);
        apply_change(&mut book, change);
        assert!(
            !hidden(&book).contains(&0),
            "the arrows have to stay reachable"
        );
    }

    #[test]
    fn a_number_comparison_compares_numbers_and_not_their_spelling() {
        // As text, "9" sorts above "10" and the filter would be exactly wrong.
        let mut book = book();
        book.sheets[0].set(
            CellRef::new(1, 1),
            Cell {
                value: CellValue::Number(9.0),
                ..Default::default()
            },
        );
        constrain(
            &mut book,
            1,
            FilterKind::Custom {
                first: Criterion {
                    op: Compare::GreaterEqual,
                    value: "20".into(),
                },
                second: None,
            },
        );
        let change = apply(&book, 0);
        apply_change(&mut book, change);
        assert_eq!(hidden(&book), [1], "only the 9");
    }

    #[test]
    fn two_comparisons_are_joined_the_way_the_filter_says() {
        let mut book = book();
        let between = |and: bool| FilterKind::Custom {
            first: Criterion {
                op: Compare::Greater,
                value: "15".into(),
            },
            second: Some((
                and,
                Criterion {
                    op: Compare::Less,
                    value: "35".into(),
                },
            )),
        };

        constrain(&mut book, 1, between(true));
        let change = apply(&book, 0);
        let undo = apply_change(&mut book, change);
        assert_eq!(hidden(&book), [1, 4], "20 and 30 survive an AND");
        apply_change(&mut book, undo);

        constrain(&mut book, 1, between(false));
        let change = apply(&book, 0);
        apply_change(&mut book, change);
        assert!(
            hidden(&book).is_empty(),
            "every row satisfies one half or the other"
        );
    }

    #[test]
    fn a_wildcard_is_a_wildcard_and_not_a_literal_asterisk() {
        let mut book = book();
        constrain(
            &mut book,
            0,
            FilterKind::Custom {
                first: Criterion {
                    op: Compare::Equal,
                    value: "*th".into(),
                },
                second: None,
            },
        );
        let change = apply(&book, 0);
        apply_change(&mut book, change);
        assert_eq!(
            hidden(&book),
            [4],
            "North and South end in th; East does not"
        );
    }

    #[test]
    fn wildcards_match_the_way_excel_spells_them() {
        assert!(wildcard("north", "n*"));
        assert!(wildcard("north", "*th"));
        assert!(wildcard("north", "n???h"));
        assert!(!wildcard("north", "n??h"));
        assert!(wildcard("north", "*"));
        assert!(wildcard("", "*"));
        assert!(!wildcard("", "?"));
        // The shape that makes a naive recursion take exponential time.
        assert!(!wildcard(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaab",
            "*a*a*a*a*a*a*c"
        ));
    }

    #[test]
    fn the_value_list_offered_is_what_the_column_actually_shows() {
        let book = book();
        let (values, blanks) = distinct(&book, 0, 0);
        assert_eq!(
            values.into_iter().collect::<Vec<_>>(),
            ["East", "North", "South"],
            "sorted, deduplicated, and without the heading"
        );
        assert!(!blanks);
    }

    #[test]
    fn clearing_shows_the_rows_and_keeps_the_arrows_removing_takes_both() {
        let mut book = book();
        constrain(
            &mut book,
            0,
            FilterKind::Values {
                values: ["North".to_string()].into_iter().collect(),
                blanks: false,
            },
        );
        let change = apply(&book, 0);
        apply_change(&mut book, change);
        assert!(!hidden(&book).is_empty());

        let change = clear(&book, 0);
        apply_change(&mut book, change);
        assert!(hidden(&book).is_empty());
        let filter = book.sheets[0].filter.as_ref().expect("arrows stay");
        assert!(!filter.is_filtering());

        let change = remove(&book, 0);
        apply_change(&mut book, change);
        assert!(book.sheets[0].filter.is_none());
    }

    #[test]
    fn a_row_outside_the_filters_range_keeps_whatever_it_had() {
        // Inside the range the filter decides; outside it, it has no opinion.
        // A footer row hidden by hand under the table must stay hidden.
        let mut book = book();
        book.sheets[0].row_heights.insert(9, 0.0);
        constrain(
            &mut book,
            0,
            FilterKind::Values {
                values: ["North".to_string()].into_iter().collect(),
                blanks: false,
            },
        );
        let change = apply(&book, 0);
        apply_change(&mut book, change);
        assert!(hidden(&book).contains(&9));
    }
}
