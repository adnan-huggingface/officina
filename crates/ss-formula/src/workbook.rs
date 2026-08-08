//! Evaluating against a real [`Workbook`].
//!
//! [`Context`] is deliberately narrow so the engine can be tested against a
//! handful of hard-coded cells. This is the adapter that points it at an actual
//! document, and the recalculation loop that drives it in dependency order.

use std::collections::BTreeMap;

use ss_model::{Cell, CellError, CellRange, CellRef, CellValue, StringTable, Workbook};

use crate::ast::Expr;
use crate::eval::{Context, Evaluator, Position};
use crate::graph::{precedents_of, AreaRef, DependencyGraph, Node};
use crate::value::{Array, Operand, RefSet, Value};
use crate::{parse, Area, Reference, SheetRef};

/// A read-only view of a workbook, shaped for the evaluator.
pub struct WorkbookContext<'a> {
    book: &'a Workbook,
}

impl<'a> WorkbookContext<'a> {
    pub fn new(book: &'a Workbook) -> Self {
        WorkbookContext { book }
    }
}

/// Lifts a stored cell value into an evaluation value, resolving interned text.
pub fn value_of(v: CellValue, strings: &StringTable) -> Value {
    match v {
        CellValue::Blank => Value::Blank,
        CellValue::Number(n) => Value::Number(n),
        CellValue::Bool(b) => Value::Bool(b),
        CellValue::Error(e) => Value::Error(e),
        CellValue::Text(id) => Value::Text(strings.resolve(id).to_string()),
    }
}

/// Stores an evaluation result back into a cell, interning any text.
pub fn store_value(v: &Value, strings: &mut StringTable) -> CellValue {
    match v {
        Value::Blank => CellValue::Blank,
        Value::Number(n) => CellValue::Number(*n),
        Value::Bool(b) => CellValue::Bool(*b),
        Value::Error(e) => CellValue::Error(*e),
        Value::Text(s) => CellValue::Text(strings.intern(s)),
    }
}

impl Context for WorkbookContext<'_> {
    fn sheet_index(&self, name: &str) -> Option<usize> {
        self.book.sheet_by_name(name).map(|(i, _)| i)
    }

    fn cell(&self, sheet: usize, at: CellRef) -> Value {
        match self.book.sheet(sheet).and_then(|s| s.get(at)) {
            Some(cell) => value_of(cell.value, &self.book.strings),
            None => Value::Blank,
        }
    }

    fn used_bounds(&self, sheet: usize) -> Option<CellRange> {
        let (start, end) = self.book.sheet(sheet)?.cells.used_range()?;
        Some(CellRange::new(start, end))
    }

    /// Resolves a defined name to what it refers to.
    ///
    /// Only names that expand to a reference or a constant are resolved. A name
    /// whose target is a *formula* — the `OFFSET`-based dynamic range trick — is
    /// left unresolved and reads as `#NAME?`, because evaluating it here would
    /// re-enter the evaluator with a fresh depth counter and a name defined in
    /// terms of itself would recurse until the stack ran out. Handling those
    /// safely needs the name expansion to go through the same depth-guarded path
    /// as everything else, which nothing here has a way to do yet.
    fn resolve_name(&self, sheet: usize, name: &str) -> Option<Operand> {
        let target = self.book.resolve_name(name, Some(sheet))?;
        let text = target
            .refers_to
            .strip_prefix('=')
            .unwrap_or(&target.refers_to);
        let expr = parse(text).ok()?;
        self.literal_operand(&expr, sheet)
    }
}

impl WorkbookContext<'_> {
    /// Converts an expression to an operand *without* evaluating anything.
    fn literal_operand(&self, expr: &Expr, scope: usize) -> Option<Operand> {
        match expr {
            Expr::Number(n) => Some(Operand::number(*n)),
            Expr::Text(s) => Some(Operand::text(s.clone())),
            Expr::Bool(b) => Some(Operand::boolean(*b)),
            Expr::Error(e) => Some(Operand::error(*e)),
            Expr::Ref(r) => self.reference_operand(r, scope),
            // A union of references, which is how a multi-area name is written.
            Expr::Binary {
                op: crate::BinaryOp::Union,
                lhs,
                rhs,
            } => {
                let (a, b) = (
                    self.literal_operand(lhs, scope)?,
                    self.literal_operand(rhs, scope)?,
                );
                match (a, b) {
                    (Operand::Ref(x), Operand::Ref(y)) => {
                        let mut areas = x.areas;
                        areas.extend(y.areas);
                        Some(Operand::Ref(RefSet { areas }))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn reference_operand(&self, r: &Reference, scope: usize) -> Option<Operand> {
        use ss_model::cell::{MAX_COLS, MAX_ROWS};
        let sheets: Vec<usize> = match &r.sheet {
            SheetRef::Current => vec![scope],
            SheetRef::Named(n) => vec![self.sheet_index(n)?],
            SheetRef::Span(a, b) => {
                let (x, y) = (self.sheet_index(a)?, self.sheet_index(b)?);
                (x.min(y)..=x.max(y)).collect()
            }
        };
        let range = match &r.area {
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
        };
        Some(Operand::Ref(RefSet {
            areas: sheets
                .into_iter()
                .map(|sheet| AreaRef { sheet, range })
                .collect(),
        }))
    }
}

/// What one recalculation pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recalculation {
    /// Dynamic-array formulas whose result spilled into neighbouring cells.
    pub spilled: usize,
    /// Anchors where a spill had nowhere to go.
    pub blocked: Vec<Node>,
    /// Formula cells that were evaluated and written back.
    pub evaluated: usize,
    /// Cells left holding `#CIRCULAR!` because they sit in a dependency cycle.
    pub circular: Vec<Node>,
    /// Formulas whose text we could not parse. Their stored value is left
    /// exactly as the file had it, which is the only safe answer: Excel's own
    /// cached result is better than one we know we cannot compute.
    pub unparsed: Vec<Node>,
}

/// Recalculates every formula in the workbook, in dependency order.
///
/// The order comes from the graph, so a cell is never computed before something
/// it reads. Cells in a cycle are not evaluated at all — with iterative
/// calculation off, a cycle has no answer, and picking one arbitrarily would be
/// worse than saying so.
pub fn recalculate(book: &mut Workbook) -> Recalculation {
    let mut parsed: BTreeMap<Node, Expr> = BTreeMap::new();
    let mut report = Recalculation::default();
    let mut graph = DependencyGraph::new();

    // Sheet names have to be resolvable while the graph is built, and the
    // borrow checker will not let that happen with the book borrowed mutably.
    let names: Vec<String> = book.sheets.iter().map(|s| s.name.clone()).collect();
    let resolve_sheet = |name: &str| names.iter().position(|n| n.eq_ignore_ascii_case(name));

    // Array formulas write to cells that hold no formula of their own, so the
    // range has to be carried alongside the expression.
    let mut spills: BTreeMap<Node, CellRange> = BTreeMap::new();

    for (index, sheet) in book.sheets.iter().enumerate() {
        for (at, cell) in sheet.cells.iter() {
            let Some(id) = cell.formula else { continue };
            let Some(formula) = sheet.formula(id) else {
                continue;
            };
            let node = Node::new(index, at);
            if let ss_model::FormulaKind::Array { range } = formula.kind {
                spills.insert(node, range);
            }
            match parse(&formula.text) {
                Ok(expr) => {
                    graph.insert(node, precedents_of(&expr, index, &resolve_sheet));
                    parsed.insert(node, expr);
                }
                // Not a failure: we keep the text and the cached value, and the
                // document still round-trips. See `error.rs`.
                Err(_) => report.unparsed.push(node),
            }
        }
    }

    let order = graph.evaluation_order();
    for node in order.sorted {
        let Some(expr) = parsed.get(&node) else {
            continue;
        };
        // Scoped so the immutable borrow ends before the write. Later cells must
        // see this one's new value, so the two cannot be batched.
        let declared = spills
            .get(&node)
            .copied()
            .filter(|r| r.rows() > 1 || r.cols() > 1);
        // A dynamic-array function decides its own extent, so the block has to
        // be produced before the range is known.
        let dynamic = is_dynamic(expr);
        let mut block = None;
        let value = {
            let ctx = WorkbookContext::new(book);
            let mut ev = Evaluator::new(&ctx, Position::new(node.sheet, node.at));
            let out = ev.eval(expr);
            if declared.is_some() || dynamic {
                block = Some(ev.spread(&out));
            }
            ev.collapse(out)
        };

        match (block, declared) {
            (Some(array), Some(range)) => write_spill(book, node.sheet, range, &array),
            (Some(array), None) if dynamic => {
                match spill_range(book, node, &array) {
                    Some(range) => {
                        clear_previous_spill(book, node);
                        write_spill(book, node.sheet, range, &array);
                        if let Some(sheet) = book.sheet_mut(node.sheet) {
                            sheet.spills.insert(node.at, range);
                        }
                        report.spilled += 1;
                    }
                    // Something is in the way. Excel reports it on the anchor
                    // and leaves the obstruction alone, which is the only
                    // answer that does not destroy the user's data.
                    None => {
                        clear_previous_spill(book, node);
                        write_back(book, node, &Value::Error(CellError::Spill));
                        report.blocked.push(node);
                    }
                }
            }
            _ => {
                clear_previous_spill(book, node);
                write_back(book, node, &value);
            }
        }
        report.evaluated += 1;
    }

    for node in order.cyclic {
        write_back(book, node, &Value::Error(CellError::Circular));
        report.circular.push(node);
    }
    report
}

/// Writes an array formula's result across the range it was entered over.
///
/// Only the anchor cell holds a formula, so the rest of the block is written
/// directly. A result smaller than the range is stretched if it is a single row
/// or column and padded with `#N/A` otherwise — Excel's answer, and the reason a
/// CSE formula entered over too many cells shows a fringe of `#N/A` rather than
/// repeating its last value.
fn write_spill(book: &mut Workbook, sheet: usize, range: CellRange, array: &crate::Array) {
    for (dr, row) in (range.start.row..=range.end.row).enumerate() {
        for (dc, col) in (range.start.col..=range.end.col).enumerate() {
            let r = if array.rows() == 1 { 0 } else { dr };
            let c = if array.cols() == 1 { 0 } else { dc };
            let value = array
                .get(r, c)
                .cloned()
                .unwrap_or(Value::Error(CellError::NotAvailable));
            write_back(book, Node::new(sheet, CellRef::new(row, col)), &value);
        }
    }
}

fn write_back(book: &mut Workbook, node: Node, value: &Value) {
    let stored = store_value(value, &mut book.strings);
    let Some(sheet) = book.sheets.get_mut(node.sheet) else {
        return;
    };
    // Keep the style and the formula handle; only the computed value changes.
    let mut cell = sheet.get(node.at).copied().unwrap_or(Cell::default());
    cell.value = stored;
    sheet.set(node.at, cell);
}

/// Evaluates a reference written the way a chart writes one, e.g.
/// `Sheet1!$B$2:$B$13`, and hands back the values in order.
///
/// This is what makes a chart redraw when a cell under it changes: the file's
/// cached numbers are what the producing application last computed, and going
/// back to the cells is the only way to be current. `None` when the reference
/// names something we cannot resolve — a closed workbook, most often — and the
/// caller falls back to the cache.
pub fn range_values(book: &Workbook, formula: &str) -> Option<Vec<Value>> {
    let text = formula.strip_prefix('=').unwrap_or(formula);
    let expr = crate::parse(text).ok()?;
    let ctx = WorkbookContext::new(book);
    let mut ev = Evaluator::new(&ctx, Position::new(0, CellRef::new(0, 0)));
    let operand = ev.eval(&expr);
    if let Operand::Value(Value::Error(_)) = operand {
        return None;
    }
    Some(ev.spread(&operand).values().cloned().collect())
}

/// True when the formula's outermost call is one that spills.
///
/// The outermost only: `SUM(UNIQUE(A1:A9))` collapses its argument and produces
/// one number, so it neither spills nor should.
fn is_dynamic(expr: &Expr) -> bool {
    match expr {
        Expr::Call { name, .. } => crate::functions::spills(name),
        _ => false,
    }
}

/// The rectangle a result would occupy, or `None` if something is in the way.
///
/// Occupied means "holds a value or a formula of its own". A cell that only
/// carries formatting is not an obstruction — shading the area a report spills
/// into is a perfectly ordinary thing to do — and neither is a cell this same
/// formula spilled into last time.
fn spill_range(book: &Workbook, node: Node, array: &Array) -> Option<CellRange> {
    let end = CellRef::new(
        node.at.row.checked_add(array.rows().max(1) as u32 - 1)?,
        node.at.col.checked_add(array.cols().max(1) as u32 - 1)?,
    );
    if !end.is_valid() {
        return None;
    }
    let range = CellRange::new(node.at, end);
    let sheet = book.sheet(node.sheet)?;
    let previous = sheet.spills.get(&node.at).copied();
    for row in range.start.row..=range.end.row {
        for col in range.start.col..=range.end.col {
            let at = CellRef::new(row, col);
            if at == node.at {
                continue;
            }
            if previous.is_some_and(|old| old.contains(at)) {
                continue;
            }
            let occupied = sheet
                .get(at)
                .is_some_and(|cell| !cell.value.is_blank() || cell.formula.is_some());
            if occupied {
                return None;
            }
        }
    }
    Some(range)
}

/// Empties whatever this formula spilled into last time.
///
/// Only the cells it owns, and only those without a formula of their own: a
/// user who typed into a spilled cell has already broken the spill, and the
/// next recalculation reports `#SPILL!` rather than overwriting them.
fn clear_previous_spill(book: &mut Workbook, node: Node) {
    let Some(sheet) = book.sheet_mut(node.sheet) else {
        return;
    };
    let Some(range) = sheet.spills.remove(&node.at) else {
        return;
    };
    for row in range.start.row..=range.end.row {
        for col in range.start.col..=range.end.col {
            let at = CellRef::new(row, col);
            if at == node.at {
                continue;
            }
            let clearable = sheet.get(at).is_some_and(|cell| cell.formula.is_none());
            if clearable {
                sheet.set(
                    at,
                    Cell {
                        value: CellValue::Blank,
                        style: sheet.get(at).map(|c| c.style).unwrap_or_default(),
                        formula: None,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::{Formula, Sheet, StyleId};

    /// Builds a sheet of formulas, one per cell in column A unless the address
    /// says otherwise.
    fn book_with(cells: &[(&str, &str)]) -> Workbook {
        let mut book = Workbook::blank();
        for (at, text) in cells {
            let at = CellRef::from_a1(at).expect("test address");
            let sheet = book.sheet_mut(0).expect("sheet 0");
            if let Some(formula) = text.strip_prefix('=') {
                let id = sheet.push_formula(Formula::normal(formula));
                sheet.set(
                    at,
                    Cell {
                        value: CellValue::Blank,
                        style: StyleId::DEFAULT,
                        formula: Some(id),
                    },
                );
            } else if let Ok(n) = text.parse::<f64>() {
                sheet.set(
                    at,
                    Cell {
                        value: CellValue::Number(n),
                        ..Cell::default()
                    },
                );
            } else {
                let id = book.strings.intern(text);
                let sheet = book.sheet_mut(0).expect("sheet 0");
                sheet.set(
                    at,
                    Cell {
                        value: CellValue::Text(id),
                        ..Cell::default()
                    },
                );
            }
        }
        book
    }

    fn value_at(book: &Workbook, at: &str) -> Value {
        let at = CellRef::from_a1(at).expect("test address");
        let cell = book.sheet(0).and_then(|s| s.get(at)).copied();
        value_of(cell.map(|c| c.value).unwrap_or_default(), &book.strings)
    }

    #[test]
    fn recalculation_follows_dependency_order_not_cell_order() {
        // C1 is written before its inputs and must still come out right, which
        // is the whole point of sorting the graph rather than sweeping the grid.
        let mut book = book_with(&[("C1", "=A1+B1"), ("B1", "=A1*2"), ("A1", "5")]);
        let report = recalculate(&mut book);
        assert_eq!(report.evaluated, 2);
        assert_eq!(value_at(&book, "B1"), Value::Number(10.0));
        assert_eq!(value_at(&book, "C1"), Value::Number(15.0));
    }

    #[test]
    fn a_long_chain_settles_in_one_pass() {
        // Each cell reads the one below it, written in the order that would make
        // a naive top-to-bottom sweep need 200 passes.
        let mut cells: Vec<(String, String)> = Vec::new();
        for row in 1..200 {
            cells.push((format!("A{row}"), format!("=A{}+1", row + 1)));
        }
        cells.push(("A200".to_string(), "0".to_string()));
        let refs: Vec<(&str, &str)> = cells
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();

        let mut book = book_with(&refs);
        let report = recalculate(&mut book);
        assert_eq!(report.evaluated, 199);
        assert_eq!(value_at(&book, "A1"), Value::Number(199.0));
        assert_eq!(value_at(&book, "A100"), Value::Number(100.0));
        assert!(report.circular.is_empty());
    }

    #[test]
    fn a_cycle_is_reported_rather_than_iterated() {
        let mut book = book_with(&[("A1", "=B1+1"), ("B1", "=A1+1"), ("C1", "=A1")]);
        let report = recalculate(&mut book);
        assert_eq!(report.evaluated, 0);
        // C1 is not itself in the cycle, but it reads a cell that is, so there
        // is no order in which it can be computed either. Marking it circular
        // says so; computing it from a value we know is wrong would not.
        assert_eq!(report.circular.len(), 3);
        for at in ["A1", "B1", "C1"] {
            assert_eq!(
                value_at(&book, at),
                Value::Error(CellError::Circular),
                "{at}"
            );
        }
    }

    #[test]
    fn a_self_reference_is_a_cycle() {
        let mut book = book_with(&[("A1", "=A1+1")]);
        let report = recalculate(&mut book);
        assert_eq!(report.circular.len(), 1);
        assert_eq!(value_at(&book, "A1"), Value::Error(CellError::Circular));
    }

    #[test]
    fn text_results_are_interned_back_into_the_workbook() {
        let mut book = book_with(&[("A1", "hello"), ("B1", "=UPPER(A1)&\"!\"")]);
        recalculate(&mut book);
        assert_eq!(value_at(&book, "B1"), Value::text("HELLO!"));
    }

    #[test]
    fn a_formula_we_cannot_parse_keeps_its_cached_value() {
        // Files contain constructs we do not handle. Overwriting Excel's own
        // answer with an error would make the document worse, not better.
        let mut book = Workbook::blank();
        let sheet = book.sheet_mut(0).expect("sheet 0");
        let id = sheet.push_formula(Formula::normal("SOMETHING[#Totals]"));
        sheet.set(
            CellRef::new(0, 0),
            Cell {
                value: CellValue::Number(42.0),
                style: StyleId::DEFAULT,
                formula: Some(id),
            },
        );
        let report = recalculate(&mut book);
        assert_eq!(report.unparsed.len(), 1);
        assert_eq!(value_at(&book, "A1"), Value::Number(42.0));
    }

    #[test]
    fn defined_names_resolve_through_the_workbook() {
        let mut book = book_with(&[("A1", "1"), ("A2", "2"), ("B1", "=SUM(Totals)")]);
        book.defined_names.push(ss_model::DefinedName {
            name: "Totals".to_string(),
            refers_to: "Sheet1!$A$1:$A$2".to_string(),
            scope: None,
        });
        recalculate(&mut book);
        assert_eq!(value_at(&book, "B1"), Value::Number(3.0));
    }

    #[test]
    fn a_stress_workbook_recalculates_correctly() {
        // A grid where every cell sums the row above it, so the last row is a
        // known function of the first and any ordering mistake shows up.
        const ROWS: u32 = 60;
        const COLS: u32 = 20;
        let mut book = Workbook::blank();
        {
            let sheet = book.sheet_mut(0).expect("sheet 0");
            for col in 0..COLS {
                sheet.set(
                    CellRef::new(0, col),
                    Cell {
                        value: CellValue::Number(1.0),
                        ..Cell::default()
                    },
                );
            }
            for row in 1..ROWS {
                for col in 0..COLS {
                    let left = if col == 0 { COLS - 1 } else { col - 1 };
                    let text = format!(
                        "{}{}+{}{}",
                        ss_model::column_name(col),
                        row,
                        ss_model::column_name(left),
                        row
                    );
                    let id = sheet.push_formula(Formula::normal(&text));
                    sheet.set(
                        CellRef::new(row, col),
                        Cell {
                            value: CellValue::Blank,
                            style: StyleId::DEFAULT,
                            formula: Some(id),
                        },
                    );
                }
            }
        }

        let report = recalculate(&mut book);
        assert_eq!(report.evaluated as u32, (ROWS - 1) * COLS);
        assert!(report.circular.is_empty(), "the grid has no cycles");

        // Verify against an independent computation of the same recurrence.
        let mut expected = vec![1.0f64; COLS as usize];
        for _ in 1..ROWS {
            let previous = expected.clone();
            let mut next = vec![0.0; COLS as usize];
            for col in 0..COLS as usize {
                let left = if col == 0 { COLS as usize - 1 } else { col - 1 };
                next[col] = previous[col] + previous[left];
            }
            expected = next;
        }
        for col in 0..COLS {
            let at = CellRef::new(ROWS - 1, col);
            let got = value_of(
                book.sheet(0).and_then(|s| s.get(at)).expect("cell").value,
                &book.strings,
            );
            assert_eq!(
                got,
                Value::Number(expected[col as usize]),
                "column {col} of the last row"
            );
        }
    }

    #[test]
    fn a_column_of_identical_formulas_intersects_per_row() {
        // The same text in three cells, three different answers. This is what
        // implicit intersection is *for*, and it only shows up once formulas
        // are evaluated at their own positions rather than at A1.
        let mut book = book_with(&[
            ("A1", "10"),
            ("A2", "20"),
            ("A3", "30"),
            ("B1", "=A1:A3"),
            ("B2", "=A1:A3"),
            ("B3", "=A1:A3"),
            ("B9", "=A1:A3"),
        ]);
        recalculate(&mut book);
        assert_eq!(value_at(&book, "B1"), Value::Number(10.0));
        assert_eq!(value_at(&book, "B2"), Value::Number(20.0));
        assert_eq!(value_at(&book, "B3"), Value::Number(30.0));
        // Row 9 is not one the range can offer.
        assert_eq!(value_at(&book, "B9"), Value::Error(CellError::Value));
    }

    #[test]
    fn an_array_formula_writes_across_the_range_it_covers() {
        // Only the anchor holds the formula; the rest of the block has to be
        // written from the same result, or a CSE formula would leave every cell
        // but the first showing whatever the file happened to cache.
        let mut book = book_with(&[("A1", "1"), ("A2", "2"), ("A3", "3")]);
        let range = CellRange::new(CellRef::new(0, 1), CellRef::new(3, 1));
        let sheet = book.sheet_mut(0).expect("sheet 0");
        let id = sheet.push_formula(Formula {
            text: "A1:A3*2".to_string(),
            kind: ss_model::FormulaKind::Array { range },
        });
        sheet.set(
            CellRef::new(0, 1),
            Cell {
                value: CellValue::Blank,
                style: StyleId::DEFAULT,
                formula: Some(id),
            },
        );
        recalculate(&mut book);
        assert_eq!(value_at(&book, "B1"), Value::Number(2.0));
        assert_eq!(value_at(&book, "B2"), Value::Number(4.0));
        assert_eq!(value_at(&book, "B3"), Value::Number(6.0));
        // Entered over one cell too many, the fringe is `#N/A` rather than a
        // repeat of the last value.
        assert_eq!(value_at(&book, "B4"), Value::Error(CellError::NotAvailable));
    }

    #[test]
    fn sheets_do_not_leak_into_each_other() {
        let mut book = Workbook::blank();
        book.sheets.push(Sheet::new("Second"));
        book.sheet_mut(1).expect("sheet 1").set(
            CellRef::new(0, 0),
            Cell {
                value: CellValue::Number(7.0),
                ..Cell::default()
            },
        );
        let sheet = book.sheet_mut(0).expect("sheet 0");
        let id = sheet.push_formula(Formula::normal("Second!A1*2"));
        sheet.set(
            CellRef::new(0, 0),
            Cell {
                value: CellValue::Blank,
                style: StyleId::DEFAULT,
                formula: Some(id),
            },
        );
        recalculate(&mut book);
        assert_eq!(value_at(&book, "A1"), Value::Number(14.0));
    }
}
