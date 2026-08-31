//! Where the reference application put every mark of the same document.
//!
//! Two steps, both under `tools/probe/`. The first asks the application that
//! owns the format for its own rendering of the file, as a PDF: `topdf.ps1`
//! asks Word, `toodf-pdf.ps1` asks LibreOffice. The second, `pdfink.py`, reads
//! out of that PDF where the words and the rules landed, and neither knows nor
//! cares which application drew it.
//!
//! **Which application answers for a document is decided by the document.**
//! Word owns `.docx` and `.doc` and is the only honest oracle for them.
//! LibreOffice is the implementation ODF is defined against in practice, and
//! Word reads `.odt` through a converter it wrote for a format it does not own
//! — so an `.odt` is measured against LibreOffice. One renderer per document,
//! which is what keeps a document to one reading and one row of `LAYOUT.md`.
//!
//! The route through paper is not a convenience. `Range.Information(5|6)`
//! answers to a twentieth of a point, but it costs Word a layout pass per call
//! — measured here at about 110ms, per word — so a sixteen-page document is
//! hours. `wordmap.ps1` still uses it, for one page at a time, by eye.
//!
//! **The answers for the corpus are committed, and that is what lets the check
//! be a check.** A rendering of a document does not change until the document
//! does, so it is written to `corpus/rendered/` and kept: the comparison then
//! needs neither application at all, runs in a few seconds, and can sit inside
//! `cargo xtask check` on a machine that has never had either installed. One is
//! needed again only to renew the reading of a document that has actually
//! changed, and the file says plainly when that is so.
//!
//! Only documents under `corpus/` are kept this way. A reading holds every word
//! of the document it read, so a reading of somebody's real document is that
//! document's text, and those are looked at from `manual_examples/` and never
//! committed — the same rule, for the same reason, one step further along.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::diff::Word;
use crate::marks::{Mark, Rect};
use crate::Reading;

/// The application a document is measured against.
///
/// Chosen by the document rather than by a flag, because it is not a preference:
/// a `.docx` measured against anything but Word, or an `.odt` measured against
/// Word's converter for a format it does not own, is a number about the wrong
/// thing. Adding a format means adding the application that owns it here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    Word,
    LibreOffice,
}

impl Renderer {
    pub fn of(path: &Path) -> Renderer {
        match path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("odt"))
        {
            true => Renderer::LibreOffice,
            false => Renderer::Word,
        }
    }

    /// What to call it in a message somebody has to act on.
    pub fn name(self) -> &'static str {
        match self {
            Renderer::Word => "Word",
            Renderer::LibreOffice => "LibreOffice",
        }
    }

    /// The script that asks it to render, under `tools/probe/`.
    fn script(self) -> &'static str {
        match self {
            Renderer::Word => "topdf.ps1",
            Renderer::LibreOffice => "toodf-pdf.ps1",
        }
    }
}

/// The three things a reading depends on, as one line of a file.
///
/// The document, and both scripts: an improvement to how words are found on a
/// page has to invalidate every reading, or the next comparison silently
/// measures against the old rule. Hashed rather than timestamped because a
/// reading is committed and a clone has no timestamps worth anything — git
/// records content, not when a file was written, and a cache keyed on mtime
/// misses on every fresh checkout.
///
/// **The renderer is not a fourth digest, and does not need to be.** Each
/// application is asked through a script of its own, so `export` already
/// differs between them, and a reading taken from one can never answer a stamp
/// computed for the other. Naming the renderer on the stamp line as well would
/// say nothing the digest does not, and would make every reading committed
/// before ODF existed stale — half an hour of driving Word to restate an answer
/// it has already given. It is named in the header instead, where a reader
/// wants it and nothing is held to it.
struct Stamp {
    renderer: Renderer,
    document: u32,
    export: u32,
    reading: u32,
}

impl Stamp {
    fn of(path: &Path) -> Result<Stamp, String> {
        if !path.exists() {
            return Err(format!("{} is not there", path.display()));
        }
        let renderer = Renderer::of(path);
        Ok(Stamp {
            renderer,
            document: digest(path),
            export: digest(&probe(renderer.script())),
            reading: digest(&probe("pdfink.py")),
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
            "# {who}'s own rendering of {name}, as the place every mark of it landed.\n\
             # Written by `cargo xtask compare`. Not by hand, and not read by anything\n\
             # but the comparison. In points, one mark to a line, of two kinds:\n\
             #   word  page  x  baseline  text\n\
             #   mark  page  x0  y0  x1  y1\n\
             {}\n\
             # Stale? `cargo xtask compare --refresh` renews it, and needs {who} for that\n\
             # one document. Everything else goes on working without it.\n",
            self.line(),
            who = self.renderer.name()
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

/// Whether a reading may be taken again, and on whose say-so.
///
/// **`--check` is [`Renew::Never`], and that is not a convenience.** A check
/// holds the corpus to evidence somebody committed; a check that quietly makes
/// the evidence it needs in order to pass is holding the corpus to nothing. The
/// difference is invisible on a machine with Word installed — the reading is
/// renewed in seconds and the gate goes green — and it is exactly how a commit
/// went out with readings stamped for an older probe script, passing here and
/// failing on any machine without Office. A stale reading under a check is a
/// fault to be reported, not a thing to repair on the way past.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Renew {
    /// Never. The reading on disk is the evidence, and if it answers another
    /// document or another probe script, that is the finding.
    Never,
    /// Where there is no reading, or the one there answers something else.
    IfStale,
    /// Always, because the document or the probes have deliberately changed.
    Always,
}

/// Asks the reference application where every mark went — or, almost always,
/// reads what it already said.
pub fn read(path: &Path, renew: Renew) -> Result<Reading, String> {
    let stamp = Stamp::of(path)?;
    let kept = reading_at(path)?;

    if renew != Renew::Always {
        if let Ok(text) = std::fs::read_to_string(&kept) {
            if stamp.answers(&text) {
                return parse(&text);
            }
        }
    }
    if renew == Renew::Never {
        return Err(format!(
            "{} answers an older {}, or an older probe script. Renewing it needs \
             {}, and a check will not do that for you: run \
             `cargo xtask compare --refresh` and commit what it writes.",
            kept.display(),
            path.file_name().unwrap_or_default().to_string_lossy(),
            stamp.renderer.name()
        ));
    }

    // Both directories, before either is written into. Word reports a missing
    // output directory as "the directory name isn't valid" from somewhere deep
    // inside the export, which reads like a fault in the document — and on a
    // fresh clone `target/` is exactly what does not exist yet.
    let paper = paper_at(path, &stamp);
    for dir in [paper.parent(), kept.parent()].into_iter().flatten() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    if renew == Renew::Always || !paper.exists() {
        render(stamp.renderer, path, &paper).map_err(|why| match kept.exists() {
            // The distinction that matters when this fails: a reading that is
            // merely out of date is a document somebody changed, or a probe
            // script somebody improved, and it should say so rather than read
            // as a machine without Word on it.
            true => format!(
                "{} was taken from an older {}, or with older probe scripts, \
                 and renewing it needs {}.\n{why}",
                kept.display(),
                path.file_name().unwrap_or_default().to_string_lossy(),
                stamp.renderer.name()
            ),
            false => why,
        })?;
    }
    let body = extract(&paper)?;
    let _ = std::fs::write(&kept, stamp.header(path) + &body);
    parse(&body)
}

fn probe(name: &str) -> PathBuf {
    crate::repo_root().join("tools").join("probe").join(name)
}

/// The reference application's own rendering of the document, as a PDF beside
/// the cache.
///
/// Both probes are PowerShell, and Word is Windows-only in any case. LibreOffice
/// is not, but the script that drives it is, and a claim to run anywhere that
/// nothing here ever runs is prose rather than a capability.
fn render(who: Renderer, path: &Path, pdf: &Path) -> Result<(), String> {
    if !cfg!(windows) {
        return Err(format!(
            "{}'s half of the comparison needs Windows and an installed {0}",
            who.name()
        ));
    }
    let script = probe(who.script());
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
        // The message somebody without Office reads, and it has one job: to
        // say that only *this document* wants it. The corpus does not — its
        // readings are committed — and a machine that cannot start powershell
        // can still run `cargo xtask compare --check` and the whole gate above
        // it, which is the thing worth knowing here and is easy to doubt when
        // the tool has just refused to do anything at all.
        .map_err(|e| {
            format!(
                "this document has no reading, and taking one needs {}: {e}.\n\
                 Only a document from outside `corpus/` ever needs it. The corpus \
                 is compared against readings committed under `corpus/rendered/`, \
                 and `--check` asks {0} for nothing.",
                who.name()
            )
        })?;
    if !out.status.success() || !pdf.exists() {
        let why = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "{} would not render {}:\n{}",
            who.name(),
            path.display(),
            why.trim()
        ));
    }
    Ok(())
}

fn extract(pdf: &Path) -> Result<String, String> {
    let script = probe("pdfink.py");
    if !script.exists() {
        return Err(format!("{} is missing", script.display()));
    }
    let out = Command::new("python")
        .arg(&script)
        .arg(pdf)
        .output()
        .map_err(|e| {
            format!(
                "the rendering is made but nothing here can read it: {e}.\n\
                 {} needs Python and PyMuPDF — `python -m pip install pymupdf`.",
                script.display()
            )
        })?;
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

/// A reading, back from the flat file it is kept in.
///
/// A line that is not a measurement is skipped rather than guessed at: the file
/// carries its own header, and a probe script that grows a third kind of row
/// must not make every older reading unreadable — it makes them *stale*, which
/// is a thing this can say.
fn parse(dump: &str) -> Result<Reading, String> {
    let mut words = Vec::new();
    let mut marks = Vec::new();
    for line in dump.lines() {
        let mut fields = line.split('\t');
        let (Some(kind), Some(page)) = (fields.next(), fields.next()) else {
            continue;
        };
        let Ok(page) = page.parse::<u32>() else {
            continue;
        };
        match kind {
            "word" => words.extend(word(page, fields)),
            "mark" => marks.extend(mark(page, fields, false)),
            "picture" => marks.extend(mark(page, fields, true)),
            _ => continue,
        }
    }
    if words.is_empty() {
        return Err("the rendering held no words at all — is the document empty?".into());
    }
    Ok(Reading { words, marks })
}

fn word<'a>(page: u32, mut fields: impl Iterator<Item = &'a str>) -> Option<Word> {
    let (Some(x), Some(baseline), Some(text)) = (fields.next(), fields.next(), fields.next())
    else {
        return None;
    };
    let (Ok(x), Ok(baseline)) = (x.parse::<f64>(), baseline.parse::<f64>()) else {
        return None;
    };
    // The text of a word may hold anything but a tab, so whatever is left of
    // the line belongs to it.
    let text: String = std::iter::once(text)
        .chain(fields)
        .collect::<Vec<_>>()
        .join("\t");
    let text = without_leader(&text);
    match text.is_empty() {
        true => None,
        false => Some(Word {
            page,
            // A rendered page has forgotten which flow drew what. The band a
            // difference is reported under is the one *we* laid it in, and a
            // word only Word has is reported without one.
            band: None,
            x,
            baseline,
            text: crate::diff::spelled(text),
        }),
    }
}

/// A rectangle of ink, and whether the rendering said outright that it is a
/// picture's box.
///
/// It says so for a raster picture and for nothing else. Everything else a
/// picture is made of reaches a rendering as the strokes that draw it, with no
/// box among them — which is exactly the distinction
/// [`crate::marks::answered`] turns on.
fn mark<'a>(page: u32, fields: impl Iterator<Item = &'a str>, picture: bool) -> Option<Mark> {
    let corners: Vec<f64> = fields.filter_map(|field| field.parse().ok()).collect();
    let &[x0, y0, x1, y1] = corners.as_slice() else {
        return None;
    };
    Some(Mark {
        page,
        rect: Rect::new(x0, y0, x1, y1),
        picture,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(dump: &str) -> Vec<Word> {
        parse(dump).expect("a well-formed dump parses").words
    }

    #[test]
    fn a_dump_becomes_words_where_word_set_them() {
        let read = words(
            "word\t5\t72.000\t100.000\tmedia\n\
             word\t5\t100.000\t100.000\toptions,\n",
        );
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].text, "media");
        assert_eq!(read[0].page, 5);
        assert_eq!(read[1].text, "options,");
        assert!((read[1].x - 100.0).abs() < 0.001);
        assert!(read[0].band.is_none());
    }

    #[test]
    fn a_line_that_is_not_a_measurement_is_skipped_rather_than_guessed_at() {
        let read = words("not a row at all\n# a header\nword\t5\t72.0\t100.0\tkept\n\n");
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].text, "kept");
    }

    /// The page's furniture, which arrives in the same file under its own kind.
    #[test]
    fn a_rule_and_a_picture_come_back_as_the_rectangles_they_are() {
        let dump = "word\t1\t72.0\t100.0\tone\n\
                    mark\t1\t72.000\t72.000\t540.000\t72.480\n\
                    picture\t2\t72.000\t72.000\t192.000\t162.000\n";
        let read = parse(dump).expect("a well-formed dump parses");
        assert_eq!(read.words.len(), 1);
        assert_eq!(read.marks.len(), 2);
        assert!(!read.marks[0].picture);
        assert!((read.marks[0].rect.width() - 468.0).abs() < 0.001);
        assert_eq!(read.marks[1].page, 2);
        assert!(read.marks[1].picture, "a raster picture says so outright");
    }

    /// A rectangle short of a corner is not a rectangle, and a guess at the
    /// fourth of them is a measurement nobody took.
    #[test]
    fn a_mark_that_is_short_a_corner_is_dropped() {
        let dump = "word\t1\t72.0\t100.0\tone\nmark\t1\t72.000\t72.000\t540.000\n";
        assert!(parse(dump).expect("the word still stands").marks.is_empty());
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
        let read = words(
            "word\t2\t72.000\t100.000\tHardware...........\n\
             word\t2\t500.000\t100.000\t11\n",
        );
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].text, "Hardware");
        assert_eq!(read[1].text, "11");
    }

    #[test]
    fn an_empty_answer_is_an_error_rather_than_a_clean_bill() {
        assert!(parse("").is_err());
    }

    /// The guarantee a check rests on, tested where it is hardest to see: on a
    /// machine that *has* Word. `tests/without_office.rs` proves the corpus can be
    /// checked without Office; this proves the check would not have quietly
    /// fetched it even if it could. A commit went out with readings stamped for
    /// an older probe script precisely because renewing one is invisible here.
    #[test]
    fn a_check_will_not_renew_a_stale_reading_even_where_word_is_installed() {
        let doc = crate::repo_root()
            .join("corpus")
            .join("docx")
            .join("minimal.docx");
        let kept = crate::target_dir()
            .join("compare")
            .join("never-renewed.docx.tsv");
        let copy = kept.with_file_name("never-renewed.docx");
        std::fs::create_dir_all(kept.parent().expect("a reading has a directory"))
            .expect("target/ is writable");
        std::fs::copy(&doc, &copy).expect("the corpus holds minimal.docx");
        std::fs::write(
            &kept,
            "# document 00000000  export 00000000  reading 00000000\n\
             word\t1\t72.000\t72.000\tstale\n",
        )
        .expect("target/ is writable");

        let Err(refused) = read(&copy, Renew::Never) else {
            panic!("the reading answers another document and must be refused");
        };
        assert!(refused.contains("--refresh"), "{refused}");
        // And the reading is still the one that was there: a check that rewrote
        // it would have made the next check pass for the wrong reason.
        let after = std::fs::read_to_string(&kept).expect("the reading is still there");
        assert!(
            after.contains("stale"),
            "a check must not rewrite a reading"
        );
    }
}
