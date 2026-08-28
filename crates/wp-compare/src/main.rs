//! Where Scriva's page differs from Word's, ranked.
//!
//! ADR 0003 decided that a person may point at a layout difference but may not
//! be the instrument that measures it. This is the instrument. It lays a
//! document out with the application's own shaper, asks Word over COM where the
//! same words went, and prints what disagrees — worst first, with one number
//! for the file.
//!
//! No window, no release build, no deployment and no screenshot: the whole run
//! is a layout pass and a file read once Word's answer has been cached.
//!
//! ```text
//! cargo xtask compare <file>       one document, ranked
//! cargo xtask compare              every .docx in corpus/
//! ```
//!
//! **What it cannot see, and what it counts twice.** Only type is compared: a
//! rule, a shading, a border and an image all move without moving the number.
//! And only *flowed* type is gathered from our side — the words inside a
//! pasted metafile, and the words of a shape such as a watermark, are drawn by
//! Scriva from somewhere other than a line, so Word's rendering has them and
//! this does not. They land in the report as words Word laid and we did not,
//! which is honest but is a floor: on a document full of diagrams the
//! unmatched count starts high and cannot be driven to nothing from here.
//! Watch `out by more than` for the work, and the unmatched count for the
//! company it keeps.

mod diff;
mod ours;
mod word;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use diff::{Kind, Report};

/// Further out than this and a word is called misplaced.
///
/// Not a tolerance the code believes in — ADR 0001 holds Scriva to a tenth of
/// a point on a line top — but the point past which a difference is certainly
/// a fault rather than the last digit of two measurements.
const THRESHOLD: f64 = 1.0;

/// How many differences to print before the tail is only a count.
const TOP: usize = 20;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut file: Option<PathBuf> = None;
    let mut refresh = false;
    let mut top = TOP;
    let mut threshold = THRESHOLD;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--refresh" => refresh = true,
            "--top" => top = next(&mut rest, "--top")?,
            "--threshold" => threshold = next(&mut rest, "--threshold")?,
            "--help" | "-h" => {
                println!("{}", usage());
                return Ok(());
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown option `{other}`\n\n{}", usage()))
            }
            other => file = Some(PathBuf::from(other)),
        }
    }

    match file {
        Some(path) => one(&path, refresh, top, threshold),
        None => corpus(refresh, threshold),
    }
}

fn next<T: std::str::FromStr>(
    args: &mut std::slice::Iter<'_, String>,
    what: &str,
) -> Result<T, String> {
    args.next()
        .ok_or_else(|| format!("{what} wants a value"))?
        .parse()
        .map_err(|_| format!("{what} wants a number"))
}

fn usage() -> String {
    "\
cargo xtask compare [file] [options]

  no file        every .docx in corpus/docx
  --refresh      ask Word again rather than using the cached answer
  --top N        how many differences to print (default 20)
  --threshold P  points past which a word counts as misplaced (default 1.0)"
        .into()
}

fn one(path: &Path, refresh: bool, top: usize, threshold: f64) -> Result<(), String> {
    let report = measure(path, refresh, threshold)?;
    println!("{}", path.display());
    println!(
        "  {} pages, Word {} — {} words matched, {} unmatched",
        report.pages_ours, report.pages_word, report.matched, report.unmatched
    );
    if report.pages_ours != report.pages_word {
        println!(
            "  the two do not agree on how many pages this is; everything below \
             is measured against the pages that do line up."
        );
    }
    println!(
        "  worst {:.2}pt   middle shift dx={:+.2} dy={:+.2}",
        report.worst, report.middle.0, report.middle.1
    );
    println!("  out by more than {threshold:.2}pt: {}", report.over);
    println!("  ── {} ──", report.scalar());

    // The two are printed apart because they are read for different reasons.
    // What is merely out of place is the work: it is ranked, and the top of it
    // is where the next fix is. What is unmatched is mostly the floor this
    // cannot see past — a document's diagrams — and it is a count with a
    // sample, so that it stops burying the ranking under itself.
    let (moved, absent): (Vec<_>, Vec<_>) = report
        .differences
        .iter()
        .partition(|found| matches!(found.kind, Kind::Moved { .. }));

    if !moved.is_empty() {
        println!();
        for found in moved.iter().take(top) {
            println!("  {}", line(found));
        }
        let rest = moved.len().saturating_sub(top);
        if rest > 0 {
            println!("  … and {rest} more out of place");
        }
    }

    if !absent.is_empty() {
        let missing = absent.iter().filter(|f| f.kind == Kind::Missing).count();
        println!();
        println!(
            "  {missing} words Word laid and we did not, {} the other way:",
            absent.len() - missing
        );
        let sample: Vec<String> = absent
            .iter()
            .take(8)
            .map(|f| format!("p{} {:?}", f.page, f.text))
            .collect();
        println!("  {}", sample.join("  "));
    }
    Ok(())
}

fn line(found: &diff::Difference) -> String {
    let what = match found.kind {
        Kind::Moved { dx, dy } => format!("dx={dx:+6.2} dy={dy:+6.2}"),
        Kind::Missing => "not laid at all ".into(),
        Kind::Extra => "laid, Word has none".into(),
    };
    let text: String = found.text.chars().take(28).collect();
    // A word only Word laid has no band: its rendering has forgotten which
    // flow drew what, and inventing one would be the report making it up.
    let band = found.band.map(|b| b.to_string()).unwrap_or_default();
    format!("page {:>3} {band:<6} {what}  {text:?}", found.page)
}

fn corpus(refresh: bool, threshold: f64) -> Result<(), String> {
    let root = repo_root().join("corpus");
    let mut paths: Vec<PathBuf> = Vec::new();
    for kind in ["docx", "doc"] {
        let dir = root.join(kind);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        paths.extend(
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(kind))),
        );
    }
    if paths.is_empty() {
        return Err(format!("no documents under {}", root.display()));
    }
    paths.sort();

    // Out of place and unplaceable are separate columns on purpose: the first
    // is work and the second is mostly what this cannot see, and one number
    // holding both tells you neither.
    println!(
        "{:<40} {:>7} {:>9} {:>9} {:>6}",
        "file", "out", "unplaced", "worst", "pages"
    );
    println!("{}", "-".repeat(75));
    let mut total = 0usize;
    let mut unplaced = 0usize;
    let mut failed = 0usize;
    for path in &paths {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        match measure(path, refresh, threshold) {
            Ok(report) => {
                total += report.over;
                unplaced += report.unmatched;
                println!(
                    "{:<40} {:>7} {:>9} {:>7.2}pt {:>6}",
                    truncate(&name, 40),
                    report.over,
                    report.unmatched,
                    report.worst,
                    report.pages_ours
                );
            }
            Err(why) => {
                failed += 1;
                println!(
                    "{:<40} {}",
                    truncate(&name, 40),
                    why.lines().next().unwrap_or("")
                );
            }
        }
    }
    println!("{}", "-".repeat(75));
    println!("{:<40} {:>7} {:>9}", "", total, unplaced);
    if failed > 0 {
        println!("{failed} of {} could not be measured", paths.len());
    }
    Ok(())
}

fn measure(path: &Path, refresh: bool, threshold: f64) -> Result<Report, String> {
    let theirs = word::read(path, refresh)?;
    let ours = ours::read(path)?;
    Ok(diff::compare(&ours, &theirs, threshold))
}

fn truncate(text: &str, at: usize) -> String {
    match text.chars().count() > at {
        true => text.chars().take(at - 1).collect::<String>() + "…",
        false => text.to_string(),
    }
}

/// The workspace root: this crate lives two directories below it.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("the crate's manifest dir always has a grandparent")
        .to_path_buf()
}

pub fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("target"))
}
