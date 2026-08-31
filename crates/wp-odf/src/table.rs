//! `<table:table>` and what is in it.
//!
//! Two things here are not what the other format does, and both of them decide
//! whether a table comes out with the right number of cells.
//!
//! **A column is repeated rather than restated.** `table:number-columns-repeated`
//! on a column, a cell or a whole row says "and this many more just like it",
//! which is how a fifty-column table is written in four elements. A reader that
//! took the attribute for decoration builds a table one column wide.
//!
//! **A spanned cell leaves a hole that is spelled out.** The cell that spans
//! carries `table:number-columns-spanned`, and every position it covers carries
//! a `<table:covered-table-cell/>` of its own. The model says the same thing
//! with a grid span and a vertical merge, and the covered cells are what tell
//! this reader which is which: a covered cell under a spanning one continues a
//! vertical merge, and a covered cell beside one is already accounted for by
//! the span and is dropped.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use wp_model::table::{Cell, CellProps, Row, RowProps, Table, TableProps, VMerge, Width};
use wp_model::units::Twips;

use crate::xml::{attr_in, end_local_name, local_name, skip_element};
use crate::Ctx;

/// Reads one `<table:table>`, whose start tag the caller has just seen.
pub fn read(reader: &mut Reader<&[u8]>, e: &BytesStart<'_>, ctx: &mut Ctx<'_>) -> Option<Table> {
    let props = attr_in(e, b"table", b"style-name")
        .and_then(|name| ctx.styles.tables.get(&name).cloned())
        .unwrap_or_default();
    let mut table = Table {
        props: TableProps {
            caption: attr_in(e, b"table", b"name").map(Into::into),
            ..props
        },
        grid: Vec::new(),
        rows: Vec::new(),
    };
    let mut columns: Vec<Width> = Vec::new();
    rows(reader, b"table", ctx, &mut table.rows, &mut columns);
    table.grid = grid(&columns);
    // A table with no rows at all is an element nothing can draw, and pushing
    // one into the body would put an empty box on the page.
    match table.rows.is_empty() {
        true => None,
        false => Some(table),
    }
}

/// The rows and columns between here and `end`, following the header and group
/// wrappers that may stand between.
fn rows(
    reader: &mut Reader<&[u8]>,
    end: &[u8],
    ctx: &mut Ctx<'_>,
    out: &mut Vec<Row>,
    columns: &mut Vec<Width>,
) {
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(_) => return,
        };
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                match name.as_slice() {
                    b"table-column" => {
                        let width = attr_in(&e, b"table", b"style-name")
                            .and_then(|name| ctx.styles.columns.get(&name).copied())
                            .unwrap_or(Width::Auto);
                        for _ in 0..repeat(&e, b"number-columns-repeated") {
                            columns.push(width);
                        }
                        if !empty {
                            skip_element(reader, &name);
                        }
                    }
                    b"table-row" => {
                        let row = row(reader, &e, empty, ctx);
                        let header = matches!(end, b"table-header-rows");
                        for _ in 0..repeat(&e, b"number-rows-repeated") {
                            let mut copy = row.clone();
                            copy.props.header = header;
                            out.push(copy);
                        }
                    }
                    // Wrappers that hold rows or columns and say nothing the
                    // model keeps: a header group, a column group, a row group.
                    b"table-header-rows"
                    | b"table-rows"
                    | b"table-columns"
                    | b"table-header-columns"
                    | b"table-column-group"
                    | b"table-row-group"
                        if !empty =>
                    {
                        rows(reader, &name, ctx, out, columns)
                    }
                    _ if !empty => skip_element(reader, &name),
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == end => return,
            Event::Eof => return,
            _ => {}
        }
    }
}

fn row(reader: &mut Reader<&[u8]>, e: &BytesStart<'_>, empty: bool, ctx: &mut Ctx<'_>) -> Row {
    let props = attr_in(e, b"table", b"style-name")
        .and_then(|name| ctx.styles.rows.get(&name).cloned())
        .unwrap_or_default();
    let mut row = Row {
        props: RowProps { ..props },
        cells: Vec::new(),
    };
    if empty {
        return row;
    }
    // How many more positions the cell most recently seen covers to its right.
    let mut covered = 0u32;
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(_) => return row,
        };
        let inner_empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                match name.as_slice() {
                    b"table-cell" => {
                        let times = repeat(&e, b"number-columns-repeated");
                        let span = repeat(&e, b"number-columns-spanned");
                        let rows_spanned = repeat(&e, b"number-rows-spanned");
                        let cell = cell(reader, &e, inner_empty, ctx, span, rows_spanned);
                        for _ in 0..times {
                            row.cells.push(cell.clone());
                        }
                        covered = (span - 1) * times;
                    }
                    b"covered-table-cell" => {
                        let times = repeat(&e, b"number-columns-repeated");
                        for _ in 0..times {
                            // Covered from the left is already inside the span
                            // of the cell that covers it, and adding a cell for
                            // it would widen the row. Covered from above is a
                            // row of its own and has to be there, or the rows
                            // below the merge shift left.
                            match covered > 0 {
                                true => covered -= 1,
                                false => row.cells.push(Cell {
                                    props: CellProps {
                                        v_merge: Some(VMerge::Continue),
                                        ..CellProps::default()
                                    },
                                    content: vec![empty_paragraph()],
                                }),
                            }
                        }
                        if !inner_empty {
                            skip_element(reader, &name);
                        }
                    }
                    _ if !inner_empty => skip_element(reader, &name),
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == b"table-row" => return row,
            Event::Eof => return row,
            _ => {}
        }
    }
}

fn cell(
    reader: &mut Reader<&[u8]>,
    e: &BytesStart<'_>,
    empty: bool,
    ctx: &mut Ctx<'_>,
    span: u32,
    rows_spanned: u32,
) -> Cell {
    let props = attr_in(e, b"table", b"style-name")
        .and_then(|name| ctx.styles.cells.get(&name).cloned())
        .unwrap_or_default();
    let mut cell = Cell {
        props: CellProps {
            grid_span: span.max(1),
            v_merge: match rows_spanned > 1 {
                true => Some(VMerge::Restart),
                false => None,
            },
            ..props
        },
        content: Vec::new(),
    };
    if !empty {
        cell.content = crate::content::blocks(reader, b"table-cell", ctx);
    }
    // A cell with nothing in it is still a cell with a height, and the layout
    // engine measures a cell by the paragraphs in it.
    if cell.content.is_empty() {
        cell.content.push(empty_paragraph());
    }
    cell
}

fn empty_paragraph() -> wp_model::doc::Block {
    wp_model::doc::Block::Paragraph(wp_model::doc::Paragraph::default())
}

/// A repeat count, which is one where the attribute is absent.
///
/// Capped, because the attribute is what ODF uses to say "to the end of the
/// sheet" in a spreadsheet and a stray large value in a text document would
/// otherwise be a hundred thousand cells nobody asked for.
fn repeat(e: &BytesStart<'_>, want: &[u8]) -> u32 {
    const MOST: u32 = 1024;
    attr_in(e, b"table", want)
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(1)
        .clamp(1, MOST)
}

/// The grid the model wants: one width per column, in twips.
///
/// A column stated as a share of the table rather than a length has no width
/// until the table is laid out, and the model's grid has nowhere to say so, so
/// a share becomes nothing and the layout divides what is left.
fn grid(columns: &[Width]) -> Vec<Twips> {
    columns
        .iter()
        .map(|width| match width {
            Width::Fixed(twips) => *twips,
            _ => Twips(0),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::{Container, TEXT_MIMETYPE};

    fn read_table(xml: &str) -> Table {
        let container = Container::empty(TEXT_MIMETYPE);
        let mut ctx = Ctx::for_tests(&container);
        let mut reader = Reader::from_str(xml);
        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) if local_name(&e) == b"table" => {
                    return read(&mut reader, &e, &mut ctx).expect("the table has rows")
                }
                Ok(Event::Eof) | Err(_) => panic!("no table in the test xml"),
                _ => {}
            }
        }
    }

    /// Four columns in one element, and a reader that missed it would build a
    /// table one column wide.
    #[test]
    fn a_repeated_column_is_that_many_columns() {
        let table = read_table(concat!(
            r#"<table:table table:name="T1">"#,
            r#"<table:table-column table:number-columns-repeated="4"/>"#,
            r#"<table:table-row><table:table-cell><text:p>a</text:p></table:table-cell></table:table-row>"#,
            r#"</table:table>"#
        ));
        assert_eq!(table.grid.len(), 4);
        assert_eq!(table.props.caption.as_deref(), Some("T1"));
    }

    /// The distinction the covered cells carry, and the one that decides
    /// whether the rows under a merge line up.
    #[test]
    fn a_cell_covered_from_the_left_is_inside_the_span_and_one_from_above_is_not() {
        let table = read_table(concat!(
            r#"<table:table>"#,
            r#"<table:table-column table:number-columns-repeated="3"/>"#,
            r#"<table:table-row>"#,
            r#"<table:table-cell table:number-columns-spanned="2" table:number-rows-spanned="2">"#,
            r#"<text:p>wide</text:p></table:table-cell>"#,
            r#"<table:covered-table-cell/>"#,
            r#"<table:table-cell><text:p>c</text:p></table:table-cell>"#,
            r#"</table:table-row>"#,
            r#"<table:table-row>"#,
            r#"<table:covered-table-cell table:number-columns-repeated="2"/>"#,
            r#"<table:table-cell><text:p>f</text:p></table:table-cell>"#,
            r#"</table:table-row>"#,
            r#"</table:table>"#
        ));
        assert_eq!(table.rows.len(), 2);
        // The first row has the spanning cell and the one beside it, and not a
        // third for the position the span already covers.
        assert_eq!(table.rows[0].cells.len(), 2);
        assert_eq!(table.rows[0].cells[0].props.grid_span, 2);
        assert_eq!(
            table.rows[0].cells[0].props.v_merge,
            Some(VMerge::Restart),
            "and it says the merge starts here"
        );
        // The second row keeps its covered cells, because the merge above them
        // has to be continued or everything after it shifts left.
        assert_eq!(table.rows[1].cells.len(), 3);
        assert_eq!(table.rows[1].cells[0].props.v_merge, Some(VMerge::Continue));
        assert_eq!(table.rows[1].cells[2].props.v_merge, None);
    }

    #[test]
    fn a_header_row_says_it_is_one() {
        let table = read_table(concat!(
            r#"<table:table>"#,
            r#"<table:table-column/>"#,
            r#"<table:table-header-rows><table:table-row>"#,
            r#"<table:table-cell><text:p>head</text:p></table:table-cell>"#,
            r#"</table:table-row></table:table-header-rows>"#,
            r#"<table:table-row><table:table-cell><text:p>body</text:p></table:table-cell></table:table-row>"#,
            r#"</table:table>"#
        ));
        assert_eq!(table.rows.len(), 2);
        assert!(table.rows[0].props.header);
        assert!(!table.rows[1].props.header);
    }
}
