//! The comparison, run on a machine that has neither of the two applications
//! it measures against.
//!
//! **The claim `--check` rests on had never been executed.** The readings of
//! the corpus are committed so that the gate needs no Office — that is why the
//! check can sit inside `cargo xtask check`, and it is written down in three
//! files. But every run of it has been on this machine, where Word is
//! installed and a stale reading is renewed in seconds without anybody
//! noticing; the path where Word is *absent* was a design and not a
//! measurement. A claim about what happens when something is missing is exactly
//! the kind that stops being true quietly.
//!
//! So the tool is run here with nothing on its PATH at all. Both applications
//! are reached by starting `powershell`, and the rendering is read by starting
//! `python`; neither can be found without a PATH, which is as near to an absent
//! Office as a machine with one installed can be brought. What these tests
//! prove is not that the code would work elsewhere — it is that the corpus is
//! checked without either application ever being asked, and that the ways of
//! needing one say so in words a person can act on.
//!
//! And they say *which* one. A document is measured against the application
//! that owns its format, so a `.docx` with no reading asks for Word and an
//! `.odt` asks for LibreOffice; a message that named the wrong one would send
//! somebody to install software that would not have helped.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The tool, with an empty PATH and nothing else changed.
fn without_word(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wp-compare"))
        .args(args)
        .env("PATH", "")
        .output()
        .expect("the binary under test is an absolute path and needs no PATH to start")
}

fn said(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr)
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("the crate's manifest dir always has a grandparent")
        .to_path_buf()
}

/// A copy of a corpus document put somewhere outside `corpus/`, and the place
/// its reading would be kept.
///
/// Outside, because a reading of a document from anywhere else goes to
/// `target/` rather than being committed — the rule that keeps somebody's real
/// document out of the tree — and it is that uncommitted case these two tests
/// are about. The reading is removed rather than assumed absent: a run with
/// Word on the PATH would have left a perfectly good one behind, and a test
/// that passes because of what a previous run did is not a test.
fn elsewhere(name: &str) -> PathBuf {
    from(name, &["docx", "minimal.docx"])
}

/// The same, for a document of another format — and so of another oracle.
fn elsewhere_odt(name: &str) -> PathBuf {
    from(name, &["odt", "second-producer.odt"])
}

fn from(name: &str, source: &[&str]) -> PathBuf {
    let dir = root().join("target").join("no-word");
    std::fs::create_dir_all(&dir).expect("target/ is writable");
    let copy = dir.join(name);
    let original = source
        .iter()
        .fold(root().join("corpus"), |path, part| path.join(part));
    std::fs::copy(&original, &copy).expect("the corpus holds the document copied here");
    let _ = std::fs::remove_file(reading_of(name));
    copy
}

fn reading_of(name: &str) -> PathBuf {
    root()
        .join("target")
        .join("compare")
        .join(format!("{name}.tsv"))
}

/// The one that matters: the whole corpus, held to the record, with no Word.
#[test]
fn the_corpus_is_checked_without_word() {
    let out = without_word(&["--check"]);
    assert!(
        out.status.success(),
        "the check needs no Word and did not pass without one:\n{}",
        said(&out)
    );
    // And it did the work rather than finding nothing to do: a sweep that
    // silently measured no documents would pass here too.
    let said = said(&out);
    assert!(
        said.contains("none worse than"),
        "the check passed without saying what it checked:\n{said}"
    );
}

/// A document nobody has a reading of, on a machine with no Word: the one
/// case where the answer really is "you need Word for this one".
#[test]
fn a_document_with_no_reading_asks_for_word_by_name() {
    let path = elsewhere("never-rendered.docx");
    let out = without_word(&[&path.to_string_lossy()]);
    let said = said(&out);
    assert!(!out.status.success(), "there is nothing to compare against");
    assert!(
        said.contains("Word"),
        "a document with no reading and no Word must say which of the two is missing:\n{said}"
    );
}

/// A reading that is out of date, on a machine with no Word. The distinction
/// that has to survive: this is a document somebody changed or a probe script
/// somebody improved, not a machine without Office on it, and the two want
/// different things done about them.
#[test]
fn a_stale_reading_says_it_is_stale_rather_than_that_word_is_missing() {
    let path = elsewhere("gone-stale.docx");
    let kept = reading_of("gone-stale.docx");
    std::fs::create_dir_all(kept.parent().expect("a reading has a directory"))
        .expect("target/ is writable");
    std::fs::write(
        &kept,
        "# document 00000000  export 00000000  reading 00000000\n\
         word\t1\t72.000\t72.000\tstale\n",
    )
    .expect("target/ is writable");

    let out = without_word(&[&path.to_string_lossy()]);
    let said = said(&out);
    assert!(
        !out.status.success(),
        "the reading answers another document"
    );
    assert!(
        said.contains("older"),
        "a stale reading must say it is stale:\n{said}"
    );
}

/// The same case for the other format, and the reason the renderer is chosen
/// by the document rather than by a flag.
#[test]
fn an_open_document_with_no_reading_asks_for_libreoffice_and_not_for_word() {
    let path = elsewhere_odt("never-rendered.odt");
    let out = without_word(&[&path.to_string_lossy()]);
    let said = said(&out);
    assert!(!out.status.success(), "there is nothing to compare against");
    assert!(
        said.contains("LibreOffice"),
        "an .odt is measured against LibreOffice and must say so:
{said}"
    );
    assert!(
        !said.contains("Word"),
        "and must not send anybody to install the application that does not          own the format:
{said}"
    );
}
