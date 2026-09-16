//! A document written by the application itself, from a script of its own
//! commands.
//!
//! Every corpus document was written by Word, and every one of them measures
//! how Scriva *reads*. Nothing measured what it *writes* — and the faults that
//! cost a week of afternoons were all there: a new document that stated no
//! defaults and came back from Word a third taller, an inserted table whose
//! cells stated no width and came back an inch wide. Each was found by
//! authoring a document in a throwaway test, carrying it to Word by hand and
//! reading the numbers off. This is that afternoon as a command.
//!
//! The script is a list of the things a user does — type, press a key, run a
//! menu command — and it is run through the same methods the menus and keys
//! run, so what lands on disk is what a user's document would be. It is saved
//! into the corpus, where Word's reading of it is committed like any other's,
//! and from then on `cargo xtask check` holds it to that reading: the next
//! time what Scriva writes changes, the test below says so, and the reading
//! has to be renewed by Word before the gate goes green.
//!
//! One command a line; `#` starts a comment. The words are the menus' own.
//!
//! The script also says what it meant, with `expect` lines, and the document
//! in the corpus is held to them as well as to its bytes. The bytes alone
//! passed for months with a bold space and a plain "bold": they were what the
//! application wrote, and nobody had said what it should have written.

use std::path::Path;

use ui_kit::egui;
use wp_model::prop::Justify;
use wp_model::units::{HalfPoint, Line240};

use crate::app::{Command, Scriva};

/// One line of the script, understood.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Text at the caret, as if typed.
    Type(String),
    /// One key press, as the document area sees it.
    Key(egui::Key, egui::Modifiers),
    /// Insert ▸ Table, with the dialog's two numbers.
    Table(usize, usize),
    /// A paragraph style by the id the file uses (`Heading1`), looked up in
    /// the document being authored.
    Style(String),
    /// A menu command that needs nothing else said.
    Run(Command),
    /// What the document should hold once written: nothing is done with it
    /// while authoring, and [`unmet`] holds a document to it.
    Expect(Expectation),
}

/// A property the text of an `expect` line has, in the document as written.
#[derive(Debug, Clone, PartialEq)]
pub enum Expectation {
    /// Every letter of the text is bold.
    Bold(String),
    Italic(String),
    Underlined(String),
    /// Every character of the text, blanks included, is neither bold, italic
    /// nor underlined.
    Plain(String),
    /// The paragraph holding the text is in this style, by its id.
    Style(String, String),
}

/// The script, one step per line that says something.
pub fn parse(text: &str) -> Result<Vec<(usize, Step)>, String> {
    let mut steps = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (word, rest) = match trimmed.split_once(char::is_whitespace) {
            Some((word, rest)) => (word, rest.trim_start()),
            None => (trimmed, ""),
        };
        let step = step(word, rest).map_err(|why| format!("line {line}: {why}"))?;
        steps.push((line, step));
    }
    Ok(steps)
}

fn step(word: &str, rest: &str) -> Result<Step, String> {
    let bare = |command: Command| -> Result<Step, String> {
        match rest.is_empty() {
            true => Ok(Step::Run(command)),
            false => Err(format!("`{word}` takes nothing after it, not `{rest}`")),
        }
    };
    Ok(match word {
        "type" => Step::Type(quoted(rest)),
        "key" => {
            let (key, modifiers) = ui_kit::drive::key_spec(rest)?;
            Step::Key(key, modifiers)
        }
        "table" => {
            let mut numbers = rest.split_whitespace().map(|n| n.parse::<usize>());
            match (numbers.next(), numbers.next(), numbers.next()) {
                (Some(Ok(rows)), Some(Ok(columns)), None) if rows > 0 && columns > 0 => {
                    Step::Table(rows, columns)
                }
                _ => return Err(format!("`table` wants rows and columns, not `{rest}`")),
            }
        }
        "style" => match rest.is_empty() {
            true => return Err("`style` wants a style id, such as Heading1".into()),
            false => Step::Style(rest.to_owned()),
        },
        "align" => Step::Run(Command::Align(match rest {
            "start" | "left" => Justify::Start,
            "center" => Justify::Center,
            "end" | "right" => Justify::End,
            "both" | "justify" => Justify::Both,
            other => {
                return Err(format!(
                    "`align` wants start, center, end or both, not `{other}`"
                ))
            }
        })),
        "size" => Step::Run(Command::Size(HalfPoint(
            rest.parse::<f64>()
                .ok()
                .filter(|points| *points > 0.0)
                .map(|points| (points * 2.0).round() as i32)
                .ok_or_else(|| format!("`size` wants a size in points, not `{rest}`"))?,
        ))),
        "line-spacing" => Step::Run(Command::LineSpacing(Line240(
            rest.parse::<f64>()
                .ok()
                .filter(|lines| *lines > 0.0)
                .map(|lines| (lines * 240.0).round() as i32)
                .ok_or_else(|| format!("`line-spacing` wants a number of lines, not `{rest}`"))?,
        ))),
        "indent" => {
            Step::Run(Command::Indent(rest.parse().map_err(|_| {
                format!("`indent` wants a number of steps, not `{rest}`")
            })?))
        }
        "borders" => Step::Run(Command::TableBorders(match rest {
            "on" => true,
            "off" => false,
            other => return Err(format!("`borders` wants on or off, not `{other}`")),
        })),
        "bold" => bare(Command::Bold)?,
        "italic" => bare(Command::Italic)?,
        "underline" => bare(Command::Underline)?,
        "strike" => bare(Command::Strike)?,
        "superscript" => bare(Command::Superscript)?,
        "subscript" => bare(Command::Subscript)?,
        "clear-formatting" => bare(Command::ClearFormatting)?,
        "page-break" => bare(Command::PageBreak)?,
        "bullets" => bare(Command::Bullets)?,
        "numbers" => bare(Command::Numbers)?,
        "select-all" => bare(Command::SelectAll)?,
        "undo" => bare(Command::Undo)?,
        "redo" => bare(Command::Redo)?,
        "merge-cells" => bare(Command::MergeCells)?,
        "edit-header" => bare(Command::EditHeader)?,
        "edit-footer" => bare(Command::EditFooter)?,
        "close-chrome" => bare(Command::CloseChrome)?,
        "page-number" => bare(Command::InsertPageNumber { of_pages: false })?,
        "page-of-pages" => bare(Command::InsertPageNumber { of_pages: true })?,
        "update-toc" => bare(Command::UpdateToc)?,
        "expect" => Step::Expect(expectation(rest)?),
        other => return Err(format!("`{other}` is not a command this script knows")),
    })
}

/// `bold "text"`, `plain "text"`, `style Heading1 "text"`.
fn expectation(rest: &str) -> Result<Expectation, String> {
    let (what, text) = rest
        .split_once(char::is_whitespace)
        .ok_or_else(|| format!("`expect` wants a property and a text, not `{rest}`"))?;
    let text = text.trim_start();
    let quoted_text = |text: &str| -> Result<String, String> {
        let inner = quoted(text);
        match inner.is_empty() || inner == text {
            true => Err(format!(
                "`expect` wants its text in double quotes, not `{text}`"
            )),
            false => Ok(inner),
        }
    };
    Ok(match what {
        "bold" => Expectation::Bold(quoted_text(text)?),
        "italic" => Expectation::Italic(quoted_text(text)?),
        "underlined" => Expectation::Underlined(quoted_text(text)?),
        "plain" => Expectation::Plain(quoted_text(text)?),
        "style" => {
            let (id, text) = text
                .split_once(char::is_whitespace)
                .ok_or_else(|| format!("`expect style` wants an id and a text, not `{text}`"))?;
            Expectation::Style(id.to_owned(), quoted_text(text.trim_start())?)
        }
        other => {
            return Err(format!(
                "`expect` knows bold, italic, underlined, plain and style, not `{other}`"
            ))
        }
    })
}

/// Every expectation the script states that `document` does not meet, each
/// said with the script's line. The text is looked for in the body's
/// paragraphs, first occurrence first, and each of its characters is
/// resolved through the style chain as Word would read it.
pub fn unmet(script: &str, document: &wp_model::Document) -> Result<Vec<String>, String> {
    use wp_model::prop::{Toggle, UnderlineKind};
    let mut failures = Vec::new();
    for (line, step) in parse(script)? {
        let Step::Expect(expectation) = step else {
            continue;
        };
        let text = match &expectation {
            Expectation::Bold(text)
            | Expectation::Italic(text)
            | Expectation::Underlined(text)
            | Expectation::Plain(text)
            | Expectation::Style(_, text) => text,
        };
        let found = document.paragraphs().into_iter().find_map(|paragraph| {
            let layers = document.styles.resolve_paragraph(&paragraph.props, None);
            let mut letters = Vec::new();
            for run in paragraph.runs() {
                let props = document.styles.resolve_run(&layers, &run.props);
                letters.extend(run.text().chars().map(|c| (c, props.clone())));
            }
            let chars: Vec<char> = letters.iter().map(|(c, _)| *c).collect();
            let wanted: Vec<char> = text.chars().collect();
            let start = chars.windows(wanted.len()).position(|w| w == wanted)?;
            Some((paragraph, letters[start..start + wanted.len()].to_vec()))
        });
        let Some((paragraph, letters)) = found else {
            failures.push(format!("line {line}: \"{text}\" is not in the document"));
            continue;
        };
        let bold = |p: &wp_model::prop::RunProps| p.toggles.is_on(Toggle::Bold);
        let italic = |p: &wp_model::prop::RunProps| p.toggles.is_on(Toggle::Italic);
        let underlined = |p: &wp_model::prop::RunProps| {
            p.underline
                .as_ref()
                .is_some_and(|u| u.kind != UnderlineKind::None)
        };
        let wrong: Vec<char> = letters
            .iter()
            .filter(|(c, props)| match &expectation {
                Expectation::Bold(_) => !c.is_whitespace() && !bold(props),
                Expectation::Italic(_) => !c.is_whitespace() && !italic(props),
                Expectation::Underlined(_) => !c.is_whitespace() && !underlined(props),
                Expectation::Plain(_) => bold(props) || italic(props) || underlined(props),
                Expectation::Style(..) => false,
            })
            .map(|(c, _)| *c)
            .collect();
        if !wrong.is_empty() {
            failures.push(format!(
                "line {line}: in \"{text}\", {:?} is not {}",
                wrong.iter().collect::<String>(),
                match &expectation {
                    Expectation::Bold(_) => "bold",
                    Expectation::Italic(_) => "italic",
                    Expectation::Underlined(_) => "underlined",
                    _ => "plain",
                }
            ));
        }
        if let Expectation::Style(id, _) = &expectation {
            let wanted = document.styles.lookup(id);
            if wanted.is_none() || paragraph.props.style != wanted {
                failures.push(format!(
                    "line {line}: the paragraph holding \"{text}\" is not in {id}"
                ));
            }
        }
    }
    Ok(failures)
}

/// The text of a `type` line: verbatim, or between double quotes where the
/// spaces at its ends matter — an editor that trims a line's end would
/// otherwise eat the space before the next word.
fn quoted(rest: &str) -> String {
    match rest
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
    {
        Some(inner) if rest.len() >= 2 => inner.to_owned(),
        _ => rest.to_owned(),
    }
}

/// A new document with the script run through it, as the user would have left
/// it before saving.
pub fn author(text: &str) -> Result<Scriva, String> {
    let steps = parse(text)?;
    let mut app = Scriva::new();
    for (line, step) in steps {
        match step {
            Step::Type(text) => app.type_text(&text),
            Step::Key(key, modifiers) => app.key(key, modifiers),
            Step::Table(rows, columns) => app.insert_table(rows, columns),
            Step::Style(id) => {
                let style = app
                    .document
                    .styles
                    .lookup(&id)
                    .ok_or_else(|| format!("line {line}: no style `{id}` in a new document"))?;
                app.run(Command::Style(style));
            }
            Step::Run(command) => app.run(command),
            Step::Expect(_) => continue,
        }
        if let Some((title, why)) = app.message.take() {
            return Err(format!(
                "line {line}: the application said \"{title}: {why}\""
            ));
        }
    }
    Ok(app)
}

/// Runs the script at `script` and saves what it made as `out`, a `.docx`.
pub fn write(script: &Path, out: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(script).map_err(|e| format!("{}: {e}", script.display()))?;
    if !out
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("docx"))
    {
        return Err(format!("{} is not a .docx", out.display()));
    }
    let mut app = author(&text).map_err(|why| format!("{}: {why}", script.display()))?;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    if app.save_to(out.to_path_buf()) {
        return Ok(());
    }
    Err(match app.message.take() {
        Some((title, why)) => format!("{}: {title}: {why}", out.display()),
        None => format!("{}: the save did not happen", out.display()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .canonicalize()
            .expect("the repository root")
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("scriva-author-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    /// The document in the corpus is what the application writes *today*. It
    /// was authored by `cargo xtask author` from the script beside it, and
    /// Word's reading of it is committed with it; the layout check holds one
    /// to the other. So a change to what Scriva writes — a writer, a default,
    /// an editing command — shows up here first, and the way through is to
    /// author it again and let Word read the new one, not to edit the file.
    #[test]
    fn the_corpus_document_is_what_the_application_authors_today() {
        let root = repo_root();
        let script = root.join("corpus").join("scriva-authored.txt");
        let kept = root
            .join("corpus")
            .join("docx")
            .join("scriva-authored.docx");
        let out = scratch("corpus").join("scriva-authored.docx");
        write(&script, &out).expect("the script runs");
        let now = std::fs::read(&out).expect("the document was written");
        let was = std::fs::read(&kept).unwrap_or_default();
        assert!(
            now == was,
            "what Scriva writes has changed: corpus/docx/scriva-authored.docx is {} bytes \
             and the same script authors {} today. Run `cargo xtask author` — it rewrites the \
             document and renews Word's reading of it — then `cargo xtask compare --record` \
             if LAYOUT.md should hold the new numbers, and commit all three.",
            was.len(),
            now.len()
        );
    }

    /// The document in the corpus is held to what its script says it
    /// meant, read back from the file: the heading is a heading, the bold
    /// word bold, the words around it plain to their spaces.
    #[test]
    fn the_authored_document_carries_what_its_script_meant() {
        let root = repo_root();
        let script = std::fs::read_to_string(root.join("corpus").join("scriva-authored.txt"))
            .expect("the script");
        let stated = parse(&script)
            .expect("the script parses")
            .into_iter()
            .filter(|(_, step)| matches!(step, Step::Expect(_)))
            .count();
        assert!(stated >= 8, "the script says what it means: {stated} lines");
        let (document, _) = wp_docx::open(
            root.join("corpus")
                .join("docx")
                .join("scriva-authored.docx"),
        )
        .expect("the corpus document opens");
        let unmet = unmet(&script, &document).expect("the script parses");
        assert!(
            unmet.is_empty(),
            "corpus/docx/scriva-authored.docx is not what its script meant:\n{}",
            unmet.join("\n")
        );
    }

    /// A document with the fault the corpus document had — a bold space
    /// before a word that is not bold — fails its expectations, and says
    /// which line and which letters; so does a text that is not there.
    #[test]
    fn an_expectation_the_document_breaks_is_reported() {
        let dir = scratch("expect");
        let script = "type Some\nbold\ntype \" bold\"\nbold\ntype \" after\"\n\
                      expect plain \"Some \"\n\
                      expect bold \"bold\"\n\
                      expect plain \" after\"\n\
                      expect italic \"bold\"\n\
                      expect style Heading1 \"Some\"\n\
                      expect bold \"missing\"\n";
        let path = dir.join("s.txt");
        std::fs::write(&path, script).unwrap();
        let out = dir.join("broken.docx");
        write(&path, &out).expect("authored, the expectations aside");
        let (document, _) = wp_docx::open(&out).expect("it opens");
        let unmet = unmet(script, &document).unwrap();
        assert_eq!(unmet.len(), 4, "{unmet:#?}");
        assert!(
            unmet[0].starts_with("line 6:") && unmet[0].contains("\" \""),
            "the bold space: {}",
            unmet[0]
        );
        assert!(unmet[1].starts_with("line 9:") && unmet[1].contains("italic"));
        assert!(unmet[2].starts_with("line 10:") && unmet[2].contains("Heading1"));
        assert!(unmet[3].starts_with("line 11:") && unmet[3].contains("not in the document"));
        assert!(
            parse("expect bold unquoted").is_err(),
            "a text without quotes is refused"
        );
        assert!(parse("expect shiny \"x\"").is_err());
    }

    /// Two runs of one script are one file. Nothing else here holds unless
    /// this does: a timestamp or an id that differed between runs would make
    /// the test above fail on every machine, and be worked around rather than
    /// understood.
    #[test]
    fn authoring_the_same_script_twice_writes_the_same_bytes() {
        let dir = scratch("twice");
        let script = dir.join("s.txt");
        std::fs::write(&script, "type one\nkey Enter\ntable 2 2\ntype A1\n").unwrap();
        let first = dir.join("first.docx");
        let second = dir.join("second.docx");
        write(&script, &first).unwrap();
        write(&script, &second).unwrap();
        assert_eq!(
            std::fs::read(first).unwrap(),
            std::fs::read(second).unwrap()
        );
    }

    #[test]
    fn a_script_runs_the_commands_it_names() {
        let app = author(
            "style Heading1\ntype A heading\nkey Enter\ntype \"body \"\nbold\ntype bold\n\
             key Enter\nbullets\ntype item\nkey Enter\nkey Enter\ntable 2 3\ntype A1\nkey Tab\ntype B1",
        )
        .unwrap();
        let paragraphs = app.document.paragraphs();
        assert_eq!(paragraphs[0].text(), "A heading");
        let heading = app.document.styles.lookup("Heading1").unwrap();
        assert_eq!(paragraphs[0].props.style, Some(heading));
        assert_eq!(paragraphs[1].text(), "body bold");
        assert!(
            paragraphs[1].runs().len() >= 2,
            "the bold word is its own run"
        );
        assert_eq!(paragraphs[2].text(), "item");
        assert!(paragraphs[2].props.numbering.is_some(), "a bullet");
        assert!(
            paragraphs[3].props.numbering.is_none(),
            "Enter on the empty item ended the list"
        );
        let table = app
            .document
            .body
            .iter()
            .find_map(|block| match block {
                wp_model::doc::Block::Table(table) => Some(table),
                _ => None,
            })
            .expect("a table");
        assert_eq!((table.rows.len(), table.rows[0].cells.len()), (2, 3));
        assert_eq!(
            table.rows[0].cells[1].text(),
            "B1",
            "Tab moved to the next cell"
        );
    }

    #[test]
    fn a_fault_in_the_script_names_its_line() {
        let refused = parse("type fine\n\n# a comment\nkey ctrl+Whatever\n").unwrap_err();
        assert!(refused.starts_with("line 4:"), "{refused}");
        assert!(refused.contains("Whatever"), "{refused}");
        let refused = parse("bold now\n").unwrap_err();
        assert!(refused.contains("takes nothing"), "{refused}");
        let refused = author("style Nowhere\n").err().expect("no such style");
        assert!(refused.contains("Nowhere"), "{refused}");
        assert_eq!(
            parse("key shift+Tab").unwrap()[0].1,
            Step::Key(egui::Key::Tab, egui::Modifiers::SHIFT)
        );
        assert_eq!(
            parse("type \"  spaced  \"").unwrap()[0].1,
            Step::Type("  spaced  ".into())
        );
    }
}
