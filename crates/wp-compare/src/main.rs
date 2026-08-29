//! Where Scriva's page differs from Word's, ranked.
//!
//! ADR 0003 decided that a person may point at a layout difference but may not
//! be the instrument that measures it. This is the instrument. It lays a
//! document out with the application's own shaper, asks Word for its own
//! rendering of the same file, and prints what disagrees — worst first, with
//! one number for the document.
//!
//! No window, no release build, no deployment and no screenshot: the whole run
//! is a layout pass and a file read, once Word's answer has been cached.
//!
//! ```text
//! cargo xtask compare <file>          one document, ranked
//! cargo xtask compare <file> --page 5 one page of it, in full
//! cargo xtask compare <file> --words  both readings, uncompared
//! cargo xtask compare                 the whole corpus, as a table
//! cargo xtask compare --check         the corpus, against LAYOUT.md
//! cargo xtask compare --record        rewrite LAYOUT.md from what is true now
//! ```
//!
//! **`--check` is the point of the rest.** A ranked list tells you where to
//! work; only a committed record tells you when work already done has come
//! undone. `LAYOUT.md` holds what every document in the corpus measured, and
//! `--check` fails on any that got worse — which is what makes a layout
//! regression arithmetic rather than something somebody has to notice, and is
//! why it runs inside `cargo xtask check` rather than beside it. Word is not
//! needed for that: its readings of the corpus are committed, and
//! `tests/without_word.rs` runs the whole check with nothing on its PATH to
//! prove it rather than to assert it. It fails in four directions, three of
//! which are quiet ones: a document that got worse — in its words or in its
//! furniture — a document nobody recorded, a document that can no longer be
//! measured at all, and a document the record still holds that the corpus no
//! longer has, a file renamed or deleted otherwise stopping being checked
//! without a word while its row sits there looking like coverage.
//!
//! **What it cannot see.** A chart's labels are drawn from the plot rather than
//! from a line and are not gathered, so a document full of charts keeps a floor
//! of words only one side laid. Three further things are deliberately left out,
//! each for a measured reason rather than a preference — a leadered tab's dots,
//! which neither renderer chose the number of; a shape's own words, which Word
//! draws into a PDF as outlines and not as text at all; and the inside of a
//! picture, which reaches a rendering as the several hundred strokes that draw
//! it and reaches us as one box, so that the most the two can honestly say to
//! each other is that Word drew into it. A whole document can be set apart the
//! same way, and one is: see [`NOT_COMPARED`].
//!
//! What it *can* see, since [`marks`], is the page's furniture: a rule, a
//! shading, a border and a picture's box are compared the same way the words
//! are and counted in their own column. Before that they were the largest thing
//! the instrument was blind to — a border could move an inch and no number
//! moved with it, which is the kind of blindness that reads as a clean bill.

mod diff;
mod marks;
mod ours;
mod word;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use diff::{Kind, Report};

/// Everything one rendering of a document put on paper.
///
/// The words and the furniture arrive together and are compared apart: they are
/// matched by different means — a word by what it says, a rule by where it is —
/// and they are counted apart in the record for the same reason. A document
/// whose type is right to a tenth of a point and whose borders are an inch out
/// is not the same document as one with the fault the other way about, and one
/// number holding both says neither.
pub struct Reading {
    pub words: Vec<diff::Word>,
    pub marks: Vec<marks::Mark>,
}

/// What one document came to, both ways.
pub struct Laid {
    words: Report,
    marks: marks::Split,
}

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
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let mut file: Option<PathBuf> = None;
    let mut refresh = false;
    let mut record = false;
    let mut words = false;
    let mut lines = false;
    let mut check = false;
    let mut top = TOP;
    let mut threshold = THRESHOLD;
    let mut only: Option<u32> = None;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--refresh" => refresh = true,
            "--record" => record = true,
            "--words" => words = true,
            "--lines" => lines = true,
            "--check" => check = true,
            "--top" => top = next(&mut rest, "--top")?,
            "--page" => only = Some(next(&mut rest, "--page")?),
            "--threshold" => threshold = next(&mut rest, "--threshold")?,
            "--help" | "-h" => {
                println!("{}", usage());
                return Ok(ExitCode::SUCCESS);
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown option `{other}`\n\n{}", usage()))
            }
            other => file = Some(PathBuf::from(other)),
        }
    }

    if let Some(path) = file {
        if record || check {
            return Err("--record and --check are about the whole corpus, not one file".into());
        }
        match (words, lines) {
            (true, _) => listing(&path, refresh, only)?,
            (_, true) => grouping(&path, refresh, only)?,
            _ => one(&path, refresh, top, threshold, only)?,
        }
        return Ok(ExitCode::SUCCESS);
    }

    if check && threshold != THRESHOLD {
        // The record counts words past one point. Checking it against a count
        // of words past some other number compares two different questions and
        // would pass or fail for that reason alone.
        return Err(format!(
            "--check holds the corpus to {REPORT}, which counts words past \
             {THRESHOLD:.2}pt; --threshold {threshold:.2} asks something else"
        ));
    }
    let measured = sweep(refresh, threshold)?;
    match (record, check) {
        (true, _) => {
            let path = repo_root().join(REPORT);
            std::fs::write(&path, report_of(&measured))
                .map_err(|e| format!("{}: {e}", path.display()))?;
            println!("{} rewritten", path.display());
            Ok(ExitCode::SUCCESS)
        }
        (false, true) => against_the_record(&measured),
        (false, false) => {
            print!("{}", table_of(&measured));
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// What the corpus is held to, and where the record of it lives.
const REPORT: &str = "LAYOUT.md";

/// The documents this comparison cannot answer for, and why.
///
/// **Not a list of documents that are hard.** A hard document is the whole
/// point: it is where the work is, and its number is large and stays large
/// until somebody fixes it. This is the other thing — documents where the two
/// renderings are answering different questions, so that any figure at all
/// would be a figure about *that*, and driving it to zero would mean making
/// Scriva draw something it should not.
///
/// Kept in code rather than in the record so that an exclusion has to be
/// argued for in a place where it is reviewed, and written into the record as
/// its own section so that it is loud. A quiet zero in a table of numbers is
/// how a thing stops being measured without anybody deciding that it should.
const NOT_COMPARED: [(&str, &str); 1] = [(
    "tracked-changes.docx",
    "Word renders a document under revision as though every change had been \
     accepted. Scriva lays out what the file stores, deletions and all. The two \
     are not the same page, and what stands between them is not a layout \
     difference — this document held a baseline of 21 unplaceable words that \
     read like a score and measured nothing.",
)];

fn why_not(name: &str) -> Option<&'static str> {
    NOT_COMPARED
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, why)| *why)
}

/// How much worse the largest single shift may get before that alone is a
/// regression.
///
/// A word already out of place drifting by a fraction is the ordinary noise of
/// a change to shaping; a word already out of place doubling is not. What
/// crosses the threshold in either direction is caught exactly by the counts,
/// which have no slack at all.
const SLACK: f64 = 0.5;

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
  --check        fail if any document is worse than LAYOUT.md records
  --record       rewrite LAYOUT.md from what is measured now
  --top N        how many differences to print (default 20)
  --page N       print only what is wrong on one page
  --words        print both readings instead of comparing them — the words,
                 and then the page's own ink as the matching sees it
  --lines        print how each reading was cut into lines, which is what
                 the matching compares (use with --page)
  --threshold P  points past which a word counts as misplaced (default 1.0)"
        .into()
}

fn one(
    path: &Path,
    refresh: bool,
    top: usize,
    threshold: f64,
    only: Option<u32>,
) -> Result<(), String> {
    let laid = measure(path, refresh, threshold)?;
    let report = &laid.words;
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
    let furniture = &laid.marks;
    println!(
        "  {} marks matched, {} unmatched, {} out — worst {:.2}pt",
        furniture.matched, furniture.lost, furniture.out, furniture.worst
    );
    if furniture.pictures > 0 {
        // Not a fault and not a pass: Word answered our box by drawing the
        // picture into it, which is as much as the two renderings can say to
        // each other about a diagram. Printed so that the silence is visible.
        println!(
            "  {} pictures Word drew into rather than drew a box for",
            furniture.pictures
        );
    }
    if furniture.refused > 0 {
        println!(
            "  {} pages held too many marks to pair, and nothing below accounts for them",
            furniture.refused
        );
    }
    if report.refused > 0 {
        println!(
            "  {} stretches were too large to pair, and nothing below accounts              for the words in them",
            report.refused
        );
    }
    println!("  ── {} ──", report.scalar());

    // The two are printed apart because they are read for different reasons.
    // What is merely out of place is the work: it is ranked, and the top of it
    // is where the next fix is. What is unmatched is mostly the floor this
    // cannot see past — a document's diagrams — and it is a count with a
    // sample, so that it stops burying the ranking under itself.
    // One page at a time is how a difference is actually worked on: the
    // document's own numbers stay above, so narrowing the list cannot quietly
    // narrow the account of the document.
    let (moved, absent): (Vec<_>, Vec<_>) = report
        .differences
        .iter()
        .filter(|found| only.is_none_or(|page| found.page == page))
        .partition(|found| matches!(found.kind, Kind::Moved { .. }));
    if let Some(page) = only {
        println!(
            "  page {page} alone: {} out of place, {} unplaced",
            moved.len(),
            absent.len()
        );
    }

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
        // One page at a time, they are printed where they are: an unplaced
        // word is only ever acted on by going and looking at that part of the
        // page, and a bare list of texts sends you to `--words` every time.
        match only {
            Some(_) => {
                for found in absent.iter().take(top) {
                    println!("  {}", line(found));
                }
                let rest = absent.len().saturating_sub(top);
                if rest > 0 {
                    println!("  … and {rest} more");
                }
            }
            None => {
                let sample: Vec<String> = absent
                    .iter()
                    .take(8)
                    .map(|f| format!("p{} {:?}", f.page, f.text))
                    .collect();
                println!("  {}", sample.join("  "));
            }
        }
    }

    // Last, and apart, because it is a different question. The words say
    // whether the text is set where Word sets it; these say whether the page
    // around the text is drawn where Word draws it, and a page can be right
    // about one and wrong about the other.
    let furniture: Vec<&diff::Difference> = laid
        .marks
        .differences
        .iter()
        .filter(|found| only.is_none_or(|page| found.page == page))
        .collect();
    if !furniture.is_empty() {
        println!();
        println!("  the page's own ink:");
        for found in furniture.iter().take(top) {
            println!("  {}", line(found));
        }
        let rest = furniture.len().saturating_sub(top);
        if rest > 0 {
            println!("  … and {rest} more");
        }
    }
    Ok(())
}

/// Both readings of a document, as they reach the comparison.
///
/// What a person reaches for the moment a report says nothing matched, and the
/// reason it is a flag rather than a script written again each time: a
/// throwaway that has been written three times is a tool that has not been
/// built. One line per word, so that two columns of `sort` and `diff` answer
/// most of what anybody wants to ask of it.
fn listing(path: &Path, refresh: bool, only: Option<u32>) -> Result<(), String> {
    let theirs = word::read(path, refresh)?;
    let ours = ours::read(path)?;
    let mut all: Vec<(&'static str, &diff::Word)> = ours
        .words
        .iter()
        .map(|word| ("ours", word))
        .chain(theirs.words.iter().map(|word| ("word", word)))
        .filter(|(_, word)| only.is_none_or(|page| word.page == page))
        .collect();
    all.sort_by(|(_, a), (_, b)| {
        a.page
            .cmp(&b.page)
            .then(a.baseline.total_cmp(&b.baseline))
            .then(a.x.total_cmp(&b.x))
    });
    println!("side	page	band	x	baseline	text");
    for (side, word) in all {
        let band = word.band.map(|b| b.to_string()).unwrap_or_default();
        println!(
            "{side}	{}	{band}	{:.3}	{:.3}	{}",
            word.page, word.x, word.baseline, word.text
        );
    }

    // The furniture as it reaches the matching — which is to say after the
    // pieces of one rule have been run together, since the raw rectangles are
    // an account of how each renderer likes to draw a border and not of where
    // the border went.
    let mut ink: Vec<(&'static str, marks::Mark)> = marks::merged(&ours.marks)
        .into_iter()
        .map(|mark| ("ours", mark))
        .chain(
            marks::merged(&theirs.marks)
                .into_iter()
                .map(|m| ("word", m)),
        )
        .filter(|(_, mark)| only.is_none_or(|page| mark.page == page))
        .collect();
    ink.sort_by(|(_, a), (_, b)| {
        a.page
            .cmp(&b.page)
            .then(a.rect.y0.total_cmp(&b.rect.y0))
            .then(a.rect.x0.total_cmp(&b.rect.x0))
    });
    println!();
    println!("side	page	kind	x0	y0	x1	y1");
    for (side, mark) in ink {
        let kind = match mark.picture {
            true => "picture",
            false => "mark",
        };
        println!(
            "{side}	{}	{kind}	{:.3}	{:.3}	{:.3}	{:.3}",
            mark.page, mark.rect.x0, mark.rect.y0, mark.rect.x1, mark.rect.y1
        );
    }
    Ok(())
}

/// How the two readings were cut into lines, side by side.
///
/// What to reach for the moment a report says a page matched nothing: the
/// comparison works on sequences of words, so two readings cut up differently
/// disagree about everything, and no amount of staring at positions shows it.
fn grouping(path: &Path, refresh: bool, only: Option<u32>) -> Result<(), String> {
    let theirs = word::read(path, refresh)?;
    let ours = ours::read(path)?;
    let pages = ours
        .words
        .iter()
        .chain(theirs.words.iter())
        .map(|word| word.page);
    let pages: std::collections::BTreeSet<u32> = match only {
        Some(page) => [page].into_iter().collect(),
        None => pages.collect(),
    };
    for page in pages {
        let (mine, said) = diff::grouping(&ours.words, &theirs.words, page);
        println!("── page {page} ──");
        for line in &mine {
            println!("ours {line}");
        }
        for line in &said {
            println!("word {line}");
        }
    }
    Ok(())
}

fn line(found: &diff::Difference) -> String {
    let what = match found.kind {
        Kind::Moved { dx, dy } => format!("dx={dx:+6.2} dy={dy:+6.2}"),
        Kind::Missing => format!("Word alone, at {:6.1},{:6.1}", found.at.0, found.at.1),
        Kind::Extra => format!("ours alone, at {:6.1},{:6.1}", found.at.0, found.at.1),
    };
    let text: String = found.text.chars().take(28).collect();
    // A word only Word laid has no band: its rendering has forgotten which
    // flow drew what, and inventing one would be the report making it up.
    let band = found.band.map(|b| b.to_string()).unwrap_or_default();
    format!("page {:>3} {band:<6} {what}  {text:?}", found.page)
}

/// What one document came to, for a table, a record, or a comparison against
/// one. A document Word would not render keeps its reason rather than a zero,
/// because a zero here reads exactly like a document with nothing wrong.
struct Measured {
    name: String,
    outcome: Result<Laid, String>,
}

fn sweep(refresh: bool, threshold: f64) -> Result<Vec<Measured>, String> {
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
    Ok(paths
        .iter()
        .map(|path| Measured {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            outcome: measure(path, refresh, threshold),
        })
        .collect())
}

/// Out of place and unplaceable are separate columns on purpose: the first is
/// work and the second is mostly what this cannot see, and one number holding
/// both tells you neither.
fn table_of(measured: &[Measured]) -> String {
    let mut out = format!(
        "{:<40} {:>6} {:>5} {:>8} {:>6} {:>5} {:>8} {:>5}\n{}\n",
        "file",
        "out",
        ">5pt",
        "unplaced",
        "marks",
        "lost",
        "worst",
        "pages",
        "-".repeat(90)
    );
    let (mut over, mut badly, mut unplaced, mut failed) = (0usize, 0usize, 0usize, 0usize);
    let (mut ink, mut lost) = (0usize, 0usize);
    for row in measured {
        let name = truncate(&row.name, 40);
        match &row.outcome {
            Ok(Laid {
                words: report,
                marks: furniture,
            }) => {
                // Still measured and still shown — they are facts about two
                // renderings — but kept out of every total, because a figure
                // that counts towards a score is a figure somebody will try to
                // drive down.
                let counted = why_not(&row.name).is_none();
                if counted {
                    over += report.over;
                    badly += report.badly;
                    unplaced += report.unmatched;
                    ink += furniture.out;
                    lost += furniture.lost;
                }
                out += &format!(
                    "{name:<40} {:>6} {:>5} {:>8} {:>6} {:>5} {:>6.2}pt {:>5}{}\n",
                    report.over,
                    report.badly,
                    report.unmatched,
                    furniture.out,
                    furniture.lost,
                    report.worst,
                    report.pages_ours,
                    match counted {
                        true => "",
                        false => "  not compared",
                    }
                );
            }
            Err(why) => {
                failed += 1;
                out += &format!("{name:<40} {}\n", why.lines().next().unwrap_or(""));
            }
        }
    }
    out += &format!(
        "{}\n{:<40} {over:>6} {badly:>5} {unplaced:>8} {ink:>6} {lost:>5}\n",
        "-".repeat(90),
        ""
    );
    if failed > 0 {
        out += &format!("{failed} of {} could not be measured\n", measured.len());
    }
    out
}

/// What the record holds for one document.
#[derive(Clone, Copy, PartialEq)]
struct Stood {
    pages: usize,
    over: usize,
    badly: usize,
    unplaced: usize,
    marks: usize,
    lost: usize,
    worst: f64,
}

/// The record the corpus is held to, as it will be committed.
fn report_of(measured: &[Measured]) -> String {
    let mut rows = String::new();
    let mut apart = String::new();
    let (mut over, mut badly, mut unplaced, mut worst) = (0usize, 0usize, 0usize, 0.0f64);
    let (mut ink, mut lost) = (0usize, 0usize);
    let mut held = 0usize;
    for row in measured {
        if let Some(why) = why_not(&row.name) {
            apart += &format!("| `{}` | {why} |\n", row.name);
            continue;
        }
        held += 1;
        match &row.outcome {
            Ok(Laid {
                words: report,
                marks: furniture,
            }) => {
                over += report.over;
                badly += report.badly;
                unplaced += report.unmatched;
                ink += furniture.out;
                lost += furniture.lost;
                worst = worst.max(report.worst);
                rows += &format!(
                    "| `{}` | {} | {} | {} | {} | {} | {} | {:.2} |\n",
                    row.name,
                    report.pages_ours,
                    report.over,
                    report.badly,
                    report.unmatched,
                    furniture.out,
                    furniture.lost,
                    report.worst
                );
            }
            // A document that could not be measured is recorded as such rather
            // than left out, so that a corpus which has quietly stopped being
            // measurable cannot read as a corpus with nothing wrong in it.
            Err(_) => rows += &format!("| `{}` | — | — | — | — | — | — | — |\n", row.name),
        }
    }
    format!(
        "# Layout report\n\n\
Generated by `cargo xtask compare --record` over `corpus/`. Do not edit by hand — regenerate it.\n\n\
Every document here is laid out by Scriva and rendered by Word itself, and the two are compared \
mark by mark: where each word's pen went down, and where each rule and shading and picture was \
put, in points from the top-left of the page. \
`cargo xtask compare --check` fails if any document gets worse than this, which is what makes a \
layout regression arithmetic rather than something a person has to notice. ADR 0003 records why \
it is not a person.\n\n\
These are not targets. They are what is true today, so that what is true tomorrow can be held \
against it.\n\n\
- **pages** — how many pages Scriva laid it in. Any change at all has to be recorded \
deliberately: pagination moving is the largest layout event there is.\n\
- **out** — words both sides laid, further than a point from where Word put them.\n\
- **>5pt** — how many of those are further out than five points. Two counts rather than one, \
because a single count cannot see work moving about: a word improving while another of the same \
size worsens leaves it unchanged.\n\
- **unplaced** — words only one side laid at all. A chart's labels are drawn from the plot rather \
than from a line and are not gathered here, so a document full of charts keeps a floor.\n\
- **marks** — rectangles of ink that are not type, further than a point from where Word drew \
them: a table border, an underline, a shading, a picture's box. Counted apart from the words \
because a page can be right about one and wrong about the other, and because they are matched \
by different means — a word by what it says, a rule by where it is.\n\
- **lost** — marks only one side drew. A picture Word answered by drawing the picture rather \
than a box is neither counted nor lost; it is set aside, which is as much as the two renderings \
can say to each other about a diagram. A shape's own words are the standing floor here: Word \
draws a WordArt watermark into a PDF as *outlines*, so its rendering has a filled shape per \
letter where ours has type, and neither side can answer the other. That is twelve of them in \
`watermark.docx`, and they are left in the count rather than set aside because a watermark's box \
is transparent — the body's own rules pass under it, and setting the box aside would take them \
with it.\n\
- **worst** — the largest single shift of a word, in points.\n\n\
| | documents | out | >5pt | unplaced | marks | lost | worst |\n\
|---|---:|---:|---:|---:|---:|---:|---:|\n\
| totals | {held} | {over} | {badly} | {unplaced} | {ink} | {lost} | {worst:.2} |\n\n\
## Every document\n\n\
| file | pages | out | >5pt | unplaced | marks | lost | worst |\n\
|---|---:|---:|---:|---:|---:|---:|---:|\n{rows}\n\
## Not compared\n\n\
Two renderings that answer different questions cannot be held to a number, and a number \
written down anyway is worse than none: it reads like a score, and driving it down would mean \
making Scriva draw something it should not. These are still laid out, still measured, and still \
noticed if they leave the corpus — they are simply not held to anything.\n\n\
| file | why |\n|---|---|\n{apart}"
    )
}

/// Every document, against what the record says it was.
///
/// The whole point of the record: something that gets worse is caught by
/// arithmetic rather than by somebody remembering what the number used to be.
/// A document nobody has recorded, one that can no longer be measured, and one
/// the record has that the corpus no longer does are all failures. The last of
/// those especially: a file renamed or deleted otherwise stops being checked
/// without a word, while its row sits in the record looking like coverage.
fn against_the_record(measured: &[Measured]) -> Result<ExitCode, String> {
    let path = repo_root().join(REPORT);
    let text = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "{}: {e}\nRun `cargo xtask compare --record` to write it.",
            path.display()
        )
    })?;
    let recorded = recorded_in(&text);

    let mut worse: Vec<String> = Vec::new();
    let mut better = 0usize;
    for row in measured {
        if why_not(&row.name).is_some() {
            continue;
        }
        let Some(&was) = recorded.get(&row.name) else {
            // Either it was never recorded, or it was recorded as a document
            // that could not be measured. Both mean the same thing here: it is
            // being held to nothing.
            worse.push(format!(
                "{}: {REPORT} has no numbers for it — run `cargo xtask compare --record`",
                row.name
            ));
            continue;
        };
        let (report, furniture) = match &row.outcome {
            Ok(laid) => (&laid.words, &laid.marks),
            Err(why) => {
                worse.push(format!(
                    "{}: could not be measured — {}",
                    row.name,
                    why.lines().next().unwrap_or("")
                ));
                continue;
            }
        };
        let mut faults: Vec<String> = Vec::new();
        // Pagination is checked in both directions. A page fewer is as much a
        // change as a page more, and neither should pass unremarked.
        if report.pages_ours != was.pages {
            faults.push(format!("pages {} to {}", was.pages, report.pages_ours));
        }
        if report.over > was.over {
            faults.push(format!("out {} to {}", was.over, report.over));
        }
        if report.badly > was.badly {
            faults.push(format!(">5pt {} to {}", was.badly, report.badly));
        }
        if report.unmatched > was.unplaced {
            faults.push(format!("unplaced {} to {}", was.unplaced, report.unmatched));
        }
        if furniture.out > was.marks {
            faults.push(format!("marks {} to {}", was.marks, furniture.out));
        }
        if furniture.lost > was.lost {
            faults.push(format!("marks lost {} to {}", was.lost, furniture.lost));
        }
        if report.worst > was.worst + SLACK {
            faults.push(format!("worst {:.2}pt to {:.2}pt", was.worst, report.worst));
        }
        match faults.is_empty() {
            true => {
                if report.over < was.over
                    || report.badly < was.badly
                    || report.unmatched < was.unplaced
                    || furniture.out < was.marks
                    || furniture.lost < was.lost
                {
                    better += 1;
                }
            }
            false => worse.push(format!("{}: {}", row.name, faults.join(", "))),
        }
    }

    // And the other direction, which nothing else here would ever look at.
    // A document set apart from the comparison is still watched for *presence*:
    // not being held to a number is not the same as not being there.
    let present = |name: &str| measured.iter().any(|row| row.name == name);
    for name in recorded.keys() {
        if !present(name) {
            worse.push(format!(
                "{name}: in {REPORT} but no longer in the corpus — it has stopped being checked"
            ));
        }
    }
    for (name, _) in NOT_COMPARED {
        if !present(name) {
            worse.push(format!(
                "{name}: set apart from the comparison in the code, but no longer in the corpus"
            ));
        }
    }

    if worse.is_empty() {
        println!("{} documents, none worse than {REPORT}", measured.len());
        if better > 0 {
            println!("{better} are better — `cargo xtask compare --record` to keep it");
        }
        return Ok(ExitCode::SUCCESS);
    }
    worse.sort();
    println!("{} of {} got worse:", worse.len(), measured.len());
    for fault in &worse {
        println!("  {fault}");
    }
    Ok(ExitCode::FAILURE)
}

/// The per-document rows of a committed report.
///
/// Read out of the table a person reads rather than out of a format of its
/// own, because a record nobody can read is a record nobody checks.
fn recorded_in(text: &str) -> std::collections::HashMap<String, Stood> {
    let mut out = std::collections::HashMap::new();
    for line in text.lines() {
        let mut cells = line.split('|').map(str::trim);
        let Some("") = cells.next() else { continue };
        let cells: Vec<&str> = cells.collect();
        let [name, pages, over, badly, unplaced, marks, lost, worst, ..] = cells.as_slice() else {
            continue;
        };
        let Some(name) = name.strip_prefix('`').and_then(|n| n.strip_suffix('`')) else {
            continue;
        };
        let (Ok(pages), Ok(over), Ok(badly), Ok(unplaced), Ok(marks), Ok(lost), Ok(worst)) = (
            pages.parse(),
            over.parse(),
            badly.parse(),
            unplaced.parse(),
            marks.parse(),
            lost.parse(),
            worst.parse(),
        ) else {
            continue;
        };
        out.insert(
            name.to_string(),
            Stood {
                pages,
                over,
                badly,
                unplaced,
                marks,
                lost,
                worst,
            },
        );
    }
    out
}

fn measure(path: &Path, refresh: bool, threshold: f64) -> Result<Laid, String> {
    let theirs = word::read(path, refresh)?;
    let ours = ours::read(path)?;
    Ok(Laid {
        words: diff::compare(&ours.words, &theirs.words, threshold),
        marks: marks::compare(&ours.marks, &theirs.marks, threshold),
    })
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
