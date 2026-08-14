//! Tables: a grid of columns, a list of rows, and two different ways a cell can
//! cover more than one square of it.
//!
//! The shape is not a rectangle of cells. `<w:tbl>` holds a `<w:tblGrid>` of
//! column widths and then rows, and a row's cells do not have to add up to the
//! grid, do not have to be the same in number from row to row, and may start
//! part way across it. Two mechanisms cover several columns or rows:
//!
//! - **`<w:gridSpan>`** — this cell is *n* grid columns wide. Horizontal, and
//!   local to its row.
//! - **`<w:vMerge>`** — this cell continues the one above. Vertical, and spread
//!   across rows that each still list a cell.
//!
//! The second carries the format's sharpest trap. A bare `<w:vMerge/>` means
//! **continue**, not restart — the opposite of every other bare on/off element
//! in WordprocessingML, where absence of `w:val` means *true*. Read the usual
//! way, every vertically merged cell in a document becomes the start of its own
//! merge, which draws as a table full of empty cells where the merged text used
//! to be.

use std::sync::Arc;

use crate::prop::{Border, Justify, Shading};
use crate::revision::Revision;
use crate::style::StyleId;
use crate::units::{Pct50, Twips};

/// A width, and the four things `w:type` can say it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Width {
    /// `auto` — decided by the content. Also what a `w:w="0"` with `type="auto"`
    /// means, which is what Word writes for an ordinary table.
    #[default]
    Auto,
    /// `dxa` — twips.
    Fixed(Twips),
    /// `pct` — fiftieths of a percent of the text column.
    Percent(Pct50),
    /// `nil` — no width at all, which is not the same as auto: a nil cell is
    /// zero wide.
    Nil,
}

impl Width {
    pub fn from_parts(kind: &str, value: i32) -> Width {
        match kind {
            "dxa" => Width::Fixed(Twips(value)),
            "pct" => Width::Percent(Pct50(value)),
            "nil" => Width::Nil,
            // `auto` and anything unrecognised.
            _ => Width::Auto,
        }
    }

    /// The width in twips, given what it might be a percentage of. `None` for
    /// `auto`, which only the layout can answer.
    pub fn resolve(self, available: Twips) -> Option<Twips> {
        match self {
            Width::Auto => None,
            Width::Fixed(twips) => Some(twips),
            Width::Percent(pct) => Some(pct.of(available)),
            Width::Nil => Some(Twips(0)),
        }
    }
}

/// The six borders a table or a cell may state.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TableBorders {
    pub top: Option<Border>,
    pub start: Option<Border>,
    pub bottom: Option<Border>,
    pub end: Option<Border>,
    /// Between rows. On a *cell* this is `tl2br`, a diagonal, and means
    /// something else entirely — which is why the reader has to know whose
    /// borders it is reading.
    pub inside_h: Option<Border>,
    /// Between columns.
    pub inside_v: Option<Border>,
}

impl TableBorders {
    pub fn is_empty(&self) -> bool {
        *self == TableBorders::default()
    }
}

/// `<w:tblCellMar>` — the padding inside every cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellMargins {
    pub top: Option<Width>,
    pub start: Option<Width>,
    pub bottom: Option<Width>,
    pub end: Option<Width>,
}

impl CellMargins {
    /// Word's own default: no padding above or below, and 0.08in either side.
    pub fn word_default() -> CellMargins {
        CellMargins {
            top: Some(Width::Fixed(Twips(0))),
            start: Some(Width::Fixed(Twips(108))),
            bottom: Some(Width::Fixed(Twips(0))),
            end: Some(Width::Fixed(Twips(108))),
        }
    }
}

/// `<w:tblLayout>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableLayout {
    /// Column widths come from the content, and the grid is a starting guess.
    /// Word's default, and the expensive one to lay out.
    #[default]
    Auto,
    /// The grid is the answer. Content that does not fit wraps or is clipped.
    Fixed,
}

/// `<w:tblLook>` — which conditional bands of the table style are drawn.
///
/// Written twice over: a legacy hex bitmask in `w:val`, and six explicit
/// attributes beside it. Word writes both and reads the attributes; a file from
/// an older producer has only the mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TableLook {
    pub first_row: bool,
    pub last_row: bool,
    pub first_column: bool,
    pub last_column: bool,
    /// Note the sense: these say *no* banding, matching the file's own
    /// `w:noHBand`. Storing them the other way round would read better and
    /// invert every table whose attribute is absent.
    pub no_h_band: bool,
    pub no_v_band: bool,
}

impl TableLook {
    /// Reads the legacy `w:val` mask.
    pub fn from_mask(mask: u32) -> TableLook {
        TableLook {
            first_row: mask & 0x0020 != 0,
            last_row: mask & 0x0040 != 0,
            first_column: mask & 0x0080 != 0,
            last_column: mask & 0x0100 != 0,
            no_h_band: mask & 0x0200 != 0,
            no_v_band: mask & 0x0400 != 0,
        }
    }

    pub fn to_mask(self) -> u32 {
        let bit = |on: bool, mask: u32| if on { mask } else { 0 };
        bit(self.first_row, 0x0020)
            | bit(self.last_row, 0x0040)
            | bit(self.first_column, 0x0080)
            | bit(self.last_column, 0x0100)
            | bit(self.no_h_band, 0x0200)
            | bit(self.no_v_band, 0x0400)
    }
}

/// `<w:tblpPr>` — a table that floats, with text wrapping round it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TableFloat {
    pub x: Option<Twips>,
    pub y: Option<Twips>,
    /// `w:horzAnchor` / `w:vertAnchor` — margin, page, or text.
    pub horizontal_anchor: FloatAnchor,
    pub vertical_anchor: FloatAnchor,
    pub left_from_text: Option<Twips>,
    pub right_from_text: Option<Twips>,
    pub top_from_text: Option<Twips>,
    pub bottom_from_text: Option<Twips>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FloatAnchor {
    #[default]
    Text,
    Margin,
    Page,
}

/// `<w:tblPr>`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableProps {
    /// `<w:tblStyle>` — the table style, which is a whole conditional formatting
    /// scheme rather than a set of properties. What it contributes depends on
    /// [`TableLook`] and on where a cell sits.
    pub style: Option<StyleId>,
    pub width: Width,
    pub justify: Option<Justify>,
    /// `<w:tblInd>` — how far in from the margin the table starts.
    pub indent: Option<Width>,
    pub borders: TableBorders,
    pub shading: Option<Shading>,
    pub layout: TableLayout,
    pub look: TableLook,
    pub cell_margins: CellMargins,
    /// `<w:tblCellSpacing>` — space *between* cells, which draws the borders
    /// twice with a gap. Rare, and visible when it is there.
    pub cell_spacing: Option<Width>,
    pub float: Option<Box<TableFloat>>,
    /// `<w:tblCaption>` and `<w:tblDescription>` — accessibility text.
    pub caption: Option<Arc<str>>,
    pub description: Option<Arc<str>>,
    pub bidi_visual: bool,
}

/// `<w:trPr><w:trHeight>` — a row height and the rule that governs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowHeight {
    /// `auto` — the content decides. The value beside it, if any, is ignored.
    Auto,
    /// `atLeast` — grows for taller content.
    AtLeast(Twips),
    /// `exact` — content past it is clipped.
    Exact(Twips),
}

/// `<w:trPr>`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RowProps {
    pub height: Option<RowHeight>,
    /// `<w:cantSplit>` — the row may not break across a page.
    pub cant_split: bool,
    /// `<w:tblHeader>` — repeat this row at the top of every page the table
    /// covers. Only meaningful on rows at the start of the table: Word stops
    /// repeating at the first row that does not say it.
    pub header: bool,
    pub cell_spacing: Option<Width>,
    pub justify: Option<Justify>,
    /// `<w:gridBefore>` / `<w:gridAfter>` — grid columns this row leaves empty
    /// at its start and end. A row that begins part way across the table is
    /// spelled this way and not with empty cells, so a reader that ignores them
    /// draws every following cell one column too far left.
    pub grid_before: u32,
    pub grid_after: u32,
    pub width_before: Option<Width>,
    pub width_after: Option<Width>,
    /// A row inserted or deleted with track changes on. Held on the row rather
    /// than in the cells, because that is where the file puts it.
    pub revision: Option<Revision>,
}

/// `<w:vMerge>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VMerge {
    /// `w:val="restart"` — this cell begins a vertical merge.
    Restart,
    /// A bare `<w:vMerge/>`, or `w:val="continue"`. **The bare element means
    /// this**, against the format's own convention everywhere else.
    Continue,
}

impl VMerge {
    /// Reads the `w:val`, with the inversion that costs a whole table.
    pub fn from_val(val: Option<&str>) -> VMerge {
        match val {
            Some("restart") => VMerge::Restart,
            // `continue`, anything unrecognised, and — the trap — nothing.
            _ => VMerge::Continue,
        }
    }
}

/// Which way the text runs inside a cell — `<w:textDirection>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDirection {
    /// Left to right, top to bottom.
    #[default]
    Horizontal,
    /// Rotated 90° clockwise — reading down the right side. `tbRl`/`vert`.
    RotatedDown,
    /// Rotated 90° anticlockwise — reading up the left side. `btLr`/`vert270`.
    RotatedUp,
}

impl TextDirection {
    pub fn from_val(text: &str) -> Option<TextDirection> {
        Some(match text {
            "lrTb" | "lrTbV" | "horz" => TextDirection::Horizontal,
            "tbRl" | "tbRlV" | "vert" => TextDirection::RotatedDown,
            "btLr" | "vert270" => TextDirection::RotatedUp,
            _ => return None,
        })
    }

    pub const fn is_rotated(self) -> bool {
        !matches!(self, TextDirection::Horizontal)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CellVAlign {
    #[default]
    Top,
    Center,
    Bottom,
}

impl CellVAlign {
    pub fn from_val(text: &str) -> Option<CellVAlign> {
        Some(match text {
            "top" => CellVAlign::Top,
            "center" | "ctr" => CellVAlign::Center,
            "bottom" | "bot" => CellVAlign::Bottom,
            _ => return None,
        })
    }
}

/// `<w:tcPr>`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CellProps {
    pub width: Width,
    /// `<w:gridSpan>`. Absent is 1, and zero — which malformed files contain —
    /// is treated as 1 rather than as a cell that occupies nothing.
    pub grid_span: u32,
    pub v_merge: Option<VMerge>,
    pub borders: TableBorders,
    /// A cell's `tl2br` and `br2tl` diagonals, which share the borders element
    /// with the four edges.
    pub diagonal_down: Option<Border>,
    pub diagonal_up: Option<Border>,
    pub shading: Option<Shading>,
    pub margins: CellMargins,
    pub v_align: CellVAlign,
    pub text_direction: TextDirection,
    /// `<w:noWrap>` — do not wrap, widen the column instead.
    pub no_wrap: bool,
    /// `<w:tcFitText>` — squeeze the text to fit the cell rather than wrapping.
    pub fit_text: bool,
    /// `<w:hideMark>` — the end-of-cell mark does not contribute height, which
    /// is how an empty cell in a merged column stays flat.
    pub hide_mark: bool,
}

impl CellProps {
    pub fn new() -> CellProps {
        CellProps {
            grid_span: 1,
            ..CellProps::default()
        }
    }

    /// How many grid columns this cell covers. Never zero.
    pub fn span(&self) -> u32 {
        self.grid_span.max(1)
    }

    /// Whether this cell is the continuation of one above rather than a cell in
    /// its own right.
    pub fn is_merged_up(&self) -> bool {
        matches!(self.v_merge, Some(VMerge::Continue))
    }
}

/// One `<w:tc>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub props: CellProps,
    /// Paragraphs, and nested tables. **A cell always ends with a paragraph** —
    /// the format requires it, and a table as the last thing in a cell is
    /// followed by an empty one. A writer that drops that paragraph produces a
    /// document Word calls damaged.
    pub content: Vec<crate::doc::Block>,
}

impl Cell {
    pub fn new() -> Cell {
        Cell {
            props: CellProps::new(),
            content: vec![crate::doc::Block::Paragraph(Default::default())],
        }
    }

    pub fn text(&self) -> String {
        crate::doc::text_of(&self.content)
    }
}

impl Default for Cell {
    fn default() -> Self {
        Cell::new()
    }
}

/// One `<w:tr>`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Row {
    pub props: RowProps,
    pub cells: Vec<Cell>,
}

impl Row {
    pub fn new() -> Row {
        Row::default()
    }

    /// How many grid columns this row covers, including the ones it skips at
    /// either end.
    pub fn grid_width(&self) -> u32 {
        self.props.grid_before
            + self.props.grid_after
            + self.cells.iter().map(|cell| cell.props.span()).sum::<u32>()
    }

    /// The cell covering a grid column, and where that cell starts.
    ///
    /// `None` for a column this row skips — which is a real answer, not a
    /// malformed row: `gridBefore` and `gridAfter` are how a row that begins
    /// half way across a table is written.
    pub fn cell_at(&self, column: u32) -> Option<(usize, &Cell)> {
        let mut at = self.props.grid_before;
        for (index, cell) in self.cells.iter().enumerate() {
            let span = cell.props.span();
            if column >= at && column < at + span {
                return Some((index, cell));
            }
            at += span;
        }
        None
    }
}

/// One `<w:tbl>`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Table {
    pub props: TableProps,
    /// `<w:tblGrid>` — the column widths, and the coordinate system every span
    /// in the table is measured against.
    pub grid: Vec<Twips>,
    pub rows: Vec<Row>,
}

impl Table {
    pub fn new() -> Table {
        Table::default()
    }

    /// How many columns the table has.
    ///
    /// The grid is the authority, and a row that covers more columns than the
    /// grid declares widens the answer — which happens in files that have been
    /// edited by more than one producer, and drawing to the grid alone would
    /// clip the extra cells off the right edge.
    pub fn columns(&self) -> u32 {
        let declared = self.grid.len() as u32;
        let used = self.rows.iter().map(Row::grid_width).max().unwrap_or(0);
        declared.max(used)
    }

    /// The row that *owns* a vertically merged cell — the one carrying
    /// `vMerge="restart"` — for a cell at (row, column).
    ///
    /// Walking up rather than down, because that is the direction the file's
    /// information runs: a continuation says only "the one above me".
    pub fn merge_origin(&self, row: usize, column: u32) -> Option<usize> {
        let mut current = row;
        loop {
            let (_, cell) = self.rows.get(current)?.cell_at(column)?;
            if !cell.props.is_merged_up() {
                return Some(current);
            }
            current = current.checked_sub(1)?;
        }
    }

    /// How many rows a merge starting here covers, including its own.
    pub fn merge_height(&self, row: usize, column: u32) -> usize {
        let mut height = 1;
        for below in row + 1..self.rows.len() {
            match self.rows[below].cell_at(column) {
                Some((_, cell)) if cell.props.is_merged_up() => height += 1,
                _ => break,
            }
        }
        height
    }

    /// The whole table's text, cell by cell, row by row.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for row in &self.rows {
            for (index, cell) in row.cells.iter().enumerate() {
                if index > 0 {
                    out.push('\t');
                }
                out.push_str(&cell.text());
            }
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{Block, Paragraph};

    fn cell(text: &str, span: u32, merge: Option<VMerge>) -> Cell {
        Cell {
            props: CellProps {
                grid_span: span,
                v_merge: merge,
                ..CellProps::new()
            },
            content: vec![Block::Paragraph(Paragraph::of(text))],
        }
    }

    #[test]
    fn a_bare_v_merge_continues_rather_than_restarting() {
        // The trap. Everywhere else in this format a bare element means true;
        // here it means "I am the *second* cell of a merge", and reading it the
        // usual way empties every merged cell in the document.
        assert_eq!(VMerge::from_val(None), VMerge::Continue);
        assert_eq!(VMerge::from_val(Some("continue")), VMerge::Continue);
        assert_eq!(VMerge::from_val(Some("restart")), VMerge::Restart);
    }

    #[test]
    fn a_grid_span_covers_several_columns_of_one_row() {
        let row = Row {
            cells: vec![cell("wide", 2, None), cell("narrow", 1, None)],
            ..Row::new()
        };
        assert_eq!(row.grid_width(), 3);
        assert_eq!(row.cell_at(0).unwrap().0, 0);
        assert_eq!(row.cell_at(1).unwrap().0, 0, "still the wide cell");
        assert_eq!(row.cell_at(2).unwrap().0, 1);
        assert!(row.cell_at(3).is_none());
    }

    #[test]
    fn a_row_may_start_part_way_across_the_grid() {
        let row = Row {
            props: RowProps {
                grid_before: 1,
                grid_after: 1,
                ..RowProps::default()
            },
            cells: vec![cell("middle", 1, None)],
        };
        assert_eq!(row.grid_width(), 3);
        assert!(row.cell_at(0).is_none(), "skipped, not missing");
        assert_eq!(row.cell_at(1).unwrap().1.text(), "middle");
        assert!(row.cell_at(2).is_none());
    }

    #[test]
    fn a_vertical_merge_is_found_by_walking_up() {
        let table = Table {
            grid: vec![Twips(1440), Twips(1440)],
            rows: vec![
                Row {
                    cells: vec![cell("head", 1, Some(VMerge::Restart)), cell("a", 1, None)],
                    ..Row::new()
                },
                Row {
                    cells: vec![cell("", 1, Some(VMerge::Continue)), cell("b", 1, None)],
                    ..Row::new()
                },
                Row {
                    cells: vec![cell("", 1, Some(VMerge::Continue)), cell("c", 1, None)],
                    ..Row::new()
                },
            ],
            ..Table::new()
        };
        assert_eq!(table.merge_origin(2, 0), Some(0));
        assert_eq!(table.merge_origin(2, 1), Some(2), "not merged");
        assert_eq!(table.merge_height(0, 0), 3);
        assert_eq!(table.merge_height(0, 1), 1);
    }

    #[test]
    fn a_grid_span_of_zero_is_still_one_column() {
        let props = CellProps {
            grid_span: 0,
            ..CellProps::new()
        };
        assert_eq!(props.span(), 1);
    }

    #[test]
    fn a_row_wider_than_the_grid_widens_the_table() {
        let table = Table {
            grid: vec![Twips(1440), Twips(1440)],
            rows: vec![Row {
                cells: vec![cell("a", 1, None), cell("b", 1, None), cell("c", 1, None)],
                ..Row::new()
            }],
            ..Table::new()
        };
        assert_eq!(
            table.columns(),
            3,
            "drawing to the grid would clip column c"
        );
    }

    #[test]
    fn a_table_look_reads_the_same_from_the_mask_as_from_the_attributes() {
        // 04A0 is the mask Word writes beside a plain table style: 0x0400
        // noVBand + 0x0080 firstColumn + 0x0020 firstRow, and nothing else.
        let look = TableLook::from_mask(0x04A0);
        assert!(look.first_row);
        assert!(look.first_column);
        assert!(!look.last_row);
        assert!(!look.last_column);
        assert!(!look.no_h_band, "rows are banded");
        assert!(look.no_v_band, "columns are not");
        assert_eq!(look.to_mask(), 0x04A0);
    }

    #[test]
    fn a_width_knows_what_it_is_a_proportion_of() {
        assert_eq!(
            Width::from_parts("dxa", 2880).resolve(Twips(9360)),
            Some(Twips(2880))
        );
        assert_eq!(
            Width::from_parts("pct", 2500).resolve(Twips(9360)),
            Some(Twips(4680))
        );
        assert_eq!(
            Width::from_parts("nil", 0).resolve(Twips(9360)),
            Some(Twips(0))
        );
        assert_eq!(Width::from_parts("auto", 0).resolve(Twips(9360)), None);
    }

    #[test]
    fn a_new_cell_already_holds_the_paragraph_the_format_requires() {
        // A `<w:tc>` with no `<w:p>` in it is a document Word calls damaged.
        let cell = Cell::new();
        assert_eq!(cell.content.len(), 1);
        assert!(matches!(cell.content[0], Block::Paragraph(_)));
    }
}
