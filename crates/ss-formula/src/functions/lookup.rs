//! Lookup and reference functions.
//!
//! Two things shape this module.
//!
//! **Positions are relative to what the user wrote.** `MATCH(x, A:A, 0)` must
//! answer with a row number counted from row 1, even though nothing below row
//! 900 is worth reading. Clipping a whole-column reference to the used range and
//! then counting from there gives an answer that is plausible, wrong, and
//! silent — the same failure C6 hit in `SUMIF`. [`Grid`] exists to keep the two
//! apart: it always addresses cells from the range's declared corner, and only
//! *iterates* the part that can hold anything.
//!
//! **Some of these return references, not values.** `SUM(OFFSET(A1,0,0,3,1))`
//! only works because `OFFSET` hands back an address. Collapsing it to a value
//! at the point of return would make the function useless for its main purpose.

use ss_model::cell::{MAX_COLS, MAX_ROWS};
use ss_model::{CellError, CellRange, CellRef};

use crate::ast::Expr;
use crate::eval::Evaluator;
use crate::graph::AreaRef;
use crate::value::{compare, Array, Operand, RefSet, Value};

use super::{arity, wildcard_match, FnImpl};

pub(super) fn lookup(name: &str) -> Option<FnImpl> {
    Some(match name {
        "ROW" => |ev: &mut Evaluator, a: &[Expr]| position(ev, a, true),
        "COLUMN" => |ev: &mut Evaluator, a: &[Expr]| position(ev, a, false),
        "ROWS" => |ev: &mut Evaluator, a: &[Expr]| extent(ev, a, true),
        "COLUMNS" => |ev: &mut Evaluator, a: &[Expr]| extent(ev, a, false),
        "AREAS" => areas,
        "CHOOSE" => choose,
        "INDEX" => index,
        "MATCH" => match_fn,
        "VLOOKUP" => |ev: &mut Evaluator, a: &[Expr]| table_lookup(ev, a, true),
        "HLOOKUP" => |ev: &mut Evaluator, a: &[Expr]| table_lookup(ev, a, false),
        "LOOKUP" => lookup_fn,
        "XLOOKUP" => xlookup,
        "XMATCH" => xmatch,
        "TRANSPOSE" => transpose,
        "OFFSET" => offset,
        "INDIRECT" => indirect,
        "ADDRESS" => address,
        _ => return None,
    })
}

/// A rectangle a lookup searches over.
///
/// The distinction between the full extent and the window is the whole point:
/// `rows`/`cols` describe the range the user wrote, so positions come out right,
/// while `row_window`/`col_window` describe what is worth visiting, so
/// `MATCH(x, A:A, 0)` does not read a million cells.
enum Grid {
    Cells {
        sheet: usize,
        full: CellRange,
        window: CellRange,
    },
    Values(Array),
}

impl Grid {
    /// Reads an argument as a searchable rectangle.
    fn of(ev: &mut Evaluator, expr: &Expr) -> Self {
        let op = ev.eval(expr);
        Self::from_operand(ev, &op)
    }

    fn from_operand(ev: &Evaluator, op: &Operand) -> Self {
        match op {
            Operand::Ref(r) if r.areas.len() == 1 => {
                let area = r.areas[0];
                Grid::Cells {
                    sheet: area.sheet,
                    full: area.range,
                    window: ev.clip_area(&area),
                }
            }
            other => Grid::Values(ev.spread(other)),
        }
    }

    fn rows(&self) -> u32 {
        match self {
            Grid::Cells { full, .. } => full.rows(),
            Grid::Values(a) => a.rows() as u32,
        }
    }

    fn cols(&self) -> u32 {
        match self {
            Grid::Cells { full, .. } => full.cols(),
            Grid::Values(a) => a.cols() as u32,
        }
    }

    /// The value at a position counted from the rectangle's own corner.
    fn get(&self, ev: &Evaluator, row: u32, col: u32) -> Value {
        if row >= self.rows() || col >= self.cols() {
            return Value::Error(CellError::Ref);
        }
        match self {
            Grid::Cells { sheet, full, .. } => ev.context().cell(
                *sheet,
                CellRef::new(full.start.row + row, full.start.col + col),
            ),
            Grid::Values(a) => a
                .get(row as usize, col as usize)
                .cloned()
                .unwrap_or(Value::Blank),
        }
    }

    /// The rows worth visiting, as offsets from the corner. Everything outside
    /// is certainly blank.
    fn row_window(&self) -> (u32, u32) {
        match self {
            Grid::Cells { full, window, .. } => (
                window.start.row - full.start.row,
                window.end.row - full.start.row,
            ),
            Grid::Values(a) => (0, a.rows().saturating_sub(1) as u32),
        }
    }

    fn col_window(&self) -> (u32, u32) {
        match self {
            Grid::Cells { full, window, .. } => (
                window.start.col - full.start.col,
                window.end.col - full.start.col,
            ),
            Grid::Values(a) => (0, a.cols().saturating_sub(1) as u32),
        }
    }

    /// The positions along a vector, in order. `down` picks the axis.
    fn line(&self, down: bool) -> std::ops::RangeInclusive<u32> {
        let (lo, hi) = if down {
            self.row_window()
        } else {
            self.col_window()
        };
        lo..=hi
    }
}

/// `ROW()` and `COLUMN()` — the formula's own position, or the positions a
/// reference covers.
fn position(ev: &mut Evaluator, args: &[Expr], row: bool) -> Operand {
    if !arity(args, 0, Some(1)) {
        return Operand::error(CellError::Value);
    }
    let Some(arg) = args.first() else {
        let at = ev.position().at;
        return Operand::number(f64::from(if row { at.row } else { at.col }) + 1.0);
    };
    let op = ev.eval(arg);
    let Operand::Ref(r) = &op else {
        return Operand::error(CellError::Value);
    };
    let Some(area) = r.areas.first() else {
        return Operand::error(CellError::Ref);
    };
    // A range answers with one number per row (or per column), which is why
    // `SUMPRODUCT(ROW(A1:A5))` sums 1 through 5.
    let range = area.range;
    let cells: Vec<Value> = if row {
        (range.start.row..=range.end.row)
            .map(|r| Value::Number(f64::from(r) + 1.0))
            .collect()
    } else {
        (range.start.col..=range.end.col)
            .map(|c| Value::Number(f64::from(c) + 1.0))
            .collect()
    };
    let array = if row {
        Array::new(cells.len(), 1, cells)
    } else {
        Array::row_vector(cells)
    };
    Operand::from_array(array)
}

fn extent(ev: &mut Evaluator, args: &[Expr], row: bool) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    let grid = Grid::of(ev, &args[0]);
    Operand::number(f64::from(if row { grid.rows() } else { grid.cols() }))
}

fn areas(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    match ev.eval(&args[0]) {
        Operand::Ref(r) => Operand::number(r.areas.len() as f64),
        _ => Operand::error(CellError::Value),
    }
}

/// `CHOOSE(index, ...)` — only the chosen argument is evaluated, so
/// `CHOOSE(1,0,1/0)` is 0 rather than an error.
fn choose(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, None) {
        return Operand::error(CellError::Value);
    }
    let index = match ev.eval_number(&args[0]) {
        Ok(n) => n.trunc(),
        Err(e) => return Operand::error(e),
    };
    if index < 1.0 || index as usize >= args.len() {
        return Operand::error(CellError::Value);
    }
    ev.eval(&args[index as usize])
}

/// `INDEX(array, row, [col], [area])`.
///
/// Returns a *reference* when given one, which is what makes `INDEX(A:A,1):INDEX(A:A,5)`
/// a range rather than a type error. A zero index means the whole row or column.
fn index(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(4)) {
        return Operand::error(CellError::Value);
    }
    let source = ev.eval(&args[0]);

    // The fourth argument picks one rectangle out of a multi-area reference.
    let source = match args.get(3) {
        Some(e) => {
            let which = match ev.eval_number(e) {
                Ok(n) => n.trunc(),
                Err(e) => return Operand::error(e),
            };
            let Operand::Ref(r) = &source else {
                return Operand::error(CellError::Value);
            };
            if which < 1.0 || which as usize > r.areas.len() {
                return Operand::error(CellError::Ref);
            }
            Operand::Ref(RefSet::one(r.areas[which as usize - 1]))
        }
        None => source,
    };

    let grid = Grid::from_operand(ev, &source);
    let mut row = match number_or(ev, args.get(1), 0.0) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Operand::error(e),
    };
    let mut col = match number_or(ev, args.get(2), 0.0) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Operand::error(e),
    };

    // With one index and a single line of cells, that index runs along the line
    // rather than always down: `INDEX(A1:D1, 3)` is C1.
    if args.len() < 3 && grid.rows() == 1 && grid.cols() > 1 {
        std::mem::swap(&mut row, &mut col);
    }
    if row < 0 || col < 0 || row > i64::from(grid.rows()) || col > i64::from(grid.cols()) {
        return Operand::error(CellError::Ref);
    }

    // Zero means every row (or column) of the other index.
    if row == 0 || col == 0 {
        return whole_line(ev, &source, &grid, row as u32, col as u32);
    }
    let (r, c) = (row as u32 - 1, col as u32 - 1);
    match &source {
        Operand::Ref(refs) if refs.areas.len() == 1 => {
            let area = refs.areas[0];
            let at = CellRef::new(area.range.start.row + r, area.range.start.col + c);
            Operand::Ref(RefSet::one(AreaRef {
                sheet: area.sheet,
                range: CellRange::new(at, at),
            }))
        }
        _ => Operand::Value(grid.get(ev, r, c)),
    }
}

/// The row or column a zero index selects.
fn whole_line(ev: &Evaluator, source: &Operand, grid: &Grid, row: u32, col: u32) -> Operand {
    if row == 0 && col == 0 {
        return source.clone();
    }
    if let Operand::Ref(refs) = source {
        if refs.areas.len() == 1 {
            let area = refs.areas[0];
            let r = area.range;
            let range = if row == 0 {
                let c = r.start.col + col - 1;
                CellRange::new(CellRef::new(r.start.row, c), CellRef::new(r.end.row, c))
            } else {
                let x = r.start.row + row - 1;
                CellRange::new(CellRef::new(x, r.start.col), CellRef::new(x, r.end.col))
            };
            return Operand::Ref(RefSet::one(AreaRef {
                sheet: area.sheet,
                range,
            }));
        }
    }
    let cells: Vec<Value> = if row == 0 {
        (0..grid.rows()).map(|r| grid.get(ev, r, col - 1)).collect()
    } else {
        (0..grid.cols()).map(|c| grid.get(ev, row - 1, c)).collect()
    };
    let array = if row == 0 {
        Array::new(cells.len(), 1, cells)
    } else {
        Array::row_vector(cells)
    };
    Operand::from_array(array)
}

/// How a lookup decides a candidate is the answer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Equal, with wildcards when the target is text.
    Exact,
    /// The largest value not exceeding the target — approximate lookup over
    /// ascending data, which is `VLOOKUP`'s default.
    AtMost,
    /// The smallest value not below the target, over descending data.
    AtLeast,
}

/// Compares a candidate against the target the way a lookup does.
///
/// Values of different types never compare: a numeric target skips text
/// entirely rather than ordering against it, which is what stops a column
/// heading from being the "largest value not exceeding" some number. Blanks are
/// skipped for the same reason — an empty cell in the middle of a lookup range
/// is a gap, not a zero.
fn order(target: &Value, candidate: &Value) -> Option<std::cmp::Ordering> {
    if candidate.is_error() || matches!(candidate, Value::Blank) {
        return None;
    }
    if std::mem::discriminant(target) != std::mem::discriminant(candidate) {
        return None;
    }
    compare(candidate, target).ok()
}

/// Finds a position in a vector, or `None` when nothing qualifies.
///
/// Walking backwards is not just a reversed loop for the approximate modes: it
/// turns "keep the last value that qualified" into "stop at the first", which
/// is the same cell whenever the data is sorted as the mode promises.
fn search(
    ev: &Evaluator,
    grid: &Grid,
    down: bool,
    target: &Value,
    mode: Mode,
    reverse: bool,
) -> Option<u32> {
    use std::cmp::Ordering;
    let wildcards = matches!(target, Value::Text(t) if t.contains(['*', '?']));
    // The closest qualifying candidate, not merely the last one. Over data
    // sorted as the mode promises the two are the same cell; over data that is
    // not, this is the answer the user meant and Excel's is undefined.
    let mut best: Option<(u32, Value)> = None;

    let line = grid.line(down);
    let positions: Box<dyn Iterator<Item = u32>> = if reverse {
        Box::new(line.rev())
    } else {
        Box::new(line)
    };

    for i in positions {
        let (r, c) = if down { (i, 0) } else { (0, i) };
        let candidate = grid.get(ev, r, c);
        let ord = order(target, &candidate);
        let exact = if wildcards && mode == Mode::Exact {
            matches!((&candidate, target), (Value::Text(a), Value::Text(b)) if wildcard_match(b, a))
        } else {
            ord == Some(Ordering::Equal)
        };
        if exact {
            return Some(i);
        }
        let qualifies = match mode {
            Mode::Exact => false,
            Mode::AtMost => ord == Some(Ordering::Less),
            Mode::AtLeast => ord == Some(Ordering::Greater),
        };
        if qualifies {
            if reverse {
                return Some(i);
            }
            let closer = match &best {
                None => true,
                Some((_, held)) => {
                    let against = compare(&candidate, held).unwrap_or(Ordering::Equal);
                    match mode {
                        Mode::AtMost => against == Ordering::Greater,
                        _ => against == Ordering::Less,
                    }
                }
            };
            if closer {
                best = Some((i, candidate));
            }
        }
    }
    best.map(|(i, _)| i)
}

/// `MATCH(value, array, [type])` — the position of a value in a vector.
///
/// Excel binary-searches for types 1 and -1 and gives undefined answers on data
/// that is not sorted as promised. This scans linearly instead, which agrees
/// wherever Excel is defined and is merely slower.
fn match_fn(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let target = ev.eval_scalar(&args[0]);
    if let Value::Error(e) = target {
        return Operand::error(e);
    }
    let grid = Grid::of(ev, &args[1]);
    let kind = match number_or(ev, args.get(2), 1.0) {
        Ok(n) => n.trunc(),
        Err(e) => return Operand::error(e),
    };
    let mode = match kind {
        k if k > 0.0 => Mode::AtMost,
        k if k < 0.0 => Mode::AtLeast,
        _ => Mode::Exact,
    };

    let down = grid.cols() == 1;
    match search(ev, &grid, down, &target, mode, false) {
        Some(i) => Operand::number(f64::from(i) + 1.0),
        None => Operand::error(CellError::NotAvailable),
    }
}

/// `VLOOKUP` and `HLOOKUP` — search the first line of a table, return from the
/// `n`th line across.
fn table_lookup(ev: &mut Evaluator, args: &[Expr], vertical: bool) -> Operand {
    if !arity(args, 3, Some(4)) {
        return Operand::error(CellError::Value);
    }
    let target = ev.eval_scalar(&args[0]);
    if let Value::Error(e) = target {
        return Operand::error(e);
    }
    let grid = Grid::of(ev, &args[1]);
    let offset = match ev.eval_number(&args[2]) {
        Ok(n) => n.trunc(),
        Err(e) => return Operand::error(e),
    };
    // The default is approximate, which is the wrong default and cannot be
    // changed: a table that is not sorted gives a wrong answer rather than
    // `#N/A`, and that is Excel's behaviour.
    let approximate = match args.get(3) {
        Some(e) => match ev.eval_bool(e) {
            Ok(b) => b,
            Err(e) => return Operand::error(e),
        },
        None => true,
    };

    let across = if vertical { grid.cols() } else { grid.rows() };
    if offset < 1.0 {
        return Operand::error(CellError::Value);
    }
    if offset > f64::from(across) {
        return Operand::error(CellError::Ref);
    }
    let mode = if approximate {
        Mode::AtMost
    } else {
        Mode::Exact
    };
    let Some(i) = search(ev, &grid, vertical, &target, mode, false) else {
        return Operand::error(CellError::NotAvailable);
    };
    let step = offset as u32 - 1;
    Operand::Value(if vertical {
        grid.get(ev, i, step)
    } else {
        grid.get(ev, step, i)
    })
}

/// `LOOKUP` — the oldest of the family, kept for files that still use it.
///
/// The array form guesses its own orientation: a rectangle taller than it is
/// wide is searched down its first column, otherwise across its first row, and
/// the result comes from the *last* line either way.
fn lookup_fn(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let target = ev.eval_scalar(&args[0]);
    if let Value::Error(e) = target {
        return Operand::error(e);
    }
    let grid = Grid::of(ev, &args[1]);

    let (result, down) = match args.get(2) {
        Some(e) => {
            let r = Grid::of(ev, e);
            let down = grid.cols() == 1;
            (Some(r), down)
        }
        None => (None, grid.rows() >= grid.cols()),
    };

    let Some(i) = search(ev, &grid, down, &target, Mode::AtMost, false) else {
        return Operand::error(CellError::NotAvailable);
    };

    match result {
        // A result vector is read along its own long axis, wherever it lies.
        Some(r) => {
            let value = if r.cols() == 1 {
                r.get(ev, i, 0)
            } else {
                r.get(ev, 0, i)
            };
            Operand::Value(value)
        }
        None => Operand::Value(if down {
            grid.get(ev, i, grid.cols() - 1)
        } else {
            grid.get(ev, grid.rows() - 1, i)
        }),
    }
}

/// `XLOOKUP(lookup, lookup_array, return_array, [if_not_found], [match_mode], [search_mode])`.
///
/// The replacement for `VLOOKUP` and its off-by-one column index: the returned
/// range is named rather than counted, so inserting a column cannot silently
/// change the answer.
fn xlookup(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, Some(6)) {
        return Operand::error(CellError::Value);
    }
    let target = ev.eval_scalar(&args[0]);
    if let Value::Error(e) = target {
        return Operand::error(e);
    }
    let haystack = Grid::of(ev, &args[1]);
    let results = Grid::of(ev, &args[2]);
    let (mode, reverse) = match modes(ev, args.get(4), args.get(5)) {
        Ok(m) => m,
        Err(e) => return Operand::error(e),
    };

    let down = haystack.cols() == 1;
    let Some(i) = search(ev, &haystack, down, &target, mode, reverse) else {
        // The fourth argument is why `IFNA(XLOOKUP(...))` is unnecessary.
        return match args.get(3) {
            Some(e) if !matches!(e, Expr::Missing) => ev.eval(e),
            _ => Operand::error(CellError::NotAvailable),
        };
    };

    // The result may be a whole row or column of a block, not a single cell:
    // `XLOOKUP` returning from a two-column range gives back both columns.
    let span = if down { results.cols() } else { results.rows() };
    let cells: Vec<Value> = (0..span)
        .map(|j| {
            let (r, c) = if down { (i, j) } else { (j, i) };
            results.get(ev, r, c)
        })
        .collect();
    let array = if down {
        Array::row_vector(cells)
    } else {
        Array::new(cells.len(), 1, cells)
    };
    Operand::from_array(array)
}

fn xmatch(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(4)) {
        return Operand::error(CellError::Value);
    }
    let target = ev.eval_scalar(&args[0]);
    if let Value::Error(e) = target {
        return Operand::error(e);
    }
    let grid = Grid::of(ev, &args[1]);
    let (mode, reverse) = match modes(ev, args.get(2), args.get(3)) {
        Ok(m) => m,
        Err(e) => return Operand::error(e),
    };
    let down = grid.cols() == 1;
    match search(ev, &grid, down, &target, mode, reverse) {
        Some(i) => Operand::number(f64::from(i) + 1.0),
        None => Operand::error(CellError::NotAvailable),
    }
}

/// The match and search modes the `X` functions share.
fn modes(
    ev: &mut Evaluator,
    match_mode: Option<&Expr>,
    search_mode: Option<&Expr>,
) -> Result<(Mode, bool), CellError> {
    let mode = match number_or(ev, match_mode, 0.0)?.trunc() as i64 {
        0 => Mode::Exact,
        -1 => Mode::AtMost,
        1 => Mode::AtLeast,
        // Mode 2 is wildcards, which `Mode::Exact` already applies to text.
        2 => Mode::Exact,
        _ => return Err(CellError::Value),
    };
    // The binary search modes differ only in speed, and 2 and -2 say which way
    // the data is sorted rather than which way to walk it.
    let reverse = matches!(number_or(ev, search_mode, 1.0)?.trunc() as i64, -1);
    Ok((mode, reverse))
}

fn transpose(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    let op = ev.eval(&args[0]);
    let a = ev.spread(&op);
    let mut cells = Vec::with_capacity(a.rows() * a.cols());
    for c in 0..a.cols() {
        for r in 0..a.rows() {
            cells.push(a.get(r, c).cloned().unwrap_or(Value::Blank));
        }
    }
    Operand::from_array(Array::new(a.cols(), a.rows(), cells))
}

/// `OFFSET(ref, rows, cols, [height], [width])` — a rectangle displaced from
/// another one. Returns a reference, so it can be summed or ranged over.
fn offset(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, Some(5)) {
        return Operand::error(CellError::Value);
    }
    let base = ev.eval(&args[0]);
    let Operand::Ref(refs) = &base else {
        return Operand::error(CellError::Value);
    };
    let Some(area) = refs.areas.first().copied() else {
        return Operand::error(CellError::Ref);
    };

    let numbers = [
        ev.eval_number(&args[1]),
        ev.eval_number(&args[2]),
        number_or(ev, args.get(3), f64::from(area.range.rows())),
        number_or(ev, args.get(4), f64::from(area.range.cols())),
    ];
    let mut n = [0i64; 4];
    for (slot, value) in n.iter_mut().zip(numbers) {
        match value {
            Ok(v) => *slot = v.trunc() as i64,
            Err(e) => return Operand::error(e),
        }
    }
    let [rows, cols, height, width] = n;
    if height <= 0 || width <= 0 {
        return Operand::error(CellError::Ref);
    }

    let start_row = i64::from(area.range.start.row) + rows;
    let start_col = i64::from(area.range.start.col) + cols;
    let end_row = start_row + height - 1;
    let end_col = start_col + width - 1;
    // Off the edge of the sheet in any direction is `#REF!`, not a clamp.
    if start_row < 0
        || start_col < 0
        || end_row >= i64::from(MAX_ROWS)
        || end_col >= i64::from(MAX_COLS)
    {
        return Operand::error(CellError::Ref);
    }
    Operand::Ref(RefSet::one(AreaRef {
        sheet: area.sheet,
        range: CellRange::new(
            CellRef::new(start_row as u32, start_col as u32),
            CellRef::new(end_row as u32, end_col as u32),
        ),
    }))
}

/// `INDIRECT(text, [a1])` — a reference assembled from text at calculation time.
///
/// The R1C1 form is `#REF!` rather than wrong: the parser is A1-only, and
/// silently reading `R1C1` as a defined name would produce a plausible answer
/// from the wrong cell.
fn indirect(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let text = match ev.eval_text(&args[0]) {
        Ok(t) => t,
        Err(e) => return Operand::error(e),
    };
    if let Some(e) = args.get(1) {
        match ev.eval_bool(e) {
            Ok(true) => {}
            Ok(false) => return Operand::error(CellError::Ref),
            Err(e) => return Operand::error(e),
        }
    }
    let Ok(expr) = crate::parse(&text) else {
        return Operand::error(CellError::Ref);
    };
    // Only a reference, never a formula: `INDIRECT("SUM(A1:A9)")` is `#REF!` in
    // Excel too, and evaluating arbitrary text here would be a way to smuggle
    // execution past the depth guard.
    match expr {
        Expr::Ref(_) => match ev.eval(&expr) {
            Operand::Value(Value::Error(_)) => Operand::error(CellError::Ref),
            other => other,
        },
        _ => Operand::error(CellError::Ref),
    }
}

/// `ADDRESS(row, col, [abs], [a1], [sheet])` — a reference as text.
fn address(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(5)) {
        return Operand::error(CellError::Value);
    }
    let (row, col) = match (ev.eval_number(&args[0]), ev.eval_number(&args[1])) {
        (Ok(r), Ok(c)) => (r.trunc(), c.trunc()),
        (Err(e), _) | (_, Err(e)) => return Operand::error(e),
    };
    if row < 1.0 || col < 1.0 || row > f64::from(MAX_ROWS) || col > f64::from(MAX_COLS) {
        return Operand::error(CellError::Value);
    }
    let kind = match number_or(ev, args.get(2), 1.0) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Operand::error(e),
    };
    if !(1..=4).contains(&kind) {
        return Operand::error(CellError::Value);
    }
    if let Some(e) = args.get(3) {
        match ev.eval_bool(e) {
            Ok(true) => {}
            // R1C1 output would be easy; producing it while `INDIRECT` cannot
            // read it back would be worse than saying no.
            Ok(false) => return Operand::error(CellError::Value),
            Err(e) => return Operand::error(e),
        }
    }

    // 1 is both absolute, 2 absolute row, 3 absolute column, 4 neither.
    let row_abs = kind <= 2;
    let col_abs = kind == 1 || kind == 3;
    let name = ss_model::column_name(col as u32 - 1);
    let mut out = String::new();
    if let Some(e) = args.get(4) {
        let sheet = match ev.eval_text(e) {
            Ok(s) => s,
            Err(e) => return Operand::error(e),
        };
        if !sheet.is_empty() {
            // A sheet name with a space or punctuation has to be quoted, and an
            // apostrophe inside one is doubled.
            if sheet.contains(|c: char| !c.is_alphanumeric() && c != '_') {
                out.push('\'');
                out.push_str(&sheet.replace('\'', "''"));
                out.push('\'');
            } else {
                out.push_str(&sheet);
            }
            out.push('!');
        }
    }
    if col_abs {
        out.push('$');
    }
    out.push_str(&name);
    if row_abs {
        out.push('$');
    }
    out.push_str(&format!("{}", row as u32));
    Operand::text(out)
}

fn number_or(ev: &mut Evaluator, arg: Option<&Expr>, default: f64) -> Result<f64, CellError> {
    match arg {
        None | Some(Expr::Missing) => Ok(default),
        Some(e) => ev.eval_number(e),
    }
}
