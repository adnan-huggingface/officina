//! Where a row or column ends up when rows or columns are inserted or deleted.
//!
//! This is small enough to look trivial and is not. The same arithmetic has to
//! govern three things that would otherwise drift apart: where the *cells* go,
//! where the *merges and sizes* go, and what the *formula text* is rewritten to
//! say. If the store moved a cell down by one and the rewriter moved the
//! reference by two, every affected formula would quietly compute the wrong
//! answer, with nothing on screen to suggest it.

use crate::cell::{CellRef, MAX_COLS, MAX_ROWS};
use crate::workbook::CellRange;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Rows,
    Columns,
}

impl Axis {
    /// The index of `at` along this axis.
    pub fn index(self, at: CellRef) -> u32 {
        match self {
            Axis::Rows => at.row,
            Axis::Columns => at.col,
        }
    }

    /// One past the last valid index along this axis.
    pub fn limit(self) -> u32 {
        match self {
            Axis::Rows => MAX_ROWS,
            Axis::Columns => MAX_COLS,
        }
    }

    /// `at` with its index along this axis replaced.
    pub fn with(self, at: CellRef, index: u32) -> CellRef {
        match self {
            Axis::Rows => CellRef::new(index, at.col),
            Axis::Columns => CellRef::new(at.row, index),
        }
    }
}

/// `count` rows or columns inserted at `at`, or deleted starting there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shift {
    pub axis: Axis,
    /// Zero-based index of the first row or column affected.
    pub at: u32,
    pub count: u32,
    pub delete: bool,
}

impl Shift {
    pub fn insert(axis: Axis, at: u32, count: u32) -> Self {
        Shift {
            axis,
            at,
            count,
            delete: false,
        }
    }

    pub fn delete(axis: Axis, at: u32, count: u32) -> Self {
        Shift {
            axis,
            at,
            count,
            delete: true,
        }
    }

    /// The inverse operation: undoing an insert is a delete of the same band.
    ///
    /// The inverse is only complete for an insert. Undoing a *delete* also has
    /// to put the removed cells back, which this cannot carry.
    pub fn inverse(self) -> Self {
        Shift {
            delete: !self.delete,
            ..self
        }
    }

    /// True when nothing at or after `index` is untouched.
    pub fn touches(&self, index: u32) -> bool {
        index >= self.at
    }

    /// Where a single index ends up, or `None` if it went away — deleted, or
    /// pushed off the end of the grid.
    pub fn point(&self, index: u32) -> Option<u32> {
        if !self.delete {
            if index < self.at {
                return Some(index);
            }
            let moved = index as u64 + self.count as u64;
            return (moved < self.axis.limit() as u64).then_some(moved as u32);
        }
        if index < self.at {
            Some(index)
        } else if index >= self.at + self.count {
            Some(index - self.count)
        } else {
            None
        }
    }

    /// Where an inclusive span ends up, or `None` if all of it went away.
    ///
    /// A span differs from a point at both edges. Inserting *inside* a range
    /// stretches it rather than moving it, and deleting part of one shrinks it
    /// to whatever survived.
    pub fn span(&self, start: u32, end: u32) -> Option<(u32, u32)> {
        if !self.delete {
            let last = self.axis.limit() - 1;
            let grow = |x: u32| {
                if x >= self.at {
                    ((x as u64 + self.count as u64).min(last as u64)) as u32
                } else {
                    x
                }
            };
            let (s, e) = (grow(start), grow(end));
            return (s <= e).then_some((s, e));
        }

        let last = self.at + self.count - 1;
        if start >= self.at && end <= last {
            return None;
        }
        let new_start = if start < self.at {
            start
        } else if start > last {
            start - self.count
        } else {
            self.at
        };
        let new_end = if end < self.at {
            end
        } else if end > last {
            end - self.count
        } else {
            // Not fully contained and the end is inside the cut, so the start
            // is above it and `at` is at least one.
            self.at - 1
        };
        Some((new_start, new_end))
    }

    /// Where a rectangle ends up, or `None` if the shift consumed it entirely.
    pub fn range(&self, range: CellRange) -> Option<CellRange> {
        let (start, end) = match self.axis {
            Axis::Rows => self.span(range.start.row, range.end.row)?,
            Axis::Columns => self.span(range.start.col, range.end.col)?,
        };
        Some(CellRange::new(
            self.axis.with(range.start, start),
            self.axis.with(range.end, end),
        ))
    }
}

/// A band of rows or columns picked up and put down somewhere else.
///
/// A move is not a shift, and the difference is the whole reason this is a
/// separate type. A shift makes rows appear or disappear, so an index can end
/// up nowhere and a reference to it becomes `#REF!`. A move only *reorders*:
/// every index still names a row afterwards, just a different one, and nothing
/// a move does can produce an error. It is a permutation, and specifically a
/// rotation of one contiguous window — which is what makes every question about
/// it answerable in constant time rather than by walking a million rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Move {
    pub axis: Axis,
    /// The band being moved, inclusive at both ends.
    pub first: u32,
    pub last: u32,
    /// The index the band comes to sit immediately before, counted in the grid
    /// as it is *now* — which is how a drop between two rows is named, and it
    /// is why `first..=last + 1` is the range that means "leave it alone".
    pub before: u32,
}

impl Move {
    pub fn new(axis: Axis, first: u32, last: u32, before: u32) -> Self {
        Move {
            axis,
            first: first.min(last),
            last: first.max(last),
            before,
        }
    }

    /// True when the band would land exactly where it already is.
    pub fn is_noop(&self) -> bool {
        (self.first..=self.last.saturating_add(1)).contains(&self.before)
    }

    /// The band's position after the move, inclusive.
    pub fn landing(&self) -> (u32, u32) {
        let count = self.last - self.first + 1;
        let first = if self.before > self.last {
            self.before - count
        } else {
            self.before
        };
        (first, first + count - 1)
    }

    /// The runs of indices this move shifts, and by how much.
    ///
    /// Two runs and never more: the band itself, and the block it steps over.
    /// Everything outside them stays where it is. Returned rather than applied
    /// so that both `point` and `span` are built from the same description and
    /// cannot come to disagree.
    fn runs(&self) -> [(u32, u32, i64); 2] {
        debug_assert!(!self.is_noop(), "a move that stays put has no runs");
        let count = i64::from(self.last - self.first + 1);
        if self.before > self.last {
            // Rightwards: the band jumps forward over the block behind it, and
            // that block slides back by the band's width.
            let gap = i64::from(self.before - self.last - 1);
            [
                (self.first, self.last, gap),
                (self.last + 1, self.before - 1, -count),
            ]
        } else {
            let gap = i64::from(self.first - self.before);
            [
                (self.first, self.last, -gap),
                (self.before, self.first - 1, count),
            ]
        }
    }

    /// Where a single index ends up. Always somewhere: nothing is removed.
    pub fn point(&self, index: u32) -> u32 {
        if self.is_noop() {
            return index;
        }
        for (from, to, by) in self.runs() {
            if from <= to && (from..=to).contains(&index) {
                return (i64::from(index) + by) as u32;
            }
        }
        index
    }

    /// The smallest span covering where `start..=end` ends up.
    ///
    /// A range is remapped by where its rows *land*, not by where its two
    /// corners land. `A1:D1` still covers the same four columns after B is
    /// moved to the end of them, so it is still `A1:D1` — mapping the corners
    /// alone would have said `A1:C1` and quietly dropped a column. Where the
    /// moved rows no longer sit together the answer is the span that covers
    /// them, because a range is one rectangle and there is nothing else to say.
    pub fn span(&self, start: u32, end: u32) -> (u32, u32) {
        if self.is_noop() {
            return (start, end);
        }
        let runs = self.runs();
        // The two runs are adjacent, so together they are exactly the window
        // the move disturbs. Anything outside it stays where it is, and each
        // run is a constant shift, so its extremes are its own two ends. Three
        // pairs of endpoints, whatever the size of the span.
        let (w0, w1) = (runs[0].0.min(runs[1].0), runs[0].1.max(runs[1].1));
        let (mut lo, mut hi) = (u32::MAX, 0u32);
        let mut note = |a: u32, b: u32| {
            lo = lo.min(a);
            hi = hi.max(b);
        };
        if start < w0 {
            note(start, end.min(w0 - 1));
        }
        if end > w1 {
            note(start.max(w1 + 1), end);
        }
        for (from, to, by) in runs {
            let (a, b) = (start.max(from), end.min(to));
            if from <= to && a <= b {
                note((i64::from(a) + by) as u32, (i64::from(b) + by) as u32);
            }
        }
        (lo, hi)
    }

    /// Where a rectangle ends up.
    pub fn range(&self, range: CellRange) -> CellRange {
        let (start, end) = match self.axis {
            Axis::Rows => self.span(range.start.row, range.end.row),
            Axis::Columns => self.span(range.start.col, range.end.col),
        };
        CellRange::new(
            self.axis.with(range.start, start),
            self.axis.with(range.end, end),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every index of `0..count` under a move, worked out the slow, obvious
    /// way: take the list, pull the band out, put it back at the destination,
    /// and read off where each one landed.
    fn by_hand(m: Move, count: u32) -> Vec<u32> {
        let order: Vec<u32> = (0..count).collect();
        let band: Vec<u32> = (m.first..=m.last).collect();
        let mut rest: Vec<u32> = order
            .iter()
            .copied()
            .filter(|i| !band.contains(i))
            .collect();
        // `before` counts in the old grid, so the insertion point among what
        // is left is however many survivors come before it.
        let at = rest.iter().filter(|i| **i < m.before).count();
        for (n, index) in band.iter().enumerate() {
            rest.insert(at + n, *index);
        }
        let mut landed = vec![0u32; count as usize];
        for (position, index) in rest.iter().enumerate() {
            landed[*index as usize] = position as u32;
        }
        landed
    }

    #[test]
    fn a_move_agrees_with_taking_the_band_out_and_putting_it_back() {
        // Every band and every destination in a small grid, against the
        // definition rather than against another formula.
        for first in 0..6u32 {
            for last in first..6 {
                for before in 0..=6u32 {
                    let m = Move::new(Axis::Columns, first, last, before);
                    let expected = by_hand(m, 6);
                    for i in 0..6u32 {
                        assert_eq!(m.point(i), expected[i as usize], "{m:?} at {i}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_span_covers_where_its_members_land_rather_than_where_its_corners_do() {
        // A to D with B moved to the far end of them still covers A to D. The
        // obvious implementation maps the corners and says A to C, quietly
        // dropping a column from every SUM that spanned the moved one.
        let m = Move::new(Axis::Columns, 1, 1, 4);
        assert_eq!(m.point(1), 3, "B lands last of the four");
        assert_eq!(m.span(0, 3), (0, 3));
        // Two columns that no longer sit together give the span that covers
        // them, because a range is one rectangle.
        assert_eq!(m.span(1, 2), (1, 3));
        // Outside the window nothing moves.
        assert_eq!(m.span(7, 9), (7, 9));
        assert_eq!(m.span(0, 9), (0, 9));
    }

    #[test]
    fn a_band_that_lands_where_it_started_is_no_move_at_all() {
        for before in 2..=5u32 {
            let m = Move::new(Axis::Rows, 2, 4, before);
            assert!(m.is_noop(), "{m:?}");
            assert_eq!(m.point(3), 3);
            assert_eq!(m.span(0, 9), (0, 9));
        }
        assert!(!Move::new(Axis::Rows, 2, 4, 1).is_noop());
        assert!(!Move::new(Axis::Rows, 2, 4, 6).is_noop());
    }

    #[test]
    fn a_moved_band_reports_where_it_came_to_rest() {
        // Rightwards: C:D moved before G lands at E:F, because C and D are
        // gone from in front of it.
        assert_eq!(Move::new(Axis::Columns, 2, 3, 6).landing(), (4, 5));
        // Leftwards it lands exactly where it was dropped.
        assert_eq!(Move::new(Axis::Columns, 4, 5, 1).landing(), (1, 2));
    }

    #[test]
    fn an_insert_moves_everything_at_or_after_it() {
        let s = Shift::insert(Axis::Rows, 3, 2);
        assert_eq!(s.point(2), Some(2));
        assert_eq!(s.point(3), Some(5));
        assert_eq!(s.point(100), Some(102));
    }

    #[test]
    fn a_delete_takes_its_own_band_and_pulls_the_rest_up() {
        let s = Shift::delete(Axis::Rows, 3, 2); // rows 3 and 4
        assert_eq!(s.point(2), Some(2));
        assert_eq!(s.point(3), None);
        assert_eq!(s.point(4), None);
        assert_eq!(s.point(5), Some(3));
    }

    #[test]
    fn inserting_inside_a_span_stretches_it_instead_of_moving_it() {
        let s = Shift::insert(Axis::Rows, 5, 1);
        assert_eq!(s.span(0, 9), Some((0, 10)));
        assert_eq!(s.span(6, 9), Some((7, 10)), "wholly below: it moves");
        assert_eq!(s.span(0, 4), Some((0, 4)), "wholly above: untouched");
    }

    #[test]
    fn deleting_across_the_edge_of_a_span_shrinks_it() {
        let s = Shift::delete(Axis::Rows, 0, 5);
        assert_eq!(s.span(2, 7), Some((0, 2)));

        let inner = Shift::delete(Axis::Rows, 2, 3);
        assert_eq!(inner.span(0, 9), Some((0, 6)));

        let over = Shift::delete(Axis::Rows, 4, 10);
        assert_eq!(over.span(2, 7), Some((2, 3)), "the tail is cut off");
    }

    #[test]
    fn a_span_entirely_inside_a_deletion_is_gone() {
        let s = Shift::delete(Axis::Rows, 1, 3);
        assert_eq!(s.span(1, 3), None);
        assert_eq!(s.span(2, 2), None);
    }

    #[test]
    fn content_pushed_past_the_last_row_falls_off_the_grid() {
        let s = Shift::insert(Axis::Rows, 0, 1);
        assert_eq!(s.point(MAX_ROWS - 1), None);
        assert_eq!(s.point(MAX_ROWS - 2), Some(MAX_ROWS - 1));
    }
}
