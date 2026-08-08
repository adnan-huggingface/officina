//! Between a delimited file and a [`Sheet`].
//!
//! The interesting decision is that a field is *interpreted*, not stored as
//! text: `5` becomes a number, `2024-01-15` becomes a date carrying a date
//! format, and `=A1+1` becomes a formula. The same interpretation typing into a
//! cell gets, because an import that produced a sheet of strings would be
//! useless for the one thing anyone imports a csv to do. It also means `007`
//! arrives as 7, which is Excel's behaviour and is what matching Excel costs.
//!
//! That interpretation lives in `ss-formula`, which this crate does not depend
//! on, so it arrives as a callback. Keeping it out is what lets `ss-csv` be
//! about delimiters and encodings and nothing else.

use std::io::{self, BufRead, Write};

use ss_model::{CellRef, Sheet};

use crate::{write_record, Dialect, Reader};

/// What an import turned out to contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Imported {
    pub rows: u32,
    pub columns: u32,
    /// Rows past the sheet's limit, which were read and dropped. Reported
    /// rather than silently ignored: a user importing a two-million-row export
    /// needs to be told half of it is not there.
    pub truncated: u64,
}

/// The number of rows a spreadsheet has, past which nothing can be stored.
const MAX_ROWS: u32 = ss_model::cell::MAX_ROWS;
const MAX_COLS: u32 = ss_model::cell::MAX_COLS;

/// Reads every record, handing each one to `row` as a slice of fields.
///
/// A callback rather than a `&mut Sheet` on purpose. Interpreting a field needs
/// the whole workbook — a date allocates a number format, a formula goes into
/// the sheet's arena — and a signature that borrowed the sheet out of the book
/// would make that impossible to express. The caller writes the cells; this
/// only decides what a record is.
pub fn read_into<R: BufRead>(
    reader: &mut Reader<R>,
    mut row: impl FnMut(u32, &[String]),
) -> io::Result<Imported> {
    let mut out = Imported::default();
    let mut index = 0u32;
    let mut fields: Vec<String> = Vec::new();
    while reader.next_record()? {
        if index >= MAX_ROWS {
            out.truncated += 1;
            continue;
        }
        fields.clear();
        fields.extend(reader.record().take(MAX_COLS as usize).map(str::to_string));
        out.columns = out.columns.max(fields.len() as u32);
        row(index, &fields);
        index += 1;
        out.rows = index;
    }
    Ok(out)
}

/// Writes the sheet's used range out, one record per row.
///
/// `text_of` gives a cell's text, which is the caller's because it needs the
/// workbook's string table and number formats. What goes out is the *displayed*
/// text — `15-Jan-24` rather than `45306` — because that is what a csv is for.
pub fn write_sheet<W: Write>(
    out: &mut W,
    sheet: &Sheet,
    dialect: Dialect,
    text_of: impl Fn(CellRef) -> String,
) -> io::Result<()> {
    let Some((start, end)) = sheet.cells.used_range() else {
        return Ok(());
    };
    // From A1 rather than from the used range's corner: a csv has no notion of
    // an offset, so starting at the first used cell would silently move every
    // column left.
    let mut row_text: Vec<String> = Vec::new();
    for row in 0..=end.row {
        row_text.clear();
        for col in 0..=end.col {
            row_text.push(text_of(CellRef::new(row, col)));
        }
        // Trailing empties are dropped: they carry no data and Excel does not
        // write them either.
        while row_text.last().is_some_and(|f| f.is_empty()) {
            row_text.pop();
        }
        write_record(out, &row_text, dialect)?;
    }
    let _ = start;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Encoding;
    use ss_model::{Cell, CellValue};
    use std::io::Cursor;

    /// Collects what the callback is handed, which is all this module decides.
    fn rows(text: &str) -> (Vec<Vec<String>>, Imported) {
        let mut reader = Reader::new(
            Cursor::new(text.as_bytes()),
            Encoding::Utf8,
            Dialect::default(),
        );
        let mut seen = Vec::new();
        let stats = read_into(&mut reader, |index, fields| {
            assert_eq!(index as usize, seen.len(), "rows arrive in order");
            seen.push(fields.to_vec());
        })
        .expect("reads");
        (seen, stats)
    }

    #[test]
    fn a_ragged_file_keeps_every_row_and_reports_the_widest() {
        let (seen, stats) = rows("a,b,c\n1,2\n3,4,5,6\n");
        assert_eq!(stats.rows, 3);
        assert_eq!(stats.columns, 4);
        assert_eq!(seen[1], vec!["1", "2"], "row 2 has two fields, not three");
        assert_eq!(seen[2].len(), 4);
    }

    #[test]
    fn an_empty_field_arrives_as_an_empty_string() {
        // What the caller does with it is the caller's; what matters here is
        // that the *position* survives, because that is the column number.
        let (seen, _) = rows("a,,c\n");
        assert_eq!(seen[0], vec!["a", "", "c"]);
    }

    #[test]
    fn writing_starts_at_a1_and_drops_trailing_empties() {
        let mut sheet = Sheet::new("s");
        sheet.set(
            CellRef::new(1, 1),
            Cell {
                value: CellValue::Number(7.0),
                ..Default::default()
            },
        );
        let mut out = Vec::new();
        write_sheet(
            &mut out,
            &sheet,
            Dialect {
                newline: crate::Newline::Lf,
                ..Dialect::default()
            },
            |at| {
                if at == CellRef::new(1, 1) {
                    "7".to_string()
                } else {
                    String::new()
                }
            },
        )
        .expect("writes");
        // Row 1 is empty and row 2 has an empty first field: both are position,
        // which a csv has no other way of expressing.
        assert_eq!(String::from_utf8(out).expect("utf-8"), "\n,7\n");
    }
}
