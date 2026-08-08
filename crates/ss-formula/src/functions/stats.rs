//! Statistical functions.
//!
//! The arithmetic is mostly ordinary. What is not ordinary is *what gets
//! counted*, and Excel answers that question differently for almost every
//! function here:
//!
//! * `COUNT` counts numbers, `COUNTA` counts anything that is not an empty
//!   cell — including an error, and including a formula that returned `""`.
//! * `AVERAGE` skips text and booleans in a range; `AVERAGEA` scores text as
//!   zero and booleans as one. The pair exists because the first behaviour is
//!   right for a column with a heading and the second is right for a column of
//!   yes/no answers, and no single rule serves both.
//! * `MAX` of nothing at all is 0, not an error — which means an empty range and
//!   a range of zeros are indistinguishable.

use ss_model::CellError;

use crate::ast::Expr;
use crate::eval::{finite, Evaluator};
use crate::value::{Operand, Value};

use super::criteria::{visit_if, visit_ifs};
use super::{arity, numeric_args, visit_args, FnImpl, Source};

pub(super) fn lookup(name: &str) -> Option<FnImpl> {
    Some(match name {
        "COUNT" => count,
        "COUNTA" => counta,
        "COUNTBLANK" => countblank,
        "AVERAGE" => average,
        "AVERAGEA" => |ev: &mut Evaluator, a: &[Expr]| {
            let (sum, n) = match everything(ev, a) {
                Ok(v) => v.iter().fold((0.0, 0.0), |(s, c), x| (s + x, c + 1.0)),
                Err(e) => return Operand::error(e),
            };
            divide(sum, n)
        },
        "MAX" => |ev: &mut Evaluator, a: &[Expr]| extreme(ev, a, true),
        "MIN" => |ev: &mut Evaluator, a: &[Expr]| extreme(ev, a, false),
        "MAXA" => |ev: &mut Evaluator, a: &[Expr]| extreme_a(ev, a, true),
        "MINA" => |ev: &mut Evaluator, a: &[Expr]| extreme_a(ev, a, false),
        "MEDIAN" => median,
        "MODE" | "MODE.SNGL" => mode,
        "LARGE" => |ev: &mut Evaluator, a: &[Expr]| nth(ev, a, true),
        "SMALL" => |ev: &mut Evaluator, a: &[Expr]| nth(ev, a, false),
        "PERCENTILE" | "PERCENTILE.INC" => {
            |ev: &mut Evaluator, a: &[Expr]| percentile_fn(ev, a, true)
        }
        "PERCENTILE.EXC" => |ev: &mut Evaluator, a: &[Expr]| percentile_fn(ev, a, false),
        "QUARTILE" | "QUARTILE.INC" => |ev: &mut Evaluator, a: &[Expr]| quartile(ev, a, true),
        "QUARTILE.EXC" => |ev: &mut Evaluator, a: &[Expr]| quartile(ev, a, false),
        "RANK" | "RANK.EQ" => |ev: &mut Evaluator, a: &[Expr]| rank(ev, a, false),
        "RANK.AVG" => |ev: &mut Evaluator, a: &[Expr]| rank(ev, a, true),
        "VAR" | "VAR.S" => |ev: &mut Evaluator, a: &[Expr]| spread(ev, a, true, false),
        "VARP" | "VAR.P" => |ev: &mut Evaluator, a: &[Expr]| spread(ev, a, false, false),
        "STDEV" | "STDEV.S" => |ev: &mut Evaluator, a: &[Expr]| spread(ev, a, true, true),
        "STDEVP" | "STDEV.P" => |ev: &mut Evaluator, a: &[Expr]| spread(ev, a, false, true),
        "AVEDEV" => avedev,
        "DEVSQ" => devsq,
        "GEOMEAN" => geomean,
        "HARMEAN" => harmean,
        "COUNTIF" => countif,
        "COUNTIFS" => countifs,
        "AVERAGEIF" => averageif,
        "AVERAGEIFS" => averageifs,
        "MINIFS" => |ev: &mut Evaluator, a: &[Expr]| extreme_ifs(ev, a, false),
        "MAXIFS" => |ev: &mut Evaluator, a: &[Expr]| extreme_ifs(ev, a, true),
        "CORREL" | "PEARSON" => |ev: &mut Evaluator, a: &[Expr]| pairwise(ev, a, correlation),
        "RSQ" => |ev: &mut Evaluator, a: &[Expr]| {
            pairwise(ev, a, |xs, ys| correlation(xs, ys).map(|r| r * r))
        },
        "COVARIANCE.P" | "COVAR" => {
            |ev: &mut Evaluator, a: &[Expr]| pairwise(ev, a, |xs, ys| covariance(xs, ys, false))
        }
        "COVARIANCE.S" => {
            |ev: &mut Evaluator, a: &[Expr]| pairwise(ev, a, |xs, ys| covariance(xs, ys, true))
        }
        // The regression pair takes its arguments the other way round from
        // everything else here: known *y* values first.
        "SLOPE" => |ev: &mut Evaluator, a: &[Expr]| {
            pairwise(ev, a, |ys, xs| line_fit(xs, ys).map(|(m, _)| m))
        },
        "INTERCEPT" => |ev: &mut Evaluator, a: &[Expr]| {
            pairwise(ev, a, |ys, xs| line_fit(xs, ys).map(|(_, b)| b))
        },
        "FORECAST" | "FORECAST.LINEAR" => forecast,
        _ => return None,
    })
}

/// Every value scored the way the `A`-suffixed functions score them: text is
/// zero, booleans are one and zero, and only genuinely empty cells are skipped.
fn everything(ev: &mut Evaluator, args: &[Expr]) -> Result<Vec<f64>, CellError> {
    let mut out = Vec::new();
    let mut err = None;
    visit_args(ev, args, &mut |v, source| {
        if err.is_some() {
            return;
        }
        match (v, source) {
            (Value::Error(e), _) => err = Some(*e),
            (Value::Blank, Source::Inside) => {}
            (Value::Blank, Source::Direct) => out.push(0.0),
            (Value::Number(n), _) => out.push(*n),
            (Value::Bool(b), _) => out.push(f64::from(*b)),
            // Text scores zero even when it spells a number, which is the whole
            // difference between `AVERAGEA` and `AVERAGE`.
            (Value::Text(_), _) => out.push(0.0),
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(out),
    }
}

fn divide(sum: f64, n: f64) -> Operand {
    if n == 0.0 {
        // An average of nothing is a division by zero, and Excel says so rather
        // than returning 0 the way `MAX` does.
        return Operand::error(CellError::Div0);
    }
    Operand::Value(finite(sum / n))
}

/// `COUNT` — numbers only. A boolean written directly counts; the same boolean
/// inside a range does not, exactly as in `SUM`.
fn count(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    let mut n = 0.0;
    visit_args(ev, args, &mut |v, source| match (v, source) {
        (Value::Number(_), _) => n += 1.0,
        (Value::Error(_), _) | (_, Source::Inside) => {}
        // Directly written text counts only if it reads as a number.
        (Value::Text(t), Source::Direct) => {
            if crate::value::text_to_number(t).is_some() {
                n += 1.0;
            }
        }
        (Value::Bool(_), Source::Direct) => n += 1.0,
        (Value::Blank, Source::Direct) => {}
    });
    Operand::number(n)
}

/// `COUNTA` — anything that is not an empty cell, errors included.
fn counta(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    let mut n = 0.0;
    visit_args(ev, args, &mut |v, _| {
        if !matches!(v, Value::Blank) {
            n += 1.0;
        }
    });
    Operand::number(n)
}

/// `COUNTBLANK(range)` — empty cells, plus the ones holding an empty string.
///
/// A formula that returned `""` looks empty and counts as blank here, which is
/// the one place Excel treats it that way. `ISBLANK` on the same cell is FALSE.
fn countblank(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    let op = ev.eval(&args[0]);
    let mut n = 0.0;
    for v in ev.spread(&op).values() {
        if matches!(v, Value::Blank) || matches!(v, Value::Text(t) if t.is_empty()) {
            n += 1.0;
        }
    }
    Operand::number(n)
}

fn average(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    match numeric_args(ev, args) {
        Ok(v) => divide(v.iter().sum(), v.len() as f64),
        Err(e) => Operand::error(e),
    }
}

/// `MAX` and `MIN`.
///
/// Over an empty set both are 0. That is not an oversight to fix: files rely on
/// it, and returning `#NUM!` would break a column of `MAX(range)` the moment the
/// range was cleared.
fn extreme(ev: &mut Evaluator, args: &[Expr], want_max: bool) -> Operand {
    match numeric_args(ev, args) {
        Ok(v) => Operand::number(pick_extreme(&v, want_max)),
        Err(e) => Operand::error(e),
    }
}

fn extreme_a(ev: &mut Evaluator, args: &[Expr], want_max: bool) -> Operand {
    match everything(ev, args) {
        Ok(v) => Operand::number(pick_extreme(&v, want_max)),
        Err(e) => Operand::error(e),
    }
}

fn pick_extreme(v: &[f64], want_max: bool) -> f64 {
    v.iter().copied().fold(
        if v.is_empty() {
            0.0
        } else if want_max {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        },
        |a, b| if want_max { a.max(b) } else { a.min(b) },
    )
}

/// Sorts ascending, which every order statistic below needs first.
fn sorted(ev: &mut Evaluator, args: &[Expr]) -> Result<Vec<f64>, CellError> {
    let mut v = numeric_args(ev, args)?;
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Ok(v)
}

fn median(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    let v = match sorted(ev, args) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    if v.is_empty() {
        return Operand::error(CellError::Num);
    }
    let mid = v.len() / 2;
    Operand::number(if v.len() % 2 == 1 {
        v[mid]
    } else {
        (v[mid - 1] + v[mid]) / 2.0
    })
}

/// `MODE` — the most common value, and the *first* one when several tie.
fn mode(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    let v = match numeric_args(ev, args) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    let mut best: Option<(f64, usize)> = None;
    for (i, x) in v.iter().enumerate() {
        let count =
            v[i..].iter().filter(|y| *y == x).count() + v[..i].iter().filter(|y| *y == x).count();
        if count > 1 && best.is_none_or(|(_, c)| count > c) {
            best = Some((*x, count));
        }
    }
    match best {
        Some((x, _)) => Operand::number(x),
        // No value repeats, so there is no mode. Excel reports `#N/A` rather
        // than picking one arbitrarily.
        None => Operand::error(CellError::NotAvailable),
    }
}

/// `LARGE(array, k)` and `SMALL(array, k)`.
fn nth(ev: &mut Evaluator, args: &[Expr], largest: bool) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let v = match sorted(ev, &args[..1]) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    let k = match ev.eval_number(&args[1]) {
        Ok(n) => n.trunc(),
        Err(e) => return Operand::error(e),
    };
    if v.is_empty() || k < 1.0 || k > v.len() as f64 {
        return Operand::error(CellError::Num);
    }
    let i = k as usize - 1;
    Operand::number(if largest { v[v.len() - 1 - i] } else { v[i] })
}

/// Linear interpolation between order statistics, which is what every
/// percentile function here reduces to.
fn interpolate(v: &[f64], position: f64) -> f64 {
    let lo = position.floor() as usize;
    let frac = position - position.floor();
    if lo + 1 >= v.len() {
        return v[v.len() - 1];
    }
    v[lo] + frac * (v[lo + 1] - v[lo])
}

fn percentile_fn(ev: &mut Evaluator, args: &[Expr], inclusive: bool) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let v = match sorted(ev, &args[..1]) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    let p = match ev.eval_number(&args[1]) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    percentile_of(&v, p, inclusive)
}

fn percentile_of(v: &[f64], p: f64, inclusive: bool) -> Operand {
    if v.is_empty() {
        return Operand::error(CellError::Num);
    }
    let n = v.len() as f64;
    if inclusive {
        if !(0.0..=1.0).contains(&p) {
            return Operand::error(CellError::Num);
        }
        Operand::number(interpolate(v, p * (n - 1.0)))
    } else {
        // The exclusive form has no 0th or 100th percentile: with n values it
        // can only place the inner n-1 gaps.
        if p <= 0.0 || p >= 1.0 || p * (n + 1.0) < 1.0 || p * (n + 1.0) > n {
            return Operand::error(CellError::Num);
        }
        Operand::number(interpolate(v, p * (n + 1.0) - 1.0))
    }
}

fn quartile(ev: &mut Evaluator, args: &[Expr], inclusive: bool) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let v = match sorted(ev, &args[..1]) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    let q = match ev.eval_number(&args[1]) {
        Ok(n) => n.trunc(),
        Err(e) => return Operand::error(e),
    };
    if !(0.0..=4.0).contains(&q) {
        return Operand::error(CellError::Num);
    }
    percentile_of(&v, q / 4.0, inclusive)
}

/// `RANK(number, ref, [order])` — where a value sits in its range.
///
/// `RANK.AVG` differs only in how it settles ties: the shared positions are
/// averaged rather than all taking the best one.
fn rank(ev: &mut Evaluator, args: &[Expr], average_ties: bool) -> Operand {
    if !arity(args, 2, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let target = match ev.eval_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    let v = match numeric_args(ev, &args[1..2]) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    let ascending = match args.get(2) {
        Some(e) => match ev.eval_number(e) {
            Ok(n) => n != 0.0,
            Err(e) => return Operand::error(e),
        },
        None => false,
    };
    if !v.contains(&target) {
        return Operand::error(CellError::NotAvailable);
    }
    let better = v
        .iter()
        .filter(|x| {
            if ascending {
                **x < target
            } else {
                **x > target
            }
        })
        .count();
    let ties = v.iter().filter(|x| **x == target).count();
    let base = better as f64 + 1.0;
    Operand::number(if average_ties {
        base + (ties as f64 - 1.0) / 2.0
    } else {
        base
    })
}

/// Variance and standard deviation, sample or population.
fn spread(ev: &mut Evaluator, args: &[Expr], sample: bool, root: bool) -> Operand {
    let v = match numeric_args(ev, args) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    let n = v.len() as f64;
    // A sample variance needs two observations; one gives a zero denominator,
    // and Excel reports that as a division by zero rather than as 0.
    if v.is_empty() || (sample && v.len() < 2) {
        return Operand::error(CellError::Div0);
    }
    let mean = v.iter().sum::<f64>() / n;
    let ss: f64 = v.iter().map(|x| (x - mean) * (x - mean)).sum();
    let variance = ss / if sample { n - 1.0 } else { n };
    Operand::Value(finite(if root { variance.sqrt() } else { variance }))
}

fn avedev(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    let v = match numeric_args(ev, args) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    if v.is_empty() {
        return Operand::error(CellError::Num);
    }
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    let total: f64 = v.iter().map(|x| (x - mean).abs()).sum();
    Operand::Value(finite(total / v.len() as f64))
}

fn devsq(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    let v = match numeric_args(ev, args) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    if v.is_empty() {
        return Operand::error(CellError::Num);
    }
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    Operand::Value(finite(v.iter().map(|x| (x - mean) * (x - mean)).sum()))
}

fn geomean(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    let v = match numeric_args(ev, args) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    // A single non-positive value makes the whole product meaningless, so this
    // is `#NUM!` rather than a signed answer.
    if v.is_empty() || v.iter().any(|x| *x <= 0.0) {
        return Operand::error(CellError::Num);
    }
    // Through logarithms, so a long series does not overflow before the root.
    let mean = v.iter().map(|x| x.ln()).sum::<f64>() / v.len() as f64;
    Operand::Value(finite(mean.exp()))
}

fn harmean(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    let v = match numeric_args(ev, args) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    if v.is_empty() || v.iter().any(|x| *x <= 0.0) {
        return Operand::error(CellError::Num);
    }
    let total: f64 = v.iter().map(|x| 1.0 / x).sum();
    Operand::Value(finite(v.len() as f64 / total))
}

fn countif(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let mut n = 0.0;
    match visit_if(ev, args, &mut |_| n += 1.0) {
        Ok(()) => Operand::number(n),
        Err(e) => Operand::error(e),
    }
}

fn countifs(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if args.is_empty() || !args.len().is_multiple_of(2) {
        return Operand::error(CellError::Value);
    }
    let mut n = 0.0;
    match visit_ifs(ev, None, args, &mut |_| n += 1.0) {
        Ok(()) => Operand::number(n),
        Err(e) => Operand::error(e),
    }
}

fn averageif(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let (mut sum, mut n) = (0.0, 0.0);
    match visit_if(ev, args, &mut |v| {
        if let Value::Number(x) = v {
            sum += x;
            n += 1.0;
        }
    }) {
        Ok(()) => divide(sum, n),
        Err(e) => Operand::error(e),
    }
}

fn averageifs(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if args.len() < 3 || !(args.len() - 1).is_multiple_of(2) {
        return Operand::error(CellError::Value);
    }
    let (mut sum, mut n) = (0.0, 0.0);
    match visit_ifs(ev, Some(&args[0]), &args[1..], &mut |v| {
        if let Value::Number(x) = v {
            sum += x;
            n += 1.0;
        }
    }) {
        Ok(()) => divide(sum, n),
        Err(e) => Operand::error(e),
    }
}

fn extreme_ifs(ev: &mut Evaluator, args: &[Expr], want_max: bool) -> Operand {
    if args.len() < 3 || !(args.len() - 1).is_multiple_of(2) {
        return Operand::error(CellError::Value);
    }
    let mut found: Vec<f64> = Vec::new();
    match visit_ifs(ev, Some(&args[0]), &args[1..], &mut |v| {
        if let Value::Number(x) = v {
            found.push(*x);
        }
    }) {
        Ok(()) => Operand::number(pick_extreme(&found, want_max)),
        Err(e) => Operand::error(e),
    }
}

/// Reads two equal-length series of numbers and hands them to a statistic.
///
/// Pairs where either side is not a number are dropped whole — that is what
/// makes `CORREL` over two columns with a shared gap work — so the two vectors
/// stay aligned by position rather than by count.
fn pairwise(
    ev: &mut Evaluator,
    args: &[Expr],
    f: impl Fn(&[f64], &[f64]) -> Option<f64>,
) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let a = ev.eval(&args[0]);
    let b = ev.eval(&args[1]);
    let (a, b) = (ev.spread(&a), ev.spread(&b));
    if a.rows() * a.cols() != b.rows() * b.cols() {
        return Operand::error(CellError::NotAvailable);
    }
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for (x, y) in a.values().zip(b.values()) {
        if let Value::Error(e) = x {
            return Operand::error(*e);
        }
        if let Value::Error(e) = y {
            return Operand::error(*e);
        }
        if let (Value::Number(x), Value::Number(y)) = (x, y) {
            xs.push(*x);
            ys.push(*y);
        }
    }
    match f(&xs, &ys) {
        Some(v) => Operand::Value(finite(v)),
        None => Operand::error(CellError::Div0),
    }
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

fn covariance(xs: &[f64], ys: &[f64], sample: bool) -> Option<f64> {
    if xs.is_empty() || (sample && xs.len() < 2) {
        return None;
    }
    let (mx, my) = (mean(xs), mean(ys));
    let total: f64 = xs.iter().zip(ys).map(|(x, y)| (x - mx) * (y - my)).sum();
    let n = xs.len() as f64;
    Some(total / if sample { n - 1.0 } else { n })
}

fn correlation(xs: &[f64], ys: &[f64]) -> Option<f64> {
    if xs.len() < 2 {
        return None;
    }
    let (mx, my) = (mean(xs), mean(ys));
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    for (x, y) in xs.iter().zip(ys) {
        sxy += (x - mx) * (y - my);
        sxx += (x - mx) * (x - mx);
        syy += (y - my) * (y - my);
    }
    let denominator = (sxx * syy).sqrt();
    (denominator != 0.0).then(|| sxy / denominator)
}

/// The least-squares line through the points, as (slope, intercept).
fn line_fit(xs: &[f64], ys: &[f64]) -> Option<(f64, f64)> {
    if xs.len() < 2 {
        return None;
    }
    let (mx, my) = (mean(xs), mean(ys));
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    for (x, y) in xs.iter().zip(ys) {
        sxy += (x - mx) * (y - my);
        sxx += (x - mx) * (x - mx);
    }
    // Every x the same: the line is vertical and has no slope.
    (sxx != 0.0).then(|| {
        let slope = sxy / sxx;
        (slope, my - slope * mx)
    })
}

/// `FORECAST(x, known_ys, known_xs)` — the fitted line evaluated at `x`.
fn forecast(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let x = match ev.eval_number(&args[0]) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    pairwise(ev, &args[1..], move |ys, xs| {
        line_fit(xs, ys).map(|(m, b)| m * x + b)
    })
}
