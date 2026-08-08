//! Deciding what a conditional format actually does to a cell.
//!
//! The rules are modeled in `ss-model`; evaluating them needs the engine, so it
//! happens here. Three things make this more than a loop over predicates:
//!
//! **A rule's formula is written against the region's top-left cell**, not
//! against the cell being tested. A rule over `B2:B100` saying `$C2="x"` means
//! "column C of *my* row" for every one of those cells, so every relative
//! reference has to be shifted before evaluation. Testing the formula as
//! written would format the whole column according to row 2.
//!
//! **Priority is workbook-wide and lower wins.** Several rules can match one
//! cell and each contributes only the properties it sets — a rule that turns
//! text red must not also clear the cell's fill. `stopIfTrue` cuts the rest off.
//!
//! **The visual kinds need the whole region.** A colour scale interpolates
//! between the region's own minimum and maximum, so nothing about one cell can
//! be decided without reading all of them. That is why this is prepared once
//! per sheet rather than asked per cell: the alternative is rescanning the
//! region for every visible cell, every frame.

use std::collections::HashMap;

use ss_model::cond::{
    CfKind, CfOperator, CfValue, CfValueKind, DvKind, DvOperator, DvSeverity, TextOp,
};
use ss_model::{CellRange, CellRef, CellValue, Dxf, Workbook};

use crate::ast::{Area, Expr};
use crate::eval::{Context, Evaluator, Position};
use crate::lexer::A1;
use crate::value::{compare, Value};
use crate::workbook::{value_of, WorkbookContext};

/// Something drawn behind or instead of the cell's text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Overlay {
    /// A colour scale: a flat shade across the whole cell.
    Shade([u8; 3]),
    /// A data bar covering `fraction` of the cell's width.
    Bar { fraction: f64, color: [u8; 3] },
}

/// Everything the conditional formats have to say about one cell.
#[derive(Debug, Clone, Default)]
pub struct Effect {
    /// The merged differential format, first matching rule winning per field.
    pub dxf: Dxf,
    pub overlay: Option<Overlay>,
    /// `showValue="0"` on a data bar or icon set: draw the bar, hide the number.
    pub hide_value: bool,
}

impl Effect {
    pub fn is_empty(&self) -> bool {
        self.dxf.is_empty() && self.overlay.is_none() && !self.hide_value
    }
}

/// The conditional formats of one sheet, prepared for asking about cells.
pub struct Formatting {
    rules: Vec<Prepared>,
}

struct Prepared {
    ranges: Vec<CellRange>,
    anchor: CellRef,
    kind: CfKind,
    /// The rule's own colours, already resolved against the workbook theme. A
    /// colour scale carries them itself rather than pointing at a dxf.
    colors: Vec<[u8; 3]>,
    dxf: Option<u32>,
    stop_if_true: bool,
    /// The rule's formulas, parsed once. A rule whose formula does not parse
    /// keeps its slot as `None` and simply never matches — better than a rule
    /// that matches everything.
    formulas: Vec<Option<Expr>>,
    stats: Option<Stats>,
}

/// What a rule needs to know about the whole region it covers.
struct Stats {
    /// Every number in the region, sorted ascending: the rank-based kinds need
    /// the order and the scales need only the ends.
    sorted: Vec<f64>,
    mean: f64,
    std_dev: f64,
    /// How many cells hold each distinct value, for duplicate detection.
    counts: HashMap<String, usize>,
}

impl Formatting {
    /// No rules at all, for a caller that needs the shape but not the answers.
    pub fn empty() -> Formatting {
        Formatting { rules: Vec::new() }
    }

    /// Reads a sheet's rules and works out everything that does not depend on
    /// which cell is being asked about.
    pub fn prepare(book: &Workbook, sheet: usize) -> Formatting {
        let Some(model) = book.sheet(sheet) else {
            return Formatting { rules: Vec::new() };
        };
        let mut rules: Vec<(i32, Prepared)> = Vec::new();
        for block in &model.conditional_formats {
            let Some(anchor) = block.anchor() else {
                continue;
            };
            for rule in &block.rules {
                let formulas: Vec<Option<Expr>> = formulas_of(&rule.kind)
                    .iter()
                    .map(|text| crate::parse(text).ok())
                    .collect();
                let stats =
                    needs_region(&rule.kind).then(|| region_stats(book, sheet, &block.ranges));
                rules.push((
                    rule.priority,
                    Prepared {
                        ranges: block.ranges.clone(),
                        anchor,
                        kind: rule.kind.clone(),
                        colors: colors_of(&rule.kind, book),
                        dxf: rule.dxf,
                        stop_if_true: rule.stop_if_true,
                        formulas,
                        stats,
                    },
                ));
            }
        }
        // Lower priority number wins, and the ordering is workbook-wide rather
        // than per-block, so the sort happens after everything is collected.
        // `sort_by_key` is stable, so equal priorities keep document order.
        rules.sort_by_key(|(priority, _)| *priority);
        Formatting {
            rules: rules.into_iter().map(|(_, r)| r).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// What the rules do to one cell, or `None` if they do nothing.
    pub fn effect(&self, book: &Workbook, sheet: usize, at: CellRef) -> Option<Effect> {
        if self.rules.is_empty() {
            return None;
        }
        let ctx = WorkbookContext::new(book);
        let value = ctx.cell(sheet, at);
        let mut effect = Effect::default();

        for rule in &self.rules {
            if !rule.ranges.iter().any(|r| r.contains(at)) {
                continue;
            }
            match &rule.kind {
                CfKind::ColorScale { stops, .. } => {
                    if let Some(shade) = rule.scale_color(&value, stops) {
                        if effect.overlay.is_none() {
                            effect.overlay = Some(Overlay::Shade(shade));
                        }
                    }
                    continue;
                }
                CfKind::DataBar {
                    min,
                    max,
                    show_value,
                    ..
                } => {
                    if let Some(fraction) = rule.bar_fraction(&value, min, max) {
                        if effect.overlay.is_none() {
                            effect.overlay = Some(Overlay::Bar {
                                fraction,
                                color: rule.colors.first().copied().unwrap_or([0x63, 0x8E, 0xC6]),
                            });
                            effect.hide_value |= !show_value;
                        }
                    }
                    continue;
                }
                CfKind::IconSet { show_value, .. } => {
                    // The icons themselves are not drawn, but hiding the value
                    // when the file says to is still the file's intent.
                    effect.hide_value |= !show_value;
                    continue;
                }
                _ => {}
            }

            if !rule.matches(&ctx, sheet, at, &value) {
                continue;
            }
            if let Some(dxf) = rule.dxf.and_then(|i| book.styles.dxf(i)) {
                merge(&mut effect.dxf, dxf);
            }
            if rule.stop_if_true {
                break;
            }
        }

        (!effect.is_empty()).then_some(effect)
    }
}

/// Fills in fields `into` has not got yet. First matching rule wins per field,
/// which is what "lower priority number wins" means once the list is sorted.
fn merge(into: &mut Dxf, from: &Dxf) {
    if into.bold.is_none() {
        into.bold = from.bold;
    }
    if into.italic.is_none() {
        into.italic = from.italic;
    }
    if into.underline.is_none() {
        into.underline = from.underline;
    }
    if into.strike.is_none() {
        into.strike = from.strike;
    }
    if into.color.is_none() {
        into.color = from.color;
    }
    if into.fill.is_none() {
        into.fill = from.fill.clone();
    }
    if into.border.is_none() {
        into.border = from.border;
    }
    if into.number_format.is_none() {
        into.number_format = from.number_format.clone();
    }
}

impl Prepared {
    fn matches(&self, ctx: &WorkbookContext<'_>, sheet: usize, at: CellRef, value: &Value) -> bool {
        let offset = (
            at.row as i64 - self.anchor.row as i64,
            at.col as i64 - self.anchor.col as i64,
        );
        match &self.kind {
            CfKind::CellIs { operator, .. } => {
                let mut operands = self
                    .formulas
                    .iter()
                    .map(|e| self.eval(ctx, sheet, at, offset, e.as_ref()));
                let first = operands.next().flatten();
                let second = operands.next().flatten();
                compare_cell(value, *operator, first.as_ref(), second.as_ref())
            }
            CfKind::Expression { .. } => self
                .eval(
                    ctx,
                    sheet,
                    at,
                    offset,
                    self.formulas.first().and_then(|e| e.as_ref()),
                )
                .map(truthy)
                .unwrap_or(false),
            CfKind::Text { op, text } => {
                let Value::Text(cell) = value else {
                    return false;
                };
                let (cell, want) = (cell.to_lowercase(), text.to_lowercase());
                match op {
                    TextOp::Contains => cell.contains(&want),
                    TextOp::NotContains => !cell.contains(&want),
                    TextOp::BeginsWith => cell.starts_with(&want),
                    TextOp::EndsWith => cell.ends_with(&want),
                }
            }
            CfKind::Presence { blanks, negated } => {
                let present = if *blanks {
                    matches!(value, Value::Blank)
                } else {
                    matches!(value, Value::Error(_))
                };
                present != *negated
            }
            CfKind::Duplicates { unique } => {
                let Some(stats) = &self.stats else {
                    return false;
                };
                let key = key_of(value);
                let count = key.and_then(|k| stats.counts.get(&k).copied()).unwrap_or(0);
                if *unique {
                    count == 1
                } else {
                    count > 1
                }
            }
            CfKind::Top10 {
                rank,
                percent,
                bottom,
            } => {
                let (Some(stats), Value::Number(n)) = (&self.stats, value) else {
                    return false;
                };
                if stats.sorted.is_empty() {
                    return false;
                }
                let take = if *percent {
                    // Excel rounds the count down but always keeps at least one.
                    ((stats.sorted.len() as f64) * f64::from(*rank) / 100.0).floor() as usize
                } else {
                    *rank as usize
                }
                .clamp(1, stats.sorted.len());
                let cut = if *bottom {
                    stats.sorted[take - 1]
                } else {
                    stats.sorted[stats.sorted.len() - take]
                };
                if *bottom {
                    *n <= cut
                } else {
                    *n >= cut
                }
            }
            CfKind::AboveAverage {
                above,
                equal_average,
                std_dev,
            } => {
                let (Some(stats), Value::Number(n)) = (&self.stats, value) else {
                    return false;
                };
                let threshold = match std_dev {
                    Some(sigmas) => {
                        stats.mean
                            + f64::from(*sigmas) * stats.std_dev * if *above { 1.0 } else { -1.0 }
                    }
                    None => stats.mean,
                };
                match (*above, *equal_average) {
                    (true, true) => *n >= threshold,
                    (true, false) => *n > threshold,
                    (false, true) => *n <= threshold,
                    (false, false) => *n < threshold,
                }
            }
            CfKind::TimePeriod { period } => {
                let Value::Number(serial) = value else {
                    return false;
                };
                in_period(*serial, period, ctx.now().floor())
            }
            // The visual kinds are handled before this is reached, and an
            // unrecognized type deliberately does nothing rather than guessing.
            _ => false,
        }
    }

    /// Evaluates a rule formula for one cell, shifting its relative references
    /// from the region's anchor to that cell.
    fn eval(
        &self,
        ctx: &WorkbookContext<'_>,
        sheet: usize,
        at: CellRef,
        offset: (i64, i64),
        expr: Option<&Expr>,
    ) -> Option<Value> {
        let expr = expr?;
        let shifted = shift(expr, offset);
        let mut ev = Evaluator::new(ctx, Position::new(sheet, at));
        Some(ev.eval_scalar(&shifted))
    }

    fn scale_color(&self, value: &Value, stops: &[CfValue]) -> Option<[u8; 3]> {
        let (stats, Value::Number(n)) = (self.stats.as_ref()?, value) else {
            return None;
        };
        let resolved = &self.colors;
        if resolved.len() < 2 || stops.len() < 2 {
            return None;
        }
        let points: Vec<f64> = stops
            .iter()
            .map(|s| threshold(s, stats))
            .collect::<Option<Vec<f64>>>()?;
        // Two colours interpolate end to end; three pivot on the middle stop.
        for window in 0..points.len().min(resolved.len()) - 1 {
            let (lo, hi) = (points[window], points[window + 1]);
            if *n <= hi || window == points.len() - 2 {
                let t = if (hi - lo).abs() < f64::EPSILON {
                    0.0
                } else {
                    ((*n - lo) / (hi - lo)).clamp(0.0, 1.0)
                };
                return Some(blend(resolved[window], resolved[window + 1], t));
            }
        }
        None
    }

    fn bar_fraction(&self, value: &Value, min: &CfValue, max: &CfValue) -> Option<f64> {
        let (stats, Value::Number(n)) = (self.stats.as_ref()?, value) else {
            return None;
        };
        let lo = threshold(min, stats)?;
        let hi = threshold(max, stats)?;
        if (hi - lo).abs() < f64::EPSILON {
            return Some(0.0);
        }
        Some(((*n - lo) / (hi - lo)).clamp(0.0, 1.0))
    }
}

/// Where a `<cfvo>` sits on the region's own scale.
fn threshold(stop: &CfValue, stats: &Stats) -> Option<f64> {
    let low = stats.sorted.first().copied()?;
    let high = stats.sorted.last().copied()?;
    Some(match stop.kind {
        CfValueKind::Min | CfValueKind::AutoMin => low,
        CfValueKind::Max | CfValueKind::AutoMax => high,
        CfValueKind::Percent => {
            let p: f64 = stop.value.parse().ok()?;
            low + (high - low) * p / 100.0
        }
        CfValueKind::Percentile => {
            let p: f64 = stop.value.parse().ok()?;
            percentile(&stats.sorted, p / 100.0)?
        }
        // A formula stop is not resolved: it needs a position, and a scale is a
        // property of the region rather than of any one cell in it.
        CfValueKind::Number | CfValueKind::Formula => stop.value.trim().parse().ok()?,
    })
}

fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let pos = p.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    Some(sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64))
}

fn blend(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let mix = |x: u8, y: u8| (f64::from(x) + (f64::from(y) - f64::from(x)) * t).round() as u8;
    [mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2])]
}

fn truthy(value: Value) -> bool {
    match value {
        Value::Bool(b) => b,
        Value::Number(n) => n != 0.0,
        Value::Text(t) => t.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn compare_cell(
    value: &Value,
    operator: CfOperator,
    first: Option<&Value>,
    second: Option<&Value>,
) -> bool {
    use std::cmp::Ordering::*;
    // A blank cell is not compared at all. Excel leaves empty cells alone even
    // when `0 < 5` would be true of the zero a blank coerces to.
    if matches!(value, Value::Blank) {
        return false;
    }
    let Some(first) = first else { return false };
    let ordering = |a: &Value, b: &Value| compare(a, b).ok();
    match operator {
        CfOperator::Equal => ordering(value, first) == Some(Equal),
        CfOperator::NotEqual => ordering(value, first).is_some_and(|o| o != Equal),
        CfOperator::GreaterThan => ordering(value, first) == Some(Greater),
        CfOperator::LessThan => ordering(value, first) == Some(Less),
        CfOperator::GreaterThanOrEqual => {
            matches!(ordering(value, first), Some(Greater) | Some(Equal))
        }
        CfOperator::LessThanOrEqual => matches!(ordering(value, first), Some(Less) | Some(Equal)),
        CfOperator::Between | CfOperator::NotBetween => {
            let Some(second) = second else { return false };
            let inside = matches!(ordering(value, first), Some(Greater) | Some(Equal))
                && matches!(ordering(value, second), Some(Less) | Some(Equal));
            inside == matches!(operator, CfOperator::Between)
        }
        CfOperator::ContainsText
        | CfOperator::NotContains
        | CfOperator::BeginsWith
        | CfOperator::EndsWith => {
            let (Value::Text(cell), Value::Text(want)) = (value, first) else {
                return false;
            };
            let (cell, want) = (cell.to_lowercase(), want.to_lowercase());
            match operator {
                CfOperator::ContainsText => cell.contains(&want),
                CfOperator::NotContains => !cell.contains(&want),
                CfOperator::BeginsWith => cell.starts_with(&want),
                _ => cell.ends_with(&want),
            }
        }
    }
}

/// Excel's date buckets, against a serial for today.
fn in_period(serial: f64, period: &str, today: f64) -> bool {
    let day = serial.floor();
    let delta = day - today;
    // Excel's week starts on Sunday. `weekday_from_serial` counts from Sunday
    // too, and is wrong in exactly the places Excel's own calendar is.
    let week_start = today - f64::from(ss_model::datetime::weekday_from_serial(today));
    match period {
        "today" => delta == 0.0,
        "yesterday" => delta == -1.0,
        "tomorrow" => delta == 1.0,
        "last7Days" => (-6.0..=0.0).contains(&delta),
        "thisWeek" => (0.0..7.0).contains(&(day - week_start)),
        "lastWeek" => (-7.0..0.0).contains(&(day - week_start)),
        "nextWeek" => (7.0..14.0).contains(&(day - week_start)),
        "thisMonth" => same_month(day, today, 0),
        "lastMonth" => same_month(day, today, -1),
        "nextMonth" => same_month(day, today, 1),
        _ => false,
    }
}

fn same_month(day: f64, today: f64, offset: i32) -> bool {
    let (a, b) = (
        ss_model::datetime::from_serial(day),
        ss_model::datetime::from_serial(today),
    );
    let (Some(a), Some(b)) = (a, b) else {
        return false;
    };
    let months = b.month as i32 - 1 + offset;
    let (year, month) = (b.year + months.div_euclid(12), months.rem_euclid(12) + 1);
    a.year == year && a.month as i32 == month
}

/// Which of a rule's fields hold formula text, in the order the file wrote them.
fn formulas_of(kind: &CfKind) -> Vec<String> {
    match kind {
        CfKind::CellIs { formulas, .. } => formulas.clone(),
        CfKind::Expression { formula } => vec![formula.clone()],
        _ => Vec::new(),
    }
}

/// A rule's own colours, resolved once against the workbook theme.
fn colors_of(kind: &CfKind, book: &Workbook) -> Vec<[u8; 3]> {
    let theme = book.styles.theme();
    match kind {
        CfKind::ColorScale { colors, .. } => {
            colors.iter().filter_map(|c| c.resolve(theme)).collect()
        }
        CfKind::DataBar { color, .. } => color.resolve(theme).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn needs_region(kind: &CfKind) -> bool {
    matches!(
        kind,
        CfKind::ColorScale { .. }
            | CfKind::DataBar { .. }
            | CfKind::Top10 { .. }
            | CfKind::AboveAverage { .. }
            | CfKind::Duplicates { .. }
    )
}

/// A key that says whether two cells hold the same thing, for duplicate rules.
fn key_of(value: &Value) -> Option<String> {
    Some(match value {
        Value::Blank => return None,
        Value::Number(n) => format!("n{n}"),
        // Excel's duplicate detection ignores case, the same as `=`.
        Value::Text(t) => format!("t{}", t.to_lowercase()),
        Value::Bool(b) => format!("b{b}"),
        Value::Error(e) => format!("e{}", e.as_str()),
    })
}

/// Reads every cell of a region once.
///
/// Clamped to the used range, so a rule written over a whole column costs what
/// the column holds rather than a million lookups.
fn region_stats(book: &Workbook, sheet: usize, ranges: &[CellRange]) -> Stats {
    let mut numbers = Vec::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    if let Some(model) = book.sheet(sheet) {
        let used = model.cells.used_range();
        for range in ranges {
            let (top, left, bottom, right) = match used {
                Some((start, end)) => (
                    range.start.row.max(start.row),
                    range.start.col.max(start.col),
                    range.end.row.min(end.row),
                    range.end.col.min(end.col),
                ),
                None => continue,
            };
            for row in top..=bottom.max(top) {
                for col in left..=right.max(left) {
                    let at = CellRef::new(row, col);
                    if !range.contains(at) {
                        continue;
                    }
                    let Some(cell) = model.get(at) else { continue };
                    if let CellValue::Number(n) = cell.value {
                        numbers.push(n);
                    }
                    let value = value_of(cell.value, &book.strings);
                    if let Some(key) = key_of(&value) {
                        *counts.entry(key).or_insert(0) += 1;
                    }
                }
            }
        }
    }
    let mut sorted = numbers.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mean = if numbers.is_empty() {
        0.0
    } else {
        numbers.iter().sum::<f64>() / numbers.len() as f64
    };
    // The population standard deviation, which is what Excel's above-average
    // rule uses — not the sample one its STDEV function defaults to.
    let std_dev = if numbers.is_empty() {
        0.0
    } else {
        (numbers.iter().map(|n| (n - mean).powi(2)).sum::<f64>() / numbers.len() as f64).sqrt()
    };
    Stats {
        sorted,
        mean,
        std_dev,
        counts,
    }
}

/// Moves every *relative* reference in an expression by `(rows, cols)`.
///
/// Anchored references stay put, which is the whole point of the dollar signs a
/// rule's author wrote: `$C2` follows the row and not the column.
fn shift(expr: &Expr, offset: (i64, i64)) -> Expr {
    let (rows, cols) = offset;
    if rows == 0 && cols == 0 {
        return expr.clone();
    }
    match expr {
        Expr::Ref(reference) => Expr::Ref(crate::ast::Reference {
            sheet: reference.sheet.clone(),
            area: shift_area(&reference.area, rows, cols),
        }),
        Expr::Call { name, args } => Expr::Call {
            name: name.clone(),
            args: args.iter().map(|a| shift(a, offset)).collect(),
        },
        Expr::Unary { op, operand } => Expr::Unary {
            op: *op,
            operand: Box::new(shift(operand, offset)),
        },
        Expr::Binary { op, lhs, rhs } => Expr::Binary {
            op: *op,
            lhs: Box::new(shift(lhs, offset)),
            rhs: Box::new(shift(rhs, offset)),
        },
        Expr::Array(grid) => Expr::Array(
            grid.iter()
                .map(|row| row.iter().map(|e| shift(e, offset)).collect())
                .collect(),
        ),
        other => other.clone(),
    }
}

fn shift_area(area: &Area, rows: i64, cols: i64) -> Area {
    match area {
        Area::Cell(a) => Area::Cell(shift_a1(*a, rows, cols)),
        Area::Range { start, end } => Area::Range {
            start: shift_a1(*start, rows, cols),
            end: shift_a1(*end, rows, cols),
        },
        Area::Cols {
            start,
            start_abs,
            end,
            end_abs,
        } => Area::Cols {
            start: shift_index(*start, cols, *start_abs),
            start_abs: *start_abs,
            end: shift_index(*end, cols, *end_abs),
            end_abs: *end_abs,
        },
        Area::Rows {
            start,
            start_abs,
            end,
            end_abs,
        } => Area::Rows {
            start: shift_index(*start, rows, *start_abs),
            start_abs: *start_abs,
            end: shift_index(*end, rows, *end_abs),
            end_abs: *end_abs,
        },
    }
}

fn shift_a1(a: A1, rows: i64, cols: i64) -> A1 {
    A1 {
        col: shift_index(a.col, cols, a.col_abs),
        col_abs: a.col_abs,
        row: shift_index(a.row, rows, a.row_abs),
        row_abs: a.row_abs,
    }
}

fn shift_index(index: u32, by: i64, anchored: bool) -> u32 {
    if anchored {
        return index;
    }
    // A reference pushed off the top or left wraps in Excel; clamping to the
    // edge is the calmer answer and the rule simply stops matching there.
    (i64::from(index) + by).max(0) as u32
}

// ------------------------------------------------------------ data validation

/// Why an entry was refused, and how hard.
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub message: String,
    pub severity: DvSeverity,
}

impl Refusal {
    /// True when the entry must not be stored at all. A warning or an
    /// information rule tells the user and then accepts, which is the whole
    /// difference between the three settings.
    pub fn blocks(&self) -> bool {
        self.severity == DvSeverity::Stop
    }
}

/// Checks a value against whatever validation covers the cell.
///
/// `None` means "allowed", including for a cell with no rule at all. The rule
/// formulas are written against the region's top-left cell, exactly like a
/// conditional format's, so they are shifted the same way.
pub fn validate(book: &Workbook, sheet: usize, at: CellRef, value: &Value) -> Option<Refusal> {
    let rule = book.sheet(sheet)?.validation_at(at)?;
    if rule.kind == DvKind::None {
        return None;
    }
    if matches!(value, Value::Blank) {
        // `allowBlank` is about *clearing* a cell, which is always allowed for
        // a rule that says so. A rule that does not still lets Delete through:
        // refusing to empty a cell would trap the user in a bad value.
        return None;
    }

    let ctx = WorkbookContext::new(book);
    let anchor = rule
        .ranges
        .iter()
        .map(|r| r.start)
        .reduce(|a, b| CellRef::new(a.row.min(b.row), a.col.min(b.col)))?;
    let offset = (
        at.row as i64 - anchor.row as i64,
        at.col as i64 - anchor.col as i64,
    );
    let evaluate = |text: &str| -> Option<Value> {
        let expr = crate::parse(text).ok()?;
        let mut ev = Evaluator::new(&ctx, Position::new(sheet, at));
        Some(ev.eval_scalar(&shift(&expr, offset)))
    };

    let ok = match rule.kind {
        DvKind::None => true,
        DvKind::List => {
            let wanted = match value {
                Value::Text(t) => t.to_lowercase(),
                other => other.to_text().unwrap_or_default().to_lowercase(),
            };
            match rule.inline_choices() {
                Some(choices) => choices.iter().any(|c| c.to_lowercase() == wanted),
                None => {
                    // A list whose source is a range: every cell in it is a
                    // choice.
                    let expr = crate::parse(&rule.formula1).ok();
                    match expr {
                        Some(expr) => {
                            let mut ev = Evaluator::new(&ctx, Position::new(sheet, at));
                            let operand = ev.eval(&expr);
                            ev.spread(&operand).values().any(|v| match v {
                                Value::Text(t) => t.to_lowercase() == wanted,
                                other => {
                                    other.to_text().unwrap_or_default().to_lowercase() == wanted
                                }
                            })
                        }
                        None => true,
                    }
                }
            }
        }
        DvKind::Custom => evaluate(&rule.formula1).map(truthy).unwrap_or(true),
        DvKind::TextLength => {
            let length = match value {
                Value::Text(t) => t.chars().count() as f64,
                other => other.to_text().unwrap_or_default().chars().count() as f64,
            };
            compare_bound(length, rule, &evaluate)
        }
        DvKind::Whole | DvKind::Decimal | DvKind::Date | DvKind::Time => {
            let Ok(number) = value.to_number() else {
                return refuse(rule);
            };
            if rule.kind == DvKind::Whole && number.fract() != 0.0 {
                return refuse(rule);
            }
            compare_bound(number, rule, &evaluate)
        }
    };

    if ok {
        None
    } else {
        refuse(rule)
    }
}

fn refuse(rule: &ss_model::cond::DataValidation) -> Option<Refusal> {
    let message = if rule.error_message.is_empty() {
        "The value you entered is not valid for this cell.".to_string()
    } else if rule.error_title.is_empty() {
        rule.error_message.clone()
    } else {
        format!("{}: {}", rule.error_title, rule.error_message)
    };
    Some(Refusal {
        message,
        severity: rule.severity,
    })
}

fn compare_bound(
    number: f64,
    rule: &ss_model::cond::DataValidation,
    evaluate: &impl Fn(&str) -> Option<Value>,
) -> bool {
    let first = evaluate(&rule.formula1).and_then(|v| v.to_number().ok());
    let second = evaluate(&rule.formula2).and_then(|v| v.to_number().ok());
    let Some(first) = first else {
        // A bound we cannot compute is not a reason to refuse the user's data.
        return true;
    };
    match rule.operator {
        DvOperator::Between => second.is_none_or(|s| number >= first && number <= s),
        DvOperator::NotBetween => second.is_none_or(|s| number < first || number > s),
        DvOperator::Equal => number == first,
        DvOperator::NotEqual => number != first,
        DvOperator::GreaterThan => number > first,
        DvOperator::LessThan => number < first,
        DvOperator::GreaterThanOrEqual => number >= first,
        DvOperator::LessThanOrEqual => number <= first,
    }
}

/// The choices a list validation offers, resolving a range source.
///
/// `None` for a cell with no list on it, which is what tells the grid whether
/// to draw a dropdown arrow at all.
pub fn choices(book: &Workbook, sheet: usize, at: CellRef) -> Option<Vec<String>> {
    let rule = book.sheet(sheet)?.validation_at(at)?;
    if rule.kind != DvKind::List {
        return None;
    }
    if let Some(inline) = rule.inline_choices() {
        return Some(inline);
    }
    let ctx = WorkbookContext::new(book);
    let expr = crate::parse(&rule.formula1).ok()?;
    let mut ev = Evaluator::new(&ctx, Position::new(sheet, at));
    let operand = ev.eval(&expr);
    let listed: Vec<String> = ev
        .spread(&operand)
        .values()
        .filter(|v| !matches!(v, Value::Blank))
        .map(|v| v.to_text().unwrap_or_default())
        .collect();
    Some(listed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_validation_accepts_only_what_it_lists() {
        let mut book = book();
        book.sheets[0].validations.push(ss_model::DataValidation {
            ranges: vec![range("A1", "A9")],
            kind: DvKind::List,
            formula1: r#""North,South""#.to_string(),
            error_message: "Pick a region".to_string(),
            ..Default::default()
        });

        assert!(validate(&book, 0, at("A1"), &Value::text("North")).is_none());
        assert!(
            validate(&book, 0, at("A1"), &Value::text("north")).is_none(),
            "the comparison is case-insensitive, as Excel's is"
        );
        let refused = validate(&book, 0, at("A1"), &Value::text("West")).expect("refused");
        assert!(refused.blocks());
        assert_eq!(refused.message, "Pick a region");
        assert!(
            validate(&book, 0, at("A1"), &Value::Blank).is_none(),
            "emptying a cell is never what a validation is for"
        );
        assert!(
            validate(&book, 0, at("B1"), &Value::text("West")).is_none(),
            "outside the region there is no rule"
        );
    }

    #[test]
    fn a_bounded_rule_reads_the_value_not_the_characters() {
        let mut book = book();
        book.sheets[0].validations.push(ss_model::DataValidation {
            ranges: vec![range("A1", "A9")],
            kind: DvKind::Whole,
            operator: DvOperator::Between,
            formula1: "1".to_string(),
            formula2: "10".to_string(),
            severity: DvSeverity::Warning,
            ..Default::default()
        });

        assert!(validate(&book, 0, at("A1"), &Value::Number(5.0)).is_none());
        assert!(validate(&book, 0, at("A1"), &Value::Number(11.0)).is_some());
        let fractional = validate(&book, 0, at("A1"), &Value::Number(5.5)).expect("refused");
        assert!(
            !fractional.blocks(),
            "a warning tells the user and then accepts"
        );
    }

    #[test]
    fn a_list_can_name_a_range_and_the_choices_come_from_it() {
        let mut book = book();
        for (a1, text) in [("H1", "Red"), ("H2", "Green"), ("H3", "Blue")] {
            let id = book.strings.intern(text);
            book.sheets[0].set(
                at(a1),
                Cell {
                    value: CellValue::Text(id),
                    ..Default::default()
                },
            );
        }
        book.sheets[0].validations.push(ss_model::DataValidation {
            ranges: vec![range("A1", "A9")],
            kind: DvKind::List,
            formula1: "$H$1:$H$3".to_string(),
            ..Default::default()
        });

        assert_eq!(
            choices(&book, 0, at("A2")),
            Some(vec![
                "Red".to_string(),
                "Green".to_string(),
                "Blue".to_string()
            ])
        );
        assert!(validate(&book, 0, at("A2"), &Value::text("Green")).is_none());
        assert!(validate(&book, 0, at("A2"), &Value::text("Puce")).is_some());
        assert_eq!(choices(&book, 0, at("B2")), None, "no rule, no arrow");
    }

    use ss_model::cond::{CfRule, ConditionalFormat};
    use ss_model::style::{Fill, Parts};
    use ss_model::Color;
    use ss_model::{Cell, StyleTable};

    fn at(a1: &str) -> CellRef {
        CellRef::from_a1(a1).expect("valid address")
    }

    fn range(a: &str, b: &str) -> CellRange {
        CellRange::new(at(a), at(b))
    }

    /// A workbook with one dxf: bold, red text on a pink fill.
    fn book() -> Workbook {
        let mut book = Workbook::blank();
        book.styles = StyleTable::from_parts(Parts {
            dxfs: vec![
                Dxf {
                    bold: Some(true),
                    color: Some(Color::rgb(0x9C, 0, 6)),
                    fill: Some(Fill::solid(Color::rgb(0xFF, 0xC7, 0xCE))),
                    ..Default::default()
                },
                Dxf {
                    italic: Some(true),
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        book
    }

    fn put(book: &mut Workbook, a1: &str, n: f64) {
        book.sheets[0].set(
            at(a1),
            Cell {
                value: CellValue::Number(n),
                ..Default::default()
            },
        );
    }

    #[test]
    fn a_rules_formula_is_shifted_from_the_regions_corner_to_each_cell() {
        // The rule is written against B2 and says "my row in column C is 1".
        // Applied as written it would format the whole column by row 2.
        let mut book = book();
        for (row, flag) in [(2, 1.0), (3, 0.0), (4, 1.0)] {
            put(&mut book, &format!("B{row}"), row as f64);
            put(&mut book, &format!("C{row}"), flag);
        }
        book.sheets[0].conditional_formats.push(ConditionalFormat {
            ranges: vec![range("B2", "B4")],
            rules: vec![CfRule {
                kind: CfKind::Expression {
                    formula: "$C2=1".to_string(),
                },
                dxf: Some(0),
                priority: 1,
                stop_if_true: false,
            }],
        });

        let cf = Formatting::prepare(&book, 0);
        let bold = |a1: &str| {
            cf.effect(&book, 0, at(a1))
                .and_then(|e| e.dxf.bold)
                .unwrap_or(false)
        };
        assert!(bold("B2"));
        assert!(!bold("B3"), "row 3's flag is 0");
        assert!(bold("B4"));
    }

    #[test]
    fn several_rules_each_contribute_what_they_set_and_the_first_wins() {
        let mut book = book();
        put(&mut book, "A1", 50.0);
        book.sheets[0].conditional_formats.push(ConditionalFormat {
            ranges: vec![range("A1", "A1")],
            rules: vec![
                CfRule {
                    kind: CfKind::CellIs {
                        operator: CfOperator::GreaterThan,
                        formulas: vec!["10".to_string()],
                    },
                    dxf: Some(1),
                    priority: 5,
                    stop_if_true: false,
                },
                CfRule {
                    kind: CfKind::CellIs {
                        operator: CfOperator::GreaterThan,
                        formulas: vec!["20".to_string()],
                    },
                    dxf: Some(0),
                    priority: 1,
                    stop_if_true: false,
                },
            ],
        });

        let effect = Formatting::prepare(&book, 0)
            .effect(&book, 0, at("A1"))
            .expect("formatted");
        assert_eq!(effect.dxf.bold, Some(true), "from the priority-1 rule");
        assert_eq!(
            effect.dxf.italic,
            Some(true),
            "and the lower-priority rule still contributes what the first did not"
        );
    }

    #[test]
    fn stop_if_true_cuts_off_everything_after_it() {
        let mut book = book();
        put(&mut book, "A1", 50.0);
        book.sheets[0].conditional_formats.push(ConditionalFormat {
            ranges: vec![range("A1", "A1")],
            rules: vec![
                CfRule {
                    kind: CfKind::CellIs {
                        operator: CfOperator::GreaterThan,
                        formulas: vec!["10".to_string()],
                    },
                    dxf: Some(0),
                    priority: 1,
                    stop_if_true: true,
                },
                CfRule {
                    kind: CfKind::CellIs {
                        operator: CfOperator::GreaterThan,
                        formulas: vec!["20".to_string()],
                    },
                    dxf: Some(1),
                    priority: 2,
                    stop_if_true: false,
                },
            ],
        });

        let effect = Formatting::prepare(&book, 0)
            .effect(&book, 0, at("A1"))
            .expect("formatted");
        assert_eq!(effect.dxf.bold, Some(true));
        assert_eq!(effect.dxf.italic, None, "the second rule never ran");
    }

    #[test]
    fn a_blank_cell_is_left_alone_by_a_comparison() {
        // `>-1` would be true of the zero a blank coerces to, and Excel still
        // does not format it.
        let mut book = book();
        book.sheets[0].conditional_formats.push(ConditionalFormat {
            ranges: vec![range("A1", "A5")],
            rules: vec![CfRule {
                kind: CfKind::CellIs {
                    operator: CfOperator::GreaterThan,
                    formulas: vec!["-1".to_string()],
                },
                dxf: Some(0),
                priority: 1,
                stop_if_true: false,
            }],
        });
        let cf = Formatting::prepare(&book, 0);
        assert!(cf.effect(&book, 0, at("A3")).is_none());
    }

    #[test]
    fn a_data_bar_measures_against_the_whole_region() {
        let mut book = book();
        for (row, value) in [(1, 0.0), (2, 50.0), (3, 100.0)] {
            put(&mut book, &format!("A{row}"), value);
        }
        book.sheets[0].conditional_formats.push(ConditionalFormat {
            ranges: vec![range("A1", "A3")],
            rules: vec![CfRule {
                kind: CfKind::DataBar {
                    min: CfValue {
                        kind: CfValueKind::Min,
                        value: String::new(),
                    },
                    max: CfValue {
                        kind: CfValueKind::Max,
                        value: String::new(),
                    },
                    color: Color::rgb(0x63, 0x8E, 0xC6),
                    show_value: true,
                },
                dxf: None,
                priority: 1,
                stop_if_true: false,
            }],
        });

        let cf = Formatting::prepare(&book, 0);
        let fraction = |a1: &str| match cf.effect(&book, 0, at(a1)).and_then(|e| e.overlay) {
            Some(Overlay::Bar { fraction, .. }) => fraction,
            other => panic!("{a1}: {other:?}"),
        };
        assert_eq!(fraction("A1"), 0.0);
        assert_eq!(fraction("A2"), 0.5);
        assert_eq!(fraction("A3"), 1.0);
    }

    #[test]
    fn duplicates_are_found_across_the_region_and_ignore_case() {
        let mut book = book();
        let a = book.strings.intern("North");
        let b = book.strings.intern("north");
        let c = book.strings.intern("South");
        for (a1, id) in [("A1", a), ("A2", b), ("A3", c)] {
            book.sheets[0].set(
                at(a1),
                Cell {
                    value: CellValue::Text(id),
                    ..Default::default()
                },
            );
        }
        book.sheets[0].conditional_formats.push(ConditionalFormat {
            ranges: vec![range("A1", "A3")],
            rules: vec![CfRule {
                kind: CfKind::Duplicates { unique: false },
                dxf: Some(0),
                priority: 1,
                stop_if_true: false,
            }],
        });

        let cf = Formatting::prepare(&book, 0);
        assert!(cf.effect(&book, 0, at("A1")).is_some());
        assert!(cf.effect(&book, 0, at("A2")).is_some());
        assert!(cf.effect(&book, 0, at("A3")).is_none());
    }
}
