//! Text to Columns and Remove Duplicates: two operations on a range that are
//! neither formulas nor formatting.
//!
//! Both are on Excel's Data tab and both do the same unusual thing — rewrite a
//! block of cells wholesale rather than change one — so they share this module
//! and the two rules that make that safe.
//!
//! **Read everything before writing anything.** The source of one row is the
//! destination of another, exactly as in a sort, so a pass that wrote as it
//! went would read cells it had already overwritten.
//!
//! **A formula that moves is rewritten.** A row lifted three places up carries
//! `=B9*2` into row 6, where it has to say `=B6*2`. That is the same
//! translation copying does, and [`crate::sort`] already owns it.

use std::collections::HashSet;

use ss_model::{Cell, CellRange, CellRef, CellValue, Sheet, StyleId, Workbook};

use crate::edit::{Change, Patch};

/// How a line of text is cut into fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Split {
    /// The characters that end a field. Excel offers tab, semicolon, comma,
    /// space, and one of your own.
    pub delimiters: Vec<char>,
    /// Whether a run of delimiters counts as one, which is what turns
    /// `Smith    John` — aligned with spaces — into two fields rather than
    /// five.
    pub merge: bool,
    /// The text qualifier: a quote around a field that holds a delimiter.
    pub quote: Option<char>,
}

impl Default for Split {
    fn default() -> Self {
        Split {
            delimiters: vec![','],
            merge: false,
            quote: Some('"'),
        }
    }
}

/// The most columns one split will write into.
///
/// A field per character is what a comma-delimited split of a long line of
/// commas produces, and the sheet is only sixteen thousand columns wide.
const MAX_FIELDS: usize = 256;

/// Cuts a single column of text into several columns.
///
/// Excel takes one column at a time and so does this: the fields land in the
/// column itself and the ones to its right, and two source columns would have
/// their results land on top of each other.
pub fn text_to_columns(
    book: &mut Workbook,
    sheet: usize,
    range: CellRange,
    how: &Split,
) -> Result<Change, String> {
    let Some(model) = book.sheet(sheet) else {
        return Ok(Change::default());
    };
    if range.cols() != 1 {
        return Err("Text to Columns takes one column at a time".to_string());
    }
    if how.delimiters.is_empty() {
        return Err("Choose at least one delimiter".to_string());
    }
    let Some(range) = crate::sort::clamped(model, range) else {
        return Ok(Change::default());
    };
    let col = range.start.col;

    // Cut everything first, so that the width to write is known before any of
    // it is written — and so that a split that would run off the sheet is
    // refused rather than half done.
    let mut rows: Vec<(u32, StyleId, Vec<String>)> = Vec::new();
    let mut widest = 1usize;
    for row in range.start.row..=range.end.row {
        let at = CellRef::new(row, col);
        let Some(cell) = model.get(at) else { continue };
        let CellValue::Text(id) = cell.value else {
            continue; // a number holds no fields, whatever is in it
        };
        let fields = fields(book.strings.resolve(id), how);
        if fields.len() > MAX_FIELDS {
            return Err(format!(
                "{} would split into {} columns, which is more than this does at once",
                at.to_a1(),
                fields.len()
            ));
        }
        widest = widest.max(fields.len());
        let style = model.get(at).map_or(StyleId::DEFAULT, |c| c.style);
        rows.push((row, style, fields));
    }
    if rows.is_empty() {
        return Ok(Change::default());
    }
    if u64::from(col) + widest as u64 > u64::from(ss_model::cell::MAX_COLS) {
        return Err("The split would run off the right edge of the sheet".to_string());
    }

    // Every field is re-read as if it had been typed, so that a column of
    // dates split out of a line comes back as dates rather than as text that
    // looks like dates.
    let mut cells: Vec<(CellRef, Option<Cell>)> = Vec::new();
    for (row, style, fields) in rows {
        for offset in 0..widest {
            let at = CellRef::new(row, col + offset as u32);
            match fields.get(offset) {
                Some(text) if !text.is_empty() => {
                    let cell = crate::edit::typed_cell(book, sheet, style, text);
                    cells.push((at, Some(cell)));
                }
                // Past the end of this line's fields: whatever was in the
                // column is part of what the split replaces.
                _ => cells.push((at, None)),
            }
        }
    }

    Ok(Change::new(
        "Text to columns",
        vec![Patch::Cells { sheet, cells }],
    ))
}

/// Cuts one line into fields.
pub fn fields(text: &str, how: &Split) -> Vec<String> {
    let mut out = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut previous_was_delimiter = true;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(quote) = how.quote {
            if c == quote {
                // A doubled quote inside a quoted field is one quote, which is
                // how a field says it contains the qualifier itself.
                if quoted && chars.peek() == Some(&quote) {
                    field.push(quote);
                    chars.next();
                } else {
                    quoted = !quoted;
                }
                previous_was_delimiter = false;
                continue;
            }
        }
        if !quoted && how.delimiters.contains(&c) {
            // Merging runs means a delimiter straight after another one ends
            // nothing, and leading delimiters are skipped for the same reason.
            if how.merge && previous_was_delimiter {
                continue;
            }
            out.push(std::mem::take(&mut field));
            previous_was_delimiter = true;
            continue;
        }
        field.push(c);
        previous_was_delimiter = false;
    }
    if !(how.merge && previous_was_delimiter) {
        out.push(field);
    }
    out
}

/// What a Remove Duplicates left behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Removed {
    pub removed: usize,
    pub kept: usize,
}

/// Removes rows of `range` whose `columns` repeat an earlier row's.
///
/// Only the range moves. Excel does the same: the cells below a removed row
/// come up within the selection and the rest of the sheet stays where it is,
/// which is why this is not a row deletion.
pub fn remove_duplicates(
    book: &mut Workbook,
    sheet: usize,
    range: CellRange,
    columns: &[u32],
    header: bool,
) -> Result<(Change, Removed), String> {
    let Some(model) = book.sheet(sheet) else {
        return Ok((Change::default(), Removed { removed: 0, kept: 0 }));
    };
    if columns.is_empty() {
        return Err("Choose at least one column to compare".to_string());
    }
    if model.merges.iter().any(|m| overlaps(*m, range)) {
        return Err("That range holds merged cells, which cannot be moved".to_string());
    }
    let Some(range) = crate::sort::clamped(model, range) else {
        return Ok((Change::default(), Removed { removed: 0, kept: 0 }));
    };
    let first = range.start.row + u32::from(header);
    if first > range.end.row {
        return Ok((Change::default(), Removed { removed: 0, kept: 0 }));
    }

    let mut seen: HashSet<Vec<Key>> = HashSet::new();
    let mut keep: Vec<u32> = Vec::new();
    for row in first..=range.end.row {
        let key: Vec<Key> = columns
            .iter()
            .map(|col| key_of(model, book, CellRef::new(row, *col)))
            .collect();
        if seen.insert(key) {
            keep.push(row);
        }
    }

    let removed = (range.end.row - first + 1) as usize - keep.len();
    let count = Removed {
        removed,
        kept: keep.len(),
    };
    if removed == 0 {
        return Ok((Change::default(), count));
    }

    // Read before writing: the row that stays is very often below the row it
    // is about to be written over.
    let taken: Vec<Vec<Option<Cell>>> = keep
        .iter()
        .map(|row| {
            (range.start.col..=range.end.col)
                .map(|col| book.sheets[sheet].get(CellRef::new(*row, col)).copied())
                .collect()
        })
        .collect();

    let mut cells: Vec<(CellRef, Option<Cell>)> = Vec::new();
    for (index, row_cells) in taken.into_iter().enumerate() {
        let to = first + index as u32;
        let delta = i64::from(to) - i64::from(keep[index]);
        for (offset, cell) in row_cells.into_iter().enumerate() {
            let at = CellRef::new(to, range.start.col + offset as u32);
            let cell = cell.map(|cell| crate::sort::moved(book, sheet, cell, delta));
            cells.push((at, cell));
        }
    }
    // The tail the survivors no longer reach.
    for row in (first + keep.len() as u32)..=range.end.row {
        for col in range.start.col..=range.end.col {
            cells.push((CellRef::new(row, col), None));
        }
    }

    Ok((
        Change::new("Remove duplicates", vec![Patch::Cells { sheet, cells }]),
        count,
    ))
}

/// What makes two cells the same cell for the purpose of finding a repeat.
///
/// Text compares without regard to case, as Excel's own does: a list holding
/// `Smith` and `SMITH` has one name in it twice.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Blank,
    /// The bit pattern, because a float is not `Eq` and two cells hold the same
    /// number exactly when they hold the same bits.
    Number(u64),
    Text(String),
    Bool(bool),
    Error(u8),
}

fn key_of(sheet: &Sheet, book: &Workbook, at: CellRef) -> Key {
    match sheet.get(at).map(|c| c.value) {
        None | Some(CellValue::Blank) => Key::Blank,
        // Negative zero and positive zero are the same number to everyone
        // except a bit pattern.
        Some(CellValue::Number(n)) => Key::Number(if n == 0.0 { 0 } else { n.to_bits() }),
        Some(CellValue::Text(id)) => Key::Text(book.strings.resolve(id).to_lowercase()),
        Some(CellValue::Bool(b)) => Key::Bool(b),
        Some(CellValue::Error(e)) => Key::Error(e as u8),
    }
}

fn overlaps(a: CellRange, b: CellRange) -> bool {
    a.start.row <= b.end.row
        && b.start.row <= a.end.row
        && a.start.col <= b.end.col
        && b.start.col <= a.end.col
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::CellValue;

    fn book_with(rows: &[&[&str]]) -> Workbook {
        let mut book = Workbook::blank();
        for (r, row) in rows.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                if text.is_empty() {
                    continue;
                }
                let cell = crate::edit::typed_cell(&mut book, 0, StyleId::DEFAULT, text);
                book.sheets[0].set(CellRef::new(r as u32, c as u32), cell);
            }
        }
        book
    }

    fn text_at(book: &Workbook, a1: &str) -> Option<String> {
        let at = CellRef::from_a1(a1)?;
        match book.sheets[0].get(at)?.value {
            CellValue::Text(id) => Some(book.strings.resolve(id).to_string()),
            CellValue::Number(n) => Some(ss_model::format_general(n)),
            _ => None,
        }
    }

    fn range(a: &str, b: &str) -> CellRange {
        CellRange::new(
            CellRef::from_a1(a).expect("valid"),
            CellRef::from_a1(b).expect("valid"),
        )
    }

    #[test]
    fn a_field_that_holds_the_delimiter_is_still_one_field() {
        let how = Split::default();
        assert_eq!(fields(r#"a,"b,c",d"#, &how), ["a", "b,c", "d"]);
        assert_eq!(fields(r#""he said ""hi""""#, &how), [r#"he said "hi""#]);
    }

    #[test]
    fn a_run_of_delimiters_is_one_only_when_asked() {
        let spaces = Split {
            delimiters: vec![' '],
            merge: false,
            quote: None,
        };
        assert_eq!(fields("Smith   John", &spaces), ["Smith", "", "", "John"]);
        let merged = Split {
            merge: true,
            ..spaces
        };
        assert_eq!(fields("Smith   John", &merged), ["Smith", "John"]);
        assert_eq!(
            fields("  Smith John  ", &merged),
            ["Smith", "John"],
            "leading and trailing runs are not empty fields either"
        );
    }

    #[test]
    fn a_split_column_lands_in_the_columns_to_its_right() {
        let mut book = book_with(&[&["a,b,c"], &["d,e"], &["f"]]);
        let change =
            text_to_columns(&mut book, 0, range("A1", "A3"), &Split::default()).expect("split");
        crate::edit::apply(&mut book, change);

        assert_eq!(text_at(&book, "A1").as_deref(), Some("a"));
        assert_eq!(text_at(&book, "B1").as_deref(), Some("b"));
        assert_eq!(text_at(&book, "C1").as_deref(), Some("c"));
        assert_eq!(text_at(&book, "B2").as_deref(), Some("e"));
        assert_eq!(
            book.sheets[0].get(CellRef::from_a1("C2").expect("valid")),
            None,
            "a short line leaves the columns past it empty"
        );
        assert_eq!(text_at(&book, "A3").as_deref(), Some("f"));
    }

    #[test]
    fn a_split_field_is_read_as_if_it_had_been_typed() {
        let mut book = book_with(&[&["Widget,12.5"]]);
        let change =
            text_to_columns(&mut book, 0, range("A1", "A1"), &Split::default()).expect("split");
        crate::edit::apply(&mut book, change);
        assert_eq!(
            book.sheets[0]
                .get(CellRef::from_a1("B1").expect("valid"))
                .map(|c| c.value),
            Some(CellValue::Number(12.5)),
            "a number that comes out of a split is a number"
        );
    }

    #[test]
    fn splitting_two_columns_at_once_is_refused_rather_than_guessed_at() {
        let mut book = book_with(&[&["a,b", "c,d"]]);
        assert!(text_to_columns(&mut book, 0, range("A1", "B1"), &Split::default()).is_err());
    }

    #[test]
    fn a_repeated_row_is_removed_and_the_rest_come_up() {
        let mut book = book_with(&[
            &["Name", "Town"],
            &["Ann", "Leeds"],
            &["Bob", "York"],
            &["ANN", "Leeds"],
            &["Cid", "Hull"],
        ]);
        let (change, count) =
            remove_duplicates(&mut book, 0, range("A1", "B5"), &[0, 1], true).expect("dedup");
        crate::edit::apply(&mut book, change);

        assert_eq!(count.removed, 1, "Ann and ANN are one name twice");
        assert_eq!(count.kept, 3);
        assert_eq!(text_at(&book, "A2").as_deref(), Some("Ann"));
        assert_eq!(text_at(&book, "A3").as_deref(), Some("Bob"));
        assert_eq!(text_at(&book, "A4").as_deref(), Some("Cid"), "moved up");
        assert_eq!(
            book.sheets[0].get(CellRef::from_a1("A5").expect("valid")),
            None,
            "and the row they came from is empty"
        );
    }

    #[test]
    fn comparing_one_column_ignores_what_the_others_say() {
        let mut book = book_with(&[
            &["Ann", "Leeds"],
            &["Ann", "York"],
            &["Bob", "Hull"],
        ]);
        let (_, count) =
            remove_duplicates(&mut book, 0, range("A1", "B3"), &[0], false).expect("dedup");
        assert_eq!(count.removed, 1, "two Anns, whatever town they live in");
    }

    #[test]
    fn a_formula_that_comes_up_says_what_it_means_where_it_lands() {
        let mut book = book_with(&[
            &["Ann", "1"],
            &["Ann", "2"],
            &["Bob", "3"],
        ]);
        // C3 doubles the number beside it; when its row moves up to row 2 the
        // reference has to come with it.
        let cell = crate::edit::typed_cell(&mut book, 0, StyleId::DEFAULT, "=B3*2");
        book.sheets[0].set(CellRef::new(2, 2), cell);

        let (change, _) =
            remove_duplicates(&mut book, 0, range("A1", "C3"), &[0], false).expect("dedup");
        crate::edit::apply(&mut book, change);

        let moved = book.sheets[0]
            .get(CellRef::from_a1("C2").expect("valid"))
            .and_then(|c| c.formula)
            .and_then(|id| book.sheets[0].formula(id))
            .map(|f| f.text.clone());
        assert_eq!(moved.as_deref(), Some("B2*2"));
    }

    #[test]
    fn a_list_with_nothing_repeated_is_not_a_change_at_all() {
        let mut book = book_with(&[&["Ann"], &["Bob"], &["Cid"]]);
        let (change, count) =
            remove_duplicates(&mut book, 0, range("A1", "A3"), &[0], false).expect("dedup");
        assert!(change.is_empty());
        assert_eq!(count.removed, 0);
        assert_eq!(count.kept, 3);
    }
}
