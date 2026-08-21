//! The fidelity harness.
//!
//! Two of the three checks in `DESIGN.md` §7, and the ones that protect users'
//! files:
//!
//! > 1. Open a real document, save it with no edits, and assert the result is
//! >    semantically identical to the input.
//! > 2. Open, make a scripted edit, save, reopen, and assert the model matches
//! >    expectation *and* every untouched part is still byte-identical.
//!
//! Any difference is a preservation bug. Check 3 (render snapshot) attaches here
//! once there is a layout engine to rasterize.
//!
//! Check 1 runs at the package level for every format, and *through the model*
//! for spreadsheets. The distinction matters: before C10 a save re-emitted the
//! bytes it read, so check 1 could only prove the container was intact. Now the
//! worksheet, string, and style parts are rebuilt on the way out, and check 1 is
//! a statement about the serializer.

use std::path::{Path, PathBuf};

use ooxml::compare::{diff, Difference};
use ooxml::{Package, PartName};
use ss_model::{Cell, CellRef, CellValue, SheetKind};
use ss_xlsx::XlsxDocument;

pub struct Report {
    pub passed: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, Vec<Difference>)>,
    pub errored: Vec<(PathBuf, String)>,
    /// Check 2, which only spreadsheets can take yet.
    pub edited: Vec<PathBuf>,
    pub edit_failed: Vec<(PathBuf, Vec<String>)>,
}

impl Report {
    pub fn is_green(&self) -> bool {
        self.failed.is_empty() && self.errored.is_empty() && self.edit_failed.is_empty()
    }

    pub fn total(&self) -> usize {
        self.passed.len() + self.failed.len() + self.errored.len()
    }
}

/// Runs both checks over every package under `corpus_dir`.
pub fn run(corpus_dir: &Path) -> Result<Report, String> {
    let mut report = Report {
        passed: Vec::new(),
        failed: Vec::new(),
        errored: Vec::new(),
        edited: Vec::new(),
        edit_failed: Vec::new(),
    };

    for path in collect_packages(corpus_dir)? {
        match round_trip(&path) {
            Ok(differences) if differences.is_empty() => report.passed.push(path.clone()),
            Ok(differences) => {
                report.failed.push((path.clone(), differences));
                continue;
            }
            Err(e) => {
                report.errored.push((path, e));
                continue;
            }
        }

        let result = if is_spreadsheet(&path) {
            edit_round_trip(&path)
        } else {
            document_edit_round_trip(&path)
        };
        match result {
            Ok(problems) if problems.is_empty() => report.edited.push(path),
            Ok(problems) => report.edit_failed.push((path, problems)),
            Err(e) => report.edit_failed.push((path, vec![e])),
        }
    }

    Ok(report)
}

/// An unstyled cell holding `value`.
fn cell(value: CellValue) -> Cell {
    Cell {
        value,
        style: ss_model::StyleId::DEFAULT,
        formula: None,
    }
}

fn is_spreadsheet(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("xlsx") | Some("xlsm") | Some("xltx")
    )
}

/// Check 1: open, save with no edits, reopen, and compare.
///
/// Spreadsheets are checked twice. Once as a save actually behaves, copying
/// through every cell nobody edited — and then again with the writer told to
/// regenerate all of them from the model. The second pass is the one with teeth:
/// the first can only fail if copying bytes went wrong, while the second fails
/// the moment our idea of a worksheet diverges from Excel's anywhere in the
/// file, whether or not an edit ever lands there.
fn round_trip(path: &Path) -> Result<Vec<Difference>, String> {
    let before = Package::open(path).map_err(|e| format!("open: {e}"))?;

    if !is_spreadsheet(path) {
        let mut buf = Vec::new();
        before
            .write(std::io::Cursor::new(&mut buf))
            .map_err(|e| format!("write: {e}"))?;
        let after = Package::read(std::io::Cursor::new(buf)).map_err(|e| format!("reopen: {e}"))?;
        return Ok(diff(&before, &after));
    }

    let mut differences = saved(path, &before, false)?;
    for mut d in saved(path, &before, true)? {
        // Same finding from a different pass; say which one saw it.
        if let Difference::XmlChanged { detail, .. } = &mut d {
            *detail = format!("{detail} (writing every cell from the model)");
        }
        if !differences.contains(&d) {
            differences.push(d);
        }
    }
    Ok(differences)
}

fn saved(path: &Path, before: &Package, regenerate: bool) -> Result<Vec<Difference>, String> {
    let mut doc = XlsxDocument::open(path).map_err(|e| format!("read workbook: {e}"))?;
    if regenerate {
        doc.flush_regenerating()
            .map_err(|e| format!("regenerate: {e}"))?;
    }
    let mut buf = Vec::new();
    doc.write_to(std::io::Cursor::new(&mut buf))
        .map_err(|e| format!("write: {e}"))?;
    let after = Package::read(std::io::Cursor::new(buf)).map_err(|e| format!("reopen: {e}"))?;
    Ok(diff(before, &after))
}

/// Check 2: open, edit, save, reopen — and account for every byte that moved.
///
/// The edit is placed past the used range on purpose. A change that overwrote
/// existing content would prove the writer can replace a cell, but not the
/// harder and more important thing: that the cells *around* it came back
/// untouched, cached values, unknown attributes, and all.
/// Check 2 for a word processing document.
///
/// Open, change the text of one paragraph, save, reopen, and ask two questions:
/// did the edit come back, and did anything else move. The second is the one
/// with teeth — a writer that reprints `document.xml` passes the first and
/// fails the second, and reprinting is exactly what would quietly drop the
/// content controls, the rsids and the equations.
fn document_edit_round_trip(path: &Path) -> Result<Vec<String>, String> {
    use wp_model::doc::{Inline, Run};

    let original = Package::open(path).map_err(|e| format!("open: {e}"))?;
    let mut package = Package::open(path).map_err(|e| format!("open: {e}"))?;
    let mut document = wp_docx::read(&package).map_err(|e| format!("read document: {e}"))?;

    const MARKER: &str = "scriva fidelity harness";
    // The first paragraph *anywhere* — one of the corpus documents is nothing
    // but a content control, and its only paragraph is inside it.
    {
        let mut paragraphs = document.paragraphs_mut();
        let Some(paragraph) = paragraphs.first_mut() else {
            return Ok(vec!["no paragraph to edit".to_owned()]);
        };
        paragraph.content = vec![Inline::Run(Run::of(MARKER))];
    }

    let expected: Vec<String> = document
        .paragraphs()
        .iter()
        .map(|paragraph| paragraph.text())
        .collect();

    wp_docx::flush(&mut document, &mut package).map_err(|e| format!("write: {e}"))?;
    let mut buf = Vec::new();
    package
        .write(std::io::Cursor::new(&mut buf))
        .map_err(|e| format!("write: {e}"))?;

    let saved = Package::read(std::io::Cursor::new(buf)).map_err(|e| format!("reopen: {e}"))?;
    let reopened = wp_docx::read(&saved).map_err(|e| format!("reread document: {e}"))?;

    let mut problems = Vec::new();
    let after: Vec<String> = reopened
        .paragraphs()
        .iter()
        .map(|paragraph| paragraph.text())
        .collect();
    if after != expected {
        problems.push(format!(
            "the text changed on the way through: {} paragraphs before, {} after",
            expected.len(),
            after.len()
        ));
    }
    if !reopened.text().contains(MARKER) {
        problems.push("the edit did not come back".to_owned());
    }

    // Only the document part may have been rewritten.
    let document_part = wp_docx::DocumentParts::locate_in(&original)
        .map(|parts| parts.document)
        .map_err(|e| format!("locate: {e}"))?;
    for (name, before, after) in changed_parts(&original, &saved) {
        if name == document_part {
            continue;
        }
        problems.push(format!(
            "{name} was rewritten though nothing in it was edited ({before} -> {after} bytes)"
        ));
    }

    Ok(problems)
}

fn edit_round_trip(path: &Path) -> Result<Vec<String>, String> {
    let original = Package::open(path).map_err(|e| format!("open: {e}"))?;
    let mut doc = XlsxDocument::open(path).map_err(|e| format!("read workbook: {e}"))?;

    let Some(index) = doc
        .workbook
        .sheets
        .iter()
        .position(|s| matches!(s.kind, SheetKind::Worksheet))
    else {
        return Ok(Vec::new()); // nothing but chart sheets: no grid to edit
    };

    let free = {
        let sheet = &doc.workbook.sheets[index];
        sheet
            .cells
            .used_range()
            .map(|(_, max)| max.row + 2)
            .unwrap_or(0)
    };
    let number_at = CellRef::new(free, 0);
    let text_at = CellRef::new(free, 1);
    let text = "calx fidelity harness";
    let interned = doc.workbook.strings.intern(text);

    {
        let sheet = &mut doc.workbook.sheets[index];
        sheet.set(number_at, cell(CellValue::Number(1234.5)));
        sheet.set(text_at, cell(CellValue::Text(interned)));
    }

    let expected = snapshot(&doc);

    let mut buf = Vec::new();
    doc.write_to(std::io::Cursor::new(&mut buf))
        .map_err(|e| format!("write: {e}"))?;

    let reopened = XlsxDocument::read(std::io::Cursor::new(buf.clone()))
        .map_err(|e| format!("reopen workbook: {e}"))?;
    let saved = Package::read(std::io::Cursor::new(buf)).map_err(|e| format!("reopen: {e}"))?;

    let mut problems = Vec::new();

    // The edit came back.
    let sheet = &reopened.workbook.sheets[index];
    match sheet.get(number_at).map(|c| c.value) {
        Some(CellValue::Number(1234.5)) => {}
        other => problems.push(format!("{} reads back as {other:?}", number_at.to_a1())),
    }
    match sheet.get(text_at).map(|c| c.value) {
        Some(CellValue::Text(id)) if reopened.workbook.strings.resolve(id) == text => {}
        other => problems.push(format!("{} reads back as {other:?}", text_at.to_a1())),
    }

    // ...and nothing else moved.
    if let Some(detail) = snapshot(&reopened).diff(&expected) {
        problems.push(format!("the model changed on the way through: {detail}"));
    }

    let touched = edited_parts(&original, &saved);
    for (name, before, after) in changed_parts(&original, &saved) {
        if touched.contains(&name) {
            continue;
        }
        problems.push(format!(
            "{name} was rewritten though nothing in it was edited ({before} -> {after} bytes)"
        ));
    }

    Ok(problems)
}

/// The parts a cell edit is allowed to change.
///
/// Worksheets, because that is where the cells are, and the shared string table,
/// because new text has to go somewhere. Everything else in the package —
/// styles, theme, the drawing, the pivot cache, `docProps` — must come back with
/// the bytes it went in with.
fn edited_parts(original: &Package, saved: &Package) -> Vec<PartName> {
    let _ = saved;
    original
        .parts()
        .filter(|p| {
            let n = p.name.as_str();
            n.contains("/worksheets/") || n.ends_with("/sharedStrings.xml")
        })
        .map(|p| p.name.clone())
        .collect()
}

fn changed_parts(before: &Package, after: &Package) -> Vec<(PartName, usize, usize)> {
    let mut out = Vec::new();
    for part in before.parts() {
        match after.part(&part.name) {
            Some(other) if other.data() == part.data() => {}
            Some(other) => out.push((part.name.clone(), part.len(), other.len())),
            None => out.push((part.name.clone(), part.len(), 0)),
        }
    }
    out
}

/// Every cell of every sheet, flattened to text.
///
/// Comparing whole workbooks needs an equality the model does not define and
/// should not: a `StrId` is meaningful only inside its own table, and a
/// `FormulaId` only inside its own sheet. Both are resolved here instead.
struct Snapshot {
    sheets: Vec<Vec<String>>,
}

impl Snapshot {
    /// The first place two snapshots disagree, or `None`.
    fn diff(&self, other: &Snapshot) -> Option<String> {
        if self.sheets.len() != other.sheets.len() {
            return Some(format!(
                "{} sheets became {}",
                other.sheets.len(),
                self.sheets.len()
            ));
        }
        for (i, (a, b)) in self.sheets.iter().zip(&other.sheets).enumerate() {
            if a.len() != b.len() {
                return Some(format!(
                    "sheet {i} had {} cells and now has {}",
                    b.len(),
                    a.len()
                ));
            }
            if let Some((x, y)) = a.iter().zip(b).find(|(x, y)| x != y) {
                return Some(format!("sheet {i}: expected `{y}`, got `{x}`"));
            }
        }
        None
    }
}

fn snapshot(doc: &XlsxDocument) -> Snapshot {
    let wb = &doc.workbook;
    Snapshot {
        sheets: wb
            .sheets
            .iter()
            .map(|sheet| {
                sheet
                    .cells
                    .iter()
                    .map(|(at, cell)| {
                        let value = match cell.value {
                            CellValue::Blank => String::new(),
                            CellValue::Number(n) => format!("n:{n}"),
                            CellValue::Text(id) => format!("t:{}", wb.strings.resolve(id)),
                            CellValue::Bool(b) => format!("b:{b}"),
                            CellValue::Error(e) => format!("e:{}", e.as_str()),
                        };
                        let formula = match cell.formula.and_then(|id| sheet.formula(id)) {
                            Some(f) => format!(" f:{}/{:?}", f.text, f.kind),
                            None => String::new(),
                        };
                        format!("{} s{} {value}{formula}", at.to_a1(), cell.style.0)
                    })
                    .collect()
            })
            .collect(),
    }
}

/// Every OOXML package under `dir`, recursively, in a stable order.
fn collect_packages(dir: &Path) -> Result<Vec<PathBuf>, String> {
    const EXTENSIONS: [&str; 6] = ["docx", "docm", "dotx", "xlsx", "xlsm", "xltx"];

    if !dir.exists() {
        return Err(format!("corpus directory {} does not exist", dir.display()));
    }

    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current)
            .map_err(|e| format!("reading {}: {e}", current.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("reading {}: {e}", current.display()))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let matches = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .is_some_and(|e| EXTENSIONS.contains(&e.as_str()));
            if matches {
                out.push(path);
            }
        }
    }

    out.sort();
    Ok(out)
}

/// Prints a report and returns whether it was green.
pub fn print(report: &Report) -> bool {
    for path in &report.passed {
        let edited = report.edited.contains(path);
        println!(
            "  ok    {}{}",
            path.display(),
            if edited { " +edit" } else { "" }
        );
    }
    for (path, differences) in &report.failed {
        println!("  FAIL  {}", path.display());
        for d in differences {
            println!("          {d}");
        }
    }
    for (path, problems) in &report.edit_failed {
        println!("  EDIT  {}", path.display());
        for p in problems {
            println!("          {p}");
        }
    }
    for (path, e) in &report.errored {
        println!("  ERROR {}  {e}", path.display());
    }

    println!();
    if report.total() == 0 {
        // Zero documents passing is not the same as a passing harness, and must
        // never be reported as success.
        println!("no documents in the corpus — nothing was verified");
        return false;
    }
    println!(
        "check 1: {} passed, {} failed, {} errored",
        report.passed.len(),
        report.failed.len(),
        report.errored.len()
    );
    println!(
        "check 2: {} passed, {} failed",
        report.edited.len(),
        report.edit_failed.len()
    );
    report.is_green()
}

/// Writes the report as a document, rather than as a line on a terminal that
/// scrolls away.
///
/// The point is that the claim in `FORMATS.md` — "an untouched save is
/// byte-identical" — is not something to be taken on trust. This is the evidence,
/// regenerated by `cargo xtask fidelity --report`, listing every file the claim
/// was checked against and every part that was compared.
pub fn markdown(report: &Report, corpus: &Path) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    let _ = writeln!(out, "# Fidelity report\n");
    let _ = writeln!(
        out,
        "Generated by `cargo xtask fidelity --report` over `{}/`. Do not edit by \
         hand — regenerate it.\n",
        // The corpus directory's *name*, not its path: a committed report
        // should not say which machine produced it.
        corpus
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "corpus".to_string())
    );

    let _ = writeln!(
        out,
        "Two checks run against every file. **Check 1** opens the file and saves \
         it with no edits: every part of the package must come back \
         byte-identical. **Check 2** opens it, changes one thing, and saves: \
         every part except the one holding the change must be byte-identical, \
         and the change must read back.\n"
    );
    let _ = writeln!(
        out,
        "A reader-and-reprinter passes neither. That is the whole reason for the \
         splice writer, and this is how it is known to be working rather than \
         believed to be.\n"
    );

    let _ = writeln!(
        out,
        "| | files | result |\n|---|---:|---|\n\
         | Check 1 — save untouched | {} | {} |\n\
         | Check 2 — save after an edit | {} | {} |\n",
        report.total(),
        outcome(report.failed.len() + report.errored.len()),
        report.edited.len() + report.edit_failed.len(),
        outcome(report.edit_failed.len()),
    );

    let _ = writeln!(out, "## Every file checked\n");
    let _ = writeln!(out, "| file | check 1 | check 2 |");
    let _ = writeln!(out, "|---|---|---|");
    let mut rows: Vec<(String, &'static str, String)> = Vec::new();
    for path in &report.passed {
        let second = match report.edited.contains(path) {
            true => "pass".to_string(),
            false => "—".to_string(),
        };
        rows.push((name(path), "pass", second));
    }
    for (path, _) in &report.failed {
        rows.push((name(path), "**FAIL**", "—".to_string()));
    }
    for (path, problems) in &report.edit_failed {
        rows.push((
            name(path),
            "pass",
            format!("**FAIL**: {}", problems.join("; ")),
        ));
    }
    for (path, error) in &report.errored {
        rows.push((name(path), "**ERROR**", error.clone()));
    }
    rows.sort();
    for (file, first, second) in rows {
        let _ = writeln!(out, "| `{file}` | {first} | {second} |");
    }

    let _ = writeln!(
        out,
        "\n## What the corpus is\n\n\
         Files produced by Word and Excel themselves, not by this project. A \
         corpus this project generated would only prove that it agrees with \
         itself.\n\n\
         There is a second corpus that is not listed here because it is not in \
         the repository: the 41 `.dotx` templates Microsoft Office installs, and \
         the two legacy `.DOC` files beside them. Every reader and writer fault \
         found in the document phases came from those rather than from these — \
         which is the argument for testing against files nobody on the project \
         chose.\n"
    );

    if !report.is_green() {
        let _ = writeln!(out, "## Failures in detail\n");
        for (path, differences) in &report.failed {
            let _ = writeln!(out, "### `{}`\n", name(path));
            for difference in differences {
                let _ = writeln!(out, "- {difference}");
            }
            let _ = writeln!(out);
        }
    }
    out
}

fn outcome(failures: usize) -> String {
    match failures {
        0 => "all pass".to_string(),
        n => format!("**{n} failing**"),
    }
}

fn name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
