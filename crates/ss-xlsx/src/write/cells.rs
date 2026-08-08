//! What a cell holds, in terms both sides of a save can be compared in.
//!
//! The writer's job is to leave alone every cell the user did not touch, so it
//! has to decide whether the cell in the model is the same cell the file
//! describes. That comparison cannot be made on bytes — the file may say
//! `12.50` where we hold `12.5` — and it cannot be made on the model's own
//! types either, because a `StrId` is an index into a table the file knows
//! nothing about. Both sides are lowered to this instead.

use ss_model::formula::Formula;
use ss_model::{Cell, CellValue, StringTable};

/// A cell value with every index resolved.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Val {
    Blank,
    Number(f64),
    Text(String),
    Bool(bool),
    Error(String),
}

impl Val {
    /// The value as the model holds it.
    pub(crate) fn of(cell: &Cell, strings: &StringTable) -> Val {
        match cell.value {
            CellValue::Blank => Val::Blank,
            CellValue::Number(n) => Val::Number(n),
            CellValue::Text(id) => Val::Text(strings.resolve(id).to_string()),
            CellValue::Bool(b) => Val::Bool(b),
            CellValue::Error(e) => Val::Error(e.as_str().to_string()),
        }
    }

    /// Whether two values would display and calculate identically.
    ///
    /// `f64` equality with a NaN clause: a stored NaN is not a value Excel can
    /// produce, but a file can contain one, and `NaN != NaN` would then rewrite
    /// that cell on every save forever.
    pub(crate) fn same(&self, other: &Val) -> bool {
        match (self, other) {
            (Val::Number(a), Val::Number(b)) => a == b || (a.is_nan() && b.is_nan()),
            _ => self == other,
        }
    }
}

/// A cell reduced to what a save has to reproduce.
#[derive(Debug, Clone)]
pub(crate) struct Content {
    pub value: Val,
    pub style: u32,
    pub formula: Option<Formula>,
}

impl Content {
    pub(crate) fn same(&self, other: &Content) -> bool {
        self.style == other.style && self.formula == other.formula && self.value.same(&other.value)
    }
}

/// A number as xlsx spells it.
///
/// Rust's shortest round-trip formatting is what we want: the fewest digits that
/// read back as the same `f64`, which is the guarantee Excel's seventeen
/// significant digits give with more bytes. It also never reaches for
/// exponential notation, so `5.0` is written `5` the way Excel writes it, and no
/// magnitude produces a form the schema would have to be checked against.
pub(crate) fn number(n: f64) -> String {
    format!("{n}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::{CellError, StyleId};

    #[test]
    fn a_stored_nan_does_not_differ_from_itself() {
        assert!(Val::Number(f64::NAN).same(&Val::Number(f64::NAN)));
        assert!(!Val::Number(1.0).same(&Val::Number(2.0)));
    }

    #[test]
    fn text_compares_by_characters_not_by_string_id() {
        let mut a = StringTable::new();
        let mut b = StringTable::new();
        b.intern("filler");
        let left = Cell {
            value: CellValue::Text(a.intern("Total")),
            style: StyleId(0),
            formula: None,
        };
        let right = Cell {
            value: CellValue::Text(b.intern("Total")),
            style: StyleId(0),
            formula: None,
        };
        // Different tables, different ids, same cell.
        assert!(Val::of(&left, &a).same(&Val::of(&right, &b)));
    }

    #[test]
    fn errors_compare_by_their_code() {
        let strings = StringTable::new();
        let cell = Cell {
            value: CellValue::Error(CellError::Div0),
            style: StyleId(0),
            formula: None,
        };
        assert_eq!(Val::of(&cell, &strings), Val::Error("#DIV/0!".into()));
    }

    #[test]
    fn numbers_are_written_the_way_excel_writes_them() {
        assert_eq!(number(5.0), "5");
        assert_eq!(number(12.5), "12.5");
        assert_eq!(number(-0.1), "-0.1");
        assert_eq!(number(45306.0), "45306");
        assert_eq!(number(1e20), "100000000000000000000");
        assert_eq!(number(0.1 + 0.2), "0.30000000000000004");
    }
}
