//! Where Word put every word of the same document.
//!
//! Two steps, both under `tools/word-probe/`: `topdf.ps1` asks Word for its own
//! rendering of the file, and `pdfwords.py` reads the baselines out of it.
//!
//! The route through paper is not a convenience. `Range.Information(5|6)`
//! answers to a twentieth of a point, but it costs Word a layout pass per call
//! — measured here at about 110ms, per word — so a sixteen-page document is
//! hours. `wordmap.ps1` still uses it, for one page at a time, by eye.
//!
//! **The answers for the corpus are committed, and that is what lets the check
//! be a check.** Word's reading of a document does not change until the
//! document does, so it is written to `corpus/rendered/` and kept: the
//! comparison then needs no Word at all, runs in a few seconds, and can sit
//! inside `cargo xtask check` on a machine that has never had Office installed.
//! Word is needed again only to renew the reading of a document that has
//! actually changed, and the file says plainly when that is so.
//!
//! Only documents under `corpus/` are kept this way. A reading holds every word
//! of the document it read, so a reading of somebody's real document is that
//! document's text, and those are looked at from `manual_examples/` and never
//! committed — the same rule, for the same reason, one step further along.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::diff::Word;

/// The three things a reading depends on, as one line of a file.
///
/// The document, and both scripts: an improvement to how words are found on a
/// page has to invalidate every reading, or the next comparison silently
/// measures against the old rule. Hashed rather than timestamped because a
/// reading is committed and a clone has no timestamps worth anything — git
/// records content, not when a file was written, and a cache keyed on mtime
/// misses on every fresh checkout.
struct Stamp {
    document: u32,
    export: u32,
    reading: u32,
}

impl Stamp {
    fn of(path: &Path) -> Result<Stamp, String> {
        if !path.exists() {
            return Err(format!("{} is not there", path.display()));
        }
        Ok(Stamp {
            document: digest(path),
            export: digest(&probe("topdf.ps1")),
            reading: digest(&probe("pdfwords.py")),
        })
    }

    fn line(&self) -> String {
        format!(
            "# document {:08x}  export {:08x}  reading {:08x}",
            self.document, self.export, self.reading
        )
    }

    fn header(&self, path: &Path) -> String {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        format!(
            "# Word's own rendering of {name}, as the place every word of it landed.\n\
             # Written by `cargo xtask compare`. Not by hand, and not read by anything\n\
             # but the comparison: `page`, `x`, `baseline`, `text`, in points.\n\
             {}\n\
             # Stale? `cargo xtask compare --refresh` renews it, and needs Word for that\n\
             # one document. Everything else goes on working without it.\n",
            self.line()
        )
    }

    /// Whether a reading on disk is a reading of *this*.
    fn answers(&self, text: &str) -> bool {
        let wanted = self.line();
        text.lines().any(|line| line.trim_end() == wanted)
    }
}

/// A small file's contents as one number. FNV-1a, which is enough to notice an
/// edit and is not being asked to withstand anyone.
fn digest(path: &Path) -> u32 {
    let Ok(bytes) = std::fs::read(path) else {
        return 0;
    };
    let mut hash: u32 = 0x811c_9dc5;
    for byte in bytes {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Where a document's reading is kept, and whether it is kept at all.
///
/// Under `corpus/`, beside the documents it reads and committed with them.
/// Anywhere else — a real document being looked at by hand — it goes to
/// `target/`, because a reading is the document's own words and those are not
/// ours to commit.
fn reading_at(path: &Path) -> Result<PathBuf, String> {
    let corpus = crate::repo_root().join("corpus");
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    match path.canonicalize().ok().is_some_and(|full| {
        corpus
            .canonicalize()
            .is_ok_and(|corpus| full.starts_with(&corpus))
    }) {
        true => Ok(corpus.join("rendered").join(format!("{name}.tsv"))),
        false => Ok(crate::target_dir()
            .join("compare")
            .join(format!("{name}.tsv"))),
    }
}

/// Where the rendering itself is kept, which is never committed: it is large,
/// it is binary, and the reading taken from it is the part anything here wants.
fn paper_at(path: &Path, stamp: &Stamp) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let stem: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    crate::target_dir().join("compare").join(format!(
        "{stem}-{:08x}-{:08x}.pdf",
        stamp.document, stamp.export
    ))
}

/// Asks Word where every word went — or, almost always, reads what it said.
pub fn read(path: &Path, refresh: bool) -> Result<Vec<Word>, String> {
    let stamp = Stamp::of(path)?;
    let kept = reading_at(path)?;

    if !refresh {
        if let Ok(text) = std::fs::read_to_string(&kept) {
            if stamp.answers(&text) {
                return parse(&text);
            }
        }
    }

    // Both directories, before either is written into. Word reports a missing
    // output directory as "the directory name isn't valid" from somewhere deep
    // inside the export, which reads like a fault in the document — and on a
    // fresh clone `target/` is exactly what does not exist yet.
    let paper = paper_at(path, &stamp);
    for dir in [paper.parent(), kept.parent()].into_iter().flatten() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    if refresh || !paper.exists() {
        render(path, &paper).map_err(|why| match kept.exists() {
            // The distinction that matters when this fails: a reading that is
            // merely out of date is a document somebody changed, or a probe
            // script somebody improved, and it should say so rather than read
            // as a machine without Word on it.
            true => format!(
                "{} was taken from an older {}, or with older probe scripts, \
                 and renewing it needs Word.\n{why}",
                kept.display(),
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
            false => why,
        })?;
    }
    let body = extract(&paper)?;
    let _ = std::fs::write(&kept, stamp.header(path) + &body);
    parse(&body)
}

fn probe(name: &str) -> PathBuf {
    crate::repo_root()
        .join("tools")
        .join("word-probe")
        .join(name)
}

/// Word's own rendering of the document, as a PDF beside the cache.
fn render(path: &Path, pdf: &Path) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("Word's half of the comparison needs Windows and an installed Word".into());
    }
    let script = probe("topdf.ps1");
    if !script.exists() {
        return Err(format!("{} is missing", script.display()));
    }
    let out = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .arg("-Path")
        .arg(path)
        .arg("-Out")
        .arg(pdf)
        .output()
        .map_err(|e| format!("could not run powershell: {e}"))?;
    if !out.status.success() || !pdf.exists() {
        let why = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "Word would not render {}:\n{}",
            path.display(),
            why.trim()
        ));
    }
    Ok(())
}

fn extract(pdf: &Path) -> Result<String, String> {
    let script = probe("pdfwords.py");
    if !script.exists() {
        return Err(format!("{} is missing", script.display()));
    }
    let out = Command::new("python")
        .arg(&script)
        .arg(pdf)
        .output()
        .map_err(|e| format!("could not run python: {e}"))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr);
        return Err(format!("reading {} failed:\n{}", pdf.display(), why.trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The word with any tab leader taken off the end of it.
///
/// Word fills a leadered tab with as many stops as the stretch takes, and so
/// does Scriva; neither count was chosen by anyone, and comparing them buries
/// the document's real differences under a page of full stops. A rendered page
/// draws no space between a contents entry and the dots that carry the eye to
/// its page number, so the two arrive here as one run and the dots have to
/// come off rather than merely be skipped.
///
/// Four of one character, so that an ellipsis and a dash survive as text.
fn without_leader(text: &str) -> &str {
    const LEADERS: [char; 6] = ['.', '_', '-', '\u{00b7}', '\u{2013}', '\u{2014}'];
    let Some(last) = text.chars().last().filter(|c| LEADERS.contains(c)) else {
        return text;
    };
    let kept = text.trim_end_matches(last);
    match text.chars().count() - kept.chars().count() >= 4 {
        true => kept,
        false => text,
    }
}

fn parse(dump: &str) -> Result<Vec<Word>, String> {
    let mut words = Vec::new();
    for line in dump.lines() {
        let mut fields = line.splitn(4, '\t');
        let (Some(page), Some(x), Some(baseline), Some(text)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let (Ok(page), Ok(x), Ok(baseline)) = (
            page.parse::<u32>(),
            x.parse::<f64>(),
            baseline.parse::<f64>(),
        ) else {
            continue;
        };
        let text = without_leader(text);
        if text.is_empty() {
            continue;
        }
        words.push(Word {
            page,
            // A rendered page has forgotten which flow drew what. The band a
            // difference is reported under is the one *we* laid it in, and a
            // word only Word has is reported without one.
            band: None,
            x,
            baseline,
            text: crate::diff::spelled(text),
        });
    }
    if words.is_empty() {
        return Err("Word's rendering held no words at all — is the document empty?".into());
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dump_becomes_words_where_word_set_them() {
        let dump = "5\t72.000\t100.000\tmedia\n5\t100.000\t100.000\toptions,\n";
        let words = parse(dump).expect("a well-formed dump parses");
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "media");
        assert_eq!(words[0].page, 5);
        assert_eq!(words[1].text, "options,");
        assert!((words[1].x - 100.0).abs() < 0.001);
        assert!(words[0].band.is_none());
    }

    #[test]
    fn a_line_that_is_not_a_measurement_is_skipped_rather_than_guessed_at() {
        let dump = "not a row at all\n5\t72.000\t100.000\tkept\n\n";
        let words = parse(dump).expect("the one good row is enough");
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, "kept");
    }

    /// A contents entry arrives from a rendered page with the dots attached,
    /// because that is how it was drawn: no space stands between them.
    #[test]
    fn a_tabs_leader_comes_off_the_word_it_was_drawn_against() {
        assert_eq!(
            without_leader("Architecture................"),
            "Architecture"
        );
        assert_eq!(without_leader("......................"), "");
        assert_eq!(without_leader("____"), "");
        assert_eq!(without_leader("Hardware"), "Hardware");
        assert_eq!(without_leader("etc..."), "etc...", "an ellipsis is text");
        assert_eq!(without_leader("well-"), "well-");
        let dump = "2\t72.000\t100.000\tHardware...........\n2\t500.000\t100.000\t11\n";
        let words = parse(dump).expect("a well-formed dump parses");
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "Hardware");
        assert_eq!(words[1].text, "11");
    }

    #[test]
    fn an_empty_answer_is_an_error_rather_than_a_clean_bill() {
        assert!(parse("").is_err());
    }
}
