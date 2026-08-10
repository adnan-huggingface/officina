//! Rewriting formula text when the grid moves under it.
//!
//! Inserting a row is not a change to one cell. Every formula anywhere in the
//! workbook that points at or below the insertion point now points at the wrong
//! cell, and each one has to be rewritten so that it still refers to the *data*
//! it referred to before. Delete a row and some of those references have nothing
//! left to point at, which is what `#REF!` means.
//!
//! Two decisions shape this module.
//!
//! It works on the **token stream, not the parse tree**. A formula the user typed
//! is theirs: `=sum( A1 , A2 )` must come back with its spaces, its lowercase
//! `sum`, and its comma placement intact. Only the reference tokens are replaced,
//! and every other byte is copied through verbatim. Reprinting from an AST would
//! silently reformat a document the user never asked to reformat.
//!
//! Relative and absolute references shift **the same way**. That surprises people
//! who expect `$A$1` to be immovable, but the dollar signs control what happens
//! when a formula is *copied*, not what happens when the grid moves. Inserting a
//! row above row 1 moves the data that was in `$A$1` down to `$A$2`, so a formula
//! that meant that data has to say `$A$2` afterwards.

use ss_model::cell::{MAX_COLS, MAX_ROWS};
use ss_model::{column_name, Axis, Shift};

use crate::lexer::{tokenize, Tok, A1};

/// Rewrites `text` for `shift`, or returns `None` if nothing in it moved.
///
/// `home` is the sheet the formula lives on, which is what an unqualified
/// reference means. `text` carries no leading `=`.
///
/// Text that does not lex is returned unchanged: a formula we cannot read is one
/// we must not damage.
pub fn translate(text: &str, home: &str, moved: &str, shift: Shift) -> Option<String> {
    let tokens = tokenize(text).ok()?;
    let mut out = String::new();
    let mut copied = 0usize;
    let mut qualifier: Option<&str> = None;
    let mut i = 0;

    while i < tokens.len() {
        if let Tok::Sheet(name) = &tokens[i].kind {
            qualifier = Some(name);
            i += 1;
            continue;
        }

        let on_target = qualifier.unwrap_or(home).eq_ignore_ascii_case(moved);
        // A range is three tokens — `A1`, `:`, `B2` — and has to move as one
        // unit, because a deletion that swallows all of it leaves a single
        // `#REF!` rather than two.
        let range = matches!(&tokens[i].kind, Tok::Cell(_))
            && matches!(tokens.get(i + 1).map(|t| &t.kind), Some(Tok::Colon))
            && matches!(tokens.get(i + 2).map(|t| &t.kind), Some(Tok::Cell(_)));

        let (span, replacement) = if range {
            let (Tok::Cell(a), Tok::Cell(b)) = (&tokens[i].kind, &tokens[i + 2].kind) else {
                unreachable!("just matched")
            };
            (3, on_target.then(|| shift_range(*a, *b, shift)).flatten())
        } else {
            (
                1,
                on_target
                    .then(|| shift_single(&tokens[i].kind, shift))
                    .flatten(),
            )
        };

        if let Some(new_text) = replacement {
            let end = tokens[i + span - 1].end;
            out.push_str(&text[copied..tokens[i].at]);
            out.push_str(&new_text);
            copied = end;
        }
        qualifier = None;
        i += span;
    }

    if copied == 0 {
        return None;
    }
    out.push_str(&text[copied..]);
    Some(out)
}

/// Rewrites `text` as if the formula had been copied `rows` down and `cols`
/// right, or `None` if nothing in it moved.
///
/// This is the *other* translation, and the one `$` exists for. Copying a
/// formula moves its relative references by the distance it travelled and
/// leaves its absolute ones where they are — which is the whole reason a user
/// types a dollar sign. A reference pushed off the edge of the grid becomes
/// `#REF!`, exactly as Excel does when you copy `=A1` from B1 to A1.
pub fn offset(text: &str, rows: i64, cols: i64) -> Option<String> {
    if rows == 0 && cols == 0 {
        return None;
    }
    let tokens = tokenize(text).ok()?;
    let mut out = String::new();
    let mut copied = 0usize;

    for token in &tokens {
        let replacement = match &token.kind {
            Tok::Cell(cell) => {
                let moved = move_cell(*cell, rows, cols);
                let after = moved.as_ref().map_or(REF_ERROR.to_string(), write_cell);
                (after != write_cell(cell)).then_some(after)
            }
            Tok::ColSpan {
                start,
                start_abs,
                end,
                end_abs,
            } => {
                let before = write_col_span(*start, *start_abs, *end, *end_abs);
                let after = match (
                    nudge(*start, *start_abs, cols, MAX_COLS),
                    nudge(*end, *end_abs, cols, MAX_COLS),
                ) {
                    (Some(s), Some(e)) => write_col_span(s, *start_abs, e, *end_abs),
                    _ => REF_ERROR.to_string(),
                };
                (after != before).then_some(after)
            }
            Tok::RowSpan {
                start,
                start_abs,
                end,
                end_abs,
            } => {
                let before = write_row_span(*start, *start_abs, *end, *end_abs);
                let after = match (
                    nudge(*start, *start_abs, rows, MAX_ROWS),
                    nudge(*end, *end_abs, rows, MAX_ROWS),
                ) {
                    (Some(s), Some(e)) => write_row_span(s, *start_abs, e, *end_abs),
                    _ => REF_ERROR.to_string(),
                };
                (after != before).then_some(after)
            }
            _ => None,
        };

        if let Some(new_text) = replacement {
            out.push_str(&text[copied..token.at]);
            out.push_str(&new_text);
            copied = token.end;
        }
    }

    if copied == 0 {
        return None;
    }
    out.push_str(&text[copied..]);
    Some(out)
}

fn move_cell(cell: A1, rows: i64, cols: i64) -> Option<A1> {
    Some(A1 {
        row: nudge(cell.row, cell.row_abs, rows, MAX_ROWS)?,
        col: nudge(cell.col, cell.col_abs, cols, MAX_COLS)?,
        ..cell
    })
}

/// Moves one index unless it is anchored, refusing to leave the grid.
fn nudge(index: u32, anchored: bool, by: i64, limit: u32) -> Option<u32> {
    if anchored {
        return Some(index);
    }
    let moved = i64::from(index) + by;
    (0..i64::from(limit))
        .contains(&moved)
        .then_some(moved as u32)
}

/// A cell, whole-column span, or whole-row span on its own.
fn shift_single(tok: &Tok, shift: Shift) -> Option<String> {
    match tok {
        Tok::Cell(cell) => {
            let moved = match shift.axis {
                Axis::Rows => shift.point(cell.row).map(|row| A1 { row, ..*cell }),
                Axis::Columns => shift.point(cell.col).map(|col| A1 { col, ..*cell }),
            };
            let after = match moved {
                Some(a) => write_cell(&a),
                None => REF_ERROR.to_string(),
            };
            (after != write_cell(cell)).then_some(after)
        }
        Tok::ColSpan {
            start,
            start_abs,
            end,
            end_abs,
        } => {
            if shift.axis != Axis::Columns {
                return None;
            }
            let before = write_col_span(*start, *start_abs, *end, *end_abs);
            let after = match shift.span(*start, *end) {
                Some((s, e)) => write_col_span(s, *start_abs, e, *end_abs),
                None => REF_ERROR.to_string(),
            };
            (after != before).then_some(after)
        }
        Tok::RowSpan {
            start,
            start_abs,
            end,
            end_abs,
        } => {
            if shift.axis != Axis::Rows {
                return None;
            }
            let before = write_row_span(*start, *start_abs, *end, *end_abs);
            let after = match shift.span(*start, *end) {
                Some((s, e)) => write_row_span(s, *start_abs, e, *end_abs),
                None => REF_ERROR.to_string(),
            };
            (after != before).then_some(after)
        }
        _ => None,
    }
}

/// The three tokens of `A1:B2`, moved together.
fn shift_range(a: A1, b: A1, shift: Shift) -> Option<String> {
    // The lexer keeps the corners as written; the span logic wants them ordered.
    let (lo_row, hi_row) = (a.row.min(b.row), a.row.max(b.row));
    let (lo_col, hi_col) = (a.col.min(b.col), a.col.max(b.col));

    let (mut a2, mut b2) = (a, b);
    match shift.axis {
        Axis::Rows => {
            let Some((lo, hi)) = shift.span(lo_row, hi_row) else {
                return Some(REF_ERROR.to_string());
            };
            a2.row = if a.row == lo_row { lo } else { hi };
            b2.row = if b.row == lo_row { lo } else { hi };
        }
        Axis::Columns => {
            let Some((lo, hi)) = shift.span(lo_col, hi_col) else {
                return Some(REF_ERROR.to_string());
            };
            a2.col = if a.col == lo_col { lo } else { hi };
            b2.col = if b.col == lo_col { lo } else { hi };
        }
    }

    let before = format!("{}:{}", write_cell(&a), write_cell(&b));
    let after = format!("{}:{}", write_cell(&a2), write_cell(&b2));
    (after != before).then_some(after)
}

/// Rewrites every qualifier naming `from` so that it names `to`.
///
/// Renaming a sheet has to reach every formula in the workbook, because a
/// qualifier is the sheet's *name* and nothing else — there is no id behind it
/// to keep the link alive. A formula left saying `Data!A1` after Data became
/// Sales does not point somewhere wrong; it points nowhere, and comes back
/// `#NAME?`.
///
/// Only qualifiers are touched. `Data` appearing as a defined name, inside a
/// string, or as part of a longer word is a different thing with the same
/// spelling, and the token stream is what tells them apart.
pub fn rename_sheet(text: &str, from: &str, to: &str) -> Option<String> {
    let tokens = tokenize(text).ok()?;
    let mut out = String::new();
    let mut copied = 0usize;

    for token in &tokens {
        let Tok::Sheet(name) = &token.kind else {
            continue;
        };
        if !name.eq_ignore_ascii_case(from) {
            continue;
        }
        out.push_str(&text[copied..token.at]);
        out.push_str(&quote_sheet(to));
        out.push('!');
        copied = token.end;
    }

    if copied == 0 {
        return None;
    }
    out.push_str(&text[copied..]);
    Some(out)
}

/// Replaces every reference into `sheet` with `#REF!`.
///
/// The whole reference goes, qualifier and address together. Excel writes
/// `#REF!A1` — it keeps the address so you can see what was lost — but that is
/// not a formula any engine can evaluate: `#REF!` is an error literal, and an
/// address cannot follow one. Ours evaluates, and evaluating to `#REF!` is
/// exactly what the cell should say.
pub fn drop_sheet(text: &str, sheet: &str) -> Option<String> {
    let tokens = tokenize(text).ok()?;
    let mut out = String::new();
    let mut copied = 0usize;
    let mut i = 0;

    while i < tokens.len() {
        let Tok::Sheet(name) = &tokens[i].kind else {
            i += 1;
            continue;
        };
        if !name.eq_ignore_ascii_case(sheet) {
            i += 1;
            continue;
        }
        // The qualifier plus whatever it qualifies: a cell, a range's three
        // tokens, or a whole-column span. A qualifier with nothing after it is
        // not something the lexer produces, but the arithmetic must not assume
        // that.
        let mut span = 1;
        if matches!(
            tokens.get(i + 1).map(|t| &t.kind),
            Some(Tok::Cell(_) | Tok::ColSpan { .. } | Tok::RowSpan { .. })
        ) {
            span = 2;
            if matches!(tokens.get(i + 1).map(|t| &t.kind), Some(Tok::Cell(_)))
                && matches!(tokens.get(i + 2).map(|t| &t.kind), Some(Tok::Colon))
                && matches!(tokens.get(i + 3).map(|t| &t.kind), Some(Tok::Cell(_)))
            {
                span = 4;
            }
        }
        out.push_str(&text[copied..tokens[i].at]);
        out.push_str(REF_ERROR);
        copied = tokens[i + span - 1].end;
        i += span;
    }

    if copied == 0 {
        return None;
    }
    out.push_str(&text[copied..]);
    Some(out)
}

/// A sheet name as a formula would have to spell it.
///
/// Anything that is not a plain identifier gets quoted, and an apostrophe
/// inside a quoted name is doubled — the same escape the lexer already undoes.
pub fn quote_sheet(name: &str) -> String {
    let plain = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.');
    // A name that reads as a cell address has to be quoted even though it is a
    // plain identifier, or `A1!B2` becomes ambiguous to a human even where the
    // lexer copes.
    let address_shaped = ss_model::CellRef::from_a1(name).is_some();
    if plain && !address_shaped {
        return name.to_string();
    }
    format!("'{}'", name.replace('\'', "''"))
}

const REF_ERROR: &str = "#REF!";

fn write_cell(cell: &A1) -> String {
    format!(
        "{}{}{}{}",
        if cell.col_abs { "$" } else { "" },
        column_name(cell.col),
        if cell.row_abs { "$" } else { "" },
        cell.row + 1
    )
}

fn write_col_span(start: u32, start_abs: bool, end: u32, end_abs: bool) -> String {
    format!(
        "{}{}:{}{}",
        if start_abs { "$" } else { "" },
        column_name(start),
        if end_abs { "$" } else { "" },
        column_name(end)
    )
}

fn write_row_span(start: u32, start_abs: bool, end: u32, end_abs: bool) -> String {
    format!(
        "{}{}:{}{}",
        if start_abs { "$" } else { "" },
        start + 1,
        if end_abs { "$" } else { "" },
        end + 1
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn here(text: &str, shift: Shift) -> String {
        translate(text, "Sheet1", "Sheet1", shift).unwrap_or_else(|| text.to_string())
    }

    fn rows_inserted(text: &str, at: u32, count: u32) -> String {
        here(text, Shift::insert(Axis::Rows, at, count))
    }

    fn rows_deleted(text: &str, at: u32, count: u32) -> String {
        here(text, Shift::delete(Axis::Rows, at, count))
    }

    fn cols_deleted(text: &str, at: u32, count: u32) -> String {
        here(text, Shift::delete(Axis::Columns, at, count))
    }

    #[test]
    fn a_reference_follows_the_data_it_named() {
        assert_eq!(rows_inserted("A1+1", 0, 1), "A2+1");
        assert_eq!(rows_inserted("A5", 9, 1), "A5", "below the insertion point");
    }

    #[test]
    fn dollars_do_not_pin_a_reference_against_a_moving_grid() {
        // `$` governs copying, not the grid moving underneath.
        assert_eq!(rows_inserted("$A$1", 0, 1), "$A$2");
        assert_eq!(cols_deleted("$C$4", 0, 1), "$B$4");
    }

    #[test]
    fn inserting_inside_a_range_stretches_it() {
        assert_eq!(rows_inserted("SUM(A1:A10)", 4, 1), "SUM(A1:A11)");
        assert_eq!(rows_inserted("SUM(A1:A10)", 0, 1), "SUM(A2:A11)");
        assert_eq!(rows_inserted("SUM(A1:A10)", 10, 1), "SUM(A1:A10)");
    }

    #[test]
    fn deleting_part_of_a_range_shrinks_it() {
        assert_eq!(rows_deleted("SUM(A3:A8)", 0, 5), "SUM(A1:A3)");
        assert_eq!(rows_deleted("SUM(A1:A10)", 2, 3), "SUM(A1:A7)");
    }

    #[test]
    fn deleting_everything_a_formula_named_is_a_ref_error() {
        assert_eq!(rows_deleted("SUM(A2:A4)", 1, 3), "SUM(#REF!)");
        assert_eq!(rows_deleted("A3*2", 2, 1), "#REF!*2");
        // One `#REF!` for the range, not one per corner.
        assert_eq!(rows_deleted("A2:A4", 0, 9), "#REF!");
    }

    #[test]
    fn the_users_own_text_survives_byte_for_byte() {
        // Spacing, case, and argument separators are the user's, not ours.
        assert_eq!(rows_inserted("sum( A1 , A9 )", 0, 1), "sum( A2 , A10 )");
        assert_eq!(
            rows_inserted("IF(A1>0,\"A1 is big\",B1)", 0, 1),
            "IF(A2>0,\"A1 is big\",B2)",
            "a reference inside a string is text, not a reference"
        );
    }

    #[test]
    fn only_the_sheet_that_moved_is_rewritten() {
        let shift = Shift::insert(Axis::Rows, 0, 1);
        assert_eq!(
            translate("Data!A1+A1", "Sheet1", "Data", shift).unwrap(),
            "Data!A2+A1"
        );
        assert_eq!(
            translate("A1+Data!A1", "Data", "Data", shift).unwrap(),
            "A2+Data!A2",
            "unqualified references belong to the formula's own sheet"
        );
        assert_eq!(translate("A1+B2", "Sheet1", "Data", shift), None);
    }

    #[test]
    fn whole_columns_and_rows_move_on_their_own_axis_only() {
        assert_eq!(cols_deleted("SUM(B:D)", 0, 1), "SUM(A:C)");
        assert_eq!(rows_deleted("SUM(B:D)", 0, 1), "SUM(B:D)");
        assert_eq!(rows_inserted("SUM(2:5)", 2, 2), "SUM(2:7)");
        assert_eq!(cols_deleted("SUM(2:5)", 0, 4), "SUM(2:5)");
        assert_eq!(cols_deleted("SUM(B:D)", 1, 3), "SUM(#REF!)");
    }

    #[test]
    fn copying_moves_relative_references_and_leaves_anchored_ones() {
        assert_eq!(offset("A1+$B$1", 2, 1).unwrap(), "B3+$B$1");
        assert_eq!(offset("$A1+A$1", 2, 1).unwrap(), "$A3+B$1");
        assert_eq!(offset("SUM(A1:A10)", 1, 0).unwrap(), "SUM(A2:A11)");
        assert_eq!(offset("A1", 0, 0), None);
    }

    #[test]
    fn copying_off_the_edge_of_the_grid_is_a_ref_error() {
        assert_eq!(offset("A1", 0, -1).unwrap(), "#REF!");
        assert_eq!(offset("B2", -5, 0).unwrap(), "#REF!");
        assert_eq!(offset("$A$1", 0, -1), None, "anchored, so it never left");
    }

    #[test]
    fn copying_leaves_everything_that_is_not_a_reference_alone() {
        assert_eq!(
            offset("IF(A1>0,\"stay A1\",SUM(B:B))", 1, 0).unwrap(),
            "IF(A2>0,\"stay A1\",SUM(B:B))"
        );
    }

    #[test]
    fn renaming_a_sheet_follows_every_qualifier_and_nothing_else() {
        assert_eq!(
            rename_sheet("Data!A1+Data!B2*2", "Data", "Sales").unwrap(),
            "Sales!A1+Sales!B2*2"
        );
        assert_eq!(
            rename_sheet("'Old Name'!A1", "Old Name", "New").unwrap(),
            "New!A1"
        );
        assert_eq!(
            rename_sheet("Data!A1", "Data", "Q1 Results").unwrap(),
            "'Q1 Results'!A1",
            "a name with a space has to be quoted on the way in"
        );
        assert_eq!(
            rename_sheet("Data!A1", "Data", "Bob's").unwrap(),
            "'Bob''s'!A1",
            "and an apostrophe doubled"
        );
        assert_eq!(
            rename_sheet("data!A1", "DATA", "X").unwrap(),
            "X!A1",
            "sheet names compare case-insensitively"
        );
        assert_eq!(
            rename_sheet("Data+\"Data!A1\"+Datastore!A1", "Data", "X"),
            None,
            "a defined name, a string, and a longer name are not qualifiers"
        );
    }

    #[test]
    fn a_name_shaped_like_a_cell_is_quoted() {
        // `Q1!A5` is legal and the lexer reads it, but `'Q1'!A5` is what Excel
        // writes and is the only spelling a reader cannot misread.
        assert_eq!(quote_sheet("Q1"), "'Q1'");
        assert_eq!(quote_sheet("Sheet1"), "Sheet1");
        assert_eq!(quote_sheet("Q1 Results"), "'Q1 Results'");
    }

    #[test]
    fn deleting_a_sheet_takes_the_whole_reference_with_it() {
        assert_eq!(drop_sheet("Data!A1+1", "Data").unwrap(), "#REF!+1");
        assert_eq!(
            drop_sheet("SUM(Data!A1:B9)", "Data").unwrap(),
            "SUM(#REF!)",
            "one #REF! for the range, not one per corner"
        );
        assert_eq!(drop_sheet("SUM(Data!A:A)", "Data").unwrap(), "SUM(#REF!)");
        assert_eq!(
            drop_sheet("Data!A1+Other!A1", "Data").unwrap(),
            "#REF!+Other!A1",
            "only the sheet that went"
        );
        assert_eq!(drop_sheet("A1+B2", "Data"), None);
        // What is left has to be a formula the engine can actually evaluate.
        assert!(tokenize(&drop_sheet("Data!A1+1", "Data").unwrap()).is_ok());
    }

    #[test]
    fn a_formula_we_cannot_read_is_left_alone() {
        assert_eq!(
            translate("SUM(", "Sheet1", "Sheet1", Shift::insert(Axis::Rows, 0, 1)),
            None
        );
    }
}
