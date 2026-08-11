//! What is selected, and how the keyboard and mouse change it.
//!
//! Selection is a list of rectangles, not one rectangle. Ctrl-clicking builds a
//! second area, and `SUM` over a multi-area selection is a thing people do. The
//! *active* cell is separate again: it is where typing lands, and it stays put
//! while shift-arrow grows the rectangle around it.
//!
//! Merges are the awkward part. A selection that touches any part of a merged
//! region has to swallow the whole thing — otherwise a fill or a delete would
//! operate on half a merge, which Excel does not allow and the file format
//! cannot express.

use ss_model::cell::{MAX_COLS, MAX_ROWS};
use ss_model::{CellRange, CellRef, Sheet};

use super::Axis;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    const fn delta(self) -> (i64, i64) {
        match self {
            Direction::Up => (-1, 0),
            Direction::Down => (1, 0),
            Direction::Left => (0, -1),
            Direction::Right => (0, 1),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// Never empty. The last entry is the one shift-extension grows.
    ranges: Vec<CellRange>,
    /// The corner a shift-extension pivots around.
    anchor: CellRef,
    /// The active cell — where typing goes and what the name box shows.
    cursor: CellRef,
    /// The corner an extension is moving. Distinct from the cursor: in Excel
    /// the active cell stays put while shift-arrow or a drag grows the
    /// rectangle, and it is this corner — not the active cell — that moves
    /// and that the view follows.
    lead: CellRef,
}

impl Default for Selection {
    fn default() -> Self {
        Selection::at(CellRef::new(0, 0))
    }
}

impl Selection {
    pub fn at(cell: CellRef) -> Self {
        Selection {
            ranges: vec![CellRange::new(cell, cell)],
            anchor: cell,
            cursor: cell,
            lead: cell,
        }
    }

    pub fn cursor(&self) -> CellRef {
        self.cursor
    }

    pub fn anchor(&self) -> CellRef {
        self.anchor
    }

    /// The cell navigation should keep in view: the moving corner while
    /// extending, which equals the cursor whenever nothing is being extended.
    pub fn lead(&self) -> CellRef {
        self.lead
    }

    pub fn ranges(&self) -> &[CellRange] {
        &self.ranges
    }

    /// The rectangle shift-extension is currently growing.
    /// How the selection reads to a person: `B4` or `B4:D9`.
    pub fn active_range_label(&self) -> String {
        let range = self.active_range();
        if range.start == range.end {
            range.start.to_a1()
        } else {
            format!("{}:{}", range.start.to_a1(), range.end.to_a1())
        }
    }

    pub fn active_range(&self) -> CellRange {
        *self.ranges.last().expect("a selection is never empty")
    }

    pub fn contains(&self, at: CellRef) -> bool {
        self.ranges.iter().any(|r| r.contains(at))
    }

    /// True when the whole selection is one cell — which is what decides
    /// whether Enter moves down or wraps inside the block.
    pub fn is_single_cell(&self) -> bool {
        self.ranges.len() == 1 && self.ranges[0].rows() == 1 && self.ranges[0].cols() == 1
    }

    /// Collapses to a single cell, discarding every other area.
    pub fn move_to(&mut self, at: CellRef, sheet: &Sheet) {
        let at = anchor_of_merge(sheet, at);
        self.ranges = vec![expand(sheet, CellRange::new(at, at))];
        self.anchor = at;
        self.cursor = at;
        self.lead = at;
    }

    /// Grows the active rectangle to reach `at`, keeping the anchor fixed.
    /// The cursor stays where it is: extending a selection does not move
    /// where typing lands.
    pub fn extend_to(&mut self, at: CellRef, sheet: &Sheet) {
        let range = expand(sheet, span(self.anchor, at));
        *self.ranges.last_mut().expect("never empty") = range;
        self.lead = at;
    }

    /// Ctrl-click: starts a new area without losing the ones already there.
    pub fn add(&mut self, at: CellRef, sheet: &Sheet) {
        let at = anchor_of_merge(sheet, at);
        self.ranges.push(expand(sheet, CellRange::new(at, at)));
        self.anchor = at;
        self.cursor = at;
        self.lead = at;
    }

    /// Arrow key: one step, collapsing the selection.
    pub fn step(&mut self, dir: Direction, sheet: &Sheet) {
        let at = offset(self.cursor, dir, 1);
        self.move_to(at, sheet);
    }

    /// Shift-arrow: one step of the rectangle's moving corner.
    pub fn extend(&mut self, dir: Direction, sheet: &Sheet) {
        let at = offset(self.lead, dir, 1);
        self.extend_to(at, sheet);
    }

    /// Ctrl-arrow: to the far end of the current run of data.
    ///
    /// The rule is Excel's and is not "go to the last used cell": from a filled
    /// cell it stops at the end of the run, from an empty one it jumps to the
    /// next filled cell, and with nothing ahead it goes to the sheet edge. It
    /// is how anyone navigates a large sheet, so getting it slightly wrong is
    /// felt immediately.
    pub fn jump(&mut self, dir: Direction, sheet: &Sheet, extend: bool) {
        if extend {
            // The walk starts from the moving corner, so repeated
            // Ctrl+Shift+arrow keeps growing rather than re-treading the
            // same run from the active cell.
            let target = boundary(sheet, self.lead, dir);
            self.extend_to(target, sheet);
        } else {
            let target = boundary(sheet, self.cursor, dir);
            self.move_to(target, sheet);
        }
    }

    /// Selects whole rows. The cursor keeps its column, as Excel's does.
    pub fn select_rows(&mut self, from: u32, to: u32, additive: bool) {
        let range = CellRange::new(
            CellRef::new(from.min(to), 0),
            CellRef::new(from.max(to), MAX_COLS - 1),
        );
        self.replace(range, additive, CellRef::new(from, self.cursor.col));
    }

    pub fn select_columns(&mut self, from: u32, to: u32, additive: bool) {
        let range = CellRange::new(
            CellRef::new(0, from.min(to)),
            CellRef::new(MAX_ROWS - 1, from.max(to)),
        );
        self.replace(range, additive, CellRef::new(self.cursor.row, from));
    }

    /// Sweeping along a header: the band the last click started grows.
    ///
    /// It rewrites the last range rather than adding one, so a sweep that began
    /// with Ctrl held keeps every band already selected and still ends up with
    /// one band per sweep instead of one per frame.
    pub fn extend_headers(&mut self, axis: Axis, from: u32, to: u32) {
        let range = match axis {
            Axis::Rows => CellRange::new(
                CellRef::new(from.min(to), 0),
                CellRef::new(from.max(to), MAX_COLS - 1),
            ),
            Axis::Columns => CellRange::new(
                CellRef::new(0, from.min(to)),
                CellRef::new(MAX_ROWS - 1, from.max(to)),
            ),
        };
        *self.ranges.last_mut().expect("never empty") = range;
        self.lead = if to >= from { range.end } else { range.start };
    }

    pub fn select_all(&mut self) {
        self.ranges = vec![CellRange::new(
            CellRef::new(0, 0),
            CellRef::new(MAX_ROWS - 1, MAX_COLS - 1),
        )];
        self.anchor = CellRef::new(0, 0);
        self.lead = self.cursor;
    }

    fn replace(&mut self, range: CellRange, additive: bool, cursor: CellRef) {
        if additive {
            self.ranges.push(range);
        } else {
            self.ranges = vec![range];
        }
        self.anchor = range.start;
        self.cursor = cursor;
        // Shift-arrow after a header click grows the band from its far end.
        self.lead = range.end;
    }

    /// Moves the cursor within the selection, as Tab and Enter do.
    ///
    /// With more than one cell selected the cursor cycles inside the block
    /// instead of leaving it — which is what makes tabbing through a form work.
    pub fn advance(&mut self, dir: Direction, sheet: &Sheet) {
        if self.is_single_cell() {
            self.step(dir, sheet);
            return;
        }
        let range = self.active_range();
        let (dr, dc) = dir.delta();
        let mut row = i64::from(self.cursor.row) + dr;
        let mut col = i64::from(self.cursor.col) + dc;

        // Falling off one edge wraps to the next line of the block, and off the
        // last line wraps to the first.
        if col > i64::from(range.end.col) {
            col = i64::from(range.start.col);
            row += 1;
        } else if col < i64::from(range.start.col) {
            col = i64::from(range.end.col);
            row -= 1;
        }
        if row > i64::from(range.end.row) {
            row = i64::from(range.start.row);
        } else if row < i64::from(range.start.row) {
            row = i64::from(range.end.row);
        }
        self.cursor = CellRef::new(
            row.clamp(i64::from(range.start.row), i64::from(range.end.row)) as u32,
            col.clamp(i64::from(range.start.col), i64::from(range.end.col)) as u32,
        );
    }

    /// The rectangle covering every selected area, for scroll-into-view.
    pub fn bounds(&self) -> CellRange {
        let mut it = self.ranges.iter();
        let first = *it.next().expect("never empty");
        it.fold(first, |acc, r| span_range(acc, *r))
    }

    /// One rectangle, keeping the cursor in place when it falls inside.
    pub fn select_range(&mut self, range: CellRange) {
        if !range.contains(self.cursor) {
            self.cursor = range.start;
        }
        self.ranges = vec![range];
        self.anchor = range.start;
        self.lead = range.end;
    }

    /// End: the last filled cell of the row, the mirror of Home.
    pub fn end_of_row(&mut self, sheet: &Sheet, extend: bool) {
        let row = if extend { self.lead.row } else { self.cursor.row };
        let Some((_, used_end)) = sheet.cells.used_range() else {
            return;
        };
        let filled =
            |col: u32| sheet.get(CellRef::new(row, col)).is_some_and(|c| !c.is_vacant());
        // The last filled column of this row; an empty row answers with the
        // sheet's own last used column, so End still goes *somewhere* useful.
        let mut col = used_end.col;
        while col > 0 && !filled(col) {
            col -= 1;
        }
        if !filled(col) {
            col = used_end.col;
        }
        let target = CellRef::new(row, col);
        if extend {
            self.extend_to(target, sheet);
        } else {
            self.move_to(target, sheet);
        }
    }
}

/// Excel's "current region": the island of data around `at`, grown until it
/// is bordered by empty rows and columns on every side. Diagonal contact
/// counts, which is why each probe strip reaches one cell past the corners.
pub fn current_region(sheet: &Sheet, at: CellRef) -> CellRange {
    let filled = |row: u32, col: u32| {
        sheet
            .get(CellRef::new(row, col))
            .is_some_and(|c| !c.is_vacant())
    };
    let Some((used_start, used_end)) = sheet.cells.used_range() else {
        return CellRange::new(at, at);
    };
    let mut range = CellRange::new(at, at);
    loop {
        let mut grown = range;
        // Probe columns one past each side, clamped to the used range so an
        // island near the edge does not walk a million empty cells.
        let c0 = range.start.col.saturating_sub(1).max(used_start.col.saturating_sub(1));
        let c1 = (range.end.col + 1).min(used_end.col);
        let r0 = range.start.row.saturating_sub(1).max(used_start.row.saturating_sub(1));
        let r1 = (range.end.row + 1).min(used_end.row);
        if range.start.row > 0 && (c0..=c1).any(|c| filled(range.start.row - 1, c)) {
            grown.start.row -= 1;
        }
        if range.end.row < used_end.row && (c0..=c1).any(|c| filled(range.end.row + 1, c)) {
            grown.end.row += 1;
        }
        if range.start.col > 0 && (r0..=r1).any(|r| filled(r, range.start.col - 1)) {
            grown.start.col -= 1;
        }
        if range.end.col < used_end.col && (r0..=r1).any(|r| filled(r, range.end.col + 1)) {
            grown.end.col += 1;
        }
        if grown == range {
            return range;
        }
        range = grown;
    }
}

fn offset(at: CellRef, dir: Direction, by: i64) -> CellRef {
    let (dr, dc) = dir.delta();
    CellRef::new(
        (i64::from(at.row) + dr * by).clamp(0, i64::from(MAX_ROWS) - 1) as u32,
        (i64::from(at.col) + dc * by).clamp(0, i64::from(MAX_COLS) - 1) as u32,
    )
}

fn span(a: CellRef, b: CellRef) -> CellRange {
    CellRange::new(
        CellRef::new(a.row.min(b.row), a.col.min(b.col)),
        CellRef::new(a.row.max(b.row), a.col.max(b.col)),
    )
}

fn span_range(a: CellRange, b: CellRange) -> CellRange {
    CellRange::new(
        CellRef::new(a.start.row.min(b.start.row), a.start.col.min(b.start.col)),
        CellRef::new(a.end.row.max(b.end.row), a.end.col.max(b.end.col)),
    )
}

/// The top-left cell of the merge covering `at`, or `at` itself.
fn anchor_of_merge(sheet: &Sheet, at: CellRef) -> CellRef {
    sheet.merge_at(at).map_or(at, |m| m.start)
}

/// Grows a rectangle until it contains every merge it touches.
///
/// Repeats because swallowing one merge can bring the rectangle into contact
/// with another. It settles quickly — each pass either grows the rectangle or
/// stops — and a sheet has far fewer merges than cells.
fn expand(sheet: &Sheet, mut range: CellRange) -> CellRange {
    loop {
        let mut grown = range;
        for merge in &sheet.merges {
            if overlaps(&range, merge) {
                grown = span_range(grown, *merge);
            }
        }
        if grown == range {
            return range;
        }
        range = grown;
    }
}

fn overlaps(a: &CellRange, b: &CellRange) -> bool {
    a.start.row <= b.end.row
        && b.start.row <= a.end.row
        && a.start.col <= b.end.col
        && b.start.col <= a.end.col
}

/// Excel's Ctrl-arrow destination.
fn boundary(sheet: &Sheet, from: CellRef, dir: Direction) -> CellRef {
    let filled = |at: CellRef| sheet.get(at).is_some_and(|c| !c.is_vacant());
    let limit = match dir {
        Direction::Up | Direction::Left => 0,
        Direction::Down => MAX_ROWS - 1,
        Direction::Right => MAX_COLS - 1,
    };
    let edge = match dir {
        Direction::Up => CellRef::new(0, from.col),
        Direction::Down => CellRef::new(limit, from.col),
        Direction::Left => CellRef::new(from.row, 0),
        Direction::Right => CellRef::new(from.row, limit),
    };

    // Nothing to walk past: a sheet with no cells at all.
    let Some((_, used_end)) = sheet.cells.used_range() else {
        return edge;
    };
    // Never walk beyond the used range — with a million empty rows below, a
    // linear scan to the sheet edge would freeze the window.
    let steps = match dir {
        Direction::Up => from.row,
        Direction::Down => used_end.row.saturating_sub(from.row) + 1,
        Direction::Left => from.col,
        Direction::Right => used_end.col.saturating_sub(from.col) + 1,
    };

    let mut at = from;
    let start_filled = filled(from);
    let mut moved = false;
    for _ in 0..steps {
        let next = offset(at, dir, 1);
        if next == at {
            break;
        }
        let next_filled = filled(next);
        if start_filled {
            // Walking a run of data: stop at its last cell.
            if !next_filled {
                return if moved {
                    at
                } else {
                    boundary(sheet, next, dir)
                };
            }
        } else if next_filled {
            // Crossing a gap: stop at the first cell of the next block.
            return next;
        }
        at = next;
        moved = true;
    }
    if start_filled && moved {
        at
    } else {
        edge
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::{Cell, CellValue};

    fn sheet_with(cells: &[&str]) -> Sheet {
        let mut sheet = Sheet::new("Sheet1");
        for at in cells {
            sheet.set(
                CellRef::from_a1(at).expect("test address"),
                Cell {
                    value: CellValue::Number(1.0),
                    ..Cell::default()
                },
            );
        }
        sheet
    }

    fn a1(s: &str) -> CellRef {
        CellRef::from_a1(s).expect("test address")
    }

    #[test]
    fn shift_arrow_grows_around_a_fixed_anchor() {
        let sheet = sheet_with(&[]);
        let mut sel = Selection::at(a1("C3"));
        sel.extend(Direction::Down, &sheet);
        sel.extend(Direction::Right, &sheet);
        assert_eq!(sel.active_range(), CellRange::new(a1("C3"), a1("D4")));
        assert_eq!(sel.anchor(), a1("C3"), "the anchor does not move");
        // Reversing past the anchor flips the rectangle rather than emptying it.
        sel.extend_to(a1("A1"), &sheet);
        assert_eq!(sel.active_range(), CellRange::new(a1("A1"), a1("C3")));
    }

    #[test]
    fn extending_moves_the_lead_and_not_the_active_cell() {
        // The bug this exists for: the cursor rode along with the growing
        // edge, so after shift-selecting a block, typing landed in the far
        // corner instead of in the cell that was active — which is not
        // where Excel puts it, and not where anyone is looking.
        let sheet = sheet_with(&[]);
        let mut sel = Selection::at(a1("B2"));
        sel.extend(Direction::Down, &sheet);
        sel.extend(Direction::Down, &sheet);
        sel.extend(Direction::Right, &sheet);
        assert_eq!(sel.active_range(), CellRange::new(a1("B2"), a1("C4")));
        assert_eq!(sel.cursor(), a1("B2"), "typing still lands at B2");
        assert_eq!(sel.lead(), a1("C4"), "the view follows the moving corner");
        // Ctrl+Shift+arrow keeps walking from the lead, not from the cursor.
        sel.jump(Direction::Down, &sheet, true);
        assert_eq!(sel.cursor(), a1("B2"));
        // A plain step collapses everything back to one cell again.
        sel.step(Direction::Right, &sheet);
        assert_eq!(sel.cursor(), sel.lead());
        assert!(sel.is_single_cell());
    }

    #[test]
    fn ctrl_click_keeps_the_areas_already_selected() {
        let sheet = sheet_with(&[]);
        let mut sel = Selection::at(a1("A1"));
        sel.extend_to(a1("A3"), &sheet);
        sel.add(a1("C1"), &sheet);
        assert_eq!(sel.ranges().len(), 2);
        assert!(sel.contains(a1("A2")));
        assert!(sel.contains(a1("C1")));
        assert!(!sel.contains(a1("B2")));
        assert_eq!(sel.cursor(), a1("C1"));
    }

    #[test]
    fn a_selection_swallows_any_merge_it_touches() {
        let mut sheet = sheet_with(&[]);
        sheet.merges.push(CellRange::new(a1("B2"), a1("C3")));
        let mut sel = Selection::at(a1("A1"));
        // Reaching B2 pulls in the whole merge, not the corner of it.
        sel.extend_to(a1("B2"), &sheet);
        assert_eq!(sel.active_range(), CellRange::new(a1("A1"), a1("C3")));
        // Clicking inside a merge selects it from its anchor.
        sel.move_to(a1("C3"), &sheet);
        assert_eq!(sel.cursor(), a1("B2"));
        assert_eq!(sel.active_range(), CellRange::new(a1("B2"), a1("C3")));
    }

    #[test]
    fn ctrl_arrow_stops_at_the_end_of_a_run() {
        // A1:A3 filled, A4:A5 empty, A6 filled.
        let sheet = sheet_with(&["A1", "A2", "A3", "A6"]);
        let mut sel = Selection::at(a1("A1"));
        sel.jump(Direction::Down, &sheet, false);
        assert_eq!(sel.cursor(), a1("A3"), "the last cell of the run");
        sel.jump(Direction::Down, &sheet, false);
        assert_eq!(sel.cursor(), a1("A6"), "across the gap to the next block");
        sel.jump(Direction::Down, &sheet, false);
        assert_eq!(sel.cursor(), a1("A1048576"), "nothing below, so the edge");
    }

    #[test]
    fn ctrl_arrow_from_an_empty_cell_finds_the_next_block() {
        let sheet = sheet_with(&["A5"]);
        let mut sel = Selection::at(a1("A1"));
        sel.jump(Direction::Down, &sheet, false);
        assert_eq!(sel.cursor(), a1("A5"));
        // And on an empty sheet it goes straight to the edge without walking
        // a million rows.
        let empty = sheet_with(&[]);
        let mut sel = Selection::at(a1("A1"));
        sel.jump(Direction::Down, &empty, false);
        assert_eq!(sel.cursor(), a1("A1048576"));
    }

    #[test]
    fn tab_cycles_inside_a_multi_cell_selection() {
        let sheet = sheet_with(&[]);
        let mut sel = Selection::at(a1("B2"));
        sel.extend_to(a1("C3"), &sheet);
        sel.cursor = a1("B2");
        sel.advance(Direction::Right, &sheet);
        assert_eq!(sel.cursor(), a1("C2"));
        sel.advance(Direction::Right, &sheet);
        assert_eq!(sel.cursor(), a1("B3"), "wraps to the next row of the block");
        sel.advance(Direction::Right, &sheet);
        sel.advance(Direction::Right, &sheet);
        assert_eq!(sel.cursor(), a1("B2"), "and around to the start");
        // A single cell leaves the block instead of cycling.
        let mut one = Selection::at(a1("B2"));
        one.advance(Direction::Right, &sheet);
        assert_eq!(one.cursor(), a1("C2"));
    }

    #[test]
    fn movement_stops_at_the_edges_of_the_sheet() {
        let sheet = sheet_with(&[]);
        let mut sel = Selection::at(a1("A1"));
        sel.step(Direction::Up, &sheet);
        sel.step(Direction::Left, &sheet);
        assert_eq!(sel.cursor(), a1("A1"));
    }

    #[test]
    fn the_current_region_is_the_island_and_only_the_island() {
        // Data at B2:C3, a diagonal straggler at D4, an unrelated island at F8.
        let sheet = sheet_with(&["B2", "B3", "C2", "C3", "D4", "F8"]);
        let region = current_region(&sheet, a1("B2"));
        assert_eq!(
            region,
            CellRange::new(a1("B2"), a1("D4")),
            "diagonal contact joins D4; F8 stays out"
        );
        // From an isolated cell the region is just that cell.
        assert_eq!(current_region(&sheet, a1("F8")), CellRange::new(a1("F8"), a1("F8")));
        // From an empty cell far away, likewise — the caller decides that
        // means select-all.
        assert_eq!(current_region(&sheet, a1("H20")), CellRange::new(a1("H20"), a1("H20")));
    }

    #[test]
    fn end_goes_to_the_last_filled_cell_of_the_row() {
        let sheet = sheet_with(&["A1", "C1", "B5"]);
        let mut sel = Selection::at(a1("A1"));
        sel.end_of_row(&sheet, false);
        assert_eq!(sel.cursor(), a1("C1"));
        // An empty row still lands at the sheet's used width.
        let mut sel = Selection::at(a1("A3"));
        sel.end_of_row(&sheet, false);
        assert_eq!(sel.cursor(), a1("C3"));
    }

    #[test]
    fn whole_row_and_column_selections_span_the_sheet() {
        let mut sel = Selection::at(a1("B2"));
        sel.select_rows(1, 2, false);
        assert_eq!(sel.active_range().cols(), MAX_COLS);
        assert_eq!(sel.active_range().rows(), 2);
        sel.select_columns(0, 0, true);
        assert_eq!(sel.ranges().len(), 2);
        assert_eq!(sel.active_range().rows(), MAX_ROWS);
    }
}
