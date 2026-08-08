//! Dynamic arrays: functions whose result is a rectangle rather than a value.
//!
//! These are the 2019-and-later additions — `UNIQUE`, `SORT`, `FILTER`,
//! `SEQUENCE`, `XLOOKUP` — and what makes them different is not the arithmetic
//! but where the answer goes. A formula returning a 5x1 array *spills* into the
//! four cells below it, which no earlier Excel did, and which needs the
//! recalculation loop to know that this particular function is one of them.
//!
//! [`spills`] is that list. It is deliberately a list of *functions* rather
//! than a rule about array results: `=A1:A5` also produces an array and in a
//! legacy file it must still be an implicit intersection, not a spill. Getting
//! that wrong would change the meaning of every old formula in the corpus.
//!
//! Excel stores these names with an `_xlfn.` prefix so that older versions
//! leave them alone; the prefix is stripped before the lookup, in `mod.rs`.

use ss_model::CellError;

use crate::ast::Expr;
use crate::eval::Evaluator;
use crate::value::{compare, Array, Operand, Value};

use super::{arity, FnImpl};

pub(super) fn lookup(name: &str) -> Option<FnImpl> {
    Some(match name {
        "UNIQUE" => unique,
        "SORT" => sort,
        "SORTBY" => sortby,
        "FILTER" => filter,
        "SEQUENCE" => sequence,
        "RANDARRAY" => randarray,
        "XLOOKUP" => xlookup,
        "XMATCH" => xmatch,
        "TAKE" => |ev: &mut Evaluator, a: &[Expr]| take_drop(ev, a, true),
        "DROP" => |ev: &mut Evaluator, a: &[Expr]| take_drop(ev, a, false),
        "TOROW" => |ev: &mut Evaluator, a: &[Expr]| flatten(ev, a, true),
        "TOCOL" => |ev: &mut Evaluator, a: &[Expr]| flatten(ev, a, false),
        "TEXTSPLIT" => textsplit,
        "TEXTBEFORE" => |ev: &mut Evaluator, a: &[Expr]| text_part(ev, a, true),
        "TEXTAFTER" => |ev: &mut Evaluator, a: &[Expr]| text_part(ev, a, false),
        "HSTACK" => |ev: &mut Evaluator, a: &[Expr]| stack(ev, a, true),
        "VSTACK" => |ev: &mut Evaluator, a: &[Expr]| stack(ev, a, false),
        _ => return None,
    })
}

/// The functions whose result spills into the cells around the formula.
///
/// A list rather than "any array result", because `=A1:A5` is an array too and
/// in a file written before 2019 it means the implicit intersection. Spilling
/// it would silently change what every legacy formula does.
pub fn spills(name: &str) -> bool {
    let bare = name.strip_prefix("_xlfn.").unwrap_or(name);
    matches!(
        bare.to_ascii_uppercase().as_str(),
        "UNIQUE"
            | "SORT"
            | "SORTBY"
            | "FILTER"
            | "SEQUENCE"
            | "RANDARRAY"
            | "XLOOKUP"
            | "TAKE"
            | "DROP"
            | "TOROW"
            | "TOCOL"
            | "TEXTSPLIT"
            | "HSTACK"
            | "VSTACK"
    )
}

fn spread(ev: &mut Evaluator, arg: &Expr) -> Array {
    let out = ev.eval(arg);
    ev.spread(&out)
}

fn rows_of(array: &Array) -> Vec<Vec<Value>> {
    (0..array.rows())
        .map(|row| {
            (0..array.cols())
                .map(|col| array.get(row, col).cloned().unwrap_or_default())
                .collect()
        })
        .collect()
}

fn from_rows(rows: Vec<Vec<Value>>) -> Operand {
    if rows.is_empty() || rows[0].is_empty() {
        return Operand::error(CellError::Calc);
    }
    let (height, width) = (rows.len(), rows[0].len());
    let flat: Vec<Value> = rows.into_iter().flatten().collect();
    Operand::from_array(Array::new(height, width, flat))
}

/// True when two rows are the same, by the same rules `=` uses.
fn same_row(a: &[Value], b: &[Value]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| compare(x, y) == Ok(std::cmp::Ordering::Equal))
}

fn unique(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let array = spread(ev, &args[0]);
    let by_column = args
        .get(1)
        .map(|e| ev.eval_scalar(e).to_bool().unwrap_or(false))
        .unwrap_or(false);
    let once_only = args
        .get(2)
        .map(|e| ev.eval_scalar(e).to_bool().unwrap_or(false))
        .unwrap_or(false);

    let mut rows = rows_of(&array);
    if by_column {
        rows = transpose(rows);
    }
    let kept: Vec<Vec<Value>> = rows
        .iter()
        .filter(|row| {
            let count = rows.iter().filter(|other| same_row(row, other)).count();
            // `exactly_once` keeps the rows that appear once; the default keeps
            // the first of each group, which is a different question.
            if once_only {
                count == 1
            } else {
                true
            }
        })
        .cloned()
        .collect();
    let mut out: Vec<Vec<Value>> = Vec::new();
    for row in kept {
        if once_only || !out.iter().any(|seen| same_row(seen, &row)) {
            out.push(row);
        }
    }
    if by_column {
        out = transpose(out);
    }
    from_rows(out)
}

fn transpose(rows: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
    if rows.is_empty() {
        return rows;
    }
    let width = rows[0].len();
    (0..width)
        .map(|col| rows.iter().map(|row| row[col].clone()).collect())
        .collect()
}

fn sort(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(4)) {
        return Operand::error(CellError::Value);
    }
    let array = spread(ev, &args[0]);
    let index = args
        .get(1)
        .map(|e| ev.eval_scalar(e).to_number().unwrap_or(1.0))
        .unwrap_or(1.0) as usize;
    let descending = args
        .get(2)
        .map(|e| ev.eval_scalar(e).to_number().unwrap_or(1.0))
        .unwrap_or(1.0)
        < 0.0;
    let by_column = args
        .get(3)
        .map(|e| ev.eval_scalar(e).to_bool().unwrap_or(false))
        .unwrap_or(false);

    let mut rows = rows_of(&array);
    if by_column {
        rows = transpose(rows);
    }
    if index == 0 || index > rows.first().map_or(0, Vec::len) {
        return Operand::error(CellError::Value);
    }
    rows.sort_by(|a, b| {
        let ordering = compare(&a[index - 1], &b[index - 1]).unwrap_or(std::cmp::Ordering::Equal);
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    });
    if by_column {
        rows = transpose(rows);
    }
    from_rows(rows)
}

fn sortby(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, None) {
        return Operand::error(CellError::Value);
    }
    let array = spread(ev, &args[0]);
    let by = spread(ev, &args[1]);
    let descending = args
        .get(2)
        .map(|e| ev.eval_scalar(e).to_number().unwrap_or(1.0))
        .unwrap_or(1.0)
        < 0.0;

    let rows = rows_of(&array);
    let keys = rows_of(&by);
    if keys.len() != rows.len() {
        return Operand::error(CellError::Value);
    }
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|a, b| {
        let ordering = compare(
            keys[*a].first().unwrap_or(&Value::Blank),
            keys[*b].first().unwrap_or(&Value::Blank),
        )
        .unwrap_or(std::cmp::Ordering::Equal);
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    });
    from_rows(order.into_iter().map(|i| rows[i].clone()).collect())
}

fn filter(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let array = spread(ev, &args[0]);
    let include = spread(ev, &args[1]);
    let rows = rows_of(&array);
    let mask = rows_of(&include);

    // The mask may be a column (one per row) or a row (one per column).
    let by_row = mask.len() == rows.len() && mask.first().map_or(0, Vec::len) == 1;
    let kept: Vec<Vec<Value>> = if by_row {
        rows.iter()
            .enumerate()
            .filter(|(index, _)| truthy(&mask[*index][0]))
            .map(|(_, row)| row.clone())
            .collect()
    } else if mask.len() == 1 && mask[0].len() == rows.first().map_or(0, Vec::len) {
        rows.iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .filter(|(index, _)| truthy(&mask[0][*index]))
                    .map(|(_, v)| v.clone())
                    .collect::<Vec<_>>()
            })
            .filter(|row| !row.is_empty())
            .collect()
    } else {
        return Operand::error(CellError::Value);
    };

    if kept.is_empty() || kept[0].is_empty() {
        // Excel returns whatever `if_empty` says, and `#CALC!` when it is not
        // given — which is a real error code and not a placeholder.
        return match args.get(2) {
            Some(expr) => ev.eval(expr),
            None => Operand::error(CellError::Calc),
        };
    }
    from_rows(kept)
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => *n != 0.0,
        _ => false,
    }
}

fn sequence(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(4)) {
        return Operand::error(CellError::Value);
    }
    let read = |ev: &mut Evaluator, index: usize, default: f64| -> f64 {
        args.get(index)
            .map(|e| ev.eval_scalar(e).to_number().unwrap_or(default))
            .unwrap_or(default)
    };
    let rows = read(ev, 0, 1.0).trunc();
    let cols = read(ev, 1, 1.0).trunc();
    let start = read(ev, 2, 1.0);
    let step = read(ev, 3, 1.0);
    if rows < 1.0 || cols < 1.0 || rows * cols > 1_048_576.0 {
        return Operand::error(CellError::Value);
    }
    let (rows, cols) = (rows as usize, cols as usize);
    let cells = (0..rows * cols)
        .map(|index| Value::Number(start + step * index as f64))
        .collect();
    Operand::from_array(Array::new(rows, cols, cells))
}

/// `RANDARRAY([rows], [cols], [min], [max], [whole_number])`.
///
/// Volatile, like `RAND`: the dependency graph already knows to recalculate it
/// every pass, and this only has to produce the numbers.
fn randarray(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 0, Some(5)) {
        return Operand::error(CellError::Value);
    }
    let read = |ev: &mut Evaluator, index: usize, default: f64| -> f64 {
        args.get(index)
            .map(|e| ev.eval_scalar(e).to_number().unwrap_or(default))
            .unwrap_or(default)
    };
    let rows = read(ev, 0, 1.0).trunc();
    let cols = read(ev, 1, 1.0).trunc();
    let low = read(ev, 2, 0.0);
    let high = read(ev, 3, 1.0);
    let whole = args
        .get(4)
        .map(|e| ev.eval_scalar(e).to_bool().unwrap_or(false))
        .unwrap_or(false);
    if rows < 1.0 || cols < 1.0 || high < low || rows * cols > 1_048_576.0 {
        return Operand::error(CellError::Value);
    }
    let (rows, cols) = (rows as usize, cols as usize);
    let cells = (0..rows * cols)
        .map(|_| {
            let raw = low + ev.next_random() * (high - low);
            Value::Number(if whole { raw.floor() } else { raw })
        })
        .collect();
    Operand::from_array(Array::new(rows, cols, cells))
}

/// `XLOOKUP(lookup, lookup_array, return_array, [if_not_found], [match_mode],
/// [search_mode])`.
///
/// The one that replaced `VLOOKUP`, and the differences are the point: the
/// return array is separate from the lookup array (so it may be to the *left*),
/// the default is an exact match rather than approximate, and a miss has an
/// answer rather than being `#N/A`.
fn xlookup(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, Some(6)) {
        return Operand::error(CellError::Value);
    }
    let needle = ev.eval_scalar(&args[0]);
    if let Value::Error(e) = needle {
        return Operand::error(e);
    }
    let haystack = spread(ev, &args[1]);
    let results = spread(ev, &args[2]);
    let mode = args
        .get(4)
        .map(|e| ev.eval_scalar(e).to_number().unwrap_or(0.0))
        .unwrap_or(0.0);
    let backwards = args
        .get(5)
        .map(|e| ev.eval_scalar(e).to_number().unwrap_or(1.0))
        .unwrap_or(1.0)
        < 0.0;

    let keys: Vec<Value> = haystack.values().cloned().collect();
    let found = search(&needle, &keys, mode, backwards);
    let Some(index) = found else {
        return match args.get(3) {
            Some(expr) => ev.eval(expr),
            None => Operand::error(CellError::NotAvailable),
        };
    };

    // The result may be a whole row or column of the return array, which is
    // what makes `XLOOKUP` able to return a record rather than a field.
    if results.rows() == keys.len() && results.cols() > 1 {
        let row: Vec<Value> = (0..results.cols())
            .map(|col| results.get(index, col).cloned().unwrap_or_default())
            .collect();
        return Operand::from_array(Array::row_vector(row));
    }
    if results.cols() == keys.len() && results.rows() > 1 {
        let column: Vec<Value> = (0..results.rows())
            .map(|row| results.get(row, index).cloned().unwrap_or_default())
            .collect();
        return Operand::from_array(Array::new(column.len(), 1, column));
    }
    let single = results.values().nth(index).cloned();
    match single {
        Some(value) => Operand::Value(value),
        None => Operand::error(CellError::Value),
    }
}

fn xmatch(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(4)) {
        return Operand::error(CellError::Value);
    }
    let needle = ev.eval_scalar(&args[0]);
    if let Value::Error(e) = needle {
        return Operand::error(e);
    }
    let haystack = spread(ev, &args[1]);
    let mode = args
        .get(2)
        .map(|e| ev.eval_scalar(e).to_number().unwrap_or(0.0))
        .unwrap_or(0.0);
    let backwards = args
        .get(3)
        .map(|e| ev.eval_scalar(e).to_number().unwrap_or(1.0))
        .unwrap_or(1.0)
        < 0.0;
    let keys: Vec<Value> = haystack.values().cloned().collect();
    match search(&needle, &keys, mode, backwards) {
        Some(index) => Operand::number(index as f64 + 1.0),
        None => Operand::error(CellError::NotAvailable),
    }
}

/// The match modes: 0 exact, −1 exact or next smaller, 1 exact or next larger,
/// 2 wildcard.
fn search(needle: &Value, keys: &[Value], mode: f64, backwards: bool) -> Option<usize> {
    let indices: Vec<usize> = if backwards {
        (0..keys.len()).rev().collect()
    } else {
        (0..keys.len()).collect()
    };

    if mode == 2.0 {
        let pattern = needle.to_text().ok()?;
        let criterion = super::wildcard_match(&pattern, "");
        let _ = criterion;
        return indices.into_iter().find(|index| {
            keys[*index]
                .to_text()
                .ok()
                .is_some_and(|text| super::wildcard_match(&pattern, &text))
        });
    }

    if let Some(exact) = indices
        .iter()
        .copied()
        .find(|index| compare(needle, &keys[*index]) == Ok(std::cmp::Ordering::Equal))
    {
        return Some(exact);
    }
    if mode == 0.0 {
        return None;
    }

    // Nearest smaller or nearest larger — and *nearest*, not "the last one
    // passed", because an XLOOKUP array need not be sorted.
    let mut best: Option<(usize, Value)> = None;
    for index in indices {
        let candidate = &keys[index];
        let Ok(ordering) = compare(candidate, needle) else {
            continue;
        };
        let usable = if mode < 0.0 {
            ordering == std::cmp::Ordering::Less
        } else {
            ordering == std::cmp::Ordering::Greater
        };
        if !usable {
            continue;
        }
        let better = match &best {
            None => true,
            Some((_, current)) => {
                let closer = compare(candidate, current).unwrap_or(std::cmp::Ordering::Equal);
                if mode < 0.0 {
                    closer == std::cmp::Ordering::Greater
                } else {
                    closer == std::cmp::Ordering::Less
                }
            }
        };
        if better {
            best = Some((index, candidate.clone()));
        }
    }
    best.map(|(index, _)| index)
}

fn take_drop(ev: &mut Evaluator, args: &[Expr], take: bool) -> Operand {
    if !arity(args, 2, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let array = spread(ev, &args[0]);
    let rows_arg = ev.eval_scalar(&args[1]).to_number().unwrap_or(0.0) as i64;
    let cols_arg = args
        .get(2)
        .map(|e| ev.eval_scalar(e).to_number().unwrap_or(0.0) as i64)
        .unwrap_or(0);

    let mut rows = rows_of(&array);
    // A negative count works from the far end, which is the whole reason these
    // exist as a pair rather than as one function with a flag.
    rows = slice(rows, rows_arg, take);
    rows = transpose(slice(transpose(rows), cols_arg, take));
    from_rows(rows)
}

fn slice(rows: Vec<Vec<Value>>, count: i64, take: bool) -> Vec<Vec<Value>> {
    if count == 0 {
        return rows;
    }
    let length = rows.len() as i64;
    let magnitude = count.abs().min(length) as usize;
    match (take, count > 0) {
        (true, true) => rows.into_iter().take(magnitude).collect(),
        (true, false) => rows.into_iter().skip(length as usize - magnitude).collect(),
        (false, true) => rows.into_iter().skip(magnitude).collect(),
        (false, false) => rows.into_iter().take(length as usize - magnitude).collect(),
    }
}

fn flatten(ev: &mut Evaluator, args: &[Expr], to_row: bool) -> Operand {
    if !arity(args, 1, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let array = spread(ev, &args[0]);
    let values: Vec<Value> = array.values().cloned().collect();
    if values.is_empty() {
        return Operand::error(CellError::Calc);
    }
    Operand::from_array(if to_row {
        Array::row_vector(values)
    } else {
        Array::new(values.len(), 1, values)
    })
}

fn textsplit(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(6)) {
        return Operand::error(CellError::Value);
    }
    let text = match ev.eval_scalar(&args[0]).to_text() {
        Ok(t) => t,
        Err(e) => return Operand::error(e),
    };
    let column_by = match ev.eval_scalar(&args[1]).to_text() {
        Ok(t) => t,
        Err(e) => return Operand::error(e),
    };
    let row_by = args
        .get(2)
        .map(|e| ev.eval_scalar(e).to_text().unwrap_or_default())
        .filter(|t| !t.is_empty());

    let lines: Vec<&str> = match &row_by {
        Some(separator) => text.split(separator.as_str()).collect(),
        None => vec![text.as_str()],
    };
    let rows: Vec<Vec<Value>> = lines
        .into_iter()
        .map(|line| {
            if column_by.is_empty() {
                vec![Value::text(line)]
            } else {
                line.split(column_by.as_str()).map(Value::text).collect()
            }
        })
        .collect();
    // A ragged split is padded, because an array has to be rectangular.
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let padded: Vec<Vec<Value>> = rows
        .into_iter()
        .map(|mut row| {
            row.resize(width, Value::Blank);
            row
        })
        .collect();
    from_rows(padded)
}

fn text_part(ev: &mut Evaluator, args: &[Expr], before: bool) -> Operand {
    if !arity(args, 2, Some(6)) {
        return Operand::error(CellError::Value);
    }
    let text = match ev.eval_scalar(&args[0]).to_text() {
        Ok(t) => t,
        Err(e) => return Operand::error(e),
    };
    let separator = match ev.eval_scalar(&args[1]).to_text() {
        Ok(t) => t,
        Err(e) => return Operand::error(e),
    };
    let occurrence = args
        .get(2)
        .map(|e| ev.eval_scalar(e).to_number().unwrap_or(1.0))
        .unwrap_or(1.0) as i64;
    if separator.is_empty() {
        return Operand::text(if before { String::new() } else { text });
    }

    let positions: Vec<usize> = text.match_indices(&separator).map(|(i, _)| i).collect();
    let index = if occurrence >= 0 {
        positions.get((occurrence.max(1) - 1) as usize).copied()
    } else {
        // A negative occurrence counts from the end, which is how you take the
        // file extension off a path with more than one dot in it.
        let from_end = positions.len() as i64 + occurrence;
        (from_end >= 0).then(|| positions[from_end as usize])
    };
    let Some(at) = index else {
        return Operand::error(CellError::NotAvailable);
    };
    Operand::text(if before {
        text[..at].to_string()
    } else {
        text[at + separator.len()..].to_string()
    })
}

fn stack(ev: &mut Evaluator, args: &[Expr], horizontal: bool) -> Operand {
    if !arity(args, 1, None) {
        return Operand::error(CellError::Value);
    }
    let mut blocks: Vec<Vec<Vec<Value>>> = Vec::new();
    for arg in args {
        blocks.push(rows_of(&spread(ev, arg)));
    }
    if horizontal {
        let height = blocks.iter().map(Vec::len).max().unwrap_or(0);
        let mut out = vec![Vec::new(); height];
        for block in &blocks {
            let width = block.first().map_or(0, Vec::len);
            for (index, row) in out.iter_mut().enumerate() {
                match block.get(index) {
                    Some(source) => row.extend(source.iter().cloned()),
                    // A shorter block is padded with `#N/A`, which is what
                    // Excel does and is visibly different from a blank.
                    None => row.extend(std::iter::repeat_n(
                        Value::Error(CellError::NotAvailable),
                        width,
                    )),
                }
            }
        }
        return from_rows(out);
    }

    let width = blocks
        .iter()
        .map(|b| b.first().map_or(0, Vec::len))
        .max()
        .unwrap_or(0);
    let mut out = Vec::new();
    for block in blocks {
        for mut row in block {
            row.resize(width, Value::Error(CellError::NotAvailable));
            out.push(row);
        }
    }
    from_rows(out)
}
