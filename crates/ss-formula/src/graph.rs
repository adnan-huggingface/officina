//! The dependency graph and recalculation order.
//!
//! Every formula cell is a node; an edge runs from each cell a formula reads to
//! the formula itself. Recalculation walks that graph in topological order, so a
//! cell is never computed before something it depends on.
//!
//! Ranges are the reason this is not a plain map of cell-to-cell edges. A single
//! `SUM(A:A)` depends on a million cells, and materializing those edges would
//! cost more than the whole workbook. Precedents are therefore stored as *areas*,
//! and the reverse lookup — "which formulas does this cell feed?" — is answered
//! by testing containment against a per-sheet list. That is a linear scan, which
//! is the honest trade: cheap to build, and fast enough while the number of
//! distinct ranges stays far below the number of cells.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use ss_model::cell::{MAX_COLS, MAX_ROWS};
use ss_model::{CellRange, CellRef};

use crate::ast::{Area, Expr, SheetRef};

/// A cell somewhere in the workbook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Node {
    /// Index into `Workbook::sheets`.
    pub sheet: usize,
    pub at: CellRef,
}

impl Node {
    pub const fn new(sheet: usize, at: CellRef) -> Self {
        Node { sheet, at }
    }
}

/// An area a formula reads from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AreaRef {
    pub sheet: usize,
    pub range: CellRange,
}

impl AreaRef {
    fn contains(&self, node: Node) -> bool {
        self.sheet == node.sheet && self.range.contains(node.at)
    }
}

/// What one formula reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Precedents {
    pub areas: Vec<AreaRef>,
    /// Defined names the formula mentions, unresolved.
    ///
    /// Kept separate because a name's target can change without the formula
    /// changing, so the edge has to be re-derived rather than baked in.
    pub names: Vec<String>,
    /// True when the formula calls a function whose value can change without any
    /// input changing — `NOW`, `TODAY`, `RAND`, `OFFSET`, `INDIRECT`.
    pub volatile: bool,
}

/// Functions that must be recalculated on every pass.
///
/// `OFFSET` and `INDIRECT` are here not because they change spontaneously but
/// because their precedents are computed at evaluation time. We cannot know what
/// they read, so we cannot know when to recalculate them — treating them as
/// volatile is the only safe answer.
const VOLATILE: [&str; 8] = [
    "NOW",
    "TODAY",
    "RAND",
    "RANDBETWEEN",
    "RANDARRAY",
    "OFFSET",
    "INDIRECT",
    "INFO",
];

fn is_volatile(name: &str) -> bool {
    // Excel writes future functions with an `_xlfn.` prefix, so strip it before
    // comparing or `_xlfn.RANDARRAY` reads as an ordinary function.
    let bare = name.strip_prefix("_xlfn.").unwrap_or(name);
    VOLATILE.iter().any(|v| bare.eq_ignore_ascii_case(v))
}

/// Collects what a parsed formula depends on.
///
/// `sheet` is the sheet the formula lives on, used for unqualified references.
/// `resolve_sheet` maps a sheet name to its index; a name that does not resolve
/// yields no edge, which is correct — a reference to a deleted sheet reads as
/// `#REF!` rather than as a dependency on nothing.
pub fn precedents_of(
    expr: &Expr,
    sheet: usize,
    resolve_sheet: &impl Fn(&str) -> Option<usize>,
) -> Precedents {
    let mut out = Precedents::default();
    expr.walk(&mut |e| match e {
        Expr::Ref(r) => {
            let range = area_to_range(&r.area);
            for s in sheets_of(&r.sheet, sheet, resolve_sheet) {
                out.areas.push(AreaRef { sheet: s, range });
            }
        }
        Expr::Name { name, .. } => out.names.push(name.clone()),
        Expr::Call { name, .. } if is_volatile(name) => out.volatile = true,
        _ => {}
    });
    out.areas.sort_unstable();
    out.areas.dedup();
    out
}

fn sheets_of(
    sheet_ref: &SheetRef,
    current: usize,
    resolve: &impl Fn(&str) -> Option<usize>,
) -> Vec<usize> {
    match sheet_ref {
        SheetRef::Current => vec![current],
        SheetRef::Named(n) => resolve(n).into_iter().collect(),
        // A 3-D reference covers every sheet between the two, by position.
        SheetRef::Span(a, b) => match (resolve(a), resolve(b)) {
            (Some(x), Some(y)) => (x.min(y)..=x.max(y)).collect(),
            _ => Vec::new(),
        },
    }
}

/// Converts a reference area to a concrete rectangle.
///
/// Whole columns and rows become their full extent. That is what they mean, and
/// clamping them to the used range here would make the graph wrong the moment a
/// cell is added outside it.
fn area_to_range(area: &Area) -> CellRange {
    match area {
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
    }
}

/// The workbook's formula cells and what each reads.
#[derive(Debug, Default)]
pub struct DependencyGraph {
    formulas: BTreeMap<Node, Precedents>,
    /// Defined name -> the cells it resolves to, so name edges can be followed.
    name_targets: HashMap<String, Vec<AreaRef>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.formulas.len()
    }

    pub fn is_empty(&self) -> bool {
        self.formulas.is_empty()
    }

    /// Records that `node` holds a formula reading `precedents`.
    pub fn insert(&mut self, node: Node, precedents: Precedents) {
        self.formulas.insert(node, precedents);
    }

    pub fn remove(&mut self, node: Node) {
        self.formulas.remove(&node);
    }

    pub fn precedents(&self, node: Node) -> Option<&Precedents> {
        self.formulas.get(&node)
    }

    /// Points a defined name at the areas it covers.
    pub fn define_name(&mut self, name: impl Into<String>, targets: Vec<AreaRef>) {
        // Names are matched case-insensitively by Excel; normalizing on the way
        // in keeps every lookup from having to remember that.
        self.name_targets
            .insert(name.into().to_ascii_uppercase(), targets);
    }

    fn areas_for_name(&self, name: &str) -> &[AreaRef] {
        self.name_targets
            .get(&name.to_ascii_uppercase())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Every formula that reads `node`, directly.
    pub fn dependents_of(&self, node: Node) -> Vec<Node> {
        let mut out = Vec::new();
        for (&formula, precedents) in &self.formulas {
            if self.reads(precedents, node) {
                out.push(formula);
            }
        }
        out
    }

    fn reads(&self, precedents: &Precedents, node: Node) -> bool {
        precedents.areas.iter().any(|a| a.contains(node))
            || precedents
                .names
                .iter()
                .any(|n| self.areas_for_name(n).iter().any(|a| a.contains(node)))
    }

    /// All formulas, in an order where every cell comes after everything it reads.
    ///
    /// Cells caught in a cycle are excluded from the order and returned
    /// separately — they cannot be evaluated, and leaving them in would either
    /// loop forever or produce an arbitrary answer.
    pub fn evaluation_order(&self) -> Order {
        self.order_over(self.formulas.keys().copied().collect())
    }

    /// The order needed to bring `changed` and everything downstream up to date.
    ///
    /// This is the incremental path: editing one cell in a large workbook should
    /// cost the size of its dependency cone, not the size of the workbook.
    /// Volatile formulas are always included — by definition we cannot tell
    /// whether their inputs moved.
    pub fn recalculation_order(&self, changed: &[Node]) -> Order {
        let mut affected: BTreeSet<Node> = BTreeSet::new();
        let mut queue: Vec<Node> = Vec::new();

        for (&node, precedents) in &self.formulas {
            if precedents.volatile {
                affected.insert(node);
                queue.push(node);
            }
        }

        // A changed cell may itself hold a formula, and it may be read by others.
        for &c in changed {
            if self.formulas.contains_key(&c) && affected.insert(c) {
                queue.push(c);
            }
            for d in self.dependents_of(c) {
                if affected.insert(d) {
                    queue.push(d);
                }
            }
        }

        while let Some(node) = queue.pop() {
            for d in self.dependents_of(node) {
                if affected.insert(d) {
                    queue.push(d);
                }
            }
        }

        self.order_over(affected)
    }

    /// Kahn's algorithm over `subset`, with edges restricted to it.
    fn order_over(&self, subset: BTreeSet<Node>) -> Order {
        // Edges only within the subset: a precedent outside it is already
        // up to date and imposes no ordering constraint.
        let mut in_degree: BTreeMap<Node, usize> = subset.iter().map(|&n| (n, 0)).collect();
        let mut edges: BTreeMap<Node, Vec<Node>> = BTreeMap::new();

        // A cell that reads itself can never settle. It is skipped when building
        // edges — a self-loop is invisible to Kahn's algorithm, which would
        // otherwise hand it back as perfectly sortable — so it is recorded here.
        let mut self_referential: BTreeSet<Node> = BTreeSet::new();

        for &node in &subset {
            let Some(precedents) = self.formulas.get(&node) else {
                continue;
            };
            if self.reads(precedents, node) {
                self_referential.insert(node);
            }
            // Which other members of the subset does `node` read?
            let mut from: HashSet<Node> = HashSet::new();
            for &other in &subset {
                if other != node && self.reads(precedents, other) {
                    from.insert(other);
                }
            }
            for source in from {
                edges.entry(source).or_default().push(node);
                *in_degree.entry(node).or_insert(0) += 1;
            }
        }

        // BTreeMap iteration makes the ready set ordered, so the output is
        // deterministic. A recalculation order that varied run to run would make
        // every downstream difference impossible to reproduce.
        let mut ready: Vec<Node> = in_degree
            .iter()
            .filter(|(n, &d)| d == 0 && !self_referential.contains(n))
            .map(|(&n, _)| n)
            .collect();
        ready.reverse(); // pop() takes the smallest first

        let mut sorted = Vec::with_capacity(subset.len());
        while let Some(node) = ready.pop() {
            sorted.push(node);
            if let Some(targets) = edges.get(&node) {
                for &t in targets {
                    let d = in_degree.get_mut(&t).expect("target is in the subset");
                    *d -= 1;
                    if *d == 0 && !self_referential.contains(&t) {
                        ready.push(t);
                    }
                }
            }
            ready.sort_unstable_by(|a, b| b.cmp(a));
        }

        // Anything with a remaining in-degree is in, or downstream of, a cycle.
        let cyclic: Vec<Node> = subset.into_iter().filter(|n| !sorted.contains(n)).collect();

        Order { sorted, cyclic }
    }
}

/// The result of a topological sort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Order {
    /// Cells to evaluate, in order.
    pub sorted: Vec<Node>,
    /// Cells in or downstream of a cycle. These get `#CIRCULAR!` rather than a value.
    pub cyclic: Vec<Node>,
}

impl Order {
    pub fn has_cycle(&self) -> bool {
        !self.cyclic.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn no_sheets(_: &str) -> Option<usize> {
        None
    }

    fn deps(formula: &str) -> Precedents {
        let e = parse(formula).expect("parses");
        precedents_of(&e, 0, &no_sheets)
    }

    fn at(a1: &str) -> Node {
        Node::new(0, CellRef::from_a1(a1).expect("valid address"))
    }

    /// Builds a graph from `(cell, formula)` pairs, all on sheet 0.
    fn graph(cells: &[(&str, &str)]) -> DependencyGraph {
        let mut g = DependencyGraph::new();
        for (a1, formula) in cells {
            let e = parse(formula).unwrap_or_else(|err| panic!("{formula:?}: {err}"));
            g.insert(at(a1), precedents_of(&e, 0, &no_sheets));
        }
        g
    }

    fn names(order: &[Node]) -> Vec<String> {
        order.iter().map(|n| n.at.to_a1()).collect()
    }

    #[test]
    fn a_single_cell_reference_is_one_precedent() {
        let p = deps("A1*2");
        assert_eq!(p.areas.len(), 1);
        assert_eq!(p.areas[0].range.start, CellRef::new(0, 0));
        assert_eq!(p.areas[0].range.end, CellRef::new(0, 0));
    }

    #[test]
    fn a_range_is_stored_as_an_area_not_expanded() {
        // Expanding SUM(A:A) into a million edges would cost more than the
        // workbook it came from.
        let p = deps("SUM(A:A)");
        assert_eq!(p.areas.len(), 1);
        assert_eq!(p.areas[0].range.rows(), MAX_ROWS);
        assert_eq!(p.areas[0].range.cols(), 1);
    }

    #[test]
    fn whole_row_references_span_every_column() {
        let p = deps("SUM(3:3)");
        assert_eq!(p.areas[0].range.cols(), MAX_COLS);
        assert_eq!(p.areas[0].range.start.row, 2);
    }

    #[test]
    fn duplicate_references_collapse() {
        let p = deps("A1+A1+A1");
        assert_eq!(p.areas.len(), 1);
    }

    #[test]
    fn volatile_functions_are_flagged() {
        assert!(deps("NOW()").volatile);
        assert!(deps("TODAY()").volatile);
        assert!(deps("RAND()").volatile);
        assert!(deps("A1+RAND()").volatile, "nested inside an expression");
        assert!(deps("OFFSET(A1,1,1)").volatile, "precedents unknowable");
        assert!(deps("INDIRECT(\"A1\")").volatile);
        assert!(!deps("SUM(A1:A9)").volatile);
    }

    #[test]
    fn future_function_prefixes_do_not_hide_volatility() {
        assert!(deps("_xlfn.RANDARRAY(2,2)").volatile);
    }

    #[test]
    fn names_are_recorded_separately_from_areas() {
        let p = deps("TaxRate*A1");
        assert_eq!(p.names, ["TaxRate"]);
        assert_eq!(p.areas.len(), 1);
    }

    #[test]
    fn a_reference_to_an_unknown_sheet_yields_no_edge() {
        // Better than an edge to sheet 0, which would recalculate the wrong cells.
        let p = deps("Missing!A1");
        assert!(p.areas.is_empty());
    }

    #[test]
    fn three_dimensional_references_cover_every_sheet_between() {
        let resolve = |name: &str| match name {
            "S1" => Some(0),
            "S2" => Some(1),
            "S3" => Some(2),
            _ => None,
        };
        let e = parse("SUM(S1:S3!A1)").expect("parses");
        let p = precedents_of(&e, 0, &resolve);
        let sheets: Vec<usize> = p.areas.iter().map(|a| a.sheet).collect();
        assert_eq!(sheets, [0, 1, 2]);
    }

    #[test]
    fn evaluation_order_puts_inputs_first() {
        let g = graph(&[("B1", "A1*2"), ("C1", "B1+1"), ("D1", "C1+B1")]);
        let order = g.evaluation_order();
        assert!(!order.has_cycle());
        assert_eq!(names(&order.sorted), ["B1", "C1", "D1"]);
    }

    #[test]
    fn a_diamond_still_evaluates_each_cell_once() {
        let g = graph(&[("B1", "A1"), ("C1", "A1"), ("D1", "B1+C1")]);
        let order = g.evaluation_order();
        let got = names(&order.sorted);
        assert_eq!(got.len(), 3);
        let pos = |a1: &str| got.iter().position(|g| g == a1).expect("present");
        assert!(pos("D1") > pos("B1"));
        assert!(pos("D1") > pos("C1"));
    }

    #[test]
    fn dependents_are_found_through_ranges() {
        let g = graph(&[("D1", "SUM(A1:B9)")]);
        assert_eq!(names(&g.dependents_of(at("A1"))), ["D1"]);
        assert_eq!(names(&g.dependents_of(at("B9"))), ["D1"], "far corner");
        assert!(g.dependents_of(at("C1")).is_empty(), "outside the range");
    }

    #[test]
    fn a_direct_cycle_is_detected_and_excluded() {
        let g = graph(&[("A1", "B1"), ("B1", "A1")]);
        let order = g.evaluation_order();
        assert!(order.has_cycle());
        assert!(order.sorted.is_empty());
        assert_eq!(names(&order.cyclic), ["A1", "B1"]);
    }

    #[test]
    fn a_self_reference_is_a_cycle() {
        let g = graph(&[("A1", "A1+1")]);
        let order = g.evaluation_order();
        assert!(order.has_cycle(), "a cell reading itself cannot settle");
    }

    #[test]
    fn a_cycle_does_not_swallow_the_healthy_cells() {
        let g = graph(&[("A1", "B1"), ("B1", "A1"), ("D1", "C1*2")]);
        let order = g.evaluation_order();
        assert_eq!(names(&order.sorted), ["D1"]);
        assert_eq!(names(&order.cyclic), ["A1", "B1"]);
    }

    #[test]
    fn cells_downstream_of_a_cycle_are_also_unevaluable() {
        let g = graph(&[("A1", "B1"), ("B1", "A1"), ("C1", "A1+1")]);
        let order = g.evaluation_order();
        assert!(
            order.cyclic.contains(&at("C1")),
            "C1 can never settle either"
        );
        assert!(order.sorted.is_empty());
    }

    #[test]
    fn recalculation_touches_only_the_dependency_cone() {
        let g = graph(&[
            ("B1", "A1*2"),
            ("C1", "B1+1"),
            ("E1", "D1*2"), // an unrelated chain
            ("F1", "E1+1"),
        ]);
        let order = g.recalculation_order(&[at("A1")]);
        assert_eq!(names(&order.sorted), ["B1", "C1"]);
    }

    #[test]
    fn recalculation_from_a_formula_cell_includes_that_cell() {
        let g = graph(&[("B1", "A1*2"), ("C1", "B1+1")]);
        let order = g.recalculation_order(&[at("B1")]);
        assert_eq!(names(&order.sorted), ["B1", "C1"]);
    }

    #[test]
    fn volatile_cells_recalculate_even_when_untouched() {
        let g = graph(&[("B1", "A1*2"), ("Z9", "NOW()")]);
        let order = g.recalculation_order(&[at("A1")]);
        assert!(
            order.sorted.contains(&at("Z9")),
            "a volatile cell has no visible input to change"
        );
    }

    #[test]
    fn editing_nothing_still_recalculates_volatiles_only() {
        let g = graph(&[("B1", "A1*2"), ("Z9", "TODAY()")]);
        let order = g.recalculation_order(&[]);
        assert_eq!(names(&order.sorted), ["Z9"]);
    }

    #[test]
    fn defined_names_create_edges_once_they_resolve() {
        let mut g = graph(&[("C1", "TaxRate*2")]);
        assert!(
            g.dependents_of(at("A1")).is_empty(),
            "unresolved names have no target yet"
        );
        g.define_name(
            "TaxRate",
            vec![AreaRef {
                sheet: 0,
                range: CellRange::new(CellRef::new(0, 0), CellRef::new(0, 0)),
            }],
        );
        assert_eq!(names(&g.dependents_of(at("A1"))), ["C1"]);
    }

    #[test]
    fn names_resolve_case_insensitively() {
        let mut g = graph(&[("C1", "taxrate*2")]);
        g.define_name(
            "TaxRate",
            vec![AreaRef {
                sheet: 0,
                range: CellRange::new(CellRef::new(0, 0), CellRef::new(0, 0)),
            }],
        );
        assert_eq!(names(&g.dependents_of(at("A1"))), ["C1"]);
    }

    #[test]
    fn sheets_are_kept_apart() {
        let mut g = DependencyGraph::new();
        let e = parse("A1*2").expect("parses");
        g.insert(
            Node::new(0, CellRef::new(1, 1)),
            precedents_of(&e, 0, &no_sheets),
        );
        g.insert(
            Node::new(1, CellRef::new(1, 1)),
            precedents_of(&e, 1, &no_sheets),
        );

        let on_sheet_0 = g.dependents_of(Node::new(0, CellRef::new(0, 0)));
        assert_eq!(on_sheet_0, [Node::new(0, CellRef::new(1, 1))]);
    }

    #[test]
    fn the_order_is_deterministic() {
        // A recalculation order that varied between runs would make any
        // downstream difference impossible to reproduce.
        let cells: Vec<(&str, &str)> =
            vec![("B1", "A1"), ("C1", "A1"), ("D1", "A1"), ("E1", "B1+C1+D1")];
        let first = graph(&cells).evaluation_order();
        for _ in 0..8 {
            assert_eq!(graph(&cells).evaluation_order(), first);
        }
    }

    #[test]
    fn a_long_chain_sorts_correctly() {
        // The stress case from the C5 exit criterion, in miniature: a chain
        // written in the reverse of its evaluation order.
        let mut cells: Vec<(String, String)> = Vec::new();
        for row in (1..200).rev() {
            cells.push((format!("A{}", row + 1), format!("A{row}*2")));
        }
        let pairs: Vec<(&str, &str)> = cells
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let order = graph(&pairs).evaluation_order();

        assert!(!order.has_cycle());
        assert_eq!(order.sorted.len(), 199);
        // Every cell must come after the one it reads.
        for (i, node) in order.sorted.iter().enumerate() {
            for (j, other) in order.sorted.iter().enumerate() {
                let precedents = graph(&pairs).precedents(*node).cloned().unwrap();
                if graph(&pairs).reads(&precedents, *other) {
                    assert!(j < i, "{} evaluated before its input", node.at.to_a1());
                }
            }
        }
    }
}
