//! Formulas as they are *stored*, not as they are evaluated.
//!
//! Evaluation belongs to `ss-formula` (C7). What lives here is only enough to
//! hold what the file said, so that a workbook we cannot yet compute is still a
//! workbook we can open and save without damage.
//!
//! The distinction matters most for shared formulas. Excel writes one master cell
//! carrying the text and a range, then a run of follower cells carrying nothing
//! but the group index. A reader that discarded the group structure and kept only
//! the master's text would silently blank every follower on save — hundreds of
//! cells at a time, in exactly the large generated sheets where nobody checks.

use crate::workbook::CellRange;

/// How a formula applies to the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormulaKind {
    /// An ordinary formula occupying one cell.
    Normal,

    /// A CSE array formula anchored at this cell and spilling over `range`.
    ///
    /// Only the anchor carries the text; the rest of the range is covered by it.
    Array { range: CellRange },

    /// The master of a shared-formula group: it holds the text every follower uses.
    Shared {
        /// The `si` group index, unique within the sheet.
        index: u32,
        /// The range the group covers. Excel usually writes it; it is not required.
        range: Option<CellRange>,
    },

    /// A member of a shared group. Its text is the master's, translated by the
    /// offset between the two cells — a translation the formula engine performs,
    /// not the reader.
    SharedFollower {
        /// The `si` group index identifying the master.
        index: u32,
    },

    /// A what-if data table (`<f t="dataTable">`).
    ///
    /// Modeled as its own kind rather than folded into `Normal` because its cells
    /// have no formula text at all — the attributes *are* the formula.
    DataTable,
}

/// One entry in a sheet's formula arena.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formula {
    /// The text with no leading `=`, exactly as stored in the file.
    ///
    /// Empty for `SharedFollower` and `DataTable`, which carry none.
    pub text: String,
    pub kind: FormulaKind,
}

impl Formula {
    /// An ordinary formula. `text` should not include the leading `=`.
    pub fn normal(text: impl Into<String>) -> Self {
        Formula {
            text: text.into(),
            kind: FormulaKind::Normal,
        }
    }

    /// True when this cell's text has to be derived from another cell's.
    pub fn borrows_text(&self) -> bool {
        matches!(self.kind, FormulaKind::SharedFollower { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellRef;

    #[test]
    fn a_shared_follower_carries_no_text_of_its_own() {
        let f = Formula {
            text: String::new(),
            kind: FormulaKind::SharedFollower { index: 3 },
        };
        assert!(f.borrows_text());
        assert!(!Formula::normal("SUM(A1:A9)").borrows_text());
    }

    #[test]
    fn array_formulas_keep_the_range_they_cover() {
        let range = CellRange::new(CellRef::new(0, 3), CellRef::new(9, 3));
        let f = Formula {
            text: "SUM(A1:A10*B1:B10)".into(),
            kind: FormulaKind::Array { range },
        };
        match f.kind {
            FormulaKind::Array { range: r } => assert_eq!(r, range),
            other => panic!("expected an array formula, got {other:?}"),
        }
    }
}
