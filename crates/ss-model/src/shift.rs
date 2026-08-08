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

#[cfg(test)]
mod tests {
    use super::*;

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
