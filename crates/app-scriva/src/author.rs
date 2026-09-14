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
            let (key, modifiers) = key(rest)?;
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
        other => return Err(format!("`{other}` is not a command this script knows")),
    })
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

/// `ctrl+shift+Home`: modifiers in front, egui's own key names last.
fn key(spec: &str) -> Result<(egui::Key, egui::Modifiers), String> {
    let mut modifiers = egui::Modifiers::NONE;
    let mut parts = spec.split('+').peekable();
    let mut name = None;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            name = Some(part);
            break;
        }
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmd" => modifiers = modifiers.plus(egui::Modifiers::COMMAND),
            "shift" => modifiers = modifiers.plus(egui::Modifiers::SHIFT),
            "alt" => modifiers = modifiers.plus(egui::Modifiers::ALT),
            other => return Err(format!("`{other}` is not a modifier (ctrl, shift, alt)")),
        }
    }
    let name = name
        .filter(|n| !n.is_empty())
        .ok_or("`key` wants a key name")?;
    let key = egui::Key::from_name(name).ok_or_else(|| {
        format!("`{name}` is not a key name egui knows (Enter, Tab, ArrowDown, Home, A)")
    })?;
    Ok((key, modifiers))
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
