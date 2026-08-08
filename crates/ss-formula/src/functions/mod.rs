//! The function library.
//!
//! Functions receive their arguments *unevaluated*. Three reasons, all of them
//! forced by Excel's semantics:
//!
//! * `IF`, `IFERROR`, `IFS`, and `SWITCH` must not evaluate the branch they do
//!   not take. `IF(A1=0,0,1/A1)` has to work when A1 is zero.
//! * Aggregation depends on how a value arrived. `SUM(TRUE)` is 1 but
//!   `SUM({TRUE})` is 0 — a boolean written directly is coerced, the same
//!   boolean inside a range is ignored. Only the argument's *shape* tells them
//!   apart, and that shape is gone once everything is flattened to values.
//! * An argument left empty is not a missing argument. `IF(A1,,5)` has three
//!   arguments, the middle one blank.

use ss_model::CellError;

use crate::ast::Expr;
use crate::eval::Evaluator;
use crate::value::{Array, Operand, Value};

/// Decimal rounding lives in the model: the number-format engine needs exactly
/// the same rule to decide what a cell displays.
pub(crate) use ss_model::numfmt::{round_decimal as decimal_round, Rounding};

mod criteria;
mod database;
pub(crate) mod date;
pub(crate) mod dynamic;
mod engineering;
mod financial;
mod info;
mod logical;
mod lookup;
mod math;
mod stats;
mod text;

pub use criteria::{matches_criteria, wildcard_match, Criterion};
pub use dynamic::spills;

/// The signature every built-in shares.
pub type FnImpl = fn(&mut Evaluator, &[Expr]) -> Operand;

/// Looks up a function by name, case-insensitively.
///
/// Excel writes functions added after 2007 with an `_xlfn.` prefix so older
/// versions leave them alone. The prefix is part of the stored name, not part of
/// what the user typed, so it is stripped before the lookup.
pub fn lookup(name: &str) -> Option<FnImpl> {
    let bare = name.strip_prefix("_xlfn.").unwrap_or(name);
    let upper = bare.to_ascii_uppercase();
    math::lookup(&upper)
        .or_else(|| logical::lookup(&upper))
        .or_else(|| text::lookup(&upper))
        .or_else(|| info::lookup(&upper))
        .or_else(|| stats::lookup(&upper))
        .or_else(|| lookup::lookup(&upper))
        .or_else(|| date::lookup(&upper))
        .or_else(|| financial::lookup(&upper))
        .or_else(|| engineering::lookup(&upper))
        .or_else(|| database::lookup(&upper))
        .or_else(|| dynamic::lookup(&upper))
}

/// Calls a function, or reports `#NAME?` if we do not have it.
pub fn call(ev: &mut Evaluator, name: &str, args: &[Expr]) -> Operand {
    match lookup(name) {
        Some(f) => f(ev, args),
        // An unknown function is `#NAME?`, the same answer Excel gives for a
        // misspelling. It is not a parse failure and must not lose the formula.
        None => Operand::error(CellError::Name),
    }
}

/// True when the argument count is in range. `max` of `None` means variadic.
pub(crate) fn arity(args: &[Expr], min: usize, max: Option<usize>) -> bool {
    args.len() >= min && max.is_none_or(|m| args.len() <= m)
}

/// Where a value reached a function from.
///
/// This distinction is the whole reason aggregation is not a simple flatten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// Written as the argument itself: `SUM("1")`.
    Direct,
    /// Found inside a range or array: `SUM(A1:A3)`.
    Inside,
}

/// Visits every value the arguments contribute, tagged with how it arrived.
pub(crate) fn visit_args(ev: &mut Evaluator, args: &[Expr], f: &mut impl FnMut(&Value, Source)) {
    for arg in args {
        let op = ev.eval(arg);
        match op {
            Operand::Value(v) => f(&v, Source::Direct),
            other => {
                let a = ev.spread(&other);
                for v in a.values() {
                    f(v, Source::Inside);
                }
            }
        }
    }
}

/// Collects the numbers an aggregation should see, with Excel's rules about
/// what counts.
///
/// A direct argument is coerced: text that looks like a number becomes one, and
/// text that does not is `#VALUE!`. A value from inside a range is only counted
/// when it is already a number — text and booleans in a range are skipped
/// silently, which is what makes `SUM` usable over a column with a header.
///
/// Errors are never skipped, wherever they came from.
pub(crate) fn numeric_args(ev: &mut Evaluator, args: &[Expr]) -> Result<Vec<f64>, CellError> {
    let mut out = Vec::new();
    let mut err = None;
    visit_args(ev, args, &mut |v, source| {
        if err.is_some() {
            return;
        }
        match (v, source) {
            (Value::Error(e), _) => err = Some(*e),
            (_, Source::Direct) => match v.to_number() {
                Ok(n) => out.push(n),
                Err(e) => err = Some(e),
            },
            (Value::Number(n), Source::Inside) => out.push(*n),
            (_, Source::Inside) => {}
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(out),
    }
}

/// Evaluates every argument to a single value, stopping at the first error.
pub(crate) fn scalar_args(ev: &mut Evaluator, args: &[Expr]) -> Result<Vec<Value>, CellError> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        let v = ev.eval_scalar(a);
        if let Value::Error(e) = v {
            return Err(e);
        }
        out.push(v);
    }
    Ok(out)
}

/// Applies a one-argument numeric function, mapping over arrays.
///
/// Excel's scalar functions are all implicitly array functions: `ABS({-1,-2})`
/// is `{1,2}`. Routing every one-argument function through here is what makes
/// that true without each of them thinking about it.
pub(crate) fn map_number(ev: &mut Evaluator, args: &[Expr], f: impl Fn(f64) -> Value) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    let op = ev.eval(&args[0]);
    match op {
        Operand::Value(v) => Operand::Value(match v.to_number() {
            Ok(n) => f(n),
            Err(e) => Value::Error(e),
        }),
        other => {
            let a = ev.spread(&other);
            let cells: Vec<Value> = a
                .values()
                .map(|v| match v.to_number() {
                    Ok(n) => f(n),
                    Err(e) => Value::Error(e),
                })
                .collect();
            Operand::from_array(Array::new(a.rows(), a.cols(), cells))
        }
    }
}

/// Applies a one-argument text function, mapping over arrays.
pub(crate) fn map_text(ev: &mut Evaluator, args: &[Expr], f: impl Fn(&str) -> Value) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    let op = ev.eval(&args[0]);
    let apply = |v: &Value| match v.to_text() {
        Ok(s) => f(&s),
        Err(e) => Value::Error(e),
    };
    match op {
        Operand::Value(v) => Operand::Value(apply(&v)),
        other => {
            let a = ev.spread(&other);
            let cells: Vec<Value> = a.values().map(apply).collect();
            Operand::from_array(Array::new(a.rows(), a.cols(), cells))
        }
    }
}
