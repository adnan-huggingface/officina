//! Checks the formula engine against Excel's own answers.
//!
//! Every formula cell in an xlsx carries the value Excel last computed for it.
//! That makes the corpus a conformance suite nobody had to write: open a
//! workbook, recalculate it from the formula text alone, and compare against
//! what Excel put there. A disagreement is a bug in us, not in the file.
//!
//! This is a stronger check than the hand-written suite in `ss-formula`, because
//! the expectations were produced by Excel rather than recalled by a person.

use std::path::{Path, PathBuf};

use ss_formula::workbook::{recalculate, value_of};
use ss_formula::Value;
use ss_model::{CellRef, Workbook};
use ss_xlsx::XlsxDocument;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/xlsx")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("../../corpus/xlsx"))
}

/// Every formula cell, with the value the file already had in it.
fn cached_values(book: &Workbook) -> Vec<(usize, CellRef, Value)> {
    let mut out = Vec::new();
    for (index, sheet) in book.sheets.iter().enumerate() {
        for (at, cell) in sheet.cells.iter() {
            if cell.formula.is_some() {
                out.push((index, at, value_of(cell.value, &book.strings)));
            }
        }
    }
    out
}

/// Excel writes the value it computed; we compute in f64 from the text. The two
/// agree to within the last bit or so, not exactly.
fn agrees(excel: &Value, ours: &Value) -> bool {
    match (excel, ours) {
        (Value::Number(a), Value::Number(b)) => {
            let scale = a.abs().max(b.abs()).max(1.0);
            (a - b).abs() <= scale * 1e-10
        }
        (Value::Text(a), Value::Text(b)) => a == b,
        (a, b) => a == b,
    }
}

struct Report {
    checked: usize,
    skipped: usize,
    disagreements: Vec<String>,
    /// Functions the formula mentioned that we have not written yet.
    missing: Vec<String>,
}

/// The functions in a formula that the library does not have.
///
/// This is what separates "not built yet" from "built wrong". A `#NAME?` is
/// expected while batches 2 and 3 are outstanding — but only when the formula
/// actually names a function we are missing. A `#NAME?` from a formula whose
/// every function *is* implemented is a real bug, and stays a failure.
fn unimplemented_functions(text: &str) -> Vec<String> {
    let Ok(expr) = ss_formula::parse(text) else {
        return Vec::new();
    };
    let mut missing = Vec::new();
    expr.walk(&mut |e| {
        if let ss_formula::Expr::Call { name, .. } = e {
            if ss_formula::functions::lookup(name).is_none() {
                missing.push(name.to_ascii_uppercase());
            }
        }
    });
    missing
}

fn check_workbook(path: &Path) -> Report {
    let doc = XlsxDocument::open(path).expect("corpus workbook opens");
    let expected = cached_values(&doc.workbook);

    let mut book = doc.workbook;
    let outcome = recalculate(&mut book);

    // A formula we could not parse keeps its cached value, so comparing it would
    // only ever confirm that we left it alone.
    let unparsed: std::collections::BTreeSet<(usize, CellRef)> =
        outcome.unparsed.iter().map(|n| (n.sheet, n.at)).collect();

    let mut report = Report {
        checked: 0,
        skipped: unparsed.len(),
        disagreements: Vec::new(),
        missing: Vec::new(),
    };

    for (sheet, at, excel) in expected {
        if unparsed.contains(&(sheet, at)) {
            continue;
        }
        let ours = book
            .sheet(sheet)
            .and_then(|s| s.get(at))
            .map(|c| value_of(c.value, &book.strings))
            .unwrap_or(Value::Blank);

        // A formula Excel had never computed has no cached value to compare to.
        if matches!(excel, Value::Blank) {
            report.skipped += 1;
            continue;
        }
        if agrees(&excel, &ours) {
            report.checked += 1;
            continue;
        }

        let text = book
            .sheet(sheet)
            .and_then(|s| s.formula_at(at))
            .map(|f| f.text.clone())
            .unwrap_or_default();
        let missing = unimplemented_functions(&text);
        if matches!(ours, Value::Error(ss_model::CellError::Name)) && !missing.is_empty() {
            report.missing.extend(missing);
            report.skipped += 1;
            continue;
        }

        report.checked += 1;
        let name = book.sheet(sheet).map(|s| s.name.as_str()).unwrap_or("?");
        report.disagreements.push(format!(
            "  {name}!{at}  ={text}\n    Excel {excel:?}\n    ours  {ours:?}"
        ));
    }
    report
}

#[test]
fn the_corpus_recalculates_to_the_values_excel_stored() {
    let dir = corpus();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("corpus at {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "xlsx"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no corpus workbooks under {} — a vacuous pass is worse than a failure",
        dir.display()
    );

    let mut checked = 0;
    let mut skipped = 0;
    let mut failures = Vec::new();
    let mut missing = std::collections::BTreeSet::new();
    for path in &files {
        let report = check_workbook(path);
        checked += report.checked;
        skipped += report.skipped;
        missing.extend(report.missing);
        if !report.disagreements.is_empty() {
            failures.push(format!(
                "{}:\n{}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                report.disagreements.join("\n")
            ));
        }
    }

    println!(
        "{checked} formula cells compared against Excel across {} workbooks, {skipped} skipped",
        files.len()
    );
    if !missing.is_empty() {
        // Printed rather than asserted: this list should shrink as C7 and C11
        // land, and a test that had to be edited every time would just get
        // edited without being read.
        println!(
            "  not implemented yet: {}",
            missing.into_iter().collect::<Vec<_>>().join(", ")
        );
    }
    assert!(
        checked > 0,
        "the corpus produced no comparable formula cells at all"
    );
    assert!(
        failures.is_empty(),
        "formulas disagree with Excel's own answers:\n{}",
        failures.join("\n")
    );
}
