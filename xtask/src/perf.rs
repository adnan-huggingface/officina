//! What the two applications cost on real files.
//!
//! Not a benchmark suite — a stopwatch. It opens every file in the corpus and
//! prints how long each stage took, so a change that makes opening a workbook
//! twice as slow is visible before a user notices it. Numbers from one machine
//! mean nothing on another; the point is the shape and the comparison against
//! the last run on the same machine.

use std::path::Path;
use std::time::Instant;

/// Anything slower than this is worth a look rather than a shrug.
const NOTABLE: u128 = 250;

pub fn run(corpus: &Path) -> Result<(), String> {
    println!("{:<44} {:>9} {:>9} {:>8}", "file", "read", "layout", "size");
    println!("{}", "-".repeat(74));

    let mut slow = Vec::new();
    for (dir, kind) in [("xlsx", Kind::Workbook), ("docx", Kind::Document)] {
        let Ok(entries) = std::fs::read_dir(corpus.join(dir)) else {
            continue;
        };
        let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                continue;
            }
            if let Some((read, layout)) = time(&path, kind) {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                println!(
                    "{:<44} {:>7}ms {:>7}ms {:>7}k",
                    truncate(&name, 44),
                    read,
                    layout,
                    size / 1024
                );
                if read + layout > NOTABLE {
                    slow.push((name.into_owned(), read + layout));
                }
            }
        }
    }

    if slow.is_empty() {
        println!("\nnothing in the corpus took longer than {NOTABLE}ms.");
    } else {
        println!("\nover {NOTABLE}ms:");
        for (name, total) in slow {
            println!("  {name}  {total}ms");
        }
    }

    // The corpus is small on purpose — it is there to be *correct*, not big. So
    // the engines are measured again against something larger than any of it,
    // built here rather than kept on disk.
    println!();
    println!("synthetic, larger than anything in the corpus:");
    for count in [1_000, 4_000, 16_000] {
        println!(
            "  {count:>6} formulas    graph + order  {:>6}ms",
            workbook_of(count)
        );
    }
    for count in [500, 2_000, 8_000] {
        println!(
            "  {count:>6} paragraphs  layout         {:>6}ms",
            document_of(count)
        );
    }
    Ok(())
}

/// A chain of formulas, indexed and ordered.
fn workbook_of(count: u32) -> u128 {
    use ss_formula::graph::{precedents_of, DependencyGraph, Node};
    use ss_model::CellRef;

    let none = |_: &str| None;
    let mut graph = DependencyGraph::new();
    // Parsing is measured elsewhere; this is about the graph.
    let parsed: Vec<_> = (1..count)
        .map(|row| {
            ss_formula::parser::parse(&format!("A{row}*2+SUM(B{row}:D{row})")).expect("parses")
        })
        .collect();

    let start = Instant::now();
    for (index, expr) in parsed.iter().enumerate() {
        graph.insert(
            Node::new(0, CellRef::new(index as u32 + 1, 0)),
            precedents_of(expr, 0, &none),
        );
    }
    let order = graph.evaluation_order();
    let elapsed = start.elapsed().as_millis();
    assert_eq!(order.sorted.len(), count as usize - 1);
    elapsed
}

/// A document of ordinary paragraphs, laid out to pages.
fn document_of(count: usize) -> u128 {
    use wp_model::doc::{Block, Inline, Paragraph, Run};

    let mut document = wp_model::Document::new();
    document.body = (0..count)
        .map(|index| {
            let mut paragraph = Paragraph::new();
            paragraph.content = vec![Inline::Run(Run::of(&format!(
                "Paragraph {index} of a document long enough that laying it out \
                 has to break lines, fill pages and start new ones."
            )))];
            Block::Paragraph(paragraph)
        })
        .collect();

    let theme = document.theme.clone();
    let marks = wp_layout::NoteMarks::of(&document);
    let fields = wp_layout::FieldValues::default();
    let contents = wp_layout::field::Contents::of(&document);
    let ctx = wp_layout::inline::Context {
        theme: &theme,
        styles: &document.styles,
        notes: &marks,
        note_mark: None,
        contents: &contents,
        table_part: None,
        default_tab: document.settings.default_tab_stop,
        no_leading: document.settings.no_leading,
        fallback_font: "Calibri",
        has_face: |_| false,
        show_revisions: true,
        show_hidden: false,
        fields: &fields,
        band: None,
        wraps: &wp_layout::block::Wraps::default(),
    };
    let start = Instant::now();
    let pages = wp_layout::block::layout(&document, &ctx, &mut wp_layout::shape::Fixed);
    let elapsed = start.elapsed().as_millis();
    assert!(!pages.is_empty());
    elapsed
}

#[derive(Clone, Copy)]
enum Kind {
    Workbook,
    Document,
}

/// (read, layout) in milliseconds, or `None` if the file will not open.
fn time(path: &Path, kind: Kind) -> Option<(u128, u128)> {
    match kind {
        Kind::Workbook => {
            let start = Instant::now();
            let mut doc = ss_xlsx::XlsxDocument::open(path).ok()?;
            let read = start.elapsed().as_millis();
            // The workbook equivalent of layout is the full recalculation:
            // building the graph, ordering it, and evaluating every formula.
            let start = Instant::now();
            let outcome = ss_formula::workbook::recalculate(&mut doc.workbook);
            let layout = start.elapsed().as_millis();
            let _ = outcome;
            Some((read, layout))
        }
        Kind::Document => {
            let start = Instant::now();
            let (document, _) = wp_docx::open(path).ok()?;
            let read = start.elapsed().as_millis();
            let start = Instant::now();
            let theme = document.theme.clone();
            let marks = wp_layout::NoteMarks::of(&document);
            let fields = wp_layout::FieldValues::default();
            let contents = wp_layout::field::Contents::of(&document);
            let ctx = wp_layout::inline::Context {
                theme: &theme,
                styles: &document.styles,
                notes: &marks,
                note_mark: None,
                contents: &contents,
                table_part: None,
                default_tab: document.settings.default_tab_stop,
                no_leading: document.settings.no_leading,
                fallback_font: "Calibri",
                has_face: |_| false,
                show_revisions: true,
                show_hidden: false,
                fields: &fields,
                band: None,
                wraps: &wp_layout::block::Wraps::default(),
            };
            let mut shaper = wp_layout::shape::Fixed;
            let pages = wp_layout::block::layout(&document, &ctx, &mut shaper);
            let layout = start.elapsed().as_millis();
            let _ = pages.len();
            Some((read, layout))
        }
    }
}

fn truncate(text: &str, at: usize) -> String {
    match text.chars().count() > at {
        true => text.chars().take(at - 1).collect::<String>() + "…",
        false => text.to_string(),
    }
}
