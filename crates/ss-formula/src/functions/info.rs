//! Information functions — the `IS*` family and friends.
//!
//! These are the only functions that must *not* propagate errors: `ISERROR`
//! would be useless if an error argument made it return an error. Each one
//! inspects its argument rather than consuming it, so none of them can go
//! through the ordinary coercion helpers.

use ss_model::CellError;

use crate::ast::Expr;
use crate::eval::Evaluator;
use crate::value::{Operand, Value};

use super::{arity, FnImpl};

pub(super) fn lookup(name: &str) -> Option<FnImpl> {
    Some(match name {
        "ISBLANK" => |ev: &mut Evaluator, a: &[Expr]| test(ev, a, |v| matches!(v, Value::Blank)),
        "ISNUMBER" => {
            |ev: &mut Evaluator, a: &[Expr]| test(ev, a, |v| matches!(v, Value::Number(_)))
        }
        "ISTEXT" => |ev: &mut Evaluator, a: &[Expr]| test(ev, a, |v| matches!(v, Value::Text(_))),
        "ISNONTEXT" => {
            |ev: &mut Evaluator, a: &[Expr]| test(ev, a, |v| !matches!(v, Value::Text(_)))
        }
        "ISLOGICAL" => {
            |ev: &mut Evaluator, a: &[Expr]| test(ev, a, |v| matches!(v, Value::Bool(_)))
        }
        "ISERROR" => |ev: &mut Evaluator, a: &[Expr]| test(ev, a, Value::is_error),
        "ISNA" => |ev: &mut Evaluator, a: &[Expr]| {
            test(ev, a, |v| v.as_error() == Some(CellError::NotAvailable))
        },
        // `ISERR` is `ISERROR` minus #N/A — the distinction between "this went
        // wrong" and "this is not applicable".
        "ISERR" => |ev: &mut Evaluator, a: &[Expr]| {
            test(ev, a, |v| {
                v.is_error() && v.as_error() != Some(CellError::NotAvailable)
            })
        },
        "ISREF" => is_ref,
        "ISEVEN" => |ev: &mut Evaluator, a: &[Expr]| parity(ev, a, 0.0),
        "ISODD" => |ev: &mut Evaluator, a: &[Expr]| parity(ev, a, 1.0),
        "NA" => |_ev: &mut Evaluator, a: &[Expr]| {
            if a.is_empty() {
                Operand::error(CellError::NotAvailable)
            } else {
                Operand::error(CellError::Value)
            }
        },
        "ERROR.TYPE" => error_type,
        "TYPE" => type_of,
        "N" => n,
        _ => return None,
    })
}

/// Runs a predicate over the argument without coercing it.
fn test(ev: &mut Evaluator, args: &[Expr], f: impl Fn(&Value) -> bool) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    Operand::boolean(f(&ev.eval_scalar(&args[0])))
}

/// True when the argument is a reference at all — the one `IS*` that looks at
/// the operand's shape rather than its value.
fn is_ref(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    Operand::boolean(matches!(ev.eval(&args[0]), Operand::Ref(_)))
}

fn parity(ev: &mut Evaluator, args: &[Expr], want: f64) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    match ev.eval_number(&args[0]) {
        Ok(n) => Operand::boolean((n.trunc().abs() % 2.0) == want),
        Err(e) => Operand::error(e),
    }
}

/// `ERROR.TYPE` maps an error to its position in Excel's fixed list, and
/// reports `#N/A` for anything that is not an error at all.
fn error_type(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    let code = match ev.eval_scalar(&args[0]).as_error() {
        Some(CellError::Null) => 1,
        Some(CellError::Div0) => 2,
        Some(CellError::Value) => 3,
        Some(CellError::Ref) => 4,
        Some(CellError::Name) => 5,
        Some(CellError::Num) => 6,
        Some(CellError::NotAvailable) => 7,
        Some(CellError::GettingData) => 8,
        Some(CellError::Spill) => 9,
        Some(CellError::Calc) => 14,
        // Ours, not Excel's; report it as the closest thing Excel has.
        Some(CellError::Circular) => 4,
        None => return Operand::error(CellError::NotAvailable),
    };
    Operand::number(f64::from(code))
}

/// `TYPE` — 1 number, 2 text, 4 logical, 16 error, 64 array. Blank counts as a
/// number, which is consistent with blank coercing to zero everywhere else.
fn type_of(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    let op = ev.eval(&args[0]);
    if let Operand::Array(_) = op {
        return Operand::number(64.0);
    }
    let code = match ev.collapse(op) {
        Value::Number(_) | Value::Blank => 1,
        Value::Text(_) => 2,
        Value::Bool(_) => 4,
        Value::Error(_) => 16,
    };
    Operand::number(f64::from(code))
}

/// `N` converts to a number without erroring on text: text is 0, not `#VALUE!`.
fn n(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    Operand::Value(match ev.eval_scalar(&args[0]) {
        Value::Number(x) => Value::Number(x),
        Value::Bool(b) => Value::Number(f64::from(b)),
        Value::Error(e) => Value::Error(e),
        _ => Value::Number(0.0),
    })
}
