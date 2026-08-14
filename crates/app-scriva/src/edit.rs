//! Editing a document, and taking it back.
//!
//! **Undo is a value, not a second implementation.** Applying a [`Change`]
//! returns the change that undoes it, so redo is the undo of the undo and the
//! two directions cannot drift apart. That is Calx's lesson (`LEARNINGS.md` §4)
//! and it transfers exactly.
//!
//! **The cost is bounded by what changed.** Typing into a paragraph remembers
//! that paragraph; splitting one remembers where; merging two remembers the two.
//! Nothing clones the body — a hundred-page document must not pay for a
//! keystroke.

use wp_model::doc::{Block, Document, Paragraph};
use wp_model::prop::{Justify, ParaProps, RunProps};

use crate::text;

/// A position in the document: which paragraph, and how far into its text.
///
/// The paragraph is named by its index in [`Document::paragraphs`] — document
/// order, tables and content controls included — because that is the only name
/// that works for a document whose paragraphs have no `w14:paraId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Caret {
    pub paragraph: usize,
    pub offset: usize,
}

/// A selection, which may run in either direction.
///
/// The *anchor* is where the selection started and the *head* is where the caret
/// is now. Keeping them apart rather than storing a sorted range is what makes
/// Shift+Left after Shift+Right shrink the selection instead of flipping it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    pub anchor: Caret,
    pub head: Caret,
}

impl Selection {
    pub fn at(caret: Caret) -> Selection {
        Selection {
            anchor: caret,
            head: caret,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// The two ends in document order.
    pub fn ordered(&self) -> (Caret, Caret) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }
}

/// One undoable change.
#[derive(Debug, Clone)]
pub enum Change {
    /// The text or properties of one paragraph.
    Paragraph {
        index: usize,
        before: Box<Paragraph>,
    },
    /// A paragraph was split in two at `index`, and the two are now `index` and
    /// `index + 1`.
    Split { index: usize },
    /// Two paragraphs became one at `index`.
    Merge {
        index: usize,
        first: Box<Paragraph>,
        second: Box<Paragraph>,
    },
    /// Several paragraphs at once — a selection spanning more than one.
    Range {
        first: usize,
        before: Vec<Paragraph>,
    },
}

/// The undo and redo stacks.
#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Change>,
    redo: Vec<Change>,
    /// Where the last change was, so consecutive typing coalesces into one
    /// entry. Word collapses a word's worth of typing into a single undo, which
    /// is what makes Ctrl+Z usable at all.
    last: Option<(usize, usize)>,
}

impl History {
    pub fn new() -> History {
        History::default()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.last = None;
    }

    /// Records a change, discarding anything that was redoable.
    pub fn push(&mut self, change: Change) {
        self.undo.push(change);
        self.redo.clear();
        self.last = None;
    }

    /// Records a change made by typing one character at `caret`.
    ///
    /// Joined to the previous entry when the caret carried straight on from
    /// where the last one ended, so a word typed is one undo rather than five.
    pub fn push_typing(&mut self, change: Change, paragraph: usize, offset: usize, word_end: bool) {
        let continues = self.last == Some((paragraph, offset)) && !self.undo.is_empty();
        if !continues {
            self.undo.push(change);
        }
        self.redo.clear();
        self.last = if word_end {
            None
        } else {
            Some((paragraph, offset + 1))
        };
    }

    pub fn undo(&mut self, document: &mut Document) -> Option<Caret> {
        let change = self.undo.pop()?;
        let (inverse, caret) = apply(document, change);
        self.redo.push(inverse);
        self.last = None;
        Some(caret)
    }

    pub fn redo(&mut self, document: &mut Document) -> Option<Caret> {
        let change = self.redo.pop()?;
        let (inverse, caret) = apply(document, change);
        self.undo.push(inverse);
        self.last = None;
        Some(caret)
    }
}

/// Applies a change and returns the one that undoes it.
fn apply(document: &mut Document, change: Change) -> (Change, Caret) {
    match change {
        Change::Paragraph { index, before } => {
            let mut paragraphs = document.paragraphs_mut();
            let Some(target) = paragraphs.get_mut(index) else {
                return (Change::Paragraph { index, before }, Caret::default());
            };
            let was = Box::new((**target).clone());
            **target = *before;
            let offset = text::len(target);
            (
                Change::Paragraph { index, before: was },
                Caret {
                    paragraph: index,
                    offset,
                },
            )
        }
        Change::Split { index } => {
            // Undoing a split is a merge.
            let (first, second) = {
                let paragraphs = document.paragraphs();
                match (paragraphs.get(index), paragraphs.get(index + 1)) {
                    (Some(a), Some(b)) => (Box::new((*a).clone()), Box::new((*b).clone())),
                    _ => return (Change::Split { index }, Caret::default()),
                }
            };
            let offset = text::len(&first);
            let joined = text::merge(&first, &second);
            replace_range(document, index..index + 2, vec![joined]);
            (
                Change::Merge {
                    index,
                    first,
                    second,
                },
                Caret {
                    paragraph: index,
                    offset,
                },
            )
        }
        Change::Merge {
            index,
            first,
            second,
        } => {
            let offset = text::len(&first);
            replace_range(document, index..index + 1, vec![*first, *second]);
            (
                Change::Split { index },
                Caret {
                    paragraph: index,
                    offset,
                },
            )
        }
        Change::Range { first, before } => {
            let count = before.len();
            let now: Vec<Paragraph> = {
                let paragraphs = document.paragraphs();
                paragraphs[first..(first + count).min(paragraphs.len())]
                    .iter()
                    .map(|p| (*p).clone())
                    .collect()
            };
            replace_range(document, first..first + now.len(), before);
            (
                Change::Range { first, before: now },
                Caret {
                    paragraph: first,
                    offset: 0,
                },
            )
        }
    }
}

/// Replaces a run of paragraphs, by index into the document's flattened walk.
///
/// Only paragraphs directly in the body can be added or removed: a paragraph
/// inside a table cell belongs to that cell, and inserting one beside it is a
/// change to the *table* rather than to the body. Stated rather than hidden —
/// pressing Enter inside a cell splits the paragraph within the cell, which is
/// what Word does, and that path goes through `replace_in_place` below.
pub fn replace_range(document: &mut Document, range: std::ops::Range<usize>, with: Vec<Paragraph>) {
    if let Some(body) = body_range(document, range.clone()) {
        document
            .body
            .splice(body, with.into_iter().map(Block::Paragraph));
        return;
    }
    replace_in_place(document, range, with);
}

/// The body indices covering a run of flattened paragraph indices, when all of
/// them are direct children of the body.
fn body_range(
    document: &Document,
    range: std::ops::Range<usize>,
) -> Option<std::ops::Range<usize>> {
    let mut flat = 0usize;
    let mut start = None;
    let mut end = None;
    for (index, block) in document.body.iter().enumerate() {
        match block {
            Block::Paragraph(_) => {
                if flat == range.start {
                    start = Some(index);
                }
                flat += 1;
                if flat == range.end {
                    end = Some(index + 1);
                }
            }
            other => {
                flat += count_paragraphs(other);
                if flat > range.start && end.is_none() && start.is_some() {
                    // The run reaches into a table or a control: not a body edit.
                    return None;
                }
            }
        }
    }
    match (start, end) {
        (Some(start), Some(end)) => Some(start..end),
        _ => None,
    }
}

fn count_paragraphs(block: &Block) -> usize {
    match block {
        Block::Paragraph(_) => 1,
        Block::Table(table) => table
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .flat_map(|cell| &cell.content)
            .map(count_paragraphs)
            .sum(),
        Block::Structured(sdt) => sdt.content.iter().map(count_paragraphs).sum(),
        _ => 0,
    }
}

/// Overwrites paragraphs in place, without changing how many there are.
fn replace_in_place(document: &mut Document, range: std::ops::Range<usize>, with: Vec<Paragraph>) {
    let mut paragraphs = document.paragraphs_mut();
    for (offset, replacement) in with.into_iter().enumerate() {
        if let Some(target) = paragraphs.get_mut(range.start + offset) {
            **target = replacement;
        }
    }
}

/// Types `input` at the selection, replacing it if it is not empty.
pub fn type_text(
    document: &mut Document,
    history: &mut History,
    selection: Selection,
    input: &str,
) -> Caret {
    let caret = delete_selection(document, history, selection);
    let Some(before) = paragraph_at(document, caret.paragraph) else {
        return caret;
    };
    let word_end = input
        .chars()
        .next()
        .is_some_and(|c| c.is_whitespace() || c.is_ascii_punctuation());
    let change = Change::Paragraph {
        index: caret.paragraph,
        before: Box::new(before),
    };
    let single = input.chars().count() == 1;
    if single {
        history.push_typing(change, caret.paragraph, caret.offset, word_end);
    } else {
        history.push(change);
    }

    let mut paragraphs = document.paragraphs_mut();
    let Some(target) = paragraphs.get_mut(caret.paragraph) else {
        return caret;
    };
    let after = text::insert(target, caret.offset, input);
    Caret {
        paragraph: caret.paragraph,
        offset: after,
    }
}

/// Removes whatever the selection covers, and returns where the caret lands.
pub fn delete_selection(
    document: &mut Document,
    history: &mut History,
    selection: Selection,
) -> Caret {
    if selection.is_empty() {
        return selection.head;
    }
    let (start, end) = selection.ordered();
    if start.paragraph == end.paragraph {
        let Some(before) = paragraph_at(document, start.paragraph) else {
            return start;
        };
        history.push(Change::Paragraph {
            index: start.paragraph,
            before: Box::new(before),
        });
        let mut paragraphs = document.paragraphs_mut();
        if let Some(target) = paragraphs.get_mut(start.paragraph) {
            text::remove(target, start.offset..end.offset);
        }
        return start;
    }

    // Across paragraphs: the first keeps its head, the last keeps its tail, and
    // the two become one.
    let before: Vec<Paragraph> = {
        let paragraphs = document.paragraphs();
        paragraphs[start.paragraph..=end.paragraph.min(paragraphs.len() - 1)]
            .iter()
            .map(|p| (*p).clone())
            .collect()
    };
    history.push(Change::Range {
        first: start.paragraph,
        before: before.clone(),
    });

    let mut head = before[0].clone();
    let head_len = text::len(&head);
    text::remove(&mut head, start.offset..head_len);
    let mut tail = before[before.len() - 1].clone();
    text::remove(&mut tail, 0..end.offset);
    let joined = text::merge(&head, &tail);
    replace_range(document, start.paragraph..end.paragraph + 1, vec![joined]);
    start
}

/// Splits the paragraph at the caret — the Enter key.
pub fn split_paragraph(
    document: &mut Document,
    history: &mut History,
    selection: Selection,
) -> Caret {
    let caret = delete_selection(document, history, selection);
    let Some(paragraph) = paragraph_at(document, caret.paragraph) else {
        return caret;
    };
    let (head, mut tail) = text::split(&paragraph, caret.offset);
    // Word's `<w:next>`: the paragraph after a heading is not a heading.
    if let Some(style) = paragraph.props.style {
        if let Some(next) = document.styles.get(style).and_then(|style| style.next) {
            if text::len(&paragraph) == caret.offset {
                tail.props.style = Some(next);
            }
        }
    }
    history.push(Change::Split {
        index: caret.paragraph,
    });
    replace_range(
        document,
        caret.paragraph..caret.paragraph + 1,
        vec![head, tail],
    );
    Caret {
        paragraph: caret.paragraph + 1,
        offset: 0,
    }
}

/// Backspace: one character, or the paragraph mark before this paragraph.
pub fn backspace(document: &mut Document, history: &mut History, selection: Selection) -> Caret {
    if !selection.is_empty() {
        return delete_selection(document, history, selection);
    }
    let caret = selection.head;
    let Some(paragraph) = paragraph_at(document, caret.paragraph) else {
        return caret;
    };
    if caret.offset > 0 {
        let previous = text::previous_char(&paragraph.text(), caret.offset);
        history.push(Change::Paragraph {
            index: caret.paragraph,
            before: Box::new(paragraph),
        });
        let mut paragraphs = document.paragraphs_mut();
        if let Some(target) = paragraphs.get_mut(caret.paragraph) {
            text::remove(target, previous..caret.offset);
        }
        return Caret {
            paragraph: caret.paragraph,
            offset: previous,
        };
    }
    if caret.paragraph == 0 {
        return caret;
    }
    join_with_previous(document, history, caret.paragraph)
}

/// Delete: one character forward, or the paragraph mark after this paragraph.
pub fn delete_forward(
    document: &mut Document,
    history: &mut History,
    selection: Selection,
) -> Caret {
    if !selection.is_empty() {
        return delete_selection(document, history, selection);
    }
    let caret = selection.head;
    let Some(paragraph) = paragraph_at(document, caret.paragraph) else {
        return caret;
    };
    let content = paragraph.text();
    if caret.offset < content.len() {
        let next = text::next_char(&content, caret.offset);
        history.push(Change::Paragraph {
            index: caret.paragraph,
            before: Box::new(paragraph),
        });
        let mut paragraphs = document.paragraphs_mut();
        if let Some(target) = paragraphs.get_mut(caret.paragraph) {
            text::remove(target, caret.offset..next);
        }
        return caret;
    }
    if caret.paragraph + 1 >= document.paragraphs().len() {
        return caret;
    }
    join_with_previous(document, history, caret.paragraph + 1)
}

/// Joins paragraph `index` onto the one before it.
fn join_with_previous(document: &mut Document, history: &mut History, index: usize) -> Caret {
    let (first, second) = {
        let paragraphs = document.paragraphs();
        match (paragraphs.get(index - 1), paragraphs.get(index)) {
            (Some(a), Some(b)) => (Box::new((*a).clone()), Box::new((*b).clone())),
            _ => return Caret::default(),
        }
    };
    let offset = text::len(&first);
    let joined = text::merge(&first, &second);
    history.push(Change::Merge {
        index: index - 1,
        first,
        second,
    });
    replace_range(document, index - 1..index + 1, vec![joined]);
    Caret {
        paragraph: index - 1,
        offset,
    }
}

/// Applies `change` to the run properties of everything the selection covers.
///
/// An empty selection is not a no-op: it changes the formatting the *next*
/// character typed will have, which is what Ctrl+B before typing does. That is
/// carried on the paragraph mark rather than in the document, and the caller
/// keeps it.
pub fn format_runs(
    document: &mut Document,
    history: &mut History,
    selection: Selection,
    change: impl Fn(&mut RunProps) + Copy,
) {
    if selection.is_empty() {
        return;
    }
    let (start, end) = selection.ordered();
    let before: Vec<Paragraph> = {
        let paragraphs = document.paragraphs();
        paragraphs[start.paragraph..=end.paragraph.min(paragraphs.len() - 1)]
            .iter()
            .map(|p| (*p).clone())
            .collect()
    };
    history.push(Change::Range {
        first: start.paragraph,
        before,
    });

    let mut paragraphs = document.paragraphs_mut();
    for index in start.paragraph..=end.paragraph {
        let Some(target) = paragraphs.get_mut(index) else {
            continue;
        };
        let from = if index == start.paragraph {
            start.offset
        } else {
            0
        };
        let to = if index == end.paragraph {
            end.offset
        } else {
            text::len(target)
        };
        split_runs_at(target, from);
        split_runs_at(target, to);
        apply_to_range(target, from..to, change);
    }
}

/// Splits whichever run straddles `offset`, so a formatting change can stop
/// there.
///
/// Without this, bolding half a word bolds the whole run it is in — which looks
/// like the selection was ignored.
fn split_runs_at(paragraph: &mut Paragraph, offset: usize) {
    let Some(spot) = text::spot_at(paragraph, offset) else {
        return;
    };
    if spot.offset == 0 {
        return;
    }
    let Some(run) = text::nth_run_mut(paragraph, spot.run) else {
        return;
    };
    let Some(wp_model::doc::Piece::Text(text)) = run.content.get(spot.piece) else {
        return;
    };
    if spot.offset >= text.len() {
        return;
    }
    let (head, tail) = (
        text[..spot.offset].to_string(),
        text[spot.offset..].to_string(),
    );
    run.content[spot.piece] = wp_model::doc::Piece::Text(head.into());
    run.content
        .insert(spot.piece + 1, wp_model::doc::Piece::Text(tail.into()));

    // The two halves are still one run, which is not enough: a run carries one
    // set of properties. The run is cut in two here so each half can differ.
    let tail_pieces = run.content.split_off(spot.piece + 1);
    let props = run.props.clone();
    let tail_run = wp_model::doc::Run {
        props,
        content: tail_pieces,
        prop_change: None,
    };
    insert_run_after(paragraph, spot.run, tail_run);
}

fn insert_run_after(paragraph: &mut Paragraph, index: usize, run: wp_model::doc::Run) {
    fn walk(
        content: &mut Vec<wp_model::doc::Inline>,
        want: usize,
        seen: &mut usize,
        run: &mut Option<wp_model::doc::Run>,
    ) {
        let mut at = 0;
        while at < content.len() {
            match &mut content[at] {
                wp_model::doc::Inline::Run(_) => {
                    if *seen == want {
                        if let Some(run) = run.take() {
                            content.insert(at + 1, wp_model::doc::Inline::Run(run));
                            return;
                        }
                    }
                    *seen += 1;
                }
                wp_model::doc::Inline::Hyperlink(link) => walk(&mut link.content, want, seen, run),
                wp_model::doc::Inline::Revised { content, .. } => walk(content, want, seen, run),
                wp_model::doc::Inline::Structured(sdt) => walk(&mut sdt.content, want, seen, run),
                wp_model::doc::Inline::Wrapper { content, .. }
                | wp_model::doc::Inline::SimpleField { content, .. } => {
                    walk(content, want, seen, run)
                }
                _ => {}
            }
            if run.is_none() {
                return;
            }
            at += 1;
        }
    }
    let mut carried = Some(run);
    let mut seen = 0;
    walk(&mut paragraph.content, index, &mut seen, &mut carried);
}

fn apply_to_range(
    paragraph: &mut Paragraph,
    range: std::ops::Range<usize>,
    change: impl Fn(&mut RunProps) + Copy,
) {
    let mut spans: Vec<usize> = Vec::new();
    let mut seen = 0usize;
    for (index, run) in paragraph.runs().iter().enumerate() {
        let width: usize = run.content.iter().map(run_piece_len).sum();
        let start = seen;
        seen += width;
        if width == 0 {
            continue;
        }
        if start < range.end && seen > range.start {
            spans.push(index);
        }
    }
    for index in spans {
        if let Some(run) = text::nth_run_mut(paragraph, index) {
            change(&mut run.props);
        }
    }
}

fn run_piece_len(piece: &wp_model::doc::Piece) -> usize {
    match piece {
        wp_model::doc::Piece::Text(text) => text.len(),
        wp_model::doc::Piece::Tab => 1,
        wp_model::doc::Piece::Break(wp_model::doc::Break::Line) => 1,
        wp_model::doc::Piece::Symbol { .. } => 1,
        _ => 0,
    }
}

/// Applies `change` to the paragraph properties of everything the selection
/// covers. Unlike run formatting, an empty selection *does* apply — a paragraph
/// is centred by putting the caret in it.
pub fn format_paragraphs(
    document: &mut Document,
    history: &mut History,
    selection: Selection,
    change: impl Fn(&mut ParaProps) + Copy,
) {
    let (start, end) = selection.ordered();
    let before: Vec<Paragraph> = {
        let paragraphs = document.paragraphs();
        if paragraphs.is_empty() {
            return;
        }
        paragraphs
            [start.paragraph.min(paragraphs.len() - 1)..=end.paragraph.min(paragraphs.len() - 1)]
            .iter()
            .map(|p| (*p).clone())
            .collect()
    };
    history.push(Change::Range {
        first: start.paragraph,
        before,
    });
    let mut paragraphs = document.paragraphs_mut();
    for index in start.paragraph..=end.paragraph {
        if let Some(target) = paragraphs.get_mut(index) {
            change(&mut target.props);
        }
    }
}

/// Whether every run the selection covers already satisfies `test`.
///
/// What a toolbar button asks to know whether it should look pressed.
pub fn all_runs(
    document: &Document,
    selection: Selection,
    test: impl Fn(&RunProps) -> bool,
) -> bool {
    let (start, end) = selection.ordered();
    let paragraphs = document.paragraphs();
    let mut any = false;
    for index in start.paragraph..=end.paragraph.min(paragraphs.len().saturating_sub(1)) {
        let Some(paragraph) = paragraphs.get(index) else {
            continue;
        };
        for run in paragraph.runs() {
            any = true;
            if !test(&run.props) {
                return false;
            }
        }
    }
    any
}

pub fn paragraph_at(document: &Document, index: usize) -> Option<Paragraph> {
    document.paragraphs().get(index).map(|p| (*p).clone())
}

/// The alignment of the paragraph the caret is in.
pub fn justify_at(document: &Document, caret: Caret) -> Option<Justify> {
    let paragraphs = document.paragraphs();
    let paragraph = paragraphs.get(caret.paragraph)?;
    document
        .styles
        .resolve_paragraph(&paragraph.props, None)
        .para
        .justify
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::doc::Block;
    use wp_model::Toggle;

    fn document(texts: &[&str]) -> Document {
        Document {
            body: texts
                .iter()
                .map(|text| Block::Paragraph(Paragraph::of(text)))
                .collect(),
            ..Document::new()
        }
    }

    fn at(paragraph: usize, offset: usize) -> Selection {
        Selection::at(Caret { paragraph, offset })
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

    #[test]
    fn typing_inserts_and_undo_takes_it_back() {
        let mut document = document(&["hello"]);
        let mut history = History::new();
        let caret = type_text(&mut document, &mut history, at(0, 5), " world");
        assert_eq!(document.text(), "hello world");
        assert_eq!(caret.offset, 11);

        history.undo(&mut document);
        assert_eq!(document.text(), "hello");
        history.redo(&mut document);
        assert_eq!(document.text(), "hello world");
    }

    #[test]
    fn a_word_typed_a_letter_at_a_time_is_one_undo() {
        // Word collapses typing into one undo per word, which is what makes
        // Ctrl+Z usable rather than a way to remove one letter.
        let mut document = document(&[""]);
        let mut history = History::new();
        let mut caret = Caret::default();
        for letter in "hello".chars() {
            caret = type_text(
                &mut document,
                &mut history,
                Selection::at(caret),
                &letter.to_string(),
            );
        }
        assert_eq!(document.text(), "hello");
        history.undo(&mut document);
        assert_eq!(document.text(), "", "the whole word went in one");
    }

    #[test]
    fn a_space_ends_the_run_of_typing() {
        let mut document = document(&[""]);
        let mut history = History::new();
        let mut caret = Caret::default();
        for letter in "one two".chars() {
            caret = type_text(
                &mut document,
                &mut history,
                Selection::at(caret),
                &letter.to_string(),
            );
        }
        history.undo(&mut document);
        assert_eq!(
            document.text(),
            "one ",
            "the second word came off on its own"
        );
    }

    #[test]
    fn enter_splits_a_paragraph_and_undo_joins_it() {
        let mut document = document(&["hello world"]);
        let mut history = History::new();
        let caret = split_paragraph(&mut document, &mut history, at(0, 5));
        assert_eq!(document.paragraphs().len(), 2);
        assert_eq!(document.text(), "hello\n world");
        assert_eq!(
            caret,
            Caret {
                paragraph: 1,
                offset: 0
            }
        );

        history.undo(&mut document);
        assert_eq!(document.paragraphs().len(), 1);
        assert_eq!(document.text(), "hello world");
        history.redo(&mut document);
        assert_eq!(document.paragraphs().len(), 2);
    }

    #[test]
    fn backspace_at_the_start_of_a_paragraph_joins_it_to_the_one_before() {
        let mut document = document(&["first", "second"]);
        let mut history = History::new();
        let caret = backspace(&mut document, &mut history, at(1, 0));
        assert_eq!(document.paragraphs().len(), 1);
        assert_eq!(document.text(), "firstsecond");
        assert_eq!(
            caret,
            Caret {
                paragraph: 0,
                offset: 5
            }
        );

        history.undo(&mut document);
        assert_eq!(document.paragraphs().len(), 2);
        assert_eq!(document.text(), "first\nsecond");
    }

    #[test]
    fn backspace_at_the_very_start_of_the_document_does_nothing() {
        let mut document = document(&["only"]);
        let mut history = History::new();
        backspace(&mut document, &mut history, at(0, 0));
        assert_eq!(document.text(), "only");
        assert!(!history.can_undo(), "nothing happened, so nothing to undo");
    }

    #[test]
    fn delete_at_the_end_joins_the_next_paragraph_up() {
        let mut document = document(&["first", "second"]);
        let mut history = History::new();
        let caret = delete_forward(&mut document, &mut history, at(0, 5));
        assert_eq!(document.text(), "firstsecond");
        assert_eq!(
            caret,
            Caret {
                paragraph: 0,
                offset: 5
            }
        );
    }

    #[test]
    fn a_selection_spanning_paragraphs_is_deleted_as_one() {
        let mut document = document(&["first", "middle", "last"]);
        let mut history = History::new();
        let caret = delete_selection(&mut document, &mut history, span((0, 2), (2, 2)));
        assert_eq!(document.paragraphs().len(), 1);
        assert_eq!(document.text(), "fist");
        assert_eq!(
            caret,
            Caret {
                paragraph: 0,
                offset: 2
            }
        );

        history.undo(&mut document);
        assert_eq!(document.paragraphs().len(), 3);
        assert_eq!(document.text(), "first\nmiddle\nlast");
    }

    #[test]
    fn typing_over_a_selection_replaces_it() {
        let mut document = document(&["hello world"]);
        let mut history = History::new();
        let caret = type_text(&mut document, &mut history, span((0, 0), (0, 5)), "goodbye");
        assert_eq!(document.text(), "goodbye world");
        assert_eq!(caret.offset, 7);
    }

    #[test]
    fn bolding_half_a_word_bolds_half_a_word() {
        // The run has to be cut where the selection stops, or the whole run
        // takes the formatting and it looks like the selection was ignored.
        let mut document = document(&["boldplain"]);
        let mut history = History::new();
        format_runs(&mut document, &mut history, span((0, 0), (0, 4)), |props| {
            props.toggles.set(Toggle::Bold, true)
        });
        let paragraphs = document.paragraphs();
        let runs = paragraphs[0].runs();
        assert_eq!(runs.len(), 2, "the run was cut in two");
        assert!(runs[0].props.bold());
        assert!(!runs[1].props.bold());
        assert_eq!(document.text(), "boldplain", "and nothing moved");
    }

    #[test]
    fn formatting_is_undoable_like_everything_else() {
        let mut document = document(&["text"]);
        let mut history = History::new();
        format_runs(&mut document, &mut history, span((0, 0), (0, 4)), |props| {
            props.toggles.set(Toggle::Italic, true)
        });
        assert!(document.paragraphs()[0].runs()[0].props.italic());
        history.undo(&mut document);
        assert!(!document.paragraphs()[0].runs()[0].props.italic());
    }

    #[test]
    fn centring_a_paragraph_needs_no_selection() {
        let mut document = document(&["one", "two"]);
        let mut history = History::new();
        format_paragraphs(&mut document, &mut history, at(1, 0), |props| {
            props.justify = Some(Justify::Center)
        });
        assert_eq!(document.paragraphs()[0].props.justify, None);
        assert_eq!(
            document.paragraphs()[1].props.justify,
            Some(Justify::Center)
        );
        history.undo(&mut document);
        assert_eq!(document.paragraphs()[1].props.justify, None);
    }

    #[test]
    fn a_selection_knows_which_end_the_caret_is_at() {
        let backwards = span((2, 0), (0, 3));
        let (start, end) = backwards.ordered();
        assert_eq!(start.paragraph, 0);
        assert_eq!(end.paragraph, 2);
        assert!(!backwards.is_empty());
        assert!(Selection::at(Caret::default()).is_empty());
    }

    #[test]
    fn redo_is_the_undo_of_the_undo() {
        // The property the whole design turns on: the two directions cannot
        // drift apart because there is only one implementation.
        let mut document = document(&["a", "b"]);
        let mut history = History::new();
        type_text(&mut document, &mut history, at(0, 1), "X");
        split_paragraph(&mut document, &mut history, at(0, 1));
        let after = document.text();

        history.undo(&mut document);
        history.undo(&mut document);
        assert_eq!(document.text(), "a\nb");
        history.redo(&mut document);
        history.redo(&mut document);
        assert_eq!(document.text(), after);
    }

    #[test]
    fn a_paragraph_inside_a_table_is_edited_in_place() {
        // Enter inside a cell splits the paragraph within the cell; it does not
        // add a paragraph to the body.
        let cell = wp_model::table::Cell {
            props: wp_model::table::CellProps::new(),
            content: vec![Block::Paragraph(Paragraph::of("in a cell"))],
        };
        let table = wp_model::table::Table {
            rows: vec![wp_model::table::Row {
                cells: vec![cell],
                ..wp_model::table::Row::new()
            }],
            ..wp_model::table::Table::new()
        };
        let mut document = Document {
            body: vec![Block::Table(table)],
            ..Document::new()
        };
        let mut history = History::new();
        type_text(&mut document, &mut history, at(0, 2), "X");
        assert_eq!(document.paragraphs()[0].text(), "inX a cell");
        assert_eq!(document.body.len(), 1, "still one table and nothing beside");
        history.undo(&mut document);
        assert_eq!(document.paragraphs()[0].text(), "in a cell");
    }
}
