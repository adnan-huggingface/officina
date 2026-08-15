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
        /// How many paragraphs stand in the range *after* the change. A
        /// deletion leaves fewer than it found, and an undo that assumed the
        /// count never moved restored the originals over whatever paragraphs
        /// happened to follow.
        now: usize,
    },
    /// The section's page setup — margins, size, orientation.
    ///
    /// Carries the caret because a page-setup change has no text position of
    /// its own: undo puts the user back where they were when they made it.
    Section {
        before: Box<wp_model::SectionProps>,
        caret: Caret,
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
        Change::Range { first, before, now } => {
            let current: Vec<Paragraph> = {
                let paragraphs = document.paragraphs();
                paragraphs[first.min(paragraphs.len())..(first + now).min(paragraphs.len())]
                    .iter()
                    .map(|p| (*p).clone())
                    .collect()
            };
            let restored = before.len();
            replace_range(document, first..first + current.len(), before);
            (
                Change::Range {
                    first,
                    before: current,
                    now: restored,
                },
                Caret {
                    paragraph: first,
                    offset: 0,
                },
            )
        }
        Change::Section { before, caret } => {
            let was = std::mem::replace(&mut document.section, *before);
            (
                Change::Section {
                    before: Box::new(was),
                    caret,
                },
                caret,
            )
        }
    }
}

/// Replaces the section's page setup, undoably.
pub fn set_section(
    document: &mut Document,
    history: &mut History,
    caret: Caret,
    section: wp_model::SectionProps,
) {
    let before = std::mem::replace(&mut document.section, section);
    history.push(Change::Section {
        before: Box::new(before),
        caret,
    });
}

/// Inserts a break — a page break, mostly — at the caret, Ctrl+Enter's job.
///
/// The caret's offset does not move for a page break: a break is not a byte
/// of text, so the caret's world has no address for the far side of it. The
/// line the caret is drawn on follows the text that now sits after the break.
pub fn insert_break(
    document: &mut Document,
    history: &mut History,
    selection: Selection,
    kind: wp_model::doc::Break,
) -> Caret {
    let caret = delete_selection(document, history, selection);
    let Some(before) = paragraph_at(document, caret.paragraph) else {
        return caret;
    };
    history.push(Change::Paragraph {
        index: caret.paragraph,
        before: Box::new(before),
    });
    let mut paragraphs = document.paragraphs_mut();
    let Some(target) = paragraphs.get_mut(caret.paragraph) else {
        return caret;
    };
    let added = text::insert_piece(target, caret.offset, wp_model::doc::Piece::Break(kind));
    Caret {
        paragraph: caret.paragraph,
        offset: caret.offset + added,
    }
}

/// Replaces a run of paragraphs, by index into the document's flattened walk.
///
/// Paragraphs can be added or removed wherever the whole range is the direct
/// children of *one* container — the body, one table cell, or one content
/// control. That is what pressing Enter or Backspace inside a cell needs: a
/// bullet list in a table splits and joins within its cell, the way it does in
/// Word. Only a range that crosses containers — a selection reaching from
/// inside a cell out to the body — cannot change how many paragraphs there
/// are, and is overwritten in place instead.
pub fn replace_range(document: &mut Document, range: std::ops::Range<usize>, with: Vec<Paragraph>) {
    let mut with = Some(with);
    let mut flat = 0usize;
    if splice_blocks(&mut document.body, &mut flat, &range, &mut with) {
        return;
    }
    if let Some(with) = with.take() {
        replace_in_place(document, range, with);
    }
}

/// Splices `with` over `range` when every paragraph of the range is a direct
/// child of one container, walking containers in the same order as
/// [`Document::paragraphs`]. Consumes `with` only on success.
fn splice_blocks(
    blocks: &mut Vec<Block>,
    flat: &mut usize,
    range: &std::ops::Range<usize>,
    with: &mut Option<Vec<Paragraph>>,
) -> bool {
    let mut start = None;
    let mut found: Option<(usize, usize)> = None;
    let mut index = 0;
    while index < blocks.len() {
        match &mut blocks[index] {
            Block::Paragraph(_) => {
                if *flat == range.start {
                    start = Some(index);
                }
                *flat += 1;
                if *flat == range.end {
                    match start {
                        Some(start) => found = Some((start, index + 1)),
                        // The range began in some other container: it cannot
                        // be spliced anywhere.
                        None => return false,
                    }
                }
            }
            Block::Table(table) => {
                for row in &mut table.rows {
                    for cell in &mut row.cells {
                        if splice_blocks(&mut cell.content, flat, range, with) {
                            return true;
                        }
                    }
                }
            }
            Block::Structured(sdt) => {
                if splice_blocks(&mut sdt.content, flat, range, with) {
                    return true;
                }
            }
            _ => {}
        }
        if let Some((from, to)) = found {
            if let Some(with) = with.take() {
                blocks.splice(from..to, with.into_iter().map(Block::Paragraph));
            }
            return true;
        }
        index += 1;
    }
    false
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
        // The range becomes the one joined paragraph.
        now: 1,
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
        now: before.len(),
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
        now: before.len(),
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
    fn a_page_setup_change_is_one_undo_step_that_returns_the_caret() {
        use wp_model::units::Twips;
        let mut document = document(&["hello"]);
        let mut history = History::new();
        let was = document.section.margins.top;
        let mut section = document.section.clone();
        section.margins.top = Twips(2880);
        let here = Caret {
            paragraph: 0,
            offset: 3,
        };
        set_section(&mut document, &mut history, here, section);
        assert_eq!(document.section.margins.top, Twips(2880));

        let caret = history.undo(&mut document).expect("there is an undo");
        assert_eq!(
            document.section.margins.top, was,
            "the old margins are back"
        );
        assert_eq!(caret, here, "undo returns to where the user was");
        history.redo(&mut document);
        assert_eq!(document.section.margins.top, Twips(2880));
    }

    #[test]
    fn a_page_break_lands_at_the_caret_and_undoes_away() {
        use wp_model::doc::{Break, Piece};
        let mut document = document(&["hello world"]);
        let mut history = History::new();
        let caret = insert_break(&mut document, &mut history, at(0, 5), Break::Page);
        // A break is not a byte of text: nothing the caret counts has changed.
        assert_eq!(
            caret,
            Caret {
                paragraph: 0,
                offset: 5
            }
        );
        let has_break = |document: &Document| {
            document.paragraphs()[0]
                .runs()
                .iter()
                .flat_map(|run| run.content.iter())
                .any(|piece| matches!(piece, Piece::Break(Break::Page)))
        };
        assert!(has_break(&document), "the break is in the paragraph");
        assert_eq!(
            document.paragraphs()[0].text(),
            "hello world",
            "and the text reads straight through it"
        );

        history.undo(&mut document);
        assert!(!has_break(&document), "undo takes the break out");
        assert_eq!(document.paragraphs()[0].text(), "hello world");
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
    fn undoing_a_deletion_restores_without_eating_what_follows() {
        // The undo once assumed the paragraph count never changed, and put the
        // two originals back over the joined paragraph *and* the innocent one
        // after it.
        let mut document = document(&["aa", "bb", "cc", "dd"]);
        let mut history = History::new();
        delete_selection(&mut document, &mut history, span((1, 1), (2, 1)));
        assert_eq!(document.text(), "aa\nbc\ndd");
        history.undo(&mut document);
        assert_eq!(document.text(), "aa\nbb\ncc\ndd");
        history.redo(&mut document);
        assert_eq!(document.text(), "aa\nbc\ndd");
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

    fn cell_document(texts: &[&str]) -> Document {
        let cell = wp_model::table::Cell {
            props: wp_model::table::CellProps::new(),
            content: texts
                .iter()
                .map(|text| Block::Paragraph(Paragraph::of(text)))
                .collect(),
        };
        let table = wp_model::table::Table {
            rows: vec![wp_model::table::Row {
                cells: vec![cell],
                ..wp_model::table::Row::new()
            }],
            ..wp_model::table::Table::new()
        };
        Document {
            body: vec![
                Block::Table(table),
                Block::Paragraph(Paragraph::of("after the table")),
            ],
            ..Document::new()
        }
    }

    /// Every paragraph's text, in flattened order — `Document::text` puts its
    /// own separators around a table, which is not what these tests measure.
    fn texts(document: &Document) -> Vec<String> {
        document
            .paragraphs()
            .iter()
            .map(|paragraph| paragraph.text())
            .collect()
    }

    #[test]
    fn enter_inside_a_cell_splits_without_eating_the_next_paragraph() {
        // The resume's bullet lists live in table cells. Splitting used to
        // overwrite the paragraph after the split with the split-off tail —
        // pressing Enter in one bullet destroyed the next.
        let mut document = cell_document(&["first item", "second item"]);
        let mut history = History::new();
        let caret = split_paragraph(&mut document, &mut history, at(0, 5));
        assert_eq!(
            texts(&document),
            ["first", " item", "second item", "after the table"]
        );
        assert_eq!(
            caret,
            Caret {
                paragraph: 1,
                offset: 0
            }
        );
        let Block::Table(table) = &document.body[0] else {
            panic!("the table is still there");
        };
        assert_eq!(
            table.rows[0].cells[0].content.len(),
            3,
            "the cell gained the new paragraph"
        );
        history.undo(&mut document);
        assert_eq!(
            texts(&document),
            ["first item", "second item", "after the table"]
        );
    }

    #[test]
    fn joining_inside_a_cell_does_not_leave_a_duplicate_behind() {
        // Backspace at the start of the second cell paragraph joins it onto
        // the first. It used to write the joined text over the first and keep
        // the second as well — every join duplicated a paragraph.
        let mut document = cell_document(&["first item", "second item"]);
        let mut history = History::new();
        let caret = backspace(&mut document, &mut history, at(1, 0));
        assert_eq!(
            texts(&document),
            ["first itemsecond item", "after the table"]
        );
        assert_eq!(
            caret,
            Caret {
                paragraph: 0,
                offset: 10
            }
        );
        history.undo(&mut document);
        assert_eq!(
            texts(&document),
            ["first item", "second item", "after the table"]
        );
    }

    #[test]
    fn deleting_a_selection_across_cell_paragraphs_joins_them() {
        let mut document = cell_document(&["first item", "second item"]);
        let mut history = History::new();
        delete_selection(&mut document, &mut history, span((0, 5), (1, 6)));
        assert_eq!(texts(&document), ["first item", "after the table"]);
        history.undo(&mut document);
        assert_eq!(
            texts(&document),
            ["first item", "second item", "after the table"]
        );
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
