//! What the assistant is shown of a document and what it may do to it: the
//! words a request is sent in, the four tools, and the proposals they leave.
//!
//! **The helper names paragraphs by number.** They are numbered from 1
//! through the text's paragraphs in the order the page shows them, table
//! cells included, which is the walk `Document::paragraphs` makes; each is
//! shown as one line of Markdown (`wp_text::markdown::line`), as it reads
//! with its changes accepted. What a line cannot show — a picture, a note's
//! reference, a field, an equation, a link, a page break — is named beside
//! the lines, and a paragraph holding one is not rewritten, since new words
//! could not keep it.
//!
//! **Every change is a proposal.** The tools edit through the functions
//! Track Changes records with, by an author named "Assistant", whatever the
//! document's own Track Changes setting says. Each proposal carries a time of
//! its own, so that its changes join neither a person's nor another
//! proposal's, and its card settles exactly them. One proposal is one step to
//! undo, and so is settling one.
//!
//! **A proposal is shaped so that either answer is exact.** Paragraphs
//! rewritten are struck whole, their marks too, and the new paragraphs stand
//! after them as insertions of their own, before the next paragraph: Accept
//! leaves the new paragraphs, Reject the old, and neither needs a formatting
//! change. Only where the last paragraph of a container is rewritten — its
//! mark cannot go — does the last new paragraph take that mark over, with its
//! properties changed as a tracked formatting change; and there, a formatting
//! change already open on the paragraph stops the proposal, since a paragraph
//! remembers one such change and not two.
//!
//! **A rewrite keeps the look of what it rewrites.** Each new paragraph takes
//! the properties of the one it replaces, a heading's style among them, and
//! its words the formatting of that paragraph's plain words, with `**` and
//! `*` made bold and italic. A line that begins with `#` is a heading of that
//! level, in the document's own style for it, and a plain line added after a
//! heading is body text.

use std::sync::Arc;

use ::assist::{Tool, ToolCall, ToolResult};
use serde_json::{json, Value};
use wp_model::doc::{Document, Inline, Paragraph, Piece};
use wp_model::prop::{ParaProps, RunProps, Toggle};
use wp_model::style::{StyleId, StyleKind};
use wp_model::{Mark, Scope};

use crate::edit::{Caret, Change, History, Selection};
use crate::revise::{self, Author, Resolve};

/// Who a proposal is by: on the page, in Review, and in the file.
pub const AUTHOR: &str = "Assistant";

/// The most paragraphs one reading hands back.
pub const MOST_READ: usize = 200;

/// The most paragraphs one proposal replaces or puts in: a proposal is a
/// passage, and a document rewritten whole is not one.
pub const MOST_WRITTEN: usize = 50;

/// How many paragraphs a request shows on each side of what it is about.
pub const AROUND: usize = 2;

/// What a request is about, as the pane's scope chips name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum About {
    Selection,
    Paragraph,
    Document,
}

impl About {
    /// The chips, in the order the pane shows them.
    pub const ALL: [About; 3] = [About::Selection, About::Paragraph, About::Document];

    pub fn name(self) -> &'static str {
        match self {
            About::Selection => "Selection",
            About::Paragraph => "Paragraph",
            About::Document => "Whole document",
        }
    }
}

/// The tools the helper may ask for, each with a schema it must keep to.
pub fn tools() -> Vec<Tool> {
    let range = |extra: (&str, Value)| {
        let mut properties = serde_json::Map::new();
        properties.insert("first".into(), json!({"type": "integer"}));
        properties.insert("last".into(), json!({"type": "integer"}));
        properties.insert(extra.0.into(), extra.1);
        properties
    };
    vec![
        Tool::new(
            "read_paragraphs",
            "Read paragraphs first to last of the document, numbered from 1, each as a line \
             of Markdown after its number in brackets.",
            json!({"type": "object", "additionalProperties": false,
                   "required": ["first", "last"],
                   "properties": {"first": {"type": "integer"}, "last": {"type": "integer"}}}),
        ),
        Tool::new(
            "replace_paragraphs",
            "Propose new text for paragraphs first to last, as Markdown with one paragraph \
             per line and no numbers, written as the paragraphs are shown. The person sees \
             it as a tracked change to accept or reject. Each new paragraph keeps the style \
             of the one it replaces. Empty markdown proposes taking the paragraphs out.",
            json!({"type": "object", "additionalProperties": false,
                   "required": ["first", "last", "markdown"],
                   "properties": range(("markdown", json!({"type": "string"})))}),
        ),
        Tool::new(
            "insert_paragraphs",
            "Propose new paragraphs after paragraph `after`, or before the first when \
             `after` is 0, as Markdown with one paragraph per line. The person sees them \
             as a tracked change to accept or reject.",
            json!({"type": "object", "additionalProperties": false,
                   "required": ["after", "markdown"],
                   "properties": {"after": {"type": "integer"},
                                  "markdown": {"type": "string"}}}),
        ),
        Tool::new(
            "comment",
            "Put a comment on paragraphs first to last without changing them: for \
             reviewing, checking, or saying what could be better.",
            json!({"type": "object", "additionalProperties": false,
                   "required": ["first", "last", "text"],
                   "properties": range(("text", json!({"type": "string"})))}),
        ),
    ]
}

/// What the pane is set up with.
pub fn setup() -> ui_kit::assist::Setup {
    ui_kit::assist::Setup {
        system: ::assist::prompt::scriva(),
        tools: tools(),
    }
}

/// The author a proposal made at `date` is by.
pub fn author_at(date: &str) -> Author {
    Author {
        date: Some(date.into()),
        ..Author::new(AUTHOR)
    }
}

// ---------------------------------------------------------------- the words

/// How many paragraphs the text has.
fn count(document: &Document) -> usize {
    document.paragraphs_in(Scope::Body).len()
}

/// Paragraph `index` (zero-based) as the helper reads it: its number and its
/// line of Markdown.
fn numbered(document: &Document, paragraph: &Paragraph, index: usize) -> String {
    let line = wp_text::markdown::line(document, &revise::accepted(paragraph));
    format!("[{}] {line}", index + 1)
}

/// Paragraphs `from..=to`, a numbered line each, and a sentence naming those
/// that hold what a line cannot show.
fn shown(document: &Document, from: usize, to: usize) -> String {
    let paragraphs = document.paragraphs_in(Scope::Body);
    let mut out: Vec<String> = Vec::new();
    let mut holding: Vec<usize> = Vec::new();
    for index in from..=to {
        let Some(paragraph) = paragraphs.get(index) else {
            break;
        };
        out.push(numbered(document, paragraph, index));
        if unseen(paragraph).is_some() {
            holding.push(index);
        }
    }
    let mut text = out.join("\n");
    if !holding.is_empty() {
        let numbers: Vec<String> = holding
            .iter()
            .map(|index| (index + 1).to_string())
            .collect();
        let (which, hold) = match numbers.len() {
            1 => (format!("Paragraph {}", numbers[0]), "holds"),
            _ => (
                format!(
                    "Paragraphs {} and {}",
                    numbers[..numbers.len() - 1].join(", "),
                    numbers[numbers.len() - 1]
                ),
                "hold",
            ),
        };
        text.push_str(&format!(
            "\n({which} {hold} a picture, a note, a field, an equation, a link or a break that \
             its line does not show and new wording could not keep: comment on it rather \
             than rewrite it.)"
        ));
    }
    text
}

/// "Here is paragraph 12:", "Here are paragraphs 10 to 14:".
fn here(from: usize, to: usize) -> String {
    match from == to {
        true => format!("Here is paragraph {}:", from + 1),
        false => format!("Here are paragraphs {} to {}:", from + 1, to + 1),
    }
}

/// "paragraph 12", "paragraphs 12 to 14", from zero-based indices.
fn named(from: usize, to: usize) -> String {
    match from == to {
        true => format!("paragraph {}", from + 1),
        false => format!("paragraphs {} to {}", from + 1, to + 1),
    }
}

/// The same, as a transcript line or a card writes it: "paragraphs 12–14".
fn short(from: usize, to: usize) -> String {
    match from == to {
        true => format!("paragraph {}", from + 1),
        false => format!("paragraphs {}\u{2013}{}", from + 1, to + 1),
    }
}

/// The text's paragraphs a selection is about, zero-based and inclusive: the
/// ones it touches, less a last one it only reaches the start of.
pub fn selected(selection: Selection) -> (usize, usize) {
    let (start, end) = selection.ordered();
    let last = match end.offset == 0 && end.paragraph > start.paragraph {
        true => end.paragraph - 1,
        false => end.paragraph,
    };
    (start.paragraph, last)
}

/// A request in the words it is sent in: the document's size and headings,
/// the paragraphs it is about with two on each side, and what was asked.
/// `selection` is where the person is in the text, and `words` how many
/// words the document has, as the status bar counts them.
pub fn request(
    document: &Document,
    selection: Selection,
    about: About,
    asked: &str,
    words: usize,
) -> String {
    let total = count(document);
    let last = total.saturating_sub(1);
    let mut out = format!(
        "The document has {} and {}.\n",
        plural(total, "paragraph"),
        plural(words, "word")
    );
    let headings: Vec<String> = document
        .paragraphs_in(Scope::Body)
        .iter()
        .enumerate()
        .filter(|(_, paragraph)| is_heading(document, paragraph))
        .map(|(index, paragraph)| numbered(document, paragraph, index))
        .collect();
    match headings.is_empty() {
        true => out.push_str("It has no headings.\n"),
        false => {
            out.push_str("Its headings:\n");
            for heading in headings {
                out.push_str(&heading);
                out.push('\n');
            }
        }
    }
    out.push('\n');
    let (from, to) = match about {
        About::Document => (0, last),
        About::Selection if !selection.is_empty() => selected(selection),
        _ => (selection.head.paragraph, selection.head.paragraph),
    };
    let (from, to) = (from.min(last), to.min(last));
    match about {
        About::Document => {
            out.push_str("The request is about the whole document.\n");
            out.push_str(&here(0, last));
            out.push('\n');
            out.push_str(&shown(document, 0, last));
        }
        _ => {
            let what = match (about, selection.is_empty()) {
                (About::Selection, false) => "the selection",
                _ => "where the caret is",
            };
            let (around_from, around_to) = (from.saturating_sub(AROUND), (to + AROUND).min(last));
            out.push_str(&format!(
                "The request is about {}, {what}. {}\n",
                named(from, to),
                here(around_from, around_to)
            ));
            out.push_str(&shown(document, around_from, around_to));
            if about == About::Selection {
                if let Some(chosen) = selected_words(document, selection) {
                    out.push_str(&format!("\nThe words selected: \u{201c}{chosen}\u{201d}"));
                }
            }
        }
    }
    out.push_str(&format!("\n\nThe request: {asked}"));
    out
}

/// What goes with a request, in a phrase, for the question asked before
/// anything is sent elsewhere.
pub fn leaves(about: About, words: usize) -> String {
    match about {
        About::Selection => {
            "The selected paragraphs, the two on each side, and the document's headings".to_owned()
        }
        About::Paragraph => {
            "The paragraph at the caret, the two on each side, and the document's headings"
                .to_owned()
        }
        About::Document => format!("The whole document, {}", plural(words, "word")),
    }
}

/// The words of a selection within one paragraph that it does not cover
/// whole, where they say more than the paragraph's number does — on one
/// line, whatever breaks they hold.
fn selected_words(document: &Document, selection: Selection) -> Option<String> {
    let (start, end) = selection.ordered();
    if selection.is_empty() || start.paragraph != end.paragraph {
        return None;
    }
    let paragraphs = document.paragraphs_in(Scope::Body);
    let text = crate::text::content(paragraphs.get(start.paragraph)?);
    let words = text.get(start.offset..end.offset)?;
    (words.len() < text.len()).then(|| {
        words
            .chars()
            .filter(|c| *c != wp_model::doc::OBJECT)
            .map(
                |c| match c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
                    true => ' ',
                    false => c,
                },
            )
            .collect()
    })
}

fn plural(count: usize, what: &str) -> String {
    let number = crate::app::notices::thousands(count);
    match count {
        1 => format!("1 {what}"),
        _ => format!("{number} {what}s"),
    }
}

fn capital(text: &str) -> String {
    let mut letters = text.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}

/// What a paragraph holds that its line of Markdown does not show, and new
/// wording would lose: a picture, a note's reference, a field, an equation,
/// a link, a content control, a page or column break.
pub fn unseen(paragraph: &Paragraph) -> Option<&'static str> {
    fn walk(content: &[Inline]) -> Option<&'static str> {
        for inline in content {
            let found = match inline {
                Inline::Run(run) => run.content.iter().find_map(|piece| match piece {
                    Piece::Drawing(_) | Piece::Embedded { .. } => Some(HOLDS_PICTURE),
                    Piece::FootnoteRef { .. } | Piece::EndnoteRef { .. } => Some(HOLDS_NOTE),
                    Piece::FieldStart { .. } => Some(HOLDS_FIELD),
                    Piece::Break(wp_model::doc::Break::Page | wp_model::doc::Break::Column) => {
                        Some(HOLDS_BREAK)
                    }
                    _ => None,
                }),
                Inline::Math(_) => Some(HOLDS_EQUATION),
                Inline::SimpleField { .. } => Some(HOLDS_FIELD),
                Inline::Hyperlink(_) => Some(HOLDS_LINK),
                Inline::Structured(_) => Some(HOLDS_CONTROL),
                Inline::Revised { content, .. } | Inline::Wrapper { content, .. } => walk(content),
                Inline::Anchor(_) => None,
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }
    walk(&revise::accepted(paragraph).content)
}

const HOLDS_PICTURE: &str = "a picture";
const HOLDS_NOTE: &str = "a footnote or endnote";
const HOLDS_FIELD: &str = "a field";
const HOLDS_BREAK: &str = "a page or column break";
const HOLDS_EQUATION: &str = "an equation";
const HOLDS_LINK: &str = "a link";
const HOLDS_CONTROL: &str = "a content control";
const UNSEEN: [&str; 7] = [
    HOLDS_PICTURE,
    HOLDS_NOTE,
    HOLDS_FIELD,
    HOLDS_BREAK,
    HOLDS_EQUATION,
    HOLDS_LINK,
    HOLDS_CONTROL,
];

/// What stops a take-out that would change nothing.
const NOTHING_THERE: &str = "nothing there to take out";

// ---------------------------------------------------------------- the tools

/// What running a tool came to.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    /// What goes back to the helper.
    pub result: ToolResult,
    /// What the transcript says was done: "proposed new wording for
    /// paragraph 12".
    pub line: Option<String>,
    /// What the person is asked to settle.
    pub proposal: Option<Proposal>,
    /// How the text's paragraphs moved, for a caret standing in them.
    pub moved: Moved,
}

/// How a proposal moved the text's paragraphs, for the carets in them: a
/// caret in paragraph `reset` goes to its start, where what is typed is
/// ordinary text; and one at paragraph `from` or after moves by `by`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Moved {
    pub reset: Option<usize>,
    pub from: usize,
    pub by: isize,
}

impl Moved {
    /// Where a caret in the text stands after the proposal.
    pub fn caret(self, caret: Caret) -> Caret {
        let mut caret = caret;
        if self.reset == Some(caret.paragraph) {
            caret.offset = 0;
        }
        if caret.paragraph >= self.from {
            caret.paragraph = caret.paragraph.saturating_add_signed(self.by);
        }
        caret
    }
}

/// A change the assistant proposed, for its card.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    pub kind: Kind,
    pub title: String,
    pub body: String,
}

/// What a card settles.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// The tracked changes made at this time.
    Changes(Arc<str>),
    /// A comment, by its id and the time it was made at: an id a deleted
    /// comment had is given to the next one.
    Comment { id: u32, date: Arc<str> },
}

/// Runs a call against the document, recording what it changes as `author`
/// — the assistant, at a time of this call's own — on `history` as one step.
pub fn run(
    document: &mut Document,
    history: &mut History,
    call: &ToolCall,
    author: &Author,
) -> Done {
    let done = match call.name.as_str() {
        "read_paragraphs" => read_call(document, call),
        "replace_paragraphs" => replace_call(document, history, call, author),
        "insert_paragraphs" => insert_call(document, history, call, author),
        "comment" => comment_call(document, history, call, author),
        other => Err(format!(
            "There is no tool called {other}: the tools are read_paragraphs, \
             replace_paragraphs, insert_paragraphs and comment."
        )),
    };
    done.unwrap_or_else(|why| refusal(call, why))
}

fn refusal(call: &ToolCall, why: String) -> Done {
    Done {
        result: ToolResult::error(call, why),
        line: None,
        proposal: None,
        moved: Moved::default(),
    }
}

/// Whether a call changes the document, and so needs the paragraph numbers
/// the helper was given to still be right.
pub fn edits(call: &ToolCall) -> bool {
    call.name != "read_paragraphs"
}

/// What a call that would change the document answers when the person has
/// changed it since the helper was shown it: nothing, and why.
pub fn changed_meanwhile(call: &ToolCall) -> Done {
    Done {
        line: Some("changed nothing: the document was edited meanwhile".to_owned()),
        ..refusal(
            call,
            "Nothing was changed: the person edited the document while you were working, so \
             the paragraph numbers you have may no longer be right. Say in a sentence what \
             you would have done, and stop; the person can ask again."
                .to_owned(),
        )
    }
}

fn number(call: &ToolCall, name: &str) -> Result<usize, String> {
    call.input
        .get(name)
        .and_then(Value::as_i64)
        .and_then(|number| usize::try_from(number).ok())
        .ok_or_else(|| format!("`{name}` must be a whole number, 0 or more."))
}

fn words<'a>(call: &'a ToolCall, name: &str) -> Result<&'a str, String> {
    call.input
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("`{name}` must be text."))
}

/// A range of paragraphs the helper named, as zero-based indices, or what is
/// wrong with it.
fn range(document: &Document, call: &ToolCall) -> Result<(usize, usize), String> {
    let (first, last) = (number(call, "first")?, number(call, "last")?);
    let total = count(document);
    if first < 1 || last < first || last > total {
        return Err(format!(
            "There are no paragraphs {first} to {last}: the document's paragraphs are \
             numbered 1 to {total}."
        ));
    }
    Ok((first - 1, last - 1))
}

fn read_call(document: &Document, call: &ToolCall) -> Result<Done, String> {
    let (from, to) = range(document, call)?;
    let shown_to = to.min(from + MOST_READ - 1);
    let mut text = shown(document, from, shown_to);
    if shown_to < to {
        text.push_str(&format!(
            "\n({} not shown: ask for them.)",
            capital(&named(shown_to + 1, to))
        ));
    }
    Ok(Done {
        result: ToolResult::ok(call, text),
        line: Some(format!("read {}", short(from, shown_to))),
        proposal: None,
        moved: Moved::default(),
    })
}

fn replace_call(
    document: &mut Document,
    history: &mut History,
    call: &ToolCall,
    author: &Author,
) -> Result<Done, String> {
    let (from, to) = range(document, call)?;
    let markdown = words(call, "markdown")?;
    let date = author.date.clone().unwrap_or_default();
    if to - from + 1 > MOST_WRITTEN {
        return Err(format!(
            "One proposal changes at most {MOST_WRITTEN} paragraphs; propose these a \
             passage at a time."
        ));
    }
    let lines = read_lines(markdown);
    if lines.len() > MOST_WRITTEN {
        return Err(format!(
            "One proposal puts in at most {MOST_WRITTEN} paragraphs; propose these a \
             passage at a time."
        ));
    }
    let mut notes = Vec::new();
    let moved = replace(document, history, from, to, &lines, author, &mut notes)
        .map_err(|why| refused(why, from, to))?;
    let (line, body) = match lines.is_empty() {
        true => (
            format!("proposed taking out {}", short(from, to)),
            "Taken out.".to_owned(),
        ),
        false => (
            format!("proposed new wording for {}", short(from, to)),
            preview(markdown),
        ),
    };
    let mut said = proposed(to, moved.by);
    for note in notes {
        said.push(' ');
        said.push_str(&note);
    }
    Ok(Done {
        result: ToolResult::ok(call, said),
        line: Some(line),
        proposal: Some(Proposal {
            kind: Kind::Changes(date),
            title: capital(&short(from, to)),
            body,
        }),
        moved,
    })
}

fn insert_call(
    document: &mut Document,
    history: &mut History,
    call: &ToolCall,
    author: &Author,
) -> Result<Done, String> {
    let after = number(call, "after")?;
    let total = count(document);
    if after > total {
        return Err(format!(
            "There is no paragraph {after}: the document's paragraphs are numbered 1 to \
             {total}, and 0 puts new ones before the first."
        ));
    }
    let markdown = words(call, "markdown")?;
    let lines = read_lines(markdown);
    if lines.is_empty() {
        return Err("There is nothing to put in: the markdown has no paragraphs.".to_owned());
    }
    if lines.len() > MOST_WRITTEN {
        return Err(format!(
            "One proposal puts in at most {MOST_WRITTEN} paragraphs; propose these a \
             passage at a time."
        ));
    }
    let date = author.date.clone().unwrap_or_default();
    let where_ = match after {
        0 => "before paragraph 1".to_owned(),
        n => format!("after paragraph {n}"),
    };
    let mut notes = Vec::new();
    let moved = insert(document, history, after, &lines, author, &mut notes).map_err(|why| {
        let at = after.saturating_sub(1);
        refused(why, at, at)
    })?;
    let new = plural(lines.len(), "new paragraph");
    let mut said = format!(
        "Proposed {new} {where_}, for the person to accept or reject. They are paragraphs {} \
         to {}, and the paragraphs after them are numbered {} higher.",
        after + 1,
        after + lines.len(),
        lines.len()
    );
    for note in notes {
        said.push(' ');
        said.push_str(&note);
    }
    Ok(Done {
        result: ToolResult::ok(call, said),
        line: Some(format!("proposed {new} {where_}")),
        proposal: Some(Proposal {
            kind: Kind::Changes(date),
            title: capital(&where_),
            body: preview(markdown),
        }),
        moved,
    })
}

fn comment_call(
    document: &mut Document,
    history: &mut History,
    call: &ToolCall,
    author: &Author,
) -> Result<Done, String> {
    let (from, to) = range(document, call)?;
    let text = words(call, "text")?.trim();
    if text.is_empty() {
        return Err("A comment needs words: `text` is empty.".to_owned());
    }
    let end = document
        .paragraphs_in(Scope::Body)
        .get(to)
        .map(|paragraph| crate::text::len(paragraph))
        .unwrap_or(0);
    let selection = Selection {
        anchor: Caret {
            paragraph: from,
            offset: 0,
        },
        head: Caret {
            paragraph: to,
            offset: end,
        },
    };
    let id = revise::add_comment(
        document,
        history,
        Scope::Body,
        selection,
        &author.name,
        &author.initials,
        text,
    );
    // Dated, as Word dates a comment, and so told apart from one that had
    // its id before.
    let date = author.date.clone().unwrap_or_default();
    if let Some(comment) = document.comments.iter_mut().find(|c| c.id == id) {
        comment.date = Some(date.clone());
    }
    Ok(Done {
        result: ToolResult::ok(
            call,
            format!(
                "Commented on {}. The person sees it beside them.",
                named(from, to)
            ),
        ),
        line: Some(format!("commented on {}", short(from, to))),
        proposal: Some(Proposal {
            kind: Kind::Comment { id, date },
            title: format!("Comment on {}", short(from, to)),
            body: preview(text),
        }),
        moved: Moved::default(),
    })
}

/// What a proposal says back: that it waits for the person, and how the
/// numbers of the paragraphs after it moved.
fn proposed(to: usize, shift: isize) -> String {
    let mut said =
        "Proposed: the person sees it as a tracked change, and accepts or rejects it.".to_owned();
    match shift {
        0 => {}
        n if n > 0 => said.push_str(&format!(
            " The paragraphs after {} are now numbered {n} higher.",
            to + 1
        )),
        n => said.push_str(&format!(
            " The paragraphs after {} are now numbered {} lower.",
            to + 1,
            -n
        )),
    }
    said
}

/// What a refusal means to the helper, in words it can act on.
fn refused(why: &str, from: usize, to: usize) -> String {
    let which = named(from, to);
    match why {
        revise::ACROSS_BLOCKS => format!(
            "Nothing was proposed for {which}: a table or a cell's edge stands among them, \
             and a proposal cannot reach across it. Propose changes to the paragraphs on one \
             side of it at a time."
        ),
        revise::CANNOT_RECORD => format!(
            "Nothing was proposed for {which}: a link, a field or a content control stands \
             there, which a proposal cannot change. Leave it, or comment instead."
        ),
        revise::FORMATTING_OPEN => format!(
            "Nothing was proposed for {which}: a change to its formatting is still open, and \
             a proposal there would have to change its formatting again. The person can \
             settle that change first; until then, comment instead."
        ),
        NOTHING_THERE => {
            format!("Nothing was proposed for {which}: there is nothing in it to take out.")
        }
        holds if UNSEEN.contains(&holds) => format!(
            "Nothing was proposed for {which}: it holds {holds}, which new wording could not \
             keep. Comment on it instead, or leave it."
        ),
        other => format!("Nothing was proposed for {which}: {other}."),
    }
}

/// The first words of what a card shows.
fn preview(text: &str) -> String {
    const MOST: usize = 280;
    let text = text.trim();
    match text.char_indices().nth(MOST) {
        Some((at, _)) => format!("{}\u{2026}", &text[..at]),
        None => text.to_owned(),
    }
}

// ------------------------------------------------------------ the proposals

/// A line of the helper's Markdown: a heading's level, if it is one, and its
/// words with their emphasis.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub heading: Option<u8>,
    pub content: Vec<Inline>,
}

/// The helper's Markdown as paragraphs, a line each, as the request shows
/// them: a paragraph number echoed at a line's start is not part of the
/// text, and a tab is a tab.
pub fn read_lines(markdown: &str) -> Vec<Line> {
    let cleaned: Vec<&str> = markdown.lines().map(unnumbered).collect();
    wp_text::markdown::read_lines(&cleaned.join("\n"))
        .into_iter()
        .map(|(heading, paragraph)| Line {
            heading,
            content: tabs(paragraph.content),
        })
        .collect()
}

/// A line without the `[12] ` the request put before it. One written
/// `\[12]` is text.
fn unnumbered(line: &str) -> &str {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix('[') else {
        return line;
    };
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    match rest[digits..].strip_prefix(']') {
        Some(after) if digits > 0 => after.strip_prefix(' ').unwrap_or(after),
        _ => line,
    }
}

/// Runs whose text holds tabs, with the tabs as tabs.
fn tabs(content: Vec<Inline>) -> Vec<Inline> {
    content
        .into_iter()
        .map(|inline| match inline {
            Inline::Run(mut run) => {
                run.content = run
                    .content
                    .into_iter()
                    .flat_map(|piece| match piece {
                        Piece::Text(text) if text.contains('\t') => {
                            let mut pieces = Vec::new();
                            for (index, part) in text.split('\t').enumerate() {
                                if index > 0 {
                                    pieces.push(Piece::Tab);
                                }
                                if !part.is_empty() {
                                    pieces.push(Piece::Text(part.into()));
                                }
                            }
                            pieces
                        }
                        other => vec![other],
                    })
                    .collect();
                Inline::Run(run)
            }
            other => other,
        })
        .collect()
}

/// Whether a paragraph style reads as a heading, and at which level.
fn level_of(document: &Document, style: StyleId) -> Option<u8> {
    let probe = Paragraph {
        props: ParaProps {
            style: Some(style),
            ..ParaProps::default()
        },
        ..Paragraph::new()
    };
    wp_model::outline::heading_level(&probe, &document.styles)
}

/// The document's own paragraph style for a heading of `level`: `HeadingN`,
/// or one that reads as that level — or, where it has none, its style for
/// the nearest level above, and which level that is.
fn heading_style(document: &Document, level: u8) -> Option<(StyleId, u8)> {
    (1..=level.clamp(1, 9)).rev().find_map(|level| {
        document
            .styles
            .lookup(&format!("Heading{level}"))
            .filter(|style| level_of(document, *style) == Some(level))
            .or_else(|| {
                document
                    .styles
                    .iter()
                    .filter(|(_, style)| style.kind == StyleKind::Paragraph)
                    .map(|(id, _)| id)
                    .find(|id| level_of(document, *id) == Some(level))
            })
            .map(|style| (style, level))
    })
}

fn is_heading(document: &Document, paragraph: &Paragraph) -> bool {
    wp_model::outline::heading_level(paragraph, &document.styles).is_some()
}

/// The properties of body text after a heading, as Enter gives them: the
/// heading's, in the style its style names to follow it — or, where that is
/// none, or a heading itself, the document's ordinary paragraph style — and
/// with no level or list number of the heading's own.
fn after_heading(document: &Document, props: &ParaProps) -> ParaProps {
    let mut after = props.clone();
    let next = props
        .style
        .and_then(|style| document.styles.get(style))
        .and_then(|style| style.next)
        .filter(|next| level_of(document, *next).is_none());
    after.style = next.or_else(|| document.styles.default_style(StyleKind::Paragraph));
    after.outline_level = None;
    after.numbering = None;
    after
}

/// The formatting of a paragraph's plain words: its first run that is
/// neither bold nor italic, or its first run.
fn plain_props(paragraph: &Paragraph) -> RunProps {
    let runs = paragraph.runs();
    runs.iter()
        .find(|run| !run.props.bold() && !run.props.italic())
        .or(runs.first())
        .map(|run| run.props.clone())
        .unwrap_or_else(|| paragraph.props.mark.as_deref().cloned().unwrap_or_default())
}

/// A new paragraph: `props`, and the line's words in `base` made bold and
/// italic where the line says. A heading line takes the document's style of
/// its level, or the nearest it has, and `notes` says so where that is not
/// the level asked for.
fn built(
    document: &Document,
    line: &Line,
    mut props: ParaProps,
    base: &RunProps,
    notes: &mut Vec<String>,
) -> Paragraph {
    if let Some(level) = line.heading {
        match heading_style(document, level) {
            Some((style, found)) => {
                props.style = Some(style);
                props.outline_level = None;
                props.numbering = None;
                if found != level {
                    let note = format!(
                        "The document has no style for a level-{level} heading, so that heading \
                         is level {found}."
                    );
                    if !notes.contains(&note) {
                        notes.push(note);
                    }
                }
            }
            None => {
                let note = "The document has no heading styles, so the lines marked as \
                            headings are ordinary paragraphs."
                    .to_owned();
                if !notes.contains(&note) {
                    notes.push(note);
                }
            }
        }
    }
    let content = line
        .content
        .iter()
        .map(|inline| match inline {
            Inline::Run(run) => {
                let mut props = base.clone();
                if run.props.bold() {
                    props.toggles.set(Toggle::Bold, true);
                }
                if run.props.italic() {
                    props.toggles.set(Toggle::Italic, true);
                }
                Inline::Run(wp_model::doc::Run {
                    props,
                    ..run.clone()
                })
            }
            other => other.clone(),
        })
        .collect();
    Paragraph {
        props,
        content,
        ..Paragraph::new()
    }
}

fn paragraph(document: &Document, index: usize) -> Option<Paragraph> {
    crate::edit::paragraph_at(document, Scope::Body, index)
}

fn length(document: &Document, index: usize) -> usize {
    paragraph(document, index)
        .map(|paragraph| crate::text::len(&paragraph))
        .unwrap_or(0)
}

fn span(from: (usize, usize), to: (usize, usize)) -> Selection {
    Selection {
        anchor: Caret {
            paragraph: from.0,
            offset: from.1,
        },
        head: Caret {
            paragraph: to.0,
            offset: to.1,
        },
    }
}

fn beside(document: &Document, first: usize, second: usize) -> bool {
    crate::edit::side_by_side(document, Scope::Body, first..second + 1)
}

fn record(history: &mut History, first: usize, before: Vec<Paragraph>, now: usize) {
    history.push(Scope::Body, Change::Range { first, before, now });
}

/// Replaces paragraphs `from..=to` with `lines` as a proposal by `author`,
/// one step on `history`; with no lines, takes them out. Says how the text's
/// paragraphs moved, or why nothing was done — in which case nothing was.
pub fn replace(
    document: &mut Document,
    history: &mut History,
    from: usize,
    to: usize,
    lines: &[Line],
    author: &Author,
    notes: &mut Vec<String>,
) -> Result<Moved, &'static str> {
    if lines.is_empty() {
        return take_out(document, history, from, to, author, notes);
    }
    if !beside(document, from, to) {
        return Err(revise::ACROSS_BLOCKS);
    }
    let old: Vec<Paragraph> = (from..=to)
        .filter_map(|index| paragraph(document, index))
        .collect();
    if old.len() != to - from + 1 {
        return Err(revise::CANNOT_RECORD);
    }
    if let Some(holds) = old.iter().find_map(unseen) {
        return Err(holds);
    }
    let last_old = &old[old.len() - 1];
    revise::can_record(
        document,
        Scope::Body,
        span((from, 0), (to, crate::text::len(last_old))),
    )?;
    let new: Vec<Paragraph> = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let at = index.min(old.len() - 1);
            let props = match index >= old.len() && is_heading(document, &old[at]) {
                true => after_heading(document, &old[at].props),
                false => old[at].props.clone(),
            };
            built(document, line, props, &plain_props(&old[at]), notes)
        })
        .collect();
    let count = new.len();
    let mut scratch = History::new();

    // The paragraph after them stands beside them: they are struck whole,
    // marks and all, and the new ones go in before it.
    if beside(document, to, to + 1) {
        revise::delete_range(
            document,
            Scope::Body,
            &mut scratch,
            span((from, 0), (to + 1, 0)),
            author,
            false,
        )?;
        let next = revise::next_revision_id(document);
        let (new, _) = revise::as_inserted(new, author, next);
        if !crate::edit::insert_before(document, Scope::Body, to + 1, new) {
            crate::edit::replace_range(document, Scope::Body, from..to + 1, old);
            return Err(revise::CANNOT_RECORD);
        }
        record(history, from, old, to - from + 1 + count);
        return Ok(Moved {
            reset: None,
            from: to + 1,
            by: count as isize,
        });
    }

    // `to` ends its container, and its mark cannot go: the last new
    // paragraph takes it over.
    let target = last_old.clone();
    if let Some(change) = &target.prop_change {
        let restyled = count > 1 || look(&target.props) != look(&new[0].props);
        if restyled && !author.made(&change.mark) {
            return Err(revise::FORMATTING_OPEN);
        }
    }
    revise::delete_range(
        document,
        Scope::Body,
        &mut scratch,
        span((from, 0), (to, crate::text::len(&target))),
        author,
        false,
    )?;
    let Some(struck) = paragraph(document, to) else {
        crate::edit::replace_range(document, Scope::Body, from..to + 1, old);
        return Err(revise::CANNOT_RECORD);
    };
    let next = revise::next_revision_id(document);
    let (mut new, next) = revise::as_inserted(new, author, next);
    let mut built_up: Vec<Paragraph> = Vec::with_capacity(count);
    // The first new paragraph's words follow the struck ones in `to`.
    let first_new = new.remove(0);
    let mut head = struck.clone();
    head.content.extend(first_new.content);
    if count == 1 {
        if revise::restyle(&mut head, &first_new.props, author, next).is_err() {
            crate::edit::replace_range(document, Scope::Body, from..to + 1, old);
            return Err(revise::FORMATTING_OPEN);
        }
        built_up.push(head);
    } else {
        // A new paragraph of its own: its mark is new, its properties its
        // own; the old mark, with what is tracked on it, goes to the last.
        head.props = first_new.props;
        head.prop_change = None;
        head.mark_revision = first_new.mark_revision;
        head.mark_deleted = None;
        head.mark_change = None;
        head.section = None;
        let mut last = new.pop().expect("more than one");
        let wanted = std::mem::replace(&mut last.props, struck.props.clone());
        last.mark_revision = struck.mark_revision.clone();
        last.mark_deleted = struck.mark_deleted.clone();
        last.mark_change = struck.mark_change.clone();
        last.section = struck.section.clone();
        if revise::restyle(&mut last, &wanted, author, next).is_err() {
            crate::edit::replace_range(document, Scope::Body, from..to + 1, old);
            return Err(revise::FORMATTING_OPEN);
        }
        built_up.push(head);
        built_up.extend(new);
        built_up.push(last);
    }
    crate::edit::replace_range(document, Scope::Body, to..to + 1, built_up);
    record(history, from, old, to - from + count);
    Ok(Moved {
        reset: Some(to),
        from: to + 1,
        by: count as isize - 1,
    })
}

/// A paragraph's properties without its mark's formatting.
fn look(props: &ParaProps) -> ParaProps {
    ParaProps {
        mark: None,
        ..props.clone()
    }
}

/// Takes paragraphs `from..=to` out, as a proposal: their text and their
/// marks deleted. Where the last of them ends its container, and so keeps
/// its mark, the mark before them goes instead, and what is left looks like
/// the paragraph before them, as Backspace leaves it; where they are the
/// whole container, an empty paragraph stays.
fn take_out(
    document: &mut Document,
    history: &mut History,
    from: usize,
    to: usize,
    author: &Author,
    notes: &mut Vec<String>,
) -> Result<Moved, &'static str> {
    if !beside(document, from, to) {
        return Err(revise::ACROSS_BLOCKS);
    }
    // **A paragraph mark goes only where there is a neighbour to join.** The
    // one after takes the text; failing that, the one before keeps its look
    // and takes it. With a table or the document's end on both sides there is
    // neither, so only the text can be struck and an empty paragraph is left
    // where it stood — which the helper is told, so that it does not report
    // the paragraph gone or call again to take the empty one out.
    let (start, end, joined) = if beside(document, to, to + 1) {
        ((from, 0), (to + 1, 0), true)
    } else if from > 0 && beside(document, from - 1, from) {
        (
            (from - 1, length(document, from - 1)),
            (to, length(document, to)),
            true,
        )
    } else {
        ((from, 0), (to, length(document, to)), false)
    };
    let looks_like = joined && start.0 < from;
    let before: Vec<Paragraph> = (start.0..=end.0)
        .filter_map(|index| paragraph(document, index))
        .collect();
    if looks_like {
        if let (Some(tail), Some(head)) = (before.last(), before.first()) {
            if let Some(change) = &tail.prop_change {
                if look(&tail.props) != look(&head.props) && !author.made(&change.mark) {
                    return Err(revise::FORMATTING_OPEN);
                }
            }
        }
    }
    revise::can_record(document, Scope::Body, span(start, end))?;
    let mut scratch = History::new();
    revise::delete_range(
        document,
        Scope::Body,
        &mut scratch,
        span(start, end),
        author,
        false,
    )?;
    if !scratch.can_undo() {
        return Err(NOTHING_THERE);
    }
    if looks_like {
        // Backspace keeps the look only after text; the paragraph before
        // may be empty, and what is left should look like it all the same.
        let model = before[0].props.clone();
        if let Some(mut tail) = paragraph(document, to) {
            let next = revise::next_revision_id(document);
            if revise::restyle(&mut tail, &model, author, next).is_ok() {
                crate::edit::replace_range(document, Scope::Body, to..to + 1, vec![tail]);
            }
        }
    }
    if !joined {
        let note = format!(
            "{} struck, and the empty paragraph stays: a table or the document's end is on \
             both sides of it, so there is no paragraph its mark can join.",
            capital(&short(from, to))
        );
        if !notes.contains(&note) {
            notes.push(note);
        }
    }
    let now = before.len();
    record(history, start.0, before, now);
    Ok(Moved::default())
}

/// Puts `lines` in after paragraph `after` (one-based; 0 for before the
/// first) as a proposal by `author`, one step on `history`: before the next
/// paragraph when it stands beside, which changes neither; otherwise at the
/// end of the container, where the last new paragraph takes over the mark
/// that ends it.
pub fn insert(
    document: &mut Document,
    history: &mut History,
    after: usize,
    lines: &[Line],
    author: &Author,
    notes: &mut Vec<String>,
) -> Result<Moved, &'static str> {
    let total = count(document);
    let neighbour_at = after.saturating_sub(1);
    let Some(neighbour) = paragraph(document, neighbour_at) else {
        return Err(revise::CANNOT_RECORD);
    };
    // What new paragraphs look like where they go: like the one before them,
    // or body text after a heading; before the first, plain.
    let (props, base) = match after {
        0 => (ParaProps::default(), RunProps::default()),
        _ => match is_heading(document, &neighbour) {
            true => (
                after_heading(document, &neighbour.props),
                RunProps::default(),
            ),
            false => (neighbour.props.clone(), plain_props(&neighbour)),
        },
    };
    let new: Vec<Paragraph> = lines
        .iter()
        .map(|line| built(document, line, props.clone(), &base, notes))
        .collect();
    let count = new.len();
    let next = revise::next_revision_id(document);

    if after == 0 || (after < total && beside(document, after - 1, after)) {
        let Some(following) = paragraph(document, after) else {
            return Err(revise::CANNOT_RECORD);
        };
        let (new, _) = revise::as_inserted(new, author, next);
        if !crate::edit::insert_before(document, Scope::Body, after, new) {
            return Err(revise::CANNOT_RECORD);
        }
        record(history, after, vec![following], count + 1);
        return Ok(Moved {
            reset: None,
            from: after,
            by: count as isize,
        });
    }

    // The paragraph before ends its container: Enter at its end, the new
    // mark its own, and the old one the last new paragraph's.
    if neighbour.prop_change.is_some() {
        return Err(revise::FORMATTING_OPEN);
    }
    let mut head = neighbour.clone();
    head.mark_revision = Some(wp_model::Revision::Inserted(author.mark(next)));
    head.mark_deleted = None;
    head.mark_change = None;
    head.section = None;
    let (mut new, next) = revise::as_inserted(new, author, next + 1);
    let last = new.last_mut().expect("at least one");
    let wanted = std::mem::replace(&mut last.props, neighbour.props.clone());
    last.mark_revision = neighbour.mark_revision.clone();
    last.mark_deleted = neighbour.mark_deleted.clone();
    last.mark_change = neighbour.mark_change.clone();
    last.section = neighbour.section.clone();
    revise::restyle(last, &wanted, author, next)?;
    let mut built_up = vec![head];
    built_up.extend(new);
    crate::edit::replace_range(
        document,
        Scope::Body,
        neighbour_at..neighbour_at + 1,
        built_up,
    );
    record(history, neighbour_at, vec![neighbour], count + 1);
    Ok(Moved {
        reset: None,
        from: after,
        by: count as isize,
    })
}

// ------------------------------------------------------------- settling

/// The assistant's changes in the text, with the time each was made at, and
/// the paragraph each is in.
fn changes(document: &Document) -> Vec<(Mark, usize)> {
    revise::tracked(document)
        .into_iter()
        .filter(|change| change.scope == Scope::Body && &*change.mark.author == AUTHOR)
        .map(|change| (change.mark, change.paragraph))
        .collect()
}

/// Whether any change of the proposal made at `date` is still in the text.
pub fn standing(document: &Document, date: &str) -> bool {
    changes(document)
        .iter()
        .any(|(mark, _)| mark.date.as_deref() == Some(date))
}

/// How many of the assistant's proposals are still open in the text.
pub fn open(document: &Document) -> usize {
    dates(document).len()
}

/// The times of the assistant's proposals still open, in the order the
/// text shows them.
fn dates(document: &Document) -> Vec<Option<Arc<str>>> {
    let mut dates: Vec<Option<Arc<str>>> = Vec::new();
    for (mark, _) in changes(document) {
        if !dates.contains(&mark.date) {
            dates.push(mark.date);
        }
    }
    dates
}

/// Settles the proposal made at `date` — every change it made, and none
/// other — as one step on `history`. How many changes there were.
pub fn settle(document: &mut Document, history: &mut History, date: &str, how: Resolve) -> usize {
    match settle_date(document, Some(date), how) {
        Some((change, settled)) => {
            history.push(Scope::Body, change);
            settled
        }
        None => 0,
    }
}

/// Settles every change the assistant made, and no one else's, as one step.
pub fn settle_all(document: &mut Document, history: &mut History, how: Resolve) -> usize {
    let mut steps = Vec::new();
    let mut settled = 0;
    for date in dates(document) {
        if let Some((change, count)) = settle_date(document, date.as_deref(), how) {
            steps.push(change);
            settled += count;
        }
    }
    if !steps.is_empty() {
        // Undone last first: each step's paragraphs are as the one before
        // left them.
        steps.reverse();
        history.push(Scope::Body, Change::Many(steps));
    }
    settled
}

/// Settles the changes of one proposal, and says how to take the settling
/// back: the paragraphs they stood in — and the one after, which a mark
/// that goes joins — as they were.
fn settle_date(
    document: &mut Document,
    date: Option<&str>,
    how: Resolve,
) -> Option<(Change, usize)> {
    let marks: Vec<(Mark, usize)> = changes(document)
        .into_iter()
        .filter(|(mark, _)| mark.date.as_deref() == date)
        .collect();
    let first = marks.iter().map(|(_, at)| *at).min()?;
    let last = marks.iter().map(|(_, at)| *at).max()?;
    let last = match beside(document, last, last + 1) {
        true => last + 1,
        false => last,
    };
    let before: Vec<Paragraph> = (first..=last)
        .filter_map(|index| paragraph(document, index))
        .collect();
    let total = count(document);
    let mut scratch = History::new();
    let settled = marks
        .iter()
        .filter(|(mark, _)| revise::resolve_one(document, &mut scratch, mark, how))
        .count();
    let now = before.len().checked_sub(total - count(document))?;
    Some((Change::Range { first, before, now }, settled))
}

/// The time a proposal made at `seconds` since 1970 carries, as Word writes
/// one: `2026-09-17T10:00:00Z`.
pub fn time_of(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest / 60 % 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests;
