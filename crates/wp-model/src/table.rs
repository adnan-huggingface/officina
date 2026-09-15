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

impl TableProps {
    /// The table's own `<w:tblInd>`, if it states one.
    pub fn props_indent(&self) -> Option<Width> {
        self.indent
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
    /// Word's own default, and the margins its built-in Table Normal style
    /// carries: no padding above or below, and 0.08in either side. It lives in
    /// the *style*, not in the table — a document that never defines Table
    /// Normal has no such padding at all, which is why [`Self::zero`], not this,
    /// is where cell-margin resolution starts.
    pub fn word_default() -> CellMargins {
        CellMargins {
            top: Some(Width::Fixed(Twips(0))),
            start: Some(Width::Fixed(Twips(108))),
            bottom: Some(Width::Fixed(Twips(0))),
            end: Some(Width::Fixed(Twips(108))),
        }
    }

    /// No padding on any side — the floor a table sits on when nothing, not
    /// even a default table style, states otherwise. Word draws such a table's
    /// text hard against the cell edge; matching that is the point.
    pub fn zero() -> CellMargins {
        CellMargins {
            top: Some(Width::Fixed(Twips(0))),
            start: Some(Width::Fixed(Twips(0))),
            bottom: Some(Width::Fixed(Twips(0))),
            end: Some(Width::Fixed(Twips(0))),
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

    /// The row's text, cell by cell, tabs between.
    pub fn text(&self) -> String {
        self.cells
            .iter()
            .map(Cell::text)
            .collect::<Vec<_>>()
            .join("\t")
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
            out.push_str(&row.text());
            out.push('\n');
        }
        out
    }
}

/// Editing the shape of a table: rows and columns in and out.
///
/// These change the grid and every row that crosses the place changed, and
/// nothing else — a span that straddles an inserted column grows by one, a
/// row that begins past it (`gridBefore`) begins one further on, and a
/// vertical merge the new row lands inside is continued rather than cut.
impl Table {
    /// A blank row shaped like row `like`: the same height, rule, shading,
    /// cell widths and spans, each cell holding one empty paragraph in the
    /// style of the paragraph its model ends with — so that text typed into
    /// the new row looks like the text beside it. Measured: this is what
    /// Word's Insert Below copies, with empty cells (LEARNINGS.md, "A column
    /// Word inserts…"). A tracked insertion is not carried, and neither is a
    /// vertical merge: [`Table::insert_row`] decides that from where the row
    /// goes.
    pub fn blank_row_like(&self, like: usize) -> Option<Row> {
        use crate::doc::{Block, Paragraph};
        let mut row = self.rows.get(like)?.clone();
        row.props.revision = None;
        for cell in &mut row.cells {
            let props = cell.content.iter().rev().find_map(|block| match block {
                Block::Paragraph(paragraph) => Some(paragraph.props.clone()),
                _ => None,
            });
            cell.props.v_merge = None;
            cell.content = vec![Block::Paragraph(Paragraph {
                props: props.unwrap_or_default(),
                ..Paragraph::default()
            })];
        }
        Some(row)
    }

    /// Puts a blank row shaped like row `like` at `at`, so that it becomes
    /// row `at`. `at` may be one past the end. A cell that lands inside a
    /// vertical merge — the row below it continues one — continues it too,
    /// which is what keeps a merged cell one cell.
    pub fn insert_row(&mut self, at: usize, like: usize) -> bool {
        if at > self.rows.len() {
            return false;
        }
        let Some(mut row) = self.blank_row_like(like) else {
            return false;
        };
        if let Some(below) = self.rows.get(at) {
            let mut column = row.props.grid_before;
            for cell in &mut row.cells {
                if below
                    .cell_at(column)
                    .is_some_and(|(_, c)| c.props.is_merged_up())
                {
                    cell.props.v_merge = Some(VMerge::Continue);
                }
                column += cell.props.span();
            }
        }
        self.rows.insert(at, row);
        true
    }

    /// Puts a grid column in at `at` — before the column that is there, or at
    /// the end when `at` is the column count — and a blank cell in every row
    /// that crosses it.
    ///
    /// Measured on Word 16: the new column takes the width of the column to
    /// its right, or of the last column when it is appended, and every other
    /// column keeps its width — the table grows by the new one
    /// (LEARNINGS.md, "A column Word inserts takes the width of the column to
    /// its right, and narrows nothing"). A cell whose span straddles the
    /// place grows by one column rather than being split; a new cell takes
    /// the formatting of the cell it is put beside, with an empty paragraph
    /// in that cell's last paragraph's style. A table stating a fixed width
    /// states the new total.
    pub fn insert_column(&mut self, at: usize) -> bool {
        let count = self.grid.len();
        if at > count {
            return false;
        }
        let Some(width) = self.grid.get(at).or(self.grid.last()).copied() else {
            return false;
        };
        self.grid.insert(at, width);
        if let Width::Fixed(total) = &mut self.props.width {
            total.0 += width.0;
        }
        let at = at as u32;
        for row in &mut self.rows {
            if at < row.props.grid_before {
                row.props.grid_before += 1;
                continue;
            }
            let mut position = row.props.grid_before;
            let mut placed = false;
            for index in 0..row.cells.len() {
                let span = row.cells[index].props.span();
                if at == position {
                    let cell = blank_cell_like(&row.cells[index], width);
                    row.cells.insert(index, cell);
                    placed = true;
                    break;
                }
                if at < position + span {
                    let cell = &mut row.cells[index];
                    cell.props.grid_span = span + 1;
                    if let Width::Fixed(own) = &mut cell.props.width {
                        own.0 += width.0;
                    }
                    placed = true;
                    break;
                }
                position += span;
            }
            if placed {
                continue;
            }
            if at == position {
                let cell = match row.cells.last() {
                    Some(last) => blank_cell_like(last, width),
                    None => {
                        let mut cell = Cell::new();
                        cell.props.width = Width::Fixed(width);
                        cell
                    }
                };
                row.cells.push(cell);
            } else if row.props.grid_after > 0 {
                row.props.grid_after += 1;
            }
            // A row shorter than the grid with no `gridAfter` to say so stays
            // as short as it was: the file left it that way.
        }
        true
    }

    /// Takes row `at` out. A vertical merge the row began is begun by the
    /// row below it instead, so that no cell is left continuing nothing.
    pub fn delete_row(&mut self, at: usize) -> bool {
        if at >= self.rows.len() {
            return false;
        }
        let removed = self.rows.remove(at);
        if let Some(below) = self.rows.get_mut(at) {
            let mut column = below.props.grid_before;
            for cell in &mut below.cells {
                let span = cell.props.span();
                if cell.props.is_merged_up()
                    && !removed
                        .cell_at(column)
                        .is_some_and(|(_, above)| above.props.is_merged_up())
                {
                    cell.props.v_merge = Some(VMerge::Restart);
                }
                column += span;
            }
        }
        true
    }

    /// Takes grid column `at` out of the grid and out of every row: a cell
    /// spanning it narrows by one column, a cell that is only it goes, and a
    /// row that begins past it begins one nearer. A row left with no cells
    /// goes with them. The other columns keep their widths.
    pub fn delete_column(&mut self, at: usize) -> bool {
        if at >= self.grid.len() {
            return false;
        }
        self.grid.remove(at);
        let at = at as u32;
        for row in &mut self.rows {
            if at < row.props.grid_before {
                row.props.grid_before -= 1;
                continue;
            }
            let mut position = row.props.grid_before;
            let mut found = None;
            for (index, cell) in row.cells.iter().enumerate() {
                let span = cell.props.span();
                if at < position + span {
                    found = Some((index, position, span));
                    break;
                }
                position += span;
            }
            match found {
                Some((index, _, 1)) => {
                    row.cells.remove(index);
                }
                Some((index, position, span)) => {
                    let cell = &mut row.cells[index];
                    cell.props.grid_span = span - 1;
                    if let Width::Fixed(_) = cell.props.width {
                        let from = position as usize;
                        let to = (from + span as usize - 1).min(self.grid.len());
                        let total: i32 = self.grid[from.min(to)..to].iter().map(|t| t.0).sum();
                        cell.props.width = Width::Fixed(Twips(total));
                    }
                }
                None if row.props.grid_after > 0 => row.props.grid_after -= 1,
                None => {}
            }
        }
        self.rows.retain(|row| !row.cells.is_empty());
        if let Width::Fixed(total) = &mut self.props.width {
            total.0 = self.grid.iter().map(|t| t.0).sum();
        }
        true
    }
}

/// A blank cell formatted like `model`, one grid column wide and `width`
/// wide, holding one empty paragraph in the style of the model's last.
fn blank_cell_like(model: &Cell, width: Twips) -> Cell {
    use crate::doc::{Block, Paragraph};
    let props = model.content.iter().rev().find_map(|block| match block {
        Block::Paragraph(paragraph) => Some(paragraph.props.clone()),
        _ => None,
    });
    Cell {
        props: CellProps {
            width: Width::Fixed(width),
            grid_span: 1,
            v_merge: None,
            ..model.props.clone()
        },
        content: vec![Block::Paragraph(Paragraph {
            props: props.unwrap_or_default(),
            ..Paragraph::default()
        })],
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

    fn table_of(widths: &[i32], rows: &[&[&str]]) -> Table {
        Table {
            grid: widths.iter().map(|w| Twips(*w)).collect(),
            rows: rows
                .iter()
                .map(|texts| Row {
                    cells: texts
                        .iter()
                        .zip(widths)
                        .map(|(text, width)| {
                            let mut cell = cell(text, 1, None);
                            cell.props.width = Width::Fixed(Twips(*width));
                            cell
                        })
                        .collect(),
                    ..Row::default()
                })
                .collect(),
            ..Table::default()
        }
    }

    fn widths(table: &Table) -> Vec<i32> {
        table.grid.iter().map(|t| t.0).collect()
    }

    #[test]
    fn a_row_inserted_below_is_blank_and_shaped_like_the_one_above() {
        let mut table = table_of(&[2880, 5760], &[&["a", "b"], &["c", "d"]]);
        table.rows[0].props.height = Some(RowHeight::AtLeast(Twips(400)));
        table.rows[0].cells[1].props.shading = Some(Shading::default());
        assert!(table.insert_row(1, 0));
        assert_eq!(table.rows.len(), 3);
        let new = &table.rows[1];
        assert_eq!(new.props.height, Some(RowHeight::AtLeast(Twips(400))));
        assert_eq!(new.cells.len(), 2);
        assert!(new.cells[1].props.shading.is_some(), "the shading came too");
        assert_eq!(new.cells[0].props.width, Width::Fixed(Twips(2880)));
        assert_eq!(new.text(), "\t", "and every cell is empty");
        assert_eq!(table.rows[2].text(), "c\td", "the row below moved down");
        assert!(table.insert_row(3, 2), "one past the end appends");
        assert!(!table.insert_row(5, 0), "two past the end is nowhere");
    }

    #[test]
    fn a_row_inserted_inside_a_vertical_merge_continues_it() {
        let mut table = Table {
            grid: vec![Twips(1000), Twips(1000)],
            rows: vec![
                Row {
                    cells: vec![cell("tall", 1, Some(VMerge::Restart)), cell("a", 1, None)],
                    ..Row::default()
                },
                Row {
                    cells: vec![cell("", 1, Some(VMerge::Continue)), cell("b", 1, None)],
                    ..Row::default()
                },
            ],
            ..Table::default()
        };
        assert!(table.insert_row(1, 0));
        assert_eq!(
            table.rows[1].cells[0].props.v_merge,
            Some(VMerge::Continue),
            "the new row is inside the merge, so it continues it"
        );
        assert_eq!(table.rows[1].cells[1].props.v_merge, None);
        assert!(table.insert_row(3, 2), "below the merge");
        assert_eq!(table.rows[3].cells[0].props.v_merge, None);
    }

    #[test]
    fn a_column_inserted_takes_its_right_neighbours_width_and_narrows_nothing() {
        // Word 16, measured: 2in, 4in, 1in; a column right of the 4in one is
        // 1in, one left of the 2in one is 2in, one after the last copies it.
        let (two, four, one) = (2880, 5760, 1440);
        let mut table = table_of(&[two, four, one], &[&["a", "b", "c"], &["d", "e", "f"]]);
        table.props.width = Width::Fixed(Twips(two + four + one));
        assert!(table.insert_column(2), "right of the 4in column");
        assert_eq!(widths(&table), vec![two, four, one, one]);
        assert_eq!(table.rows[0].text(), "a\tb\t\tc");
        assert_eq!(table.rows[1].cells[2].props.width, Width::Fixed(Twips(one)));
        assert_eq!(
            table.props.width,
            Width::Fixed(Twips(two + four + one + one))
        );
        assert!(table.insert_column(0), "left of the 2in column");
        assert_eq!(widths(&table), vec![two, two, four, one, one]);
        assert_eq!(table.rows[0].text(), "\ta\tb\t\tc");
        assert!(table.insert_column(5), "after the last");
        assert_eq!(widths(&table), vec![two, two, four, one, one, one]);
        assert_eq!(table.rows[1].text(), "\td\te\t\tf\t");
        assert!(!table.insert_column(7), "past that is nowhere");
    }

    #[test]
    fn a_column_inserted_under_a_span_widens_the_span_and_moves_a_late_row_along() {
        let mut table = table_of(&[1000, 1000, 1000], &[&["a", "b", "c"]]);
        table.rows.push(Row {
            cells: vec![cell("wide", 2, None), cell("z", 1, None)],
            ..Row::default()
        });
        table.rows[1].cells[0].props.width = Width::Fixed(Twips(2000));
        table.rows.push(Row {
            props: RowProps {
                grid_before: 2,
                ..RowProps::default()
            },
            cells: vec![cell("late", 1, None)],
        });
        assert!(table.insert_column(1));
        assert_eq!(widths(&table), vec![1000, 1000, 1000, 1000]);
        assert_eq!(table.rows[0].text(), "a\t\tb\tc");
        let wide = &table.rows[1].cells[0];
        assert_eq!(wide.props.grid_span, 3, "the span grew by the column");
        assert_eq!(wide.props.width, Width::Fixed(Twips(3000)));
        assert_eq!(table.rows[1].cells.len(), 2);
        assert_eq!(
            table.rows[2].props.grid_before, 3,
            "the late row starts one further on"
        );
    }

    #[test]
    fn a_deleted_row_hands_its_merge_to_the_row_below() {
        let mut table = Table {
            grid: vec![Twips(1000), Twips(1000)],
            rows: vec![
                Row {
                    cells: vec![cell("tall", 1, Some(VMerge::Restart)), cell("a", 1, None)],
                    ..Row::default()
                },
                Row {
                    cells: vec![cell("", 1, Some(VMerge::Continue)), cell("b", 1, None)],
                    ..Row::default()
                },
                Row {
                    cells: vec![cell("", 1, Some(VMerge::Continue)), cell("c", 1, None)],
                    ..Row::default()
                },
            ],
            ..Table::default()
        };
        assert!(table.delete_row(0));
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].cells[0].props.v_merge, Some(VMerge::Restart));
        assert_eq!(table.rows[1].cells[0].props.v_merge, Some(VMerge::Continue));
        assert!(table.delete_row(1), "a middle continuation goes quietly");
        assert_eq!(table.rows[0].cells[0].props.v_merge, Some(VMerge::Restart));
        assert!(!table.delete_row(1), "nothing there");
    }

    #[test]
    fn a_deleted_column_narrows_the_grid_and_nothing_else() {
        let mut table = table_of(&[2880, 5760, 1440], &[&["a", "b", "c"], &["d", "e", "f"]]);
        table.props.width = Width::Fixed(Twips(2880 + 5760 + 1440));
        assert!(table.delete_column(1));
        assert_eq!(widths(&table), vec![2880, 1440]);
        assert_eq!(table.rows[0].text(), "a\tc");
        assert_eq!(
            table.rows[1].cells[1].props.width,
            Width::Fixed(Twips(1440))
        );
        assert_eq!(table.props.width, Width::Fixed(Twips(2880 + 1440)));
        assert!(!table.delete_column(2), "no third column any more");
    }

    #[test]
    fn a_deleted_column_under_a_span_narrows_the_span_and_empties_no_row_but_a_bare_one() {
        let mut table = table_of(&[1000, 1000, 1000], &[&["a", "b", "c"]]);
        table.rows.push(Row {
            cells: vec![cell("wide", 2, None), cell("z", 1, None)],
            ..Row::default()
        });
        table.rows[1].cells[0].props.width = Width::Fixed(Twips(2000));
        table.rows.push(Row {
            props: RowProps {
                grid_before: 2,
                ..RowProps::default()
            },
            cells: vec![cell("late", 1, None)],
        });
        assert!(table.delete_column(0));
        assert_eq!(widths(&table), vec![1000, 1000]);
        assert_eq!(table.rows[0].text(), "b\tc");
        assert_eq!(table.rows[1].cells[0].props.grid_span, 1);
        assert_eq!(
            table.rows[1].cells[0].props.width,
            Width::Fixed(Twips(1000))
        );
        assert_eq!(table.rows[2].props.grid_before, 1);
        assert!(table.delete_column(1), "the late row's only column");
        assert_eq!(table.rows.len(), 2, "and the row went with it");
    }
}
