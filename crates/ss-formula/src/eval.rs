//! Evaluation: turning a parsed expression into a value.
//!
//! Three things here are load-bearing and easy to get subtly wrong.
//!
//! **Errors are values, not failures.** `#DIV/0!` flows through the engine like
//! any other value and lands in a cell. Nothing below returns `Result` at the
//! top level, because there is no failure — only results the user can see.
//!
//! **References are not their contents.** A range stays a `RefSet` until
//! something asks for values. That is what lets `ROW(A5)` answer without reading
//! A5, and what lets `OFFSET` and `INDEX` hand back an address that can be
//! summed or used as one end of a range.
//!
//! **Arrays broadcast.** Any operator applied to arrays produces an array, with
//! single rows and columns stretched to fit. Where the shapes genuinely disagree,
//! the missing cells are `#N/A` rather than an error over the whole result.

use std::cmp::Ordering;

use ss_model::cell::MAX_ROWS;
use ss_model::{CellError, CellRange, CellRef};

use crate::ast::{Area, BinaryOp, Expr, Reference, SheetRef, UnaryOp};
use crate::functions;
use crate::graph::AreaRef;
use crate::value::{compare, Array, Operand, RefSet, Value};

/// What the evaluator needs from the workbook around it.
///
/// Deliberately narrow: the formula engine should be testable against a handful
/// of hard-coded cells, and should not need a whole document to run.
pub trait Context {
    /// Index of a sheet by name, case-insensitively. `None` for a sheet that
    /// does not exist, which reads as `#REF!`.
    fn sheet_index(&self, name: &str) -> Option<usize>;

    /// The value in a cell. Blank for an empty or non-existent cell.
    fn cell(&self, sheet: usize, at: CellRef) -> Value;

    /// The rectangle beyond which a sheet is certainly empty.
    ///
    /// This is what keeps `SUM(A:A)` from visiting a million rows. Returning
    /// `None` means "no cells at all"; returning too *large* a rectangle is
    /// merely slow, but returning too small a one loses data.
    fn used_bounds(&self, sheet: usize) -> Option<CellRange>;

    /// A defined name, scoped to `sheet` first and then to the workbook.
    fn resolve_name(&self, sheet: usize, name: &str) -> Option<Operand> {
        let _ = (sheet, name);
        None
    }

    /// The current moment, as a serial number.
    ///
    /// On the trait so `TODAY` and `NOW` are testable — a test that could only
    /// fail at midnight is not a test. The default is UTC; a host that knows the
    /// user's time zone should override it, because Excel's `NOW` is local.
    fn now(&self) -> f64 {
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        ss_model::datetime::UNIX_EPOCH_SERIAL_F64 + seconds / 86_400.0
    }
}

/// The cell a formula is being evaluated for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub sheet: usize,
    pub at: CellRef,
}

impl Position {
    pub const fn new(sheet: usize, at: CellRef) -> Self {
        Position { sheet, at }
    }
}

/// Guards against a formula whose *evaluation* nests deeply even though its
/// text did not — `IF` chains built by expansion, mostly.
const MAX_DEPTH: u32 = 128;

pub struct Evaluator<'a> {
    ctx: &'a dyn Context,
    position: Position,
    depth: u32,
    rng: u64,
}

impl<'a> Evaluator<'a> {
    pub fn new(ctx: &'a dyn Context, position: Position) -> Self {
        Evaluator {
            ctx,
            position,
            depth: 0,
            // A fixed seed, so a recalculation is reproducible in tests. Callers
            // that want fresh randomness reseed per pass.
            rng: 0x2545_F491_4F6C_DD1D,
        }
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.rng = seed | 1;
        self
    }

    pub const fn position(&self) -> Position {
        self.position
    }

    pub fn context(&self) -> &'a dyn Context {
        self.ctx
    }

    /// A uniform value in `[0, 1)`, for `RAND` and friends.
    ///
    /// xorshift64*: not a good generator for anything that matters, entirely
    /// adequate for a spreadsheet, and free of a dependency.
    pub fn next_random(&mut self) -> f64 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let x = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
        // 53 bits is exactly what an f64 mantissa holds.
        (x >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Evaluates an expression to whatever it naturally produces — a value, an
    /// array, or a reference.
    pub fn eval(&mut self, expr: &Expr) -> Operand {
        if self.depth >= MAX_DEPTH {
            return Operand::error(CellError::Num);
        }
        self.depth += 1;
        let out = self.eval_inner(expr);
        self.depth -= 1;
        out
    }

    fn eval_inner(&mut self, expr: &Expr) -> Operand {
        match expr {
            Expr::Number(n) => Operand::number(*n),
            Expr::Text(s) => Operand::text(s.clone()),
            Expr::Bool(b) => Operand::boolean(*b),
            Expr::Error(e) => Operand::error(*e),
            Expr::Missing => Operand::Value(Value::Blank),
            Expr::Ref(r) => self.reference(r),
            Expr::Name { sheet, name } => self.name(sheet, name),
            Expr::Array(rows) => self.array_literal(rows),
            Expr::Unary { op, operand } => {
                let v = self.eval(operand);
                self.unary(*op, v)
            }
            Expr::Binary { op, lhs, rhs } => {
                let a = self.eval(lhs);
                let b = self.eval(rhs);
                self.binary(*op, a, b)
            }
            Expr::Call { name, args } => functions::call(self, name, args),
        }
    }

    /// Evaluates to a single value, collapsing arrays and references.
    pub fn eval_scalar(&mut self, expr: &Expr) -> Value {
        let op = self.eval(expr);
        self.collapse(op)
    }

    /// Reduces an operand to the one value a scalar context wants, applying
    /// **implicit intersection** to a reference.
    ///
    /// A range used where a single value is expected does not collapse to its
    /// first cell — it collapses to the cell in line with the formula.
    /// `=A1:A10` in C5 is A5, and in C11 it is `#VALUE!`, because the range has
    /// no row 11 to offer. This is the rule modern Excel spells `@`, and it is
    /// why a column of `=$B$1:$B$5*2` down five rows gives five different
    /// answers rather than five copies of the first.
    ///
    /// Arrays are not references and do not intersect; they take their first
    /// value, which is what a literal in scalar position does.
    pub fn collapse(&self, op: Operand) -> Value {
        match op {
            Operand::Value(v) => v,
            Operand::Array(a) => a.first(),
            Operand::Ref(r) => match r.areas.first() {
                Some(a) => match self.intersect_with_position(a) {
                    Some(at) => self.ctx.cell(a.sheet, at),
                    None => Value::Error(CellError::Value),
                },
                None => Value::Error(CellError::Ref),
            },
        }
    }

    /// The single cell an area offers the formula's own row and column, if any.
    fn intersect_with_position(&self, area: &AreaRef) -> Option<CellRef> {
        let range = area.range;
        let (rows, cols) = (range.rows(), range.cols());
        if rows == 1 && cols == 1 {
            return Some(range.start);
        }
        let here = self.position.at;
        if rows == 1 && (range.start.col..=range.end.col).contains(&here.col) {
            return Some(CellRef::new(range.start.row, here.col));
        }
        if cols == 1 && (range.start.row..=range.end.row).contains(&here.row) {
            return Some(CellRef::new(here.row, range.start.col));
        }
        // A block, or a line the formula does not sit beside. Excel has nothing
        // to pick and says so rather than guessing.
        None
    }

    pub fn eval_number(&mut self, expr: &Expr) -> Result<f64, CellError> {
        self.eval_scalar(expr).to_number()
    }

    pub fn eval_text(&mut self, expr: &Expr) -> Result<String, CellError> {
        self.eval_scalar(expr).to_text()
    }

    pub fn eval_bool(&mut self, expr: &Expr) -> Result<bool, CellError> {
        self.eval_scalar(expr).to_bool()
    }

    /// Reads every value an operand covers, in row-major order.
    pub fn spread(&self, op: &Operand) -> Array {
        match op {
            Operand::Value(v) => Array::new(1, 1, vec![v.clone()]),
            Operand::Array(a) => a.clone(),
            Operand::Ref(r) => self.spread_ref(r),
        }
    }

    fn spread_ref(&self, refs: &RefSet) -> Array {
        // A multi-area reference has no single rectangular shape. Laying it out
        // as one row is what Excel does when such a thing reaches an array
        // context, and it is what aggregation wants anyway.
        if refs.areas.len() != 1 {
            let mut cells = Vec::new();
            for area in &refs.areas {
                self.visit_area(area, &mut |v| cells.push(v));
            }
            return Array::row_vector(cells);
        }
        let area = refs.areas[0];
        let range = self.clip(area);
        let rows = range.rows() as usize;
        let cols = range.cols() as usize;
        let mut cells = Vec::with_capacity(rows * cols);
        for row in range.start.row..=range.end.row {
            for col in range.start.col..=range.end.col {
                cells.push(self.ctx.cell(area.sheet, CellRef::new(row, col)));
            }
        }
        Array::new(rows, cols, cells)
    }

    /// Visits the values in one area without building an array for them.
    pub fn visit_area(&self, area: &AreaRef, f: &mut impl FnMut(Value)) {
        let range = self.clip(*area);
        for row in range.start.row..=range.end.row {
            for col in range.start.col..=range.end.col {
                f(self.ctx.cell(area.sheet, CellRef::new(row, col)));
            }
        }
    }

    /// Trims an area to the part of the sheet that can hold anything.
    ///
    /// Without this, `SUM(A:A)` is a million `cell()` calls for one number.
    pub fn clip_area(&self, area: &AreaRef) -> CellRange {
        self.clip(*area)
    }

    fn clip(&self, area: AreaRef) -> CellRange {
        let range = area.range;
        // Only whole-column and whole-row references are worth clipping; an
        // ordinary range is already the size the user asked for.
        if range.rows() < MAX_ROWS / 2 && range.cols() < 128 {
            return range;
        }
        let Some(used) = self.ctx.used_bounds(area.sheet) else {
            // No cells at all: keep one cell so the shape stays non-empty.
            return CellRange::new(range.start, range.start);
        };
        let start = CellRef::new(
            range.start.row.max(used.start.row),
            range.start.col.max(used.start.col),
        );
        let end = CellRef::new(
            range.end.row.min(used.end.row),
            range.end.col.min(used.end.col),
        );
        if end.row < start.row || end.col < start.col {
            CellRange::new(range.start, range.start)
        } else {
            CellRange::new(start, end)
        }
    }

    fn reference(&mut self, r: &Reference) -> Operand {
        let sheets: Vec<usize> = match &r.sheet {
            SheetRef::Current => vec![self.position.sheet],
            SheetRef::Named(n) => match self.ctx.sheet_index(n) {
                Some(i) => vec![i],
                None => return Operand::error(CellError::Ref),
            },
            SheetRef::Span(a, b) => match (self.ctx.sheet_index(a), self.ctx.sheet_index(b)) {
                (Some(x), Some(y)) => (x.min(y)..=x.max(y)).collect(),
                _ => return Operand::error(CellError::Ref),
            },
        };
        let Some(range) = area_to_range(&r.area) else {
            return Operand::error(CellError::Ref);
        };
        Operand::Ref(RefSet {
            areas: sheets
                .into_iter()
                .map(|sheet| AreaRef { sheet, range })
                .collect(),
        })
    }

    fn name(&mut self, sheet: &SheetRef, name: &str) -> Operand {
        let scope = match sheet {
            SheetRef::Current => self.position.sheet,
            SheetRef::Named(n) => match self.ctx.sheet_index(n) {
                Some(i) => i,
                None => return Operand::error(CellError::Ref),
            },
            SheetRef::Span(..) => return Operand::error(CellError::Ref),
        };
        self.ctx
            .resolve_name(scope, name)
            .unwrap_or(Operand::error(CellError::Name))
    }

    fn array_literal(&mut self, rows: &[Vec<Expr>]) -> Operand {
        let height = rows.len();
        let width = rows.iter().map(Vec::len).max().unwrap_or(0);
        if height == 0 || width == 0 {
            return Operand::error(CellError::Value);
        }
        let mut cells = Vec::with_capacity(height * width);
        for row in rows {
            for col in 0..width {
                // A literal with uneven rows is not something Excel will write,
                // but a hand-edited file can contain one. Pad rather than reject.
                cells.push(match row.get(col) {
                    Some(e) => self.eval_scalar(e),
                    None => Value::Error(CellError::NotAvailable),
                });
            }
        }
        Operand::Array(Array::new(height, width, cells))
    }

    fn unary(&mut self, op: UnaryOp, operand: Operand) -> Operand {
        self.map_unary(operand, |v| match op {
            UnaryOp::Neg => Value::from(v.to_number().map(|n| -n)),
            // Unary plus is not a coercion: `+"abc"` is the text "abc" in
            // Excel, not `#VALUE!`. It passes its operand through untouched,
            // which is why it is not simply dropped by the parser.
            UnaryOp::Plus => v,
            UnaryOp::Percent => Value::from(v.to_number().map(|n| n / 100.0)),
        })
    }

    fn map_unary(&self, operand: Operand, f: impl Fn(Value) -> Value) -> Operand {
        match operand {
            Operand::Value(v) => Operand::Value(f(v)),
            other => {
                let a = self.spread(&other);
                let cells: Vec<Value> = a.values().cloned().map(&f).collect();
                Operand::from_array(Array::new(a.rows(), a.cols(), cells))
            }
        }
    }

    fn binary(&mut self, op: BinaryOp, lhs: Operand, rhs: Operand) -> Operand {
        match op {
            BinaryOp::Range => return self.range_op(lhs, rhs),
            BinaryOp::Intersect => return self.intersect_op(lhs, rhs),
            BinaryOp::Union => return self.union_op(lhs, rhs),
            _ => {}
        }
        self.broadcast(lhs, rhs, |a, b| apply_scalar(op, a, b))
    }

    /// Applies a scalar operation across two operands, stretching to fit.
    pub fn broadcast(
        &self,
        lhs: Operand,
        rhs: Operand,
        f: impl Fn(&Value, &Value) -> Value,
    ) -> Operand {
        if let (Operand::Value(a), Operand::Value(b)) = (&lhs, &rhs) {
            return Operand::Value(f(a, b));
        }
        let a = self.spread(&lhs);
        let b = self.spread(&rhs);
        let rows = a.rows().max(b.rows());
        let cols = a.cols().max(b.cols());
        let mut cells = Vec::with_capacity(rows * cols);
        for r in 0..rows {
            for c in 0..cols {
                // A single row or column stretches; anything else that does not
                // reach is `#N/A` in that slot only, which is Excel's answer.
                let x = pick(&a, r, c);
                let y = pick(&b, r, c);
                cells.push(match (x, y) {
                    (Some(x), Some(y)) => f(x, y),
                    _ => Value::Error(CellError::NotAvailable),
                });
            }
        }
        Operand::from_array(Array::new(rows, cols, cells))
    }

    /// `A1:B2` — the smallest rectangle covering both operands.
    fn range_op(&mut self, lhs: Operand, rhs: Operand) -> Operand {
        let (Operand::Ref(a), Operand::Ref(b)) = (&lhs, &rhs) else {
            return Operand::error(CellError::Value);
        };
        let (Some(x), Some(y)) = (a.areas.first(), b.areas.first()) else {
            return Operand::error(CellError::Ref);
        };
        if x.sheet != y.sheet {
            return Operand::error(CellError::Ref);
        }
        let corners = [x.range.start, x.range.end, y.range.start, y.range.end];
        let range = CellRange::new(
            CellRef::new(
                corners.iter().map(|c| c.row).min().unwrap_or(0),
                corners.iter().map(|c| c.col).min().unwrap_or(0),
            ),
            CellRef::new(
                corners.iter().map(|c| c.row).max().unwrap_or(0),
                corners.iter().map(|c| c.col).max().unwrap_or(0),
            ),
        );
        Operand::Ref(RefSet::one(AreaRef {
            sheet: x.sheet,
            range,
        }))
    }

    /// A space between references — their overlap. `#NULL!` when they miss.
    fn intersect_op(&mut self, lhs: Operand, rhs: Operand) -> Operand {
        let (Operand::Ref(a), Operand::Ref(b)) = (&lhs, &rhs) else {
            return Operand::error(CellError::Null);
        };
        let (Some(x), Some(y)) = (a.areas.first(), b.areas.first()) else {
            return Operand::error(CellError::Null);
        };
        if x.sheet != y.sheet {
            return Operand::error(CellError::Null);
        }
        let start = CellRef::new(
            x.range.start.row.max(y.range.start.row),
            x.range.start.col.max(y.range.start.col),
        );
        let end = CellRef::new(
            x.range.end.row.min(y.range.end.row),
            x.range.end.col.min(y.range.end.col),
        );
        if start.row > end.row || start.col > end.col {
            return Operand::error(CellError::Null);
        }
        Operand::Ref(RefSet::one(AreaRef {
            sheet: x.sheet,
            range: CellRange::new(start, end),
        }))
    }

    /// A comma between references — both of them, as one multi-area reference.
    fn union_op(&mut self, lhs: Operand, rhs: Operand) -> Operand {
        let (Operand::Ref(a), Operand::Ref(b)) = (lhs, rhs) else {
            return Operand::error(CellError::Value);
        };
        let mut areas = a.areas;
        areas.extend(b.areas);
        Operand::Ref(RefSet { areas })
    }
}

/// The value at `(row, col)`, stretching a single row or column to fit.
fn pick(a: &Array, row: usize, col: usize) -> Option<&Value> {
    let r = if a.rows() == 1 { 0 } else { row };
    let c = if a.cols() == 1 { 0 } else { col };
    a.get(r, c)
}

fn apply_scalar(op: BinaryOp, a: &Value, b: &Value) -> Value {
    match op {
        BinaryOp::Concat => match (a.to_text(), b.to_text()) {
            (Ok(x), Ok(y)) => Value::Text(x + &y),
            (Err(e), _) | (_, Err(e)) => Value::Error(e),
        },
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Ge => {
            match compare(a, b) {
                Err(e) => Value::Error(e),
                Ok(ord) => Value::Bool(match op {
                    BinaryOp::Eq => ord == Ordering::Equal,
                    BinaryOp::Ne => ord != Ordering::Equal,
                    BinaryOp::Lt => ord == Ordering::Less,
                    BinaryOp::Gt => ord == Ordering::Greater,
                    BinaryOp::Le => ord != Ordering::Greater,
                    _ => ord != Ordering::Less,
                }),
            }
        }
        _ => {
            // The left operand's error wins, which is the order Excel reports.
            let x = match a.to_number() {
                Ok(x) => x,
                Err(e) => return Value::Error(e),
            };
            let y = match b.to_number() {
                Ok(y) => y,
                Err(e) => return Value::Error(e),
            };
            arithmetic(op, x, y)
        }
    }
}

/// The arithmetic operators, with Excel's answers at the awkward inputs.
pub fn arithmetic(op: BinaryOp, x: f64, y: f64) -> Value {
    let out = match op {
        BinaryOp::Add => x + y,
        BinaryOp::Sub => x - y,
        BinaryOp::Mul => x * y,
        BinaryOp::Div => {
            if y == 0.0 {
                return Value::Error(CellError::Div0);
            }
            x / y
        }
        BinaryOp::Pow => return power(x, y),
        _ => return Value::Error(CellError::Value),
    };
    finite(out)
}

/// Excel's `^`, which is not `f64::powf`.
///
/// A negative base with a fractional exponent is `#NUM!` rather than NaN — Excel
/// does not reach for the complex plane — and `0^-1` is a division by zero
/// because that is what it is.
pub fn power(x: f64, y: f64) -> Value {
    if x == 0.0 {
        if y == 0.0 {
            return Value::Number(1.0);
        }
        if y < 0.0 {
            return Value::Error(CellError::Div0);
        }
        return Value::Number(0.0);
    }
    if x < 0.0 && y.fract() != 0.0 {
        return Value::Error(CellError::Num);
    }
    finite(x.powf(y))
}

/// Turns an out-of-range result into the error Excel reports for it.
pub fn finite(x: f64) -> Value {
    if x.is_finite() {
        Value::Number(x)
    } else {
        // Overflow and NaN are both `#NUM!`: Excel has no infinity to report.
        Value::Error(CellError::Num)
    }
}

/// Converts a parsed reference area to a concrete rectangle.
fn area_to_range(area: &Area) -> Option<CellRange> {
    use ss_model::cell::MAX_COLS;
    Some(match area {
        Area::Cell(a) => {
            let at = CellRef::new(a.row, a.col);
            CellRange::new(at, at)
        }
        Area::Range { start, end } => CellRange::new(
            CellRef::new(start.row, start.col),
            CellRef::new(end.row, end.col),
        ),
        Area::Cols { start, end, .. } => CellRange::new(
            CellRef::new(0, *start),
            CellRef::new(MAX_ROWS - 1, (*end).min(MAX_COLS - 1)),
        ),
        Area::Rows { start, end, .. } => CellRange::new(
            CellRef::new(*start, 0),
            CellRef::new((*end).min(MAX_ROWS - 1), MAX_COLS - 1),
        ),
    })
}
