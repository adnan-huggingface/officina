//! Where Word put every word of the same document.
//!
//! Two steps, both under `tools/word-probe/`: `topdf.ps1` asks Word for its own
//! rendering of the file, and `pdfwords.py` reads the baselines out of it. The
//! answer is **cached** — the render costs seconds and the extraction costs
//! more, and neither changes until the document does, which is what lets the
//! comparison sit inside an edit loop.
//!
//! The route through paper is not a convenience. `Range.Information(5|6)`
//! answers to a twentieth of a point, but it costs Word a layout pass per call
//! — measured here at about 110ms, per word — so a sixteen-page document is
//! hours. `wordmap.ps1` still uses it, for one page at a time, by eye.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::diff::Word;

/// Where the two halves of the answer for a document are kept.
///
/// Keyed by everything that would change it, and by nothing that would not.
/// The rendering depends on the document and on the script that asks Word for
/// it; the reading depends on those *and* on the script that reads the paper.
/// Keeping them apart means an improvement to how words are found on a page
/// re-reads every PDF already on disk and does not ask Word for any of them
/// again — which is the difference between a minute and an afternoon, and the
/// reason a stale extraction is never what you are looking at.
///
/// Length and modification time rather than a hash of the document: this is a
/// cache, and reading sixteen megabytes to decide whether to read a cache
/// defeats the cache. The scripts are small, so they are hashed outright.
fn cached_at(path: &Path) -> Result<(PathBuf, PathBuf), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let stamp = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let stem: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let dir = crate::target_dir().join("compare");
    let paper = digest(&probe("topdf.ps1"));
    let reading = digest(&probe("pdfwords.py"));
    let base = format!("{stem}-{}-{stamp}-{paper:08x}", meta.len());
    Ok((
        dir.join(format!("{base}.pdf")),
        dir.join(format!("{base}-{reading:08x}.tsv")),
    ))
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

/// Asks Word where every word went, through the cache.
pub fn read(path: &Path, refresh: bool) -> Result<Vec<Word>, String> {
    let (pdf, dump) = cached_at(path)?;
    let cached = match refresh {
        true => None,
        false => std::fs::read_to_string(&dump).ok(),
    };
    let text = match cached {
        Some(text) => text,
        None => {
            if let Some(dir) = dump.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            if refresh || !pdf.exists() {
                render(path, &pdf)?;
            }
            let text = extract(&pdf)?;
            let _ = std::fs::write(&dump, &text);
            text
        }
    };
    parse(&text)
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
            text: text.to_string(),
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
