//! What the dependency graph costs as a workbook gets large.
//!
//! Not a benchmark — a benchmark that fails on a slow machine is a test nobody
//! trusts. This asserts the *shape* of the cost instead: doubling the number of
//! formulas must not quadruple the time. A linear scan through every formula for
//! every lookup does exactly that, and it is what this replaced.

use std::time::Instant;

use ss_formula::graph::{precedents_of, DependencyGraph, Node};
use ss_formula::parser::parse;
use ss_model::CellRef;

fn no_sheets(_: &str) -> Option<usize> {
    None
}

/// A chain of `count` formulas, each reading the one above and a small range.
fn chain(count: u32) -> DependencyGraph {
    let mut graph = DependencyGraph::new();
    for row in 1..count {
        let text = format!("A{row}*2+SUM(B{row}:D{row})");
        let expr = parse(&text).expect("parses");
        graph.insert(
            Node::new(0, CellRef::new(row, 0)),
            precedents_of(&expr, 0, &no_sheets),
        );
    }
    graph
}

fn time_order(count: u32) -> u128 {
    let graph = chain(count);
    let start = Instant::now();
    let order = graph.evaluation_order();
    let elapsed = start.elapsed().as_micros().max(1);
    assert_eq!(order.sorted.len(), count as usize - 1);
    assert!(!order.has_cycle());
    elapsed
}

#[test]
fn ordering_does_not_get_quadratically_slower_as_the_workbook_grows() {
    // Warm the allocator and the parser so the first measurement is not the one
    // that pays for them.
    let _ = time_order(500);

    let small = time_order(2_000);
    let large = time_order(8_000);

    // Four times the formulas. Linear would be four times the work; quadratic
    // would be sixteen. Ten is generous enough to survive a loaded machine and
    // tight enough to fail if the scan ever comes back.
    let ratio = large as f64 / small as f64;
    println!("2000 formulas: {small}us; 8000 formulas: {large}us; ratio {ratio:.1}");
    assert!(
        ratio < 10.0,
        "four times the formulas took {ratio:.1} times as long \
         ({small}us then {large}us) — the reverse lookup is scanning again"
    );
}

#[test]
fn a_whole_column_reference_still_finds_its_dependents() {
    // The case the index deliberately does not cover: `SUM(A:A)` is too broad to
    // bucket, so it lives on the scanned list. It must still be found.
    let mut graph = DependencyGraph::new();
    let expr = parse("SUM(A:A)").expect("parses");
    let total = Node::new(0, CellRef::new(0, 5));
    graph.insert(total, precedents_of(&expr, 0, &no_sheets));

    for row in [0u32, 500, 100_000] {
        let cell = Node::new(0, CellRef::new(row, 0));
        assert_eq!(
            graph.dependents_of(cell),
            vec![total],
            "row {row} feeds the total"
        );
    }
    assert!(
        graph
            .dependents_of(Node::new(0, CellRef::new(7, 1)))
            .is_empty(),
        "column B does not"
    );
}

#[test]
fn a_formula_that_is_replaced_stops_answering_for_its_old_range() {
    // The index is maintained rather than rebuilt, so a stale entry would be
    // invisible until a cell recalculated for no reason — or worse, did not.
    let mut graph = DependencyGraph::new();
    let node = Node::new(0, CellRef::new(0, 5));
    graph.insert(
        node,
        precedents_of(&parse("SUM(A1:A10)").expect("parses"), 0, &no_sheets),
    );
    assert_eq!(
        graph.dependents_of(Node::new(0, CellRef::new(2, 0))),
        vec![node]
    );

    graph.insert(
        node,
        precedents_of(&parse("SUM(C1:C10)").expect("parses"), 0, &no_sheets),
    );
    assert!(
        graph
            .dependents_of(Node::new(0, CellRef::new(2, 0)))
            .is_empty(),
        "column A no longer feeds it"
    );
    assert_eq!(
        graph.dependents_of(Node::new(0, CellRef::new(2, 2))),
        vec![node]
    );

    graph.remove(node);
    assert!(graph
        .dependents_of(Node::new(0, CellRef::new(2, 2)))
        .is_empty());
}
