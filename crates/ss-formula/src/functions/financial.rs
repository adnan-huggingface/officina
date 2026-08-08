//! Financial functions.
//!
//! Two conventions run through all of them and getting either wrong gives
//! answers that look plausible and are not:
//!
//! **Money you pay out is negative.** `PMT(0.05/12, 360, 200000)` is *minus*
//! 1073.64, because the payment leaves your hand. Every function here follows
//! the sign convention, and a caller that ignores it gets a loan that pays the
//! borrower.
//!
//! **`type` is when in the period the payment falls**: 0 for the end (the
//! default, an ordinary annuity) and 1 for the beginning (an annuity due). One
//! period's interest separates them, which on a mortgage is thousands.
//!
//! The rate solvers — `RATE`, `IRR`, `XIRR` — have no closed form. They use
//! Newton's method with a bisection fallback, because Newton alone diverges on
//! a cash-flow series with more than one sign change, and returning `#NUM!` for
//! a solvable problem is worse than being slower.

use ss_model::CellError;

use crate::ast::Expr;
use crate::eval::Evaluator;
use crate::value::{Operand, Value};

use super::{arity, scalar_args, visit_args, FnImpl, Source};

pub(super) fn lookup(name: &str) -> Option<FnImpl> {
    Some(match name {
        "PMT" => |ev: &mut Evaluator, a: &[Expr]| annuity(ev, a, Part::Payment),
        "PV" => |ev: &mut Evaluator, a: &[Expr]| annuity(ev, a, Part::Present),
        "FV" => |ev: &mut Evaluator, a: &[Expr]| annuity(ev, a, Part::Future),
        "NPER" => |ev: &mut Evaluator, a: &[Expr]| annuity(ev, a, Part::Periods),
        "RATE" => rate,
        "IPMT" => |ev: &mut Evaluator, a: &[Expr]| part_payment(ev, a, true),
        "PPMT" => |ev: &mut Evaluator, a: &[Expr]| part_payment(ev, a, false),
        "CUMIPMT" => |ev: &mut Evaluator, a: &[Expr]| cumulative(ev, a, true),
        "CUMPRINC" => |ev: &mut Evaluator, a: &[Expr]| cumulative(ev, a, false),
        "NPV" => npv,
        "IRR" => irr,
        "MIRR" => mirr,
        "XNPV" => xnpv,
        "XIRR" => xirr,
        "SLN" => sln,
        "SYD" => syd,
        "DB" => db,
        "DDB" => ddb,
        "EFFECT" => effect,
        "NOMINAL" => nominal,
        "RRI" => rri,
        "PDURATION" => pduration,
        "DOLLARDE" => |ev: &mut Evaluator, a: &[Expr]| fractional(ev, a, true),
        "DOLLARFR" => |ev: &mut Evaluator, a: &[Expr]| fractional(ev, a, false),
        _ => return None,
    })
}

/// Which unknown of the annuity equation is being solved for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Payment,
    Present,
    Future,
    Periods,
}

/// Reads the arguments as numbers, filling missing trailing ones with zero.
fn numbers(ev: &mut Evaluator, args: &[Expr], count: usize) -> Result<Vec<f64>, CellError> {
    let values = scalar_args(ev, args)?;
    let mut out = vec![0.0; count];
    for (index, value) in values.iter().enumerate().take(count) {
        out[index] = match value {
            Value::Blank => 0.0,
            other => other.to_number()?,
        };
    }
    Ok(out)
}

/// The annuity identity, rearranged for whichever term is unknown.
///
/// `pv(1+r)^n + pmt(1 + r·type)((1+r)^n − 1)/r + fv = 0`, with the zero-rate
/// case handled separately because the division by `r` is undefined there — and
/// a zero-interest loan is a perfectly ordinary thing to model.
fn annuity(ev: &mut Evaluator, args: &[Expr], part: Part) -> Operand {
    let wanted = match part {
        Part::Payment => (3, 5),
        Part::Present => (3, 5),
        Part::Future => (3, 5),
        Part::Periods => (3, 5),
    };
    if !arity(args, wanted.0, Some(wanted.1)) {
        return Operand::error(CellError::Value);
    }
    let n = match numbers(ev, args, 5) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    let due = n[4] != 0.0;

    let result = match part {
        Part::Payment => {
            let (rate, periods, pv, fv) = (n[0], n[1], n[2], n[3]);
            if periods == 0.0 {
                return Operand::error(CellError::Num);
            }
            Some(pmt(rate, periods, pv, fv, due))
        }
        Part::Future => {
            let (rate, periods, payment, pv) = (n[0], n[1], n[2], n[3]);
            Some(fv(rate, periods, payment, pv, due))
        }
        Part::Present => {
            let (rate, periods, payment, future) = (n[0], n[1], n[2], n[3]);
            if rate == 0.0 {
                Some(-future - payment * periods)
            } else {
                let growth = (1.0 + rate).powf(periods);
                let factor = if due { 1.0 + rate } else { 1.0 };
                Some(-(future + payment * factor * (growth - 1.0) / rate) / growth)
            }
        }
        Part::Periods => {
            let (rate, payment, pv, future) = (n[0], n[1], n[2], n[3]);
            if rate == 0.0 {
                if payment == 0.0 {
                    None
                } else {
                    Some(-(pv + future) / payment)
                }
            } else {
                let factor = if due { 1.0 + rate } else { 1.0 };
                let adjusted = payment * factor / rate;
                let numerator = adjusted - future;
                let denominator = pv + adjusted;
                if numerator / denominator <= 0.0 {
                    None
                } else {
                    Some((numerator / denominator).ln() / (1.0 + rate).ln())
                }
            }
        }
    };
    match result.filter(|v| v.is_finite()) {
        Some(value) => Operand::number(value),
        None => Operand::error(CellError::Num),
    }
}

fn pmt(rate: f64, periods: f64, pv: f64, fv: f64, due: bool) -> f64 {
    if rate == 0.0 {
        return -(pv + fv) / periods;
    }
    let growth = (1.0 + rate).powf(periods);
    let factor = if due { 1.0 + rate } else { 1.0 };
    -(pv * growth + fv) * rate / (factor * (growth - 1.0))
}

fn fv(rate: f64, periods: f64, payment: f64, pv: f64, due: bool) -> f64 {
    if rate == 0.0 {
        return -(pv + payment * periods);
    }
    let growth = (1.0 + rate).powf(periods);
    let factor = if due { 1.0 + rate } else { 1.0 };
    -(pv * growth + payment * factor * (growth - 1.0) / rate)
}

/// `IPMT` and `PPMT`: how one period's payment splits.
fn part_payment(ev: &mut Evaluator, args: &[Expr], interest: bool) -> Operand {
    if !arity(args, 4, Some(6)) {
        return Operand::error(CellError::Value);
    }
    let n = match numbers(ev, args, 6) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    let (rate, period, periods, pv, future, due) = (n[0], n[1], n[2], n[3], n[4], n[5] != 0.0);
    if period < 1.0 || period > periods {
        return Operand::error(CellError::Num);
    }
    let payment = pmt(rate, periods, pv, future, due);
    // The balance at the start of the period is the future value of everything
    // up to the period before it.
    // `fv` already carries the sign convention: with money received as a
    // positive present value it comes back negative, so the interest on it is
    // negative too — money paid out. Negating here would hand the borrower the
    // interest.
    let balance = fv(rate, period - 1.0, payment, pv, due);
    let mut owed = balance * rate;
    if due && period > 1.0 {
        owed /= 1.0 + rate;
    }
    if due && period == 1.0 {
        // A payment at the start of the first period earns no interest.
        owed = 0.0;
    }
    Operand::number(if interest { owed } else { payment - owed })
}

/// `CUMIPMT` and `CUMPRINC`, summed over a span of periods.
fn cumulative(ev: &mut Evaluator, args: &[Expr], interest: bool) -> Operand {
    if !arity(args, 6, Some(6)) {
        return Operand::error(CellError::Value);
    }
    let n = match numbers(ev, args, 6) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    let (rate, periods, pv, start, end, due) = (n[0], n[1], n[2], n[3], n[4], n[5] != 0.0);
    if rate <= 0.0 || periods <= 0.0 || pv <= 0.0 || start < 1.0 || end < start || end > periods {
        return Operand::error(CellError::Num);
    }
    let payment = pmt(rate, periods, pv, 0.0, due);
    let mut total = 0.0;
    for period in (start as u64)..=(end as u64) {
        let balance = fv(rate, period as f64 - 1.0, payment, pv, due);
        let mut owed = balance * rate;
        if due {
            owed = if period == 1 {
                0.0
            } else {
                owed / (1.0 + rate)
            };
        }
        total += if interest { owed } else { payment - owed };
    }
    Operand::number(total)
}

/// Solves for the rate, which has no closed form.
fn rate(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, Some(6)) {
        return Operand::error(CellError::Value);
    }
    let mut n = match numbers(ev, args, 6) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    if args.len() < 6 || n[5] == 0.0 {
        n[5] = 0.1; // Excel's default guess.
    }
    let (periods, payment, pv, future, due) = (n[0], n[1], n[2], n[3], n[4] != 0.0);
    let residual = |rate: f64| {
        if rate == 0.0 {
            pv + payment * periods + future
        } else {
            let growth = (1.0 + rate).powf(periods);
            let factor = if due { 1.0 + rate } else { 1.0 };
            pv * growth + payment * factor * (growth - 1.0) / rate + future
        }
    };
    match solve(residual, n[5]) {
        Some(rate) => Operand::number(rate),
        None => Operand::error(CellError::Num),
    }
}

/// `NPV(rate, ...)`: the first cash flow is discounted by one period.
///
/// Excel's `NPV` is not the textbook NPV. The textbook version treats the first
/// flow as happening now; Excel discounts it once, so an initial outlay has to
/// be added *outside* the call. Matching the textbook here would give every
/// existing spreadsheet a different answer.
fn npv(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, None) {
        return Operand::error(CellError::Value);
    }
    let rate = match ev.eval_scalar(&args[0]).to_number() {
        Ok(r) => r,
        Err(e) => return Operand::error(e),
    };
    if rate == -1.0 {
        return Operand::error(CellError::Div0);
    }
    let flows = match cash_flows(ev, &args[1..]) {
        Ok(flows) => flows,
        Err(e) => return Operand::error(e),
    };
    let total = flows
        .iter()
        .enumerate()
        .map(|(index, flow)| flow / (1.0 + rate).powi(index as i32 + 1))
        .sum::<f64>();
    Operand::number(total)
}

fn irr(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let flows = match cash_flows(ev, &args[..1]) {
        Ok(flows) => flows,
        Err(e) => return Operand::error(e),
    };
    let guess = match args.get(1) {
        Some(expr) => match ev.eval_scalar(expr).to_number() {
            Ok(g) => g,
            Err(e) => return Operand::error(e),
        },
        None => 0.1,
    };
    let residual = |rate: f64| {
        flows
            .iter()
            .enumerate()
            .map(|(index, flow)| flow / (1.0 + rate).powi(index as i32))
            .sum::<f64>()
    };
    match solve(residual, guess) {
        Some(rate) => Operand::number(rate),
        None => Operand::error(CellError::Num),
    }
}

/// `MIRR`: reinvestment and finance rates that differ, which is what makes it
/// answerable when `IRR` has several roots.
fn mirr(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let flows = match cash_flows(ev, &args[..1]) {
        Ok(flows) => flows,
        Err(e) => return Operand::error(e),
    };
    let finance = match ev.eval_scalar(&args[1]).to_number() {
        Ok(r) => r,
        Err(e) => return Operand::error(e),
    };
    let reinvest = match ev.eval_scalar(&args[2]).to_number() {
        Ok(r) => r,
        Err(e) => return Operand::error(e),
    };
    let periods = flows.len();
    if periods < 2 {
        return Operand::error(CellError::Div0);
    }
    let negatives: f64 = flows
        .iter()
        .enumerate()
        .filter(|(_, f)| **f < 0.0)
        .map(|(i, f)| f / (1.0 + finance).powi(i as i32))
        .sum();
    let positives: f64 = flows
        .iter()
        .enumerate()
        .filter(|(_, f)| **f > 0.0)
        .map(|(i, f)| f * (1.0 + reinvest).powi((periods - 1 - i) as i32))
        .sum();
    if negatives == 0.0 {
        return Operand::error(CellError::Div0);
    }
    let ratio = -positives / negatives;
    if ratio <= 0.0 {
        return Operand::error(CellError::Num);
    }
    Operand::number(ratio.powf(1.0 / (periods as f64 - 1.0)) - 1.0)
}

fn xnpv(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let rate = match ev.eval_scalar(&args[0]).to_number() {
        Ok(r) => r,
        Err(e) => return Operand::error(e),
    };
    let (flows, dates) = match dated_flows(ev, args) {
        Ok(pair) => pair,
        Err(e) => return Operand::error(e),
    };
    let start = dates[0];
    let total = flows
        .iter()
        .zip(&dates)
        .map(|(flow, date)| flow / (1.0 + rate).powf((date - start) / 365.0))
        .sum::<f64>();
    Operand::number(total)
}

fn xirr(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let (flows, dates) = match dated_flows(ev, &args[..2]) {
        Ok(pair) => pair,
        Err(e) => return Operand::error(e),
    };
    let guess = match args.get(2) {
        Some(expr) => match ev.eval_scalar(expr).to_number() {
            Ok(g) => g,
            Err(e) => return Operand::error(e),
        },
        None => 0.1,
    };
    let start = dates[0];
    let residual = |rate: f64| {
        flows
            .iter()
            .zip(&dates)
            .map(|(flow, date)| flow / (1.0 + rate).powf((date - start) / 365.0))
            .sum::<f64>()
    };
    match solve(residual, guess) {
        Some(rate) => Operand::number(rate),
        None => Operand::error(CellError::Num),
    }
}

/// The first two arguments of `XNPV`/`XIRR`, as parallel series.
fn dated_flows(ev: &mut Evaluator, args: &[Expr]) -> Result<(Vec<f64>, Vec<f64>), CellError> {
    let offset = usize::from(args.len() == 3);
    let flows = cash_flows(ev, &args[offset..offset + 1])?;
    let dates = cash_flows(ev, &args[offset + 1..offset + 2])?;
    if flows.len() != dates.len() || flows.is_empty() {
        return Err(CellError::Num);
    }
    Ok((flows, dates))
}

/// Every number in the arguments, in order, skipping text and blanks.
fn cash_flows(ev: &mut Evaluator, args: &[Expr]) -> Result<Vec<f64>, CellError> {
    let mut out = Vec::new();
    let mut error = None;
    visit_args(ev, args, &mut |value, source| match value {
        Value::Number(n) => out.push(*n),
        Value::Bool(b) if source == Source::Direct => out.push(f64::from(*b)),
        Value::Error(e) => error = error.or(Some(*e)),
        _ => {}
    });
    match error {
        Some(e) => Err(e),
        None => Ok(out),
    }
}

/// Newton's method, falling back to bisection.
///
/// Newton alone diverges whenever the cash flows change sign more than once,
/// which is common — a project with a mid-life reinvestment has two roots. The
/// bracket search finds a sign change and bisects it, which always converges if
/// a root exists at all.
fn solve(residual: impl Fn(f64) -> f64, guess: f64) -> Option<f64> {
    let mut rate = guess;
    for _ in 0..64 {
        let value = residual(rate);
        if !value.is_finite() {
            break;
        }
        if value.abs() < 1e-10 {
            return Some(rate);
        }
        // A numeric derivative: the analytic one differs per caller and the
        // step is far below the tolerance either way.
        let step = 1e-7;
        let slope = (residual(rate + step) - value) / step;
        if slope.abs() < 1e-14 {
            break;
        }
        let next = rate - value / slope;
        if !next.is_finite() {
            break;
        }
        if (next - rate).abs() < 1e-12 {
            return Some(next);
        }
        rate = next;
    }

    // Bracket, then bisect.
    let mut low = -0.999_999;
    let mut high = 1.0;
    let mut low_value = residual(low);
    let mut high_value = residual(high);
    let mut steps = 0;
    while low_value * high_value > 0.0 && steps < 64 {
        high *= 2.0;
        high_value = residual(high);
        steps += 1;
    }
    if !(low_value.is_finite() && high_value.is_finite()) || low_value * high_value > 0.0 {
        return None;
    }
    for _ in 0..200 {
        let middle = (low + high) / 2.0;
        let value = residual(middle);
        if value.abs() < 1e-12 || (high - low).abs() < 1e-13 {
            return Some(middle);
        }
        if value * low_value < 0.0 {
            high = middle;
        } else {
            low = middle;
            low_value = value;
        }
    }
    None
}

fn sln(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let n = match numbers(ev, args, 3) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    if n[2] == 0.0 {
        return Operand::error(CellError::Div0);
    }
    Operand::number((n[0] - n[1]) / n[2])
}

fn syd(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 4, Some(4)) {
        return Operand::error(CellError::Value);
    }
    let n = match numbers(ev, args, 4) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    let (cost, salvage, life, period) = (n[0], n[1], n[2], n[3]);
    if life <= 0.0 || period < 1.0 || period > life {
        return Operand::error(CellError::Num);
    }
    Operand::number((cost - salvage) * (life - period + 1.0) * 2.0 / (life * (life + 1.0)))
}

/// `DB`: fixed-declining balance, with Excel's rate rounded to three places.
///
/// The rounding is not an approximation, it is the definition: Excel rounds the
/// rate to three decimals before applying it, and a version that does not
/// drifts a little further from Excel every period.
fn db(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 4, Some(5)) {
        return Operand::error(CellError::Value);
    }
    let mut n = match numbers(ev, args, 5) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    if args.len() < 5 {
        n[4] = 12.0;
    }
    let (cost, salvage, life, period, months) = (n[0], n[1], n[2], n[3], n[4]);
    if cost <= 0.0 || life <= 0.0 || period < 1.0 {
        return Operand::error(CellError::Num);
    }
    // The three-place rounding is the definition, not an approximation: Excel
    // rounds the rate before applying it, and a version that does not drifts
    // further from Excel every period.
    let rate = ((1.0 - (salvage / cost).powf(1.0 / life)) * 1000.0).round() / 1000.0;

    let first = cost * rate * months / 12.0;
    if period == 1.0 {
        return Operand::number(first);
    }
    let mut total = first;
    let mut value = 0.0;
    for index in 2..=(period as u64) {
        value = (cost - total) * rate;
        if index as f64 == (life + 1.0).floor() && months < 12.0 {
            value = (cost - total) * rate * (12.0 - months) / 12.0;
        }
        total += value;
    }
    Operand::number(value)
}

/// `DDB`: double-declining balance, never dropping below the salvage value.
fn ddb(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 4, Some(5)) {
        return Operand::error(CellError::Value);
    }
    let mut n = match numbers(ev, args, 5) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    if args.len() < 5 {
        n[4] = 2.0;
    }
    let (cost, salvage, life, period, factor) = (n[0], n[1], n[2], n[3], n[4]);
    if life <= 0.0 || period < 1.0 || factor <= 0.0 {
        return Operand::error(CellError::Num);
    }
    let mut total = 0.0;
    let mut value = 0.0;
    for _ in 1..=(period.ceil() as u64) {
        value = ((cost - total) * factor / life)
            .min(cost - salvage - total)
            .max(0.0);
        total += value;
    }
    Operand::number(value)
}

fn effect(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let n = match numbers(ev, args, 2) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    let periods = n[1].trunc();
    if n[0] <= 0.0 || periods < 1.0 {
        return Operand::error(CellError::Num);
    }
    Operand::number((1.0 + n[0] / periods).powf(periods) - 1.0)
}

fn nominal(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let n = match numbers(ev, args, 2) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    let periods = n[1].trunc();
    if n[0] <= 0.0 || periods < 1.0 {
        return Operand::error(CellError::Num);
    }
    Operand::number(((1.0 + n[0]).powf(1.0 / periods) - 1.0) * periods)
}

fn rri(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let n = match numbers(ev, args, 3) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    if n[0] <= 0.0 || n[1] <= 0.0 {
        return Operand::error(CellError::Num);
    }
    Operand::number((n[2] / n[1]).powf(1.0 / n[0]) - 1.0)
}

fn pduration(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let n = match numbers(ev, args, 3) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    if n[0] <= 0.0 || n[1] <= 0.0 || n[2] <= 0.0 {
        return Operand::error(CellError::Num);
    }
    Operand::number((n[2].ln() - n[1].ln()) / (1.0 + n[0]).ln())
}

/// `DOLLARDE` and `DOLLARFR`: prices quoted in sixteenths and thirty-seconds.
fn fractional(ev: &mut Evaluator, args: &[Expr], to_decimal: bool) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let n = match numbers(ev, args, 2) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    let denominator = n[1].trunc();
    if denominator < 1.0 {
        return Operand::error(if denominator == 0.0 {
            CellError::Div0
        } else {
            CellError::Num
        });
    }
    let whole = n[0].trunc();
    let fraction = n[0] - whole;
    Operand::number(if to_decimal {
        // 1.02 in thirty-seconds is 1 + 2/32.
        let digits = denominator.log10().ceil().max(1.0);
        whole + fraction * 10f64.powf(digits) / denominator
    } else {
        let digits = denominator.log10().ceil().max(1.0);
        whole + fraction * denominator / 10f64.powf(digits)
    })
}
