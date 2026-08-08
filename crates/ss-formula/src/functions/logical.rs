//! Logical functions.
//!
//! The branching ones are the reason arguments arrive unevaluated. `IF(A1=0, 0,
//! 1/A1)` must not divide when A1 is zero, and `IFERROR(1/A1, "")` must not
//! evaluate the fallback when there is nothing wrong.

use ss_model::CellError;

use crate::ast::Expr;
use crate::eval::Evaluator;
use crate::value::{Operand, Value};

use super::{arity, visit_args, FnImpl, Source};

pub(super) fn lookup(name: &str) -> Option<FnImpl> {
    Some(match name {
        "IF" => if_,
        "IFS" => ifs,
        "IFERROR" => |ev: &mut Evaluator, a: &[Expr]| on_error(ev, a, None),
        "IFNA" => |ev: &mut Evaluator, a: &[Expr]| on_error(ev, a, Some(CellError::NotAvailable)),
        "AND" => |ev: &mut Evaluator, a: &[Expr]| combine(ev, a, true),
        "OR" => |ev: &mut Evaluator, a: &[Expr]| combine(ev, a, false),
        "XOR" => xor,
        "NOT" => not,
        "SWITCH" => switch,
        "TRUE" => |_ev: &mut Evaluator, a: &[Expr]| constant(a, true),
        "FALSE" => |_ev: &mut Evaluator, a: &[Expr]| constant(a, false),
        _ => return None,
    })
}

fn constant(args: &[Expr], v: bool) -> Operand {
    if args.is_empty() {
        Operand::boolean(v)
    } else {
        Operand::error(CellError::Value)
    }
}

/// `IF(test, then, [else])`.
///
/// An omitted `else` yields `FALSE`, but an *empty* one yields 0. They are
/// different arguments — `IF(1=2,1)` and `IF(1=2,1,)` do not agree — which is
/// why `Expr::Missing` exists as a distinct node.
fn if_(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let test = match ev.eval_bool(&args[0]) {
        Ok(b) => b,
        Err(e) => return Operand::error(e),
    };
    let branch = if test { args.get(1) } else { args.get(2) };
    match branch {
        None => Operand::boolean(false),
        Some(Expr::Missing) => Operand::number(0.0),
        Some(e) => ev.eval(e),
    }
}

/// `IFS(test1, value1, test2, value2, ...)` — the first true test wins, and
/// none of them being true is `#N/A` rather than blank.
fn ifs(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if args.len() < 2 || !args.len().is_multiple_of(2) {
        return Operand::error(CellError::Value);
    }
    for pair in args.chunks(2) {
        match ev.eval_bool(&pair[0]) {
            Ok(true) => return ev.eval(&pair[1]),
            Ok(false) => {}
            Err(e) => return Operand::error(e),
        }
    }
    Operand::error(CellError::NotAvailable)
}

/// `IFERROR` and `IFNA`, which differ only in which errors they catch.
fn on_error(ev: &mut Evaluator, args: &[Expr], only: Option<CellError>) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let value = ev.eval(&args[0]);
    let caught = match &value {
        Operand::Value(Value::Error(e)) => only.is_none_or(|want| want == *e),
        // An array whose *first* cell is an error is not an error overall; only
        // a scalar error triggers the fallback.
        _ => false,
    };
    if caught {
        ev.eval(&args[1])
    } else {
        value
    }
}

/// `AND` and `OR`.
///
/// Both ignore text and blanks found *inside* a range — that is what lets them
/// run over a column with a header — but reject text supplied directly unless it
/// spells a boolean. If nothing usable is found at all, the answer is `#VALUE!`,
/// not `TRUE`.
fn combine(ev: &mut Evaluator, args: &[Expr], all: bool) -> Operand {
    if args.is_empty() {
        return Operand::error(CellError::Value);
    }
    let mut seen = false;
    let mut acc = all;
    let mut err = None;

    visit_args(ev, args, &mut |v, source| {
        if err.is_some() {
            return;
        }
        match (v, source) {
            (Value::Error(e), _) => err = Some(*e),
            (Value::Blank | Value::Text(_), Source::Inside) => {}
            _ => match v.to_bool() {
                Ok(b) => {
                    seen = true;
                    acc = if all { acc && b } else { acc || b };
                }
                Err(e) => err = Some(e),
            },
        }
    });

    match (err, seen) {
        (Some(e), _) => Operand::error(e),
        (None, false) => Operand::error(CellError::Value),
        (None, true) => Operand::boolean(acc),
    }
}

/// `XOR` is true when an odd number of its arguments are.
fn xor(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if args.is_empty() {
        return Operand::error(CellError::Value);
    }
    let mut count = 0usize;
    let mut seen = false;
    let mut err = None;

    visit_args(ev, args, &mut |v, source| {
        if err.is_some() {
            return;
        }
        match (v, source) {
            (Value::Error(e), _) => err = Some(*e),
            (Value::Blank | Value::Text(_), Source::Inside) => {}
            _ => match v.to_bool() {
                Ok(b) => {
                    seen = true;
                    count += usize::from(b);
                }
                Err(e) => err = Some(e),
            },
        }
    });

    match (err, seen) {
        (Some(e), _) => Operand::error(e),
        (None, false) => Operand::error(CellError::Value),
        (None, true) => Operand::boolean(count % 2 == 1),
    }
}

fn not(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    match ev.eval_bool(&args[0]) {
        Ok(b) => Operand::boolean(!b),
        Err(e) => Operand::error(e),
    }
}

/// `SWITCH(expression, case1, value1, ..., [default])`.
///
/// The trailing default is optional, so an odd number of arguments after the
/// expression means the last one is it.
fn switch(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if args.len() < 3 {
        return Operand::error(CellError::Value);
    }
    let subject = ev.eval_scalar(&args[0]);
    if let Value::Error(e) = subject {
        return Operand::error(e);
    }

    let rest = &args[1..];
    let pairs = rest.len() / 2;
    for i in 0..pairs {
        let case = ev.eval_scalar(&rest[i * 2]);
        if let Value::Error(e) = case {
            return Operand::error(e);
        }
        if crate::value::compare(&subject, &case) == Ok(std::cmp::Ordering::Equal) {
            return ev.eval(&rest[i * 2 + 1]);
        }
    }
    match rest.len() % 2 {
        1 => ev.eval(&rest[rest.len() - 1]),
        _ => Operand::error(CellError::NotAvailable),
    }
}
