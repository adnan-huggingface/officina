//! Autofilters.
//!
//! A filter is two things that are easy to confuse. The **rule** — this range
//! is filtered, and column 2 shows only these values — lives here and is
//! written into the worksheet as `<autoFilter>`. The **result** — these rows
//! are not on screen — is not stored separately at all: it is the ordinary
//! hidden-row state the file already has, `<row hidden="1">`, which is why a
//! workbook filtered in Excel and opened somewhere that has never heard of
//! filters still shows the right rows.
//!
//! Keeping them apart is what makes "clear the filter" and "unhide the rows"
//! two different operations, and it is also the reason a filtered sheet saved
//! by Calx is legible to Excel: Excel re-derives the hiding from the rule when
//! it wants to, and until then it trusts the rows.

use std::collections::BTreeSet;

use crate::workbook::CellRange;

/// A sheet's autofilter: the range it covers and one entry per constrained
/// column.
///
/// Columns with no entry are unconstrained — the file stores nothing for them,
/// and neither do we. The arrow is drawn on every column in `range`; only the
/// ones in `columns` are actually filtering.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoFilter {
    /// Including the header row, which is how `ref` is written.
    pub range: CellRange,
    pub columns: Vec<FilterColumn>,
}

/// One column's constraint.
#[derive(Debug, Clone, PartialEq)]
pub struct FilterColumn {
    /// Offset from the *left edge of the filter range*, not a sheet column.
    /// This is `colId`, and it is relative for the same reason the file makes
    /// it relative: inserting a column left of the range must not silently
    /// re-point every criterion.
    pub col: u32,
    pub kind: FilterKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilterKind {
    /// The checkbox list: show a row when its cell's *displayed text* is one of
    /// these.
    ///
    /// Displayed text, not value, because that is what the file stores and what
    /// the user ticked. A date shown as `15/01/2024` is filtered by those
    /// characters; matching on the serial would make the list and the sheet
    /// disagree about what the same cell is.
    Values {
        values: BTreeSet<String>,
        /// `<filters blank="1">` — whether empty cells are shown.
        blanks: bool,
    },
    /// One or two comparisons, joined by and/or. Excel's "custom filter".
    Custom {
        first: Criterion,
        second: Option<(bool, Criterion)>,
    },
}

/// A single comparison in a custom filter.
#[derive(Debug, Clone, PartialEq)]
pub struct Criterion {
    pub op: Compare,
    /// The right-hand side as written, interpreted by the caller: a number if
    /// it parses as one, otherwise text with `*` and `?` as wildcards.
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compare {
    Equal,
    NotEqual,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
}

impl Compare {
    /// The `operator` attribute's spelling in the file.
    pub const fn code(self) -> &'static str {
        match self {
            Compare::Equal => "equal",
            Compare::NotEqual => "notEqual",
            Compare::Greater => "greaterThan",
            Compare::GreaterEqual => "greaterThanOrEqual",
            Compare::Less => "lessThan",
            Compare::LessEqual => "lessThanOrEqual",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Some(match code {
            "equal" => Compare::Equal,
            "notEqual" => Compare::NotEqual,
            "greaterThan" => Compare::Greater,
            "greaterThanOrEqual" => Compare::GreaterEqual,
            "lessThan" => Compare::Less,
            "lessThanOrEqual" => Compare::LessEqual,
            _ => return None,
        })
    }

    /// Applies the comparison to an ordering.
    pub const fn holds(self, ordering: std::cmp::Ordering) -> bool {
        use std::cmp::Ordering::*;
        match (self, ordering) {
            (Compare::Equal, Equal) => true,
            (Compare::NotEqual, Equal) => false,
            (Compare::NotEqual, _) => true,
            (Compare::Greater, Greater) => true,
            (Compare::GreaterEqual, Greater | Equal) => true,
            (Compare::Less, Less) => true,
            (Compare::LessEqual, Less | Equal) => true,
            _ => false,
        }
    }
}

impl AutoFilter {
    /// A filter over `range` with nothing constrained yet — the arrows appear
    /// and every row stays visible.
    pub fn over(range: CellRange) -> Self {
        AutoFilter {
            range,
            columns: Vec::new(),
        }
    }

    /// The row the arrows sit on. Excel's filter range always includes it.
    pub fn header_row(&self) -> u32 {
        self.range.start.row
    }

    /// The first row a filter can hide.
    pub fn first_data_row(&self) -> u32 {
        self.range.start.row + 1
    }

    pub fn column(&self, col: u32) -> Option<&FilterColumn> {
        self.columns.iter().find(|c| c.col == col)
    }

    /// True when at least one column is actually constraining something.
    pub fn is_filtering(&self) -> bool {
        !self.columns.is_empty()
    }

    /// Sets or clears one column's constraint.
    pub fn set(&mut self, col: u32, kind: Option<FilterKind>) {
        self.columns.retain(|c| c.col != col);
        if let Some(kind) = kind {
            self.columns.push(FilterColumn { col, kind });
            self.columns.sort_by_key(|c| c.col);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CellRef;

    fn range(a: &str, b: &str) -> CellRange {
        CellRange::new(
            CellRef::from_a1(a).expect("a1"),
            CellRef::from_a1(b).expect("a1"),
        )
    }

    #[test]
    fn the_header_row_is_inside_the_range_and_is_never_hidden_by_it() {
        let filter = AutoFilter::over(range("A1", "C9"));
        assert_eq!(filter.header_row(), 0);
        assert_eq!(filter.first_data_row(), 1);
    }

    #[test]
    fn setting_a_column_twice_replaces_rather_than_stacks() {
        let mut filter = AutoFilter::over(range("A1", "C9"));
        filter.set(
            1,
            Some(FilterKind::Values {
                values: ["a".to_string()].into_iter().collect(),
                blanks: false,
            }),
        );
        filter.set(
            1,
            Some(FilterKind::Values {
                values: ["b".to_string()].into_iter().collect(),
                blanks: false,
            }),
        );
        assert_eq!(filter.columns.len(), 1);

        filter.set(1, None);
        assert!(!filter.is_filtering(), "clearing the last one clears it");
    }

    #[test]
    fn every_operator_round_trips_through_its_file_spelling() {
        for op in [
            Compare::Equal,
            Compare::NotEqual,
            Compare::Greater,
            Compare::GreaterEqual,
            Compare::Less,
            Compare::LessEqual,
        ] {
            assert_eq!(Compare::from_code(op.code()), Some(op));
        }
    }

    #[test]
    fn comparisons_read_an_ordering_the_way_their_names_say() {
        use std::cmp::Ordering::*;
        assert!(Compare::GreaterEqual.holds(Equal));
        assert!(Compare::GreaterEqual.holds(Greater));
        assert!(!Compare::GreaterEqual.holds(Less));
        assert!(Compare::NotEqual.holds(Less));
        assert!(Compare::NotEqual.holds(Greater));
        assert!(!Compare::NotEqual.holds(Equal));
    }
}
