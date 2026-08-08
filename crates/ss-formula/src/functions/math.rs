//! Math and trigonometry.
//!
//! Most of these are thin wrappers over `f64`. The ones that are not are the
//! ones where Excel disagrees with IEEE arithmetic: `MOD` takes the sign of its
//! divisor, `INT` floors while `TRUNC` truncates, `ROUND` goes half away from
//! zero on the *decimal* value, and every domain error is a `#NUM!` rather than
//! a NaN.

use ss_model::CellError;

use crate::ast::Expr;
use crate::eval::{finite, power, Evaluator};
use crate::value::{Operand, Value};

use super::criteria::{visit_if, visit_ifs};
use super::{arity, decimal_round, map_number, numeric_args, FnImpl, Rounding};

pub(super) fn lookup(name: &str) -> Option<FnImpl> {
    Some(match name {
        "ABS" => |ev, a| map_number(ev, a, |x| finite(x.abs())),
        "SIGN" => |ev, a| map_number(ev, a, |x| Value::Number(x.signum() * f64::from(x != 0.0))),
        "SQRT" => |ev, a| {
            map_number(ev, a, |x| {
                if x < 0.0 {
                    Value::Error(CellError::Num)
                } else {
                    finite(x.sqrt())
                }
            })
        },
        "EXP" => |ev, a| map_number(ev, a, |x| finite(x.exp())),
        "LN" => |ev, a| map_number(ev, a, |x| positive_log(x, std::f64::consts::E)),
        "LOG10" => |ev, a| map_number(ev, a, |x| positive_log(x, 10.0)),
        "LOG" => log,
        "POWER" => |ev: &mut Evaluator, a: &[Expr]| two_numbers(ev, a, power),
        "MOD" => |ev: &mut Evaluator, a: &[Expr]| two_numbers(ev, a, modulo),
        "QUOTIENT" => |ev: &mut Evaluator, a: &[Expr]| {
            two_numbers(ev, a, |x, y| {
                if y == 0.0 {
                    Value::Error(CellError::Div0)
                } else {
                    finite((x / y).trunc())
                }
            })
        },
        "INT" => |ev, a| map_number(ev, a, |x| finite(x.floor())),
        "TRUNC" => truncate,
        "ROUND" => |ev: &mut Evaluator, a: &[Expr]| round_with(ev, a, Rounding::HalfAway),
        "ROUNDUP" => |ev: &mut Evaluator, a: &[Expr]| round_with(ev, a, Rounding::Up),
        "ROUNDDOWN" => |ev: &mut Evaluator, a: &[Expr]| round_with(ev, a, Rounding::Down),
        "MROUND" => |ev: &mut Evaluator, a: &[Expr]| two_numbers(ev, a, mround),
        "CEILING" => |ev: &mut Evaluator, a: &[Expr]| two_numbers(ev, a, |x, s| step(x, s, true)),
        "FLOOR" => |ev: &mut Evaluator, a: &[Expr]| two_numbers(ev, a, |x, s| step(x, s, false)),
        "CEILING.MATH" => |ev: &mut Evaluator, a: &[Expr]| step_math(ev, a, true),
        "FLOOR.MATH" => |ev: &mut Evaluator, a: &[Expr]| step_math(ev, a, false),
        "ISO.CEILING" => iso_ceiling,
        "EVEN" => |ev, a| map_number(ev, a, |x| parity_round(x, 2.0)),
        "ODD" => |ev, a| map_number(ev, a, odd),
        "FACT" => |ev, a| map_number(ev, a, factorial),
        "COMBIN" => |ev: &mut Evaluator, a: &[Expr]| two_numbers(ev, a, combinations),
        "PERMUT" => |ev: &mut Evaluator, a: &[Expr]| two_numbers(ev, a, permutations),
        "GCD" => |ev: &mut Evaluator, a: &[Expr]| whole_number_fold(ev, a, gcd2),
        "LCM" => |ev: &mut Evaluator, a: &[Expr]| whole_number_fold(ev, a, lcm2),

        "SUM" => |ev: &mut Evaluator, a: &[Expr]| fold(ev, a, 0.0, |acc, x| acc + x),
        "SUMSQ" => |ev: &mut Evaluator, a: &[Expr]| fold(ev, a, 0.0, |acc, x| acc + x * x),
        "PRODUCT" => product,
        "SUMPRODUCT" => sumproduct,
        "SUMIF" => sumif,
        "SUMIFS" => sumifs,

        "PI" => |_ev: &mut Evaluator, a: &[Expr]| {
            if a.is_empty() {
                Operand::number(std::f64::consts::PI)
            } else {
                Operand::error(CellError::Value)
            }
        },
        "RAND" => |ev: &mut Evaluator, a: &[Expr]| {
            if a.is_empty() {
                Operand::number(ev.next_random())
            } else {
                Operand::error(CellError::Value)
            }
        },
        "RANDBETWEEN" => randbetween,

        "SIN" => |ev, a| map_number(ev, a, |x| finite(x.sin())),
        "COS" => |ev, a| map_number(ev, a, |x| finite(x.cos())),
        "TAN" => |ev, a| map_number(ev, a, |x| finite(x.tan())),
        "CSC" => |ev, a| map_number(ev, a, |x| reciprocal(x.sin())),
        "SEC" => |ev, a| map_number(ev, a, |x| reciprocal(x.cos())),
        "COT" => |ev, a| map_number(ev, a, |x| reciprocal(x.tan())),
        "ASIN" => |ev, a| map_number(ev, a, |x| bounded(x, f64::asin)),
        "ACOS" => |ev, a| map_number(ev, a, |x| bounded(x, f64::acos)),
        "ATAN" => |ev, a| map_number(ev, a, |x| finite(x.atan())),
        "ATAN2" => |ev: &mut Evaluator, a: &[Expr]| two_numbers(ev, a, atan2),
        "SINH" => |ev, a| map_number(ev, a, |x| finite(x.sinh())),
        "COSH" => |ev, a| map_number(ev, a, |x| finite(x.cosh())),
        "TANH" => |ev, a| map_number(ev, a, |x| finite(x.tanh())),
        "ASINH" => |ev, a| map_number(ev, a, |x| finite(x.asinh())),
        "ACOSH" => |ev, a| {
            map_number(ev, a, |x| {
                if x < 1.0 {
                    Value::Error(CellError::Num)
                } else {
                    finite(x.acosh())
                }
            })
        },
        "ATANH" => |ev, a| {
            map_number(ev, a, |x| {
                if x <= -1.0 || x >= 1.0 {
                    Value::Error(CellError::Num)
                } else {
                    finite(x.atanh())
                }
            })
        },
        "DEGREES" => |ev, a| map_number(ev, a, |x| finite(x.to_degrees())),
        "RADIANS" => |ev, a| map_number(ev, a, |x| finite(x.to_radians())),
        _ => return None,
    })
}

/// Evaluates exactly two numeric arguments and combines them.
fn two_numbers(ev: &mut Evaluator, args: &[Expr], f: impl Fn(f64, f64) -> Value + Copy) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let a = ev.eval(&args[0]);
    let b = ev.eval(&args[1]);
    ev.broadcast(a, b, |x, y| match (x.to_number(), y.to_number()) {
        (Ok(x), Ok(y)) => f(x, y),
        (Err(e), _) | (_, Err(e)) => Value::Error(e),
    })
}

fn fold(ev: &mut Evaluator, args: &[Expr], init: f64, f: impl Fn(f64, f64) -> f64) -> Operand {
    if args.is_empty() {
        return Operand::error(CellError::Value);
    }
    match numeric_args(ev, args) {
        Ok(ns) => Operand::Value(finite(ns.into_iter().fold(init, f))),
        Err(e) => Operand::error(e),
    }
}

/// `PRODUCT` of nothing at all is 0, not 1 — an empty range contributes no
/// factors and Excel reports zero rather than the multiplicative identity.
fn product(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if args.is_empty() {
        return Operand::error(CellError::Value);
    }
    match numeric_args(ev, args) {
        Ok(ns) if ns.is_empty() => Operand::number(0.0),
        Ok(ns) => Operand::Value(finite(ns.into_iter().product())),
        Err(e) => Operand::error(e),
    }
}

fn positive_log(x: f64, base: f64) -> Value {
    if x <= 0.0 {
        Value::Error(CellError::Num)
    } else {
        finite(x.log(base))
    }
}

fn log(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let x = match ev.eval_number(&args[0]) {
        Ok(x) => x,
        Err(e) => return Operand::error(e),
    };
    let base = match args.get(1) {
        Some(e) => match ev.eval_number(e) {
            Ok(b) => b,
            Err(e) => return Operand::error(e),
        },
        None => 10.0,
    };
    if base <= 0.0 || base == 1.0 {
        return Operand::error(CellError::Num);
    }
    Operand::Value(positive_log(x, base))
}

fn reciprocal(x: f64) -> Value {
    if x == 0.0 {
        Value::Error(CellError::Div0)
    } else {
        finite(1.0 / x)
    }
}

fn bounded(x: f64, f: impl Fn(f64) -> f64) -> Value {
    if !(-1.0..=1.0).contains(&x) {
        Value::Error(CellError::Num)
    } else {
        finite(f(x))
    }
}

/// `ATAN2(x, y)` — note the argument order, which is the reverse of every
/// programming language's `atan2` and a reliable source of wrong angles.
fn atan2(x: f64, y: f64) -> Value {
    if x == 0.0 && y == 0.0 {
        return Value::Error(CellError::Div0);
    }
    finite(y.atan2(x))
}

/// `MOD` follows the sign of the divisor, not the dividend.
///
/// `MOD(-3, 2)` is 1 in Excel and -1 in C, Rust, and most of everything else.
fn modulo(x: f64, y: f64) -> Value {
    if y == 0.0 {
        return Value::Error(CellError::Div0);
    }
    finite(x - y * (x / y).floor())
}

fn round_with(ev: &mut Evaluator, args: &[Expr], mode: Rounding) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let a = ev.eval(&args[0]);
    let b = ev.eval(&args[1]);
    ev.broadcast(a, b, move |x, d| match (x.to_number(), d.to_number()) {
        (Ok(x), Ok(d)) => finite(decimal_round(x, clamp_digits(d), mode)),
        (Err(e), _) | (_, Err(e)) => Value::Error(e),
    })
}

/// `TRUNC`'s second argument is optional; `ROUND`'s is not.
fn truncate(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let digits = match args.get(1) {
        Some(e) => match ev.eval_number(e) {
            Ok(d) => clamp_digits(d),
            Err(e) => return Operand::error(e),
        },
        None => 0,
    };
    map_number(ev, &args[..1], move |x| {
        finite(decimal_round(x, digits, Rounding::Down))
    })
}

/// Digit counts beyond this cannot change an f64, and keep the decimal
/// reconstruction inside the exponent range.
fn clamp_digits(d: f64) -> i32 {
    if !d.is_finite() {
        return 0;
    }
    (d.trunc() as i64).clamp(-300, 300) as i32
}

fn mround(x: f64, multiple: f64) -> Value {
    if multiple == 0.0 {
        return Value::Number(0.0);
    }
    if x != 0.0 && (x < 0.0) != (multiple < 0.0) {
        // Excel refuses to round a number toward a multiple of the opposite sign.
        return Value::Error(CellError::Num);
    }
    finite(multiple * decimal_round(x / multiple, 0, Rounding::HalfAway))
}

/// The legacy `CEILING` and `FLOOR`, which are `significance * ceil(x /
/// significance)` and its floor counterpart.
///
/// Written that way the sign handling falls out on its own: `CEILING(-4.5, -2)`
/// is -6 because -4.5 / -2 is 2.25, whose ceiling is 3.
fn step(x: f64, significance: f64, up: bool) -> Value {
    if significance == 0.0 {
        return if up {
            Value::Number(0.0)
        } else {
            Value::Error(CellError::Div0)
        };
    }
    if x > 0.0 && significance < 0.0 {
        return Value::Error(CellError::Num);
    }
    let q = x / significance;
    finite(significance * if up { q.ceil() } else { q.floor() })
}

/// `CEILING.MATH` / `FLOOR.MATH`: the significance sign is ignored, and a third
/// argument decides which way negative numbers go.
fn step_math(ev: &mut Evaluator, args: &[Expr], up: bool) -> Operand {
    if !arity(args, 1, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let x = match ev.eval_number(&args[0]) {
        Ok(x) => x,
        Err(e) => return Operand::error(e),
    };
    let significance = match args.get(1) {
        Some(e) => match ev.eval_number(e) {
            Ok(s) => s.abs(),
            Err(e) => return Operand::error(e),
        },
        None => 1.0,
    };
    let away_from_zero = match args.get(2) {
        Some(e) => match ev.eval_number(e) {
            Ok(m) => m != 0.0,
            Err(e) => return Operand::error(e),
        },
        None => false,
    };
    Operand::Value(step_math_value(x, significance, up, away_from_zero))
}

fn step_math_value(x: f64, significance: f64, up: bool, away_from_zero: bool) -> Value {
    if significance == 0.0 {
        return Value::Number(0.0);
    }
    let q = x / significance;
    // For a positive number the mode does not apply. For a negative one, "away
    // from zero" flips which direction the step goes.
    let toward_positive = if x >= 0.0 { up } else { up != away_from_zero };
    finite(significance * if toward_positive { q.ceil() } else { q.floor() })
}

fn iso_ceiling(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let x = match ev.eval_number(&args[0]) {
        Ok(x) => x,
        Err(e) => return Operand::error(e),
    };
    let significance = match args.get(1) {
        Some(e) => match ev.eval_number(e) {
            Ok(s) => s.abs(),
            Err(e) => return Operand::error(e),
        },
        None => 1.0,
    };
    Operand::Value(step_math_value(x, significance, true, false))
}

/// `EVEN`, and the shared half of `ODD`: round away from zero to a multiple.
fn parity_round(x: f64, multiple: f64) -> Value {
    if x == 0.0 {
        return Value::Number(0.0);
    }
    let q = (x / multiple).abs().ceil();
    finite(q * multiple * x.signum())
}

fn odd(x: f64) -> Value {
    if x == 0.0 {
        return Value::Number(1.0);
    }
    // Shift into the even grid, round away from zero, shift back.
    let sign = x.signum();
    let n = ((x.abs() + 1.0) / 2.0).ceil() * 2.0 - 1.0;
    finite(sign * n)
}

/// The largest factorial an f64 can hold. 171! overflows to infinity.
const MAX_FACTORIAL: f64 = 170.0;

fn factorial(x: f64) -> Value {
    let n = x.trunc();
    if !(0.0..=MAX_FACTORIAL).contains(&n) {
        return Value::Error(CellError::Num);
    }
    let mut acc = 1.0f64;
    let mut i = 2.0;
    while i <= n {
        acc *= i;
        i += 1.0;
    }
    finite(acc)
}

fn combinations(n: f64, k: f64) -> Value {
    let (n, k) = (n.trunc(), k.trunc());
    if n < 0.0 || k < 0.0 || k > n {
        return Value::Error(CellError::Num);
    }
    // Multiplicative form, so C(1000, 2) does not have to go through 1000!.
    let k = k.min(n - k);
    let mut acc = 1.0f64;
    let mut i = 0.0;
    while i < k {
        acc = acc * (n - i) / (i + 1.0);
        i += 1.0;
    }
    finite(acc.round())
}

fn permutations(n: f64, k: f64) -> Value {
    let (n, k) = (n.trunc(), k.trunc());
    if n < 0.0 || k < 0.0 || k > n {
        return Value::Error(CellError::Num);
    }
    let mut acc = 1.0f64;
    let mut i = 0.0;
    while i < k {
        acc *= n - i;
        i += 1.0;
    }
    finite(acc)
}

/// `GCD` and `LCM` take whole numbers only, and reject negatives.
fn whole_number_fold(
    ev: &mut Evaluator,
    args: &[Expr],
    f: impl Fn(u64, u64) -> Option<u64>,
) -> Operand {
    if args.is_empty() {
        return Operand::error(CellError::Value);
    }
    let numbers = match numeric_args(ev, args) {
        Ok(ns) => ns,
        Err(e) => return Operand::error(e),
    };
    let mut acc: u64 = 0;
    let mut first = true;
    for n in numbers {
        let n = n.trunc();
        if !(0.0..=2f64.powi(53)).contains(&n) {
            return Operand::error(CellError::Num);
        }
        let n = n as u64;
        acc = if first {
            n
        } else {
            match f(acc, n) {
                Some(v) => v,
                None => return Operand::error(CellError::Num),
            }
        };
        first = false;
    }
    Operand::number(acc as f64)
}

fn gcd2(a: u64, b: u64) -> Option<u64> {
    let (mut a, mut b) = (a, b);
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    Some(a)
}

fn lcm2(a: u64, b: u64) -> Option<u64> {
    if a == 0 || b == 0 {
        return Some(0);
    }
    let g = gcd2(a, b)?;
    (a / g).checked_mul(b)
}

fn randbetween(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let low = match ev.eval_number(&args[0]) {
        Ok(n) => n.ceil(),
        Err(e) => return Operand::error(e),
    };
    let high = match ev.eval_number(&args[1]) {
        Ok(n) => n.floor(),
        Err(e) => return Operand::error(e),
    };
    if low > high {
        return Operand::error(CellError::Num);
    }
    let span = high - low + 1.0;
    Operand::number(low + (ev.next_random() * span).floor().min(span - 1.0))
}

/// `SUMPRODUCT` — element-wise product, summed. Non-numeric entries count as
/// zero rather than erroring, which is what makes the `(a=b)*(c=d)` idiom work.
fn sumproduct(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if args.is_empty() {
        return Operand::error(CellError::Value);
    }
    let mut arrays = Vec::with_capacity(args.len());
    for a in args {
        let op = ev.eval(a);
        if let Operand::Value(Value::Error(e)) = op {
            return Operand::error(e);
        }
        arrays.push(ev.spread(&op));
    }
    let (rows, cols) = (arrays[0].rows(), arrays[0].cols());
    if arrays.iter().any(|a| a.rows() != rows || a.cols() != cols) {
        return Operand::error(CellError::Value);
    }
    let mut total = 0.0;
    for r in 0..rows {
        for c in 0..cols {
            let mut term = 1.0;
            for a in &arrays {
                term *= match a.get(r, c) {
                    Some(Value::Number(n)) => *n,
                    Some(Value::Bool(b)) => f64::from(*b),
                    Some(Value::Error(e)) => return Operand::error(*e),
                    _ => 0.0,
                };
            }
            total += term;
        }
    }
    Operand::Value(finite(total))
}

/// `SUMIF(range, criteria, [sum_range])`.
fn sumif(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let mut total = 0.0;
    match visit_if(ev, args, &mut |v| {
        if let Value::Number(n) = v {
            total += n;
        }
    }) {
        Ok(()) => Operand::Value(finite(total)),
        Err(e) => Operand::error(e),
    }
}

/// `SUMIFS(sum_range, criteria_range1, criteria1, ...)` — note that the summed
/// range comes *first* here and last in `SUMIF`.
fn sumifs(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if args.len() < 3 || !(args.len() - 1).is_multiple_of(2) {
        return Operand::error(CellError::Value);
    }
    let mut total = 0.0;
    match visit_ifs(ev, Some(&args[0]), &args[1..], &mut |v| {
        if let Value::Number(n) = v {
            total += n;
        }
    }) {
        Ok(()) => Operand::Value(finite(total)),
        Err(e) => Operand::error(e),
    }
}
