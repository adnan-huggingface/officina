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

mod criteria;
mod info;
mod logical;
mod math;
mod text;

pub use criteria::{matches_criteria, wildcard_match, Criterion};

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

/// How a value is rounded once the digit to drop has been found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rounding {
    /// Half away from zero: 2.5 goes to 3, and -2.5 goes to -3.
    HalfAway,
    /// Always away from zero.
    Up,
    /// Always toward zero.
    Down,
}

/// Rounds to `digits` places after the decimal point, in decimal.
///
/// The detour through a decimal string is not decoration. `ROUND(1.005, 2)` is
/// 1.01 in Excel, but 1.005 as an f64 is really 1.00499999999999989, so scaling
/// by 100 and rounding gives 1.00. Excel rounds the fifteen-significant-digit
/// decimal it would *display*, and matching it means doing the same.
pub(crate) fn decimal_round(x: f64, digits: i32, mode: Rounding) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let sign = if x < 0.0 { -1.0 } else { 1.0 };

    let sci = format!("{:.14e}", x.abs());
    let (mantissa, exp) = sci
        .split_once('e')
        .expect("`{:e}` always emits an exponent");
    let exp: i32 = exp.parse().expect("`{:e}` emits a decimal exponent");
    let all: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).collect();

    // Digit `i` has place value 10^(exp - i); we keep those down to 10^-digits.
    let keep = exp + digits + 1;

    if keep <= 0 {
        let carry = match mode {
            Rounding::HalfAway => keep == 0 && all[0] >= b'5',
            Rounding::Up => true,
            Rounding::Down => false,
        };
        return if carry {
            sign * 10f64.powi(-digits)
        } else {
            0.0
        };
    }
    let keep = keep as usize;
    if keep >= all.len() {
        return x;
    }

    let mut kept: Vec<u8> = all[..keep].to_vec();
    let carry = match mode {
        Rounding::HalfAway => all[keep] >= b'5',
        Rounding::Up => all[keep..].iter().any(|&d| d != b'0'),
        Rounding::Down => false,
    };

    let mut exp = exp;
    if carry {
        let mut i = keep;
        loop {
            if i == 0 {
                // Every digit was a nine: 999 became 1000, one place wider.
                kept.insert(0, b'1');
                kept.pop();
                exp += 1;
                break;
            }
            i -= 1;
            if kept[i] == b'9' {
                kept[i] = b'0';
            } else {
                kept[i] += 1;
                break;
            }
        }
    }

    let digits_text = String::from_utf8(kept).expect("ASCII digits");
    let rebuilt: f64 = format!("{}e{}", digits_text, exp - (keep as i32 - 1))
        .parse()
        .unwrap_or(x.abs());
    sign * rebuilt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_is_half_away_from_zero_not_bankers() {
        // Rust's `f64::round` agrees here, but `format!("{:.0}")` does not — it
        // rounds half to even, which would make this 2.
        assert_eq!(decimal_round(2.5, 0, Rounding::HalfAway), 3.0);
        assert_eq!(decimal_round(-2.5, 0, Rounding::HalfAway), -3.0);
        assert_eq!(decimal_round(1.5, 0, Rounding::HalfAway), 2.0);
        assert_eq!(decimal_round(0.5, 0, Rounding::HalfAway), 1.0);
    }

    #[test]
    fn rounding_works_on_the_decimal_not_the_binary_value() {
        // The case that catches every naive implementation.
        assert_eq!(decimal_round(1.005, 2, Rounding::HalfAway), 1.01);
        assert_eq!(decimal_round(2.675, 2, Rounding::HalfAway), 2.68);
        assert_eq!(decimal_round(1.015, 2, Rounding::HalfAway), 1.02);
    }

    #[test]
    fn rounding_to_negative_digits_rounds_left_of_the_point() {
        assert_eq!(decimal_round(1234.0, -2, Rounding::HalfAway), 1200.0);
        assert_eq!(decimal_round(1250.0, -2, Rounding::HalfAway), 1300.0);
        assert_eq!(decimal_round(1234.0, -5, Rounding::HalfAway), 0.0);
        assert_eq!(decimal_round(1234.0, -3, Rounding::Up), 2000.0);
    }

    #[test]
    fn rounding_carries_all_the_way_through_nines() {
        assert_eq!(decimal_round(9.99, 1, Rounding::HalfAway), 10.0);
        assert_eq!(decimal_round(0.999, 2, Rounding::Up), 1.0);
        assert_eq!(decimal_round(-9.99, 1, Rounding::HalfAway), -10.0);
    }

    #[test]
    fn up_and_down_move_away_from_and_toward_zero() {
        assert_eq!(decimal_round(3.2, 0, Rounding::Up), 4.0);
        assert_eq!(decimal_round(-3.2, 0, Rounding::Up), -4.0);
        assert_eq!(decimal_round(3.9, 0, Rounding::Down), 3.0);
        assert_eq!(decimal_round(-3.9, 0, Rounding::Down), -3.0);
        assert_eq!(decimal_round(0.001, 1, Rounding::Up), 0.1);
        assert_eq!(decimal_round(0.001, 1, Rounding::Down), 0.0);
    }
}
