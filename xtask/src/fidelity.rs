//! The fidelity harness.
//!
//! Check 1 of the three in `DESIGN.md` §7, and the one that protects users' files:
//!
//! > Open a real document, save it with no edits, and assert the result is
//! > semantically identical to the input.
//!
//! Any difference is a preservation bug. Checks 2 (edit round-trip) and 3 (render
//! snapshot) attach here once there is a model to edit and a layout engine to
//! rasterize.

use std::path::{Path, PathBuf};

use ooxml::compare::{diff, Difference};
use ooxml::Package;

pub struct Report {
    pub passed: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, Vec<Difference>)>,
    pub errored: Vec<(PathBuf, String)>,
}

impl Report {
    pub fn is_green(&self) -> bool {
        self.failed.is_empty() && self.errored.is_empty()
    }

    pub fn total(&self) -> usize {
        self.passed.len() + self.failed.len() + self.errored.len()
    }
}

/// Runs the no-op round-trip check over every package under `corpus_dir`.
pub fn run(corpus_dir: &Path) -> Result<Report, String> {
    let mut report = Report {
        passed: Vec::new(),
        failed: Vec::new(),
        errored: Vec::new(),
    };

    for path in collect_packages(corpus_dir)? {
        match round_trip(&path) {
            Ok(differences) if differences.is_empty() => report.passed.push(path),
            Ok(differences) => report.failed.push((path, differences)),
            Err(e) => report.errored.push((path, e)),
        }
    }

    Ok(report)
}

/// Opens, rewrites, and reopens a package, returning what changed.
fn round_trip(path: &Path) -> Result<Vec<Difference>, String> {
    let before = Package::open(path).map_err(|e| format!("open: {e}"))?;

    let mut rewritten = Vec::new();
    before
        .write(std::io::Cursor::new(&mut rewritten))
        .map_err(|e| format!("write: {e}"))?;

    let after =
        Package::read(std::io::Cursor::new(rewritten)).map_err(|e| format!("reopen: {e}"))?;

    Ok(diff(&before, &after))
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
        println!("  ok    {}", path.display());
    }
    for (path, differences) in &report.failed {
        println!("  FAIL  {}", path.display());
        for d in differences {
            println!("          {d}");
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
        "{} passed, {} failed, {} errored",
        report.passed.len(),
        report.failed.len(),
        report.errored.len()
    );
    report.is_green()
}
