//! The database family: `DSUM`, `DGET`, and the rest.
//!
//! All twelve are the same function with a different aggregate, and all of the
//! difficulty is in the *criteria range* rather than in the arithmetic.
//!
//! A criteria range is a small table: a header row naming columns, and rows of
//! conditions under it. **Conditions across a row are ANDed and rows are ORed**
//! — that is the whole language, and it is what makes a criteria range able to
//! express "Region is North and Sales over 100, or Region is South at all"
//! without a formula. Reading it as one flat list of conditions gives an answer
//! that is often right by coincidence and wrong when it matters.
//!
//! A criteria column may also be named twice, which is how a *range* is
//! expressed: two `Sales` columns, one `>=100` and one `<=200`.

use ss_model::CellError;

use crate::ast::Expr;
use crate::eval::Evaluator;
use crate::value::{Array, Operand, Value};

use super::criteria::{matches_criteria, Criterion};
use super::{arity, FnImpl};

pub(super) fn lookup(name: &str) -> Option<FnImpl> {
    Some(match name {
        "DSUM" => |ev: &mut Evaluator, a: &[Expr]| database(ev, a, Aggregate::Sum),
        "DCOUNT" => |ev: &mut Evaluator, a: &[Expr]| database(ev, a, Aggregate::Count),
        "DCOUNTA" => |ev: &mut Evaluator, a: &[Expr]| database(ev, a, Aggregate::CountA),
        "DAVERAGE" => |ev: &mut Evaluator, a: &[Expr]| database(ev, a, Aggregate::Average),
        "DMAX" => |ev: &mut Evaluator, a: &[Expr]| database(ev, a, Aggregate::Max),
        "DMIN" => |ev: &mut Evaluator, a: &[Expr]| database(ev, a, Aggregate::Min),
        "DPRODUCT" => |ev: &mut Evaluator, a: &[Expr]| database(ev, a, Aggregate::Product),
        "DGET" => |ev: &mut Evaluator, a: &[Expr]| database(ev, a, Aggregate::Get),
        "DSTDEV" => |ev: &mut Evaluator, a: &[Expr]| database(ev, a, Aggregate::StDev),
        "DSTDEVP" => |ev: &mut Evaluator, a: &[Expr]| database(ev, a, Aggregate::StDevP),
        "DVAR" => |ev: &mut Evaluator, a: &[Expr]| database(ev, a, Aggregate::Var),
        "DVARP" => |ev: &mut Evaluator, a: &[Expr]| database(ev, a, Aggregate::VarP),
        _ => return None,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Aggregate {
    Sum,
    Count,
    CountA,
    Average,
    Max,
    Min,
    Product,
    Get,
    StDev,
    StDevP,
    Var,
    VarP,
}

fn database(ev: &mut Evaluator, args: &[Expr], aggregate: Aggregate) -> Operand {
    if !arity(args, 3, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let table = {
        let out = ev.eval(&args[0]);
        ev.spread(&out)
    };
    let field = ev.eval_scalar(&args[1]);
    let criteria = {
        let out = ev.eval(&args[2]);
        ev.spread(&out)
    };
    if let Value::Error(e) = field {
        return Operand::error(e);
    }
    if table.rows() < 2 || criteria.rows() < 1 {
        return Operand::error(CellError::Value);
    }

    let Some(column) = field_index(&table, &field) else {
        return Operand::error(CellError::Value);
    };

    let mut selected: Vec<Value> = Vec::new();
    for row in 1..table.rows() {
        if !row_matches(&table, row, &criteria) {
            continue;
        }
        if let Some(value) = table.get(row, column) {
            selected.push(value.clone());
        }
    }

    finish(aggregate, &selected)
}

/// Which column the `field` argument names.
///
/// Excel accepts either the header's text or a one-based column number, and
/// files use both — a column number survives renaming the header, and a name
/// survives inserting a column.
fn field_index(table: &Array, field: &Value) -> Option<usize> {
    match field {
        Value::Number(n) => {
            let index = *n as i64 - 1;
            (index >= 0 && (index as usize) < table.cols()).then_some(index as usize)
        }
        Value::Text(name) => (0..table.cols()).find(|column| {
            table
                .get(0, *column)
                .and_then(|v| v.to_text().ok())
                .is_some_and(|header| header.trim().eq_ignore_ascii_case(name.trim()))
        }),
        _ => None,
    }
}

/// True when a data row satisfies any one criteria row completely.
fn row_matches(table: &Array, row: usize, criteria: &Array) -> bool {
    for rule_row in 1..criteria.rows() {
        let mut all = true;
        let mut any_condition = false;
        for rule_col in 0..criteria.cols() {
            let Some(condition) = criteria.get(rule_row, rule_col) else {
                continue;
            };
            if matches!(condition, Value::Blank) {
                continue;
            }
            let Some(header) = criteria.get(0, rule_col) else {
                continue;
            };
            let Some(column) = field_index(table, header) else {
                // A criteria column naming something the table has not got
                // matches nothing, rather than matching everything.
                all = false;
                break;
            };
            any_condition = true;
            let value = table.get(row, column).cloned().unwrap_or_default();
            if !matches_criteria(&Criterion::parse(condition), &value) {
                all = false;
                break;
            }
        }
        // A criteria row with nothing in it matches everything, which is how a
        // blank row under the header means "no filter".
        if all && (any_condition || criteria.cols() > 0) {
            return true;
        }
    }
    // No criteria rows at all: nothing is selected, which is Excel's answer and
    // not "everything".
    false
}

fn finish(aggregate: Aggregate, selected: &[Value]) -> Operand {
    if let Some(Value::Error(e)) = selected.iter().find(|v| matches!(v, Value::Error(_))) {
        return Operand::error(*e);
    }
    let numbers: Vec<f64> = selected
        .iter()
        .filter_map(|v| match v {
            Value::Number(n) => Some(*n),
            _ => None,
        })
        .collect();

    match aggregate {
        Aggregate::Count => Operand::number(numbers.len() as f64),
        Aggregate::CountA => Operand::number(
            selected
                .iter()
                .filter(|v| !matches!(v, Value::Blank))
                .count() as f64,
        ),
        Aggregate::Get => match selected
            .iter()
            .filter(|v| !matches!(v, Value::Blank))
            .collect::<Vec<_>>()
            .as_slice()
        {
            // `DGET` is the one that insists on exactly one match, and says so
            // two different ways: nothing found is #VALUE!, more than one is
            // #NUM!.
            [] => Operand::error(CellError::Value),
            [only] => Operand::Value((*only).clone()),
            _ => Operand::error(CellError::Num),
        },
        Aggregate::Sum => Operand::number(numbers.iter().sum()),
        Aggregate::Product => Operand::number(numbers.iter().product()),
        Aggregate::Average => {
            if numbers.is_empty() {
                Operand::error(CellError::Div0)
            } else {
                Operand::number(numbers.iter().sum::<f64>() / numbers.len() as f64)
            }
        }
        Aggregate::Max => match numbers.iter().copied().reduce(f64::max) {
            Some(n) => Operand::number(n),
            None => Operand::number(0.0),
        },
        Aggregate::Min => match numbers.iter().copied().reduce(f64::min) {
            Some(n) => Operand::number(n),
            None => Operand::number(0.0),
        },
        Aggregate::StDev | Aggregate::Var | Aggregate::StDevP | Aggregate::VarP => {
            let sample = matches!(aggregate, Aggregate::StDev | Aggregate::Var);
            let divisor = numbers.len() as f64 - if sample { 1.0 } else { 0.0 };
            if numbers.is_empty() || divisor <= 0.0 {
                return Operand::error(CellError::Div0);
            }
            let mean = numbers.iter().sum::<f64>() / numbers.len() as f64;
            let variance = numbers.iter().map(|n| (n - mean).powi(2)).sum::<f64>() / divisor;
            Operand::number(
                if matches!(aggregate, Aggregate::StDev | Aggregate::StDevP) {
                    variance.sqrt()
                } else {
                    variance
                },
            )
        }
    }
}
