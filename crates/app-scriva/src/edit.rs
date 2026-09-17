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

use wp_model::doc::{Block, Document, Paragraph, Scope};
use wp_model::prop::{Justify, ParaProps, RunProps};

use crate::text;

/// A position in one of the document's flows: which paragraph, and how far
/// into its text.
///
/// The paragraph is named by its index in that flow's own walk — document
/// order, tables and content controls included — because that is the only name
/// that works for a document whose paragraphs have no `w14:paraId`. *Which*
/// flow is a [`Scope`], and it is carried beside the caret rather than in it:
/// a selection cannot span two of them, so one scope answers for both ends and
/// for the change they make.
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
    /// The document's comments, whole — what a comment posted, answered,
    /// resolved or deleted changes, beside the anchors in the text. A list
    /// rather than one comment, because a reply is a comment of its own and
    /// deleting one takes its replies too.
    Comments { before: Vec<wp_model::Comment> },
    /// Several changes that are one thing to undo: a comment's anchors in the
    /// text and the comment itself are two changes to the model and one to
    /// the person who made them.
    Many(Vec<Change>),
    /// The section's page setup — margins, size, orientation.
    ///
    /// Carries the caret because a page-setup change has no text position of
    /// its own: undo puts the user back where they were when they made it.
    Section {
        before: Box<wp_model::SectionProps>,
        caret: Caret,
    },
    /// Whole blocks of the body — a table inserted or removed. Paragraph
    /// indices name positions in the flattened walk, which cannot say "this
    /// table, as one thing", so this variant speaks in body positions instead.
    Blocks {
        index: usize,
        before: Vec<wp_model::doc::Block>,
        /// How many blocks stand in the range after the change, for the same
        /// reason [`Change::Range`] counts its paragraphs.
        now: usize,
    },
    /// The header and footer bodies, with the sections that reference them
    /// and the settings that decide which of them a page ever shows — one
    /// change, because a header exists only through its reference and
    /// restoring one without the other leaves a reference pointing at nothing.
    ///
    /// *Sections*, plural, and all of them: "Link to Previous" is a change to
    /// one section that changes what every section after it shows, and every
    /// section but the last lives on the paragraph that ends it rather than in
    /// one place a single entry could name.
    ///
    /// `<w:evenAndOddHeaders>` rides along for the same reason the sections
    /// do: it is a *document* setting, but the only thing it decides is
    /// whether a section's even-page band is used, and turning it off is a
    /// change to what the page shows exactly as removing the band would be.
    Chrome {
        headers: Vec<wp_model::doc::HeaderFooter>,
        sections: Vec<wp_model::SectionProps>,
        settings: Box<wp_model::Settings>,
        caret: Caret,
    },
}

/// One change and the flow it was made in.
///
/// The scope has to be remembered with the change: undoing a header's edit
/// against the body would restore the header's paragraph over whatever
/// paragraph of the text happened to share its number.
#[derive(Debug, Clone)]
struct Entry {
    scope: Scope,
    change: Change,
}

/// The undo and redo stacks.
#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    /// Where the last change was, so consecutive typing coalesces into one
    /// entry. Word collapses a word's worth of typing into a single undo, which
    /// is what makes Ctrl+Z usable at all.
    last: Option<(Scope, usize, usize)>,
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
    pub fn push(&mut self, scope: Scope, change: Change) {
        self.undo.push(Entry { scope, change });
        self.redo.clear();
        self.last = None;
    }

    /// Records a change made by typing one character at `caret`.
    ///
    /// Joined to the previous entry when the caret carried straight on from
    /// where the last one ended, so a word typed is one undo rather than five.
    pub fn push_typing(
        &mut self,
        scope: Scope,
        change: Change,
        paragraph: usize,
        offset: usize,
        word_end: bool,
    ) {
        let continues = self.last == Some((scope, paragraph, offset)) && !self.undo.is_empty();
        if !continues {
            self.undo.push(Entry { scope, change });
        }
        self.redo.clear();
        self.last = if word_end {
            None
        } else {
            Some((scope, paragraph, offset + 1))
        };
    }

    /// Takes the last change back, and says where the caret goes — including
    /// which flow, so undoing an edit made in a header opens that header again
    /// rather than moving a caret in the body to the same number.
    pub fn undo(&mut self, document: &mut Document) -> Option<(Scope, Caret)> {
        let entry = self.undo.pop()?;
        let (inverse, caret) = apply(document, entry.scope, entry.change);
        self.redo.push(Entry {
            scope: entry.scope,
            change: inverse,
        });
        self.last = None;
        Some((entry.scope, caret))
    }

    pub fn redo(&mut self, document: &mut Document) -> Option<(Scope, Caret)> {
        let entry = self.redo.pop()?;
        let (inverse, caret) = apply(document, entry.scope, entry.change);
        self.undo.push(Entry {
            scope: entry.scope,
            change: inverse,
        });
        self.last = None;
        Some((entry.scope, caret))
    }
}

/// Applies a change and returns the one that undoes it.
fn apply(document: &mut Document, scope: Scope, change: Change) -> (Change, Caret) {
    match change {
        Change::Paragraph { index, before } => {
            let mut paragraphs = document.paragraphs_in_mut(scope);
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
                let paragraphs = document.paragraphs_in(scope);
                match (paragraphs.get(index), paragraphs.get(index + 1)) {
                    (Some(a), Some(b)) => (Box::new((*a).clone()), Box::new((*b).clone())),
                    _ => return (Change::Split { index }, Caret::default()),
                }
            };
            let offset = text::len(&first);
            let joined = text::merge(&first, &second);
            replace_range(document, scope, index..index + 2, vec![joined]);
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
            replace_range(document, scope, index..index + 1, vec![*first, *second]);
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
                let paragraphs = document.paragraphs_in(scope);
                paragraphs[first.min(paragraphs.len())..(first + now).min(paragraphs.len())]
                    .iter()
                    .map(|p| (*p).clone())
                    .collect()
            };
            let restored = before.len();
            replace_range(document, scope, first..first + current.len(), before);
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
        Change::Comments { before } => {
            let was = std::mem::replace(&mut document.comments, before);
            (Change::Comments { before: was }, Caret::default())
        }
        Change::Many(changes) => {
            // Applied in order; undone in the reverse order, which is what
            // makes the inverse of a sequence a sequence.
            let mut inverses = Vec::with_capacity(changes.len());
            let mut caret = Caret::default();
            for change in changes {
                let (inverse, at) = apply(document, scope, change);
                inverses.push(inverse);
                caret = at;
            }
            inverses.reverse();
            (Change::Many(inverses), caret)
        }
        Change::Blocks { index, before, now } => {
            let Some(blocks) = document.blocks_mut(scope) else {
                return (Change::Blocks { index, before, now }, Caret::default());
            };
            let index = index.min(blocks.len());
            let end = (index + now).min(blocks.len());
            let restored = before.len();
            let was: Vec<wp_model::doc::Block> = blocks.splice(index..end, before).collect();
            let caret = Caret {
                paragraph: paragraphs_before_block(document, scope, index),
                offset: 0,
            };
            (
                Change::Blocks {
                    index,
                    before: was,
                    now: restored,
                },
                caret,
            )
        }
        Change::Chrome {
            headers,
            sections,
            settings,
            caret,
        } => {
            let was_headers = std::mem::replace(&mut document.headers, headers);
            let was_sections = document.section_props();
            document.set_section_props(&sections);
            let was_settings = std::mem::replace(&mut document.settings, *settings);
            (
                Change::Chrome {
                    headers: was_headers,
                    sections: was_sections,
                    settings: Box::new(was_settings),
                    caret,
                },
                caret,
            )
        }
    }
}

/// How many paragraphs of the flattened walk come before `block` of the flow —
/// the bridge from a block position back to a caret.
fn paragraphs_before_block(document: &Document, scope: Scope, block: usize) -> usize {
    let blocks = document.blocks(scope);
    blocks[..block.min(blocks.len())]
        .iter()
        .map(paragraphs_in_block)
        .sum()
}

/// How many paragraphs of the flattened walk a block contributes, mirroring
/// [`Document::paragraphs`] exactly — the two disagreeing would put the caret
/// in a different paragraph than the one the block position names.
fn paragraphs_in_block(block: &wp_model::doc::Block) -> usize {
    use wp_model::doc::Block;
    match block {
        Block::Paragraph(_) => 1,
        Block::Table(table) => table
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .flat_map(|cell| &cell.content)
            .map(paragraphs_in_block)
            .sum(),
        Block::Structured(sdt) => sdt.content.iter().map(paragraphs_in_block).sum(),
        Block::Anchor(_) | Block::AltChunk { .. } => 0,
    }
}

/// The table the caret is in: its body position, and the row and cell holding
/// the caret's paragraph. `None` when the caret is not in one.
///
/// A caret inside a *nested* table answers with the outer cell, which is the
/// cell a table command edits — the outer table is the one whose block the
/// undo history replaces whole.
pub fn table_cell_at(
    document: &Document,
    scope: Scope,
    caret: Caret,
) -> Option<(usize, usize, usize)> {
    use wp_model::doc::Block;
    let mut counted = 0;
    for (index, block) in document.blocks(scope).iter().enumerate() {
        let within = paragraphs_in_block(block);
        if caret.paragraph < counted + within {
            let Block::Table(table) = block else {
                return None;
            };
            let mut inside = caret.paragraph - counted;
            for (at_row, row) in table.rows.iter().enumerate() {
                for (at_cell, cell) in row.cells.iter().enumerate() {
                    let held: usize = cell.content.iter().map(paragraphs_in_block).sum();
                    if inside < held {
                        return Some((index, at_row, at_cell));
                    }
                    inside -= held;
                }
            }
            return None;
        }
        counted += within;
    }
    None
}

/// The paragraphs of one cell, as a range of the flow's flattened walk — what
/// Tab selects when it arrives in the cell.
pub fn cell_paragraphs(
    document: &Document,
    scope: Scope,
    block: usize,
    row: usize,
    cell: usize,
) -> Option<std::ops::Range<usize>> {
    use wp_model::doc::Block;
    let Some(Block::Table(table)) = document.blocks(scope).get(block) else {
        return None;
    };
    let mut first = paragraphs_before_block(document, scope, block);
    for (at_row, cells) in table.rows.iter().map(|row| &row.cells).enumerate() {
        for (at_cell, here) in cells.iter().enumerate() {
            let held: usize = here.content.iter().map(paragraphs_in_block).sum();
            if (at_row, at_cell) == (row, cell) {
                return Some(first..first + held);
            }
            first += held;
        }
    }
    None
}

/// Adds a row to the end of the table at `block`, shaped and formatted as its
/// last row and empty, undoably — what Tab does in a table's last cell.
pub fn append_row(document: &mut Document, scope: Scope, history: &mut History, block: usize) {
    table_change(document, scope, history, block, |table| {
        let rows = table.rows.len();
        rows > 0 && table.insert_row(rows, rows - 1)
    });
}

/// Runs one change against the table at `block` as one undo step: the whole
/// table before it, restored by undo, because a row or a column is not a
/// thing the flattened walk of paragraphs can name. Nothing is recorded when
/// `change` says it did nothing.
pub fn table_change(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    block: usize,
    change: impl FnOnce(&mut wp_model::table::Table) -> bool,
) -> bool {
    use wp_model::doc::Block;
    let Some(before) = document.blocks(scope).get(block).cloned() else {
        return false;
    };
    let Some(Block::Table(table)) = document
        .blocks_mut(scope)
        .and_then(|blocks| blocks.get_mut(block))
    else {
        return false;
    };
    if !change(table) {
        return false;
    }
    history.push(
        scope,
        Change::Blocks {
            index: block,
            before: vec![before],
            now: 1,
        },
    );
    true
}

/// Takes the table at `block` out, leaving an empty paragraph where it stood
/// — a document is never left with nothing at a place a caret can be — and
/// answers with the caret on that paragraph. One undo step brings the table
/// back whole.
pub fn delete_table(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    block: usize,
) -> Option<Caret> {
    use wp_model::doc::Block;
    let before = document.blocks(scope).get(block).cloned()?;
    if !matches!(before, Block::Table(_)) {
        return None;
    }
    let paragraph = paragraphs_before_block(document, scope, block);
    let blocks = document.blocks_mut(scope)?;
    blocks[block] = Block::Paragraph(Paragraph::new());
    history.push(
        scope,
        Change::Blocks {
            index: block,
            before: vec![before],
            now: 1,
        },
    );
    Some(Caret {
        paragraph,
        offset: 0,
    })
}

/// Inserts a block above the paragraph the caret is in, undoably.
///
/// Above rather than at the caret's offset: a table is not a character, and
/// Word's own insert puts it before the current paragraph too. The caret lands
/// on the block's first paragraph — inside the new table's first cell — or
/// stays where it was for a block with no paragraph to land on.
pub fn insert_block(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    selection: Selection,
    block: wp_model::doc::Block,
) -> Caret {
    let (caret, _) = selection.ordered();
    // The top-level block whose flattened paragraphs contain the caret; past
    // the end means the flow's end.
    let mut counted = 0;
    let mut index = document.blocks(scope).len();
    for (at, candidate) in document.blocks(scope).iter().enumerate() {
        let within = paragraphs_in_block(candidate);
        if caret.paragraph < counted + within {
            index = at;
            break;
        }
        counted += within;
    }
    let landing = paragraphs_before_block(document, scope, index);
    let has_paragraph = paragraphs_in_block(&block) > 0;
    let Some(blocks) = document.blocks_mut(scope) else {
        return caret;
    };
    history.push(
        scope,
        Change::Blocks {
            index,
            before: Vec::new(),
            now: 1,
        },
    );
    blocks.insert(index, block);
    if has_paragraph {
        Caret {
            paragraph: landing,
            offset: 0,
        }
    } else {
        caret
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
    history.push(
        Scope::Body,
        Change::Section {
            before: Box::new(before),
            caret,
        },
    );
}

/// Inserts a break — a page break, mostly — at the caret, Ctrl+Enter's job,
/// and ends the paragraph after it, which is what Word writes for the key.
///
/// The paragraph mark is what gives the caret somewhere to go. A break is not
/// a byte of text, so an offset cannot say which side of it the caret is on,
/// and a caret left at the break's offset typed everything that followed onto
/// the page the break was meant to leave. After the mark the caret is at the
/// head of the next paragraph, on the new page, and the layout already lets
/// the mark ride the line its break ended rather than open a line of its own.
/// One undo takes both away.
pub fn insert_break(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    selection: Selection,
    kind: wp_model::doc::Break,
) -> Caret {
    let caret = delete_selection(document, scope, history, selection);
    if kind == wp_model::doc::Break::Page {
        if let Some((block, row, _)) = table_cell_at(document, scope, caret) {
            return break_table_before_row(document, scope, history, block, row, caret);
        }
    }
    let Some(before) = paragraph_at(document, scope, caret.paragraph) else {
        return caret;
    };
    let (mut head, tail) = text::split(&before, caret.offset);
    text::insert_piece(&mut head, caret.offset, wp_model::doc::Piece::Break(kind));
    history.push(
        scope,
        Change::Range {
            first: caret.paragraph,
            before: vec![before],
            now: 2,
        },
    );
    replace_range(
        document,
        scope,
        caret.paragraph..caret.paragraph + 1,
        vec![head, tail],
    );
    Caret {
        paragraph: caret.paragraph + 1,
        offset: 0,
    }
}

/// Ctrl+Enter with the caret in a table cell: the table is split before the
/// caret's row, and the break stands in a paragraph of its own between the
/// two halves. Nothing goes into the cell.
///
/// Measured on Word, not designed: a page break *inside* a cell is nothing
/// to Word's layout, wherever in the cell it is, and the layout here ignores
/// one too. So a break put in the cell would be a keystroke that did nothing
/// visible. Word's own Ctrl+Enter in a cell does exactly this split, and in
/// the first row it leaves no empty table above the break. The caret stays
/// where it was in its cell, which is now the second table's. One undo puts
/// the table back together.
fn break_table_before_row(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    block: usize,
    row: usize,
    caret: Caret,
) -> Caret {
    let Some(Block::Table(table)) = document.blocks(scope).get(block).cloned() else {
        return caret;
    };
    let mut upper = table.clone();
    let mut lower = table;
    lower.rows = upper.rows.split_off(row);
    let mut breaking = Paragraph::new();
    let mut run = wp_model::doc::Run::new();
    run.content
        .push(wp_model::doc::Piece::Break(wp_model::doc::Break::Page));
    breaking.content.push(wp_model::doc::Inline::Run(run));
    let mut with = Vec::with_capacity(3);
    if !upper.rows.is_empty() {
        with.push(Block::Table(upper));
    }
    with.push(Block::Paragraph(breaking));
    with.push(Block::Table(lower));
    let now = with.len();
    let Some(blocks) = document.blocks_mut(scope) else {
        return caret;
    };
    let before: Vec<Block> = blocks.splice(block..block + 1, with).collect();
    history.push(
        scope,
        Change::Blocks {
            index: block,
            before,
            now,
        },
    );
    // The break's paragraph comes before the caret's cell in the walk, and
    // nothing else moved.
    Caret {
        paragraph: caret.paragraph + 1,
        offset: caret.offset,
    }
}

/// Replaces a run of paragraphs, by index into the flow's flattened walk.
///
/// Paragraphs can be added or removed wherever the whole range is the direct
/// children of *one* container — the body, one table cell, or one content
/// control. That is what pressing Enter or Backspace inside a cell needs: a
/// bullet list in a table splits and joins within its cell, the way it does in
/// Word. Only a range that crosses containers — a selection reaching from
/// inside a cell out to the body — cannot change how many paragraphs there
/// are, and is overwritten in place instead.
pub fn replace_range(
    document: &mut Document,
    scope: Scope,
    range: std::ops::Range<usize>,
    with: Vec<Paragraph>,
) {
    // Nothing replaced is an insertion: after the paragraph before the
    // range, in its container, or before the first.
    if range.is_empty() {
        insert_paragraphs_at(document, scope, range.start, with);
        return;
    }
    let mut with = Some(with);
    let mut flat = 0usize;
    if let Some(blocks) = document.blocks_mut(scope) {
        if splice_blocks(blocks, &mut flat, &range, &mut with) {
            return;
        }
    }
    if let Some(with) = with.take() {
        // In place, only as many as there were: a caller changing how many
        // there are asks [`side_by_side`] first.
        debug_assert_eq!(with.len(), range.len(), "a count changed across containers");
        replace_in_place(document, scope, range, with);
    }
}

/// Puts `with` in as paragraphs `at..` of the flow's flattened walk: after
/// paragraph `at - 1`, beside it in its container.
fn insert_paragraphs_at(document: &mut Document, scope: Scope, at: usize, with: Vec<Paragraph>) {
    let blocks = document.blocks(scope);
    let (steps, index) = match at
        .checked_sub(1)
        .and_then(|before| place_of(blocks, &mut 0, before))
    {
        Some((steps, index)) => (steps, index + 1),
        None => place_of(blocks, &mut 0, at).unwrap_or((Vec::new(), blocks.len())),
    };
    let Some(container) = document
        .blocks_mut(scope)
        .and_then(|blocks| container_mut(blocks, &steps))
    else {
        return;
    };
    let index = index.min(container.len());
    container.splice(index..index, with.into_iter().map(Block::Paragraph));
}

/// Whether paragraphs `range` of the flow's flattened walk stand side by side
/// in one container, with no table or content control among them: the only
/// range an edit may change the number of paragraphs in.
pub fn side_by_side(document: &Document, scope: Scope, range: std::ops::Range<usize>) -> bool {
    fn walk(blocks: &[Block], flat: &mut usize, range: &std::ops::Range<usize>) -> Option<bool> {
        let mut inside = false;
        for block in blocks {
            match block {
                Block::Paragraph(_) => {
                    inside |= *flat == range.start;
                    *flat += 1;
                    if *flat == range.end {
                        return Some(inside);
                    }
                }
                // A table, a content control, a bookmark's edge or an
                // imported chunk between them: a join would take it.
                _ if inside => return Some(false),
                Block::Table(table) => {
                    for cell in table.rows.iter().flat_map(|row| &row.cells) {
                        if let Some(answer) = walk(&cell.content, flat, range) {
                            return Some(answer);
                        }
                    }
                }
                Block::Structured(sdt) => {
                    if let Some(answer) = walk(&sdt.content, flat, range) {
                        return Some(answer);
                    }
                }
                _ => {}
            }
        }
        None
    }
    range.is_empty() || walk(document.blocks(scope), &mut 0, &range).unwrap_or(false)
}

/// One step down from a flow to a container of blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Cell {
        block: usize,
        row: usize,
        cell: usize,
    },
    Control {
        block: usize,
    },
}

/// Where paragraph `paragraph` of the flattened walk stands: the steps down to
/// its container, and its index among that container's blocks.
fn place_of(blocks: &[Block], flat: &mut usize, paragraph: usize) -> Option<(Vec<Step>, usize)> {
    for (index, block) in blocks.iter().enumerate() {
        match block {
            Block::Paragraph(_) => {
                if *flat == paragraph {
                    return Some((Vec::new(), index));
                }
                *flat += 1;
            }
            Block::Table(table) => {
                for (at_row, row) in table.rows.iter().enumerate() {
                    for (at_cell, cell) in row.cells.iter().enumerate() {
                        if let Some((mut steps, at)) = place_of(&cell.content, flat, paragraph) {
                            steps.insert(
                                0,
                                Step::Cell {
                                    block: index,
                                    row: at_row,
                                    cell: at_cell,
                                },
                            );
                            return Some((steps, at));
                        }
                    }
                }
            }
            Block::Structured(sdt) => {
                if let Some((mut steps, at)) = place_of(&sdt.content, flat, paragraph) {
                    steps.insert(0, Step::Control { block: index });
                    return Some((steps, at));
                }
            }
            _ => {}
        }
    }
    None
}

/// The steps down to the container paragraph `paragraph` stands in.
fn container_of(document: &Document, scope: Scope, paragraph: usize) -> Option<Vec<Step>> {
    place_of(document.blocks(scope), &mut 0, paragraph).map(|(steps, _)| steps)
}

/// The container `steps` lead to.
fn container_mut<'a>(mut blocks: &'a mut Vec<Block>, steps: &[Step]) -> Option<&'a mut Vec<Block>> {
    for step in steps {
        blocks = match *step {
            Step::Cell { block, row, cell } => match blocks.get_mut(block)? {
                Block::Table(table) => &mut table.rows.get_mut(row)?.cells.get_mut(cell)?.content,
                _ => return None,
            },
            Step::Control { block } => match blocks.get_mut(block)? {
                Block::Structured(sdt) => &mut sdt.content,
                _ => return None,
            },
        };
    }
    Some(blocks)
}

/// Deletes from `start` to `end` — two paragraphs of one container with a
/// table or a content control between them — taking what stands between
/// whole, as Word does, and joining the two. One undo step gives back the
/// blocks as they were, tables and all.
fn delete_across_blocks(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    start: Caret,
    end: Caret,
) -> Option<Caret> {
    let blocks = document.blocks(scope);
    let (steps, first) = place_of(blocks, &mut 0, start.paragraph)?;
    let (other, last) = place_of(blocks, &mut 0, end.paragraph)?;
    if steps != other || last <= first {
        return None;
    }
    // The flow's own blocks the change lies in: the range itself, or the one
    // block — a table, a content control — that holds its container.
    let (from, to) = match steps.first() {
        None => (first, last),
        Some(Step::Cell { block, .. } | Step::Control { block }) => (*block, *block),
    };
    let before: Vec<Block> = blocks[from..=to].to_vec();
    let container = container_mut(document.blocks_mut(scope)?, &steps)?;
    let (Block::Paragraph(head), Block::Paragraph(tail)) = (&container[first], &container[last])
    else {
        return None;
    };
    let mut head = head.clone();
    let mut tail = tail.clone();
    let head_len = text::len(&head);
    text::remove(&mut head, start.offset..head_len);
    text::remove(&mut tail, 0..end.offset);
    container.splice(first..=last, [Block::Paragraph(text::merge(&head, &tail))]);
    history.push(
        scope,
        Change::Blocks {
            index: from,
            before,
            // The joined paragraph, or the block that holds its container.
            now: 1,
        },
    );
    Some(start)
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
            // A table or a content control inside the range would go with the
            // splice, rows and all: such a range is not one container's.
            Block::Table(_) | Block::Structured(_) | Block::Anchor(_) | Block::AltChunk { .. }
                if start.is_some() =>
            {
                return false
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
fn replace_in_place(
    document: &mut Document,
    scope: Scope,
    range: std::ops::Range<usize>,
    with: Vec<Paragraph>,
) {
    let mut paragraphs = document.paragraphs_in_mut(scope);
    for (offset, replacement) in with.into_iter().enumerate() {
        if let Some(target) = paragraphs.get_mut(range.start + offset) {
            **target = replacement;
        }
    }
}

/// Types `input` at the selection, replacing it if it is not empty.
pub fn type_text(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    selection: Selection,
    input: &str,
) -> Caret {
    type_text_with(document, scope, history, selection, input, None)
}

/// The same, with the typed text in `with` rather than the formatting a
/// caret there would have: the formatting chosen at the caret with nothing
/// selected — bold pressed at the end of a word, a colour picked there —
/// which Word keeps for the typing that follows. The text goes in as a run
/// of its own, cut into whatever run the caret was in.
pub fn type_text_with(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    selection: Selection,
    input: &str,
    with: Option<wp_model::RunProps>,
) -> Caret {
    let caret = delete_selection(document, scope, history, selection);
    let Some(before) = paragraph_at(document, scope, caret.paragraph) else {
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
        history.push_typing(scope, change, caret.paragraph, caret.offset, word_end);
    } else {
        history.push(scope, change);
    }

    let mut paragraphs = document.paragraphs_in_mut(scope);
    let Some(target) = paragraphs.get_mut(caret.paragraph) else {
        return caret;
    };
    let placed = with.and_then(|props| {
        let at = crate::revise::top_level_split(target, caret.offset)?;
        target.content.insert(
            at,
            wp_model::doc::Inline::Run(wp_model::doc::Run {
                props,
                content: vec![wp_model::doc::Piece::Text(input.into())],
                prop_change: None,
            }),
        );
        Some(caret.offset + input.len())
    });
    let after = placed.unwrap_or_else(|| text::insert(target, caret.offset, input));
    Caret {
        paragraph: caret.paragraph,
        offset: after,
    }
}

/// Removes whatever the selection covers, and returns where the caret lands.
pub fn delete_selection(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    selection: Selection,
) -> Caret {
    if selection.is_empty() {
        return selection.head;
    }
    let (start, end) = selection.ordered();
    if start.paragraph == end.paragraph {
        let Some(before) = paragraph_at(document, scope, start.paragraph) else {
            return start;
        };
        history.push(
            scope,
            Change::Paragraph {
                index: start.paragraph,
                before: Box::new(before),
            },
        );
        let mut paragraphs = document.paragraphs_in_mut(scope);
        if let Some(target) = paragraphs.get_mut(start.paragraph) {
            text::remove(target, start.offset..end.offset);
        }
        return start;
    }

    // Across paragraphs: the first keeps its head, the last keeps its tail, and
    // the two become one.
    let before: Vec<Paragraph> = {
        let paragraphs = document.paragraphs_in(scope);
        paragraphs[start.paragraph..=end.paragraph.min(paragraphs.len() - 1)]
            .iter()
            .map(|p| (*p).clone())
            .collect()
    };
    // Across cells — the two ends in different cells, or one in a cell and
    // one out — nothing is joined: cells are not paragraphs, and Word clears
    // what the selection covers in each and leaves the cells standing. The
    // first keeps its head and the last its tail as before; what lies
    // between is emptied. Joined and written over the first cell alone, the
    // middle cell kept its text and the last its whole.
    if container_of(document, scope, start.paragraph)
        != container_of(document, scope, end.paragraph)
    {
        history.push(
            scope,
            Change::Range {
                first: start.paragraph,
                before: before.clone(),
                now: before.len(),
            },
        );
        let last = before.len() - 1;
        let cleared: Vec<Paragraph> = before
            .iter()
            .enumerate()
            .map(|(index, paragraph)| {
                let mut paragraph = paragraph.clone();
                let len = text::len(&paragraph);
                let (from, to) = match index {
                    0 => (start.offset.min(len), len),
                    i if i == last => (0, end.offset.min(len)),
                    _ => (0, len),
                };
                text::remove(&mut paragraph, from..to);
                paragraph
            })
            .collect();
        replace_range(document, scope, start.paragraph..end.paragraph + 1, cleared);
        return start;
    }
    if !side_by_side(document, scope, start.paragraph..end.paragraph + 1) {
        return delete_across_blocks(document, scope, history, start, end).unwrap_or(start);
    }
    history.push(
        scope,
        Change::Range {
            first: start.paragraph,
            before: before.clone(),
            // The range becomes the one joined paragraph.
            now: 1,
        },
    );

    let mut head = before[0].clone();
    let head_len = text::len(&head);
    text::remove(&mut head, start.offset..head_len);
    let mut tail = before[before.len() - 1].clone();
    text::remove(&mut tail, 0..end.offset);
    let joined = text::merge(&head, &tail);
    replace_range(
        document,
        scope,
        start.paragraph..end.paragraph + 1,
        vec![joined],
    );
    start
}

/// Splits the paragraph at the caret — the Enter key.
pub fn split_paragraph(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    selection: Selection,
) -> Caret {
    let caret = delete_selection(document, scope, history, selection);
    let Some(paragraph) = paragraph_at(document, scope, caret.paragraph) else {
        return caret;
    };
    let (mut head, mut tail) = text::split(&paragraph, caret.offset);
    crate::revise::distinct_ids(
        &mut [&mut head, &mut tail],
        crate::revise::next_revision_id(document),
    );
    // Word's `<w:next>`: the paragraph after a heading is not a heading.
    if let Some(style) = paragraph.props.style {
        if let Some(next) = document.styles.get(style).and_then(|style| style.next) {
            if text::len(&paragraph) == caret.offset {
                tail.props.style = Some(next);
            }
        }
    }
    history.push(
        scope,
        Change::Split {
            index: caret.paragraph,
        },
    );
    replace_range(
        document,
        scope,
        caret.paragraph..caret.paragraph + 1,
        vec![head, tail],
    );
    Caret {
        paragraph: caret.paragraph + 1,
        offset: 0,
    }
}

/// Backspace: one character, or the paragraph mark before this paragraph.
pub fn backspace(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    selection: Selection,
) -> Caret {
    if !selection.is_empty() {
        return delete_selection(document, scope, history, selection);
    }
    let caret = selection.head;
    let Some(paragraph) = paragraph_at(document, scope, caret.paragraph) else {
        return caret;
    };
    if caret.offset > 0 {
        let previous = text::previous_char(&text::content(&paragraph), caret.offset);
        history.push(
            scope,
            Change::Paragraph {
                index: caret.paragraph,
                before: Box::new(paragraph),
            },
        );
        let mut paragraphs = document.paragraphs_in_mut(scope);
        if let Some(target) = paragraphs.get_mut(caret.paragraph) {
            text::remove(target, previous..caret.offset);
        }
        return Caret {
            paragraph: caret.paragraph,
            offset: previous,
        };
    }
    // Nothing joins across a cell's edge: the last paragraph of one cell and
    // the first of the next are not side by side.
    if caret.paragraph == 0
        || !side_by_side(document, scope, caret.paragraph - 1..caret.paragraph + 1)
    {
        return caret;
    }
    join_with_previous(document, scope, history, caret.paragraph)
}

/// Delete: one character forward, or the paragraph mark after this paragraph.
pub fn delete_forward(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    selection: Selection,
) -> Caret {
    if !selection.is_empty() {
        return delete_selection(document, scope, history, selection);
    }
    let caret = selection.head;
    let Some(paragraph) = paragraph_at(document, scope, caret.paragraph) else {
        return caret;
    };
    let content = text::content(&paragraph);
    if caret.offset < content.len() {
        let next = text::next_char(&content, caret.offset);
        history.push(
            scope,
            Change::Paragraph {
                index: caret.paragraph,
                before: Box::new(paragraph),
            },
        );
        let mut paragraphs = document.paragraphs_in_mut(scope);
        if let Some(target) = paragraphs.get_mut(caret.paragraph) {
            text::remove(target, caret.offset..next);
        }
        return caret;
    }
    if caret.paragraph + 1 >= document.paragraphs_in(scope).len()
        || !side_by_side(document, scope, caret.paragraph..caret.paragraph + 2)
    {
        return caret;
    }
    join_with_previous(document, scope, history, caret.paragraph + 1)
}

/// Joins paragraph `index` onto the one before it.
fn join_with_previous(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    index: usize,
) -> Caret {
    let (first, second) = {
        let paragraphs = document.paragraphs_in(scope);
        match (paragraphs.get(index - 1), paragraphs.get(index)) {
            (Some(a), Some(b)) => (Box::new((*a).clone()), Box::new((*b).clone())),
            _ => return Caret::default(),
        }
    };
    let offset = text::len(&first);
    let joined = text::merge(&first, &second);
    history.push(
        scope,
        Change::Merge {
            index: index - 1,
            first,
            second,
        },
    );
    replace_range(document, scope, index - 1..index + 1, vec![joined]);
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
    scope: Scope,
    history: &mut History,
    selection: Selection,
    change: impl Fn(&mut RunProps) + Copy,
) {
    if selection.is_empty() {
        return;
    }
    let (start, end) = selection.ordered();
    let before: Vec<Paragraph> = {
        let paragraphs = document.paragraphs_in(scope);
        paragraphs[start.paragraph..=end.paragraph.min(paragraphs.len() - 1)]
            .iter()
            .map(|p| (*p).clone())
            .collect()
    };
    history.push(
        scope,
        Change::Range {
            first: start.paragraph,
            now: before.len(),
            before,
        },
    );

    let mut paragraphs = document.paragraphs_in_mut(scope);
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
        let end_of_paragraph = to == text::len(target);
        split_runs_at(target, from);
        split_runs_at(target, to);
        apply_to_range(target, from..to, change);
        // Word counts the paragraph mark as selected once the selection
        // reaches the end of the paragraph, and sets it with the rest. The mark
        // is what gives an empty paragraph its height and what a caret typing
        // there inherits, so a pass that puts a whole document into one face
        // would otherwise leave every blank line standing at the size the
        // document was started in — taller than its neighbours, and enough of
        // them to push the last of the text onto another page.
        if end_of_paragraph {
            let mut mark = target.props.mark.as_deref().cloned().unwrap_or_default();
            change(&mut mark);
            target.props.mark = Some(Box::new(mark));
        }
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
    scope: Scope,
    history: &mut History,
    selection: Selection,
    change: impl Fn(&mut ParaProps) + Copy,
) {
    let (start, end) = selection.ordered();
    let before: Vec<Paragraph> = {
        let paragraphs = document.paragraphs_in(scope);
        if paragraphs.is_empty() {
            return;
        }
        paragraphs
            [start.paragraph.min(paragraphs.len() - 1)..=end.paragraph.min(paragraphs.len() - 1)]
            .iter()
            .map(|p| (*p).clone())
            .collect()
    };
    history.push(
        scope,
        Change::Range {
            first: start.paragraph,
            now: before.len(),
            before,
        },
    );
    let mut paragraphs = document.paragraphs_in_mut(scope);
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
    scope: Scope,
    selection: Selection,
    test: impl Fn(&RunProps) -> bool,
) -> bool {
    let (start, end) = selection.ordered();
    let paragraphs = document.paragraphs_in(scope);
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

pub fn paragraph_at(document: &Document, scope: Scope, index: usize) -> Option<Paragraph> {
    document.paragraph_in(scope, index).cloned()
}

/// What the selection covers, as paragraphs — runs, properties and all.
///
/// The paragraphs at the ends are the document's own with everything outside the
/// selection removed, so a bold word stays bold and a bulleted item stays a
/// bulleted item. Copying the *text* is what loses that, and copying the text is
/// all the clipboard could do.
pub fn copy_range(document: &Document, scope: Scope, selection: Selection) -> Vec<Paragraph> {
    if selection.is_empty() {
        return Vec::new();
    }
    let (start, end) = selection.ordered();
    let paragraphs = document.paragraphs_in(scope);
    if start.paragraph >= paragraphs.len() {
        return Vec::new();
    }
    let last = end.paragraph.min(paragraphs.len() - 1);
    if start.paragraph == last {
        let mut only = paragraphs[start.paragraph].clone();
        let total = text::len(&only);
        text::remove(&mut only, end.offset.min(total)..total);
        text::remove(&mut only, 0..start.offset.min(total));
        return vec![only];
    }
    let mut out = Vec::with_capacity(last - start.paragraph + 1);
    let mut first = paragraphs[start.paragraph].clone();
    let head = start.offset.min(text::len(&first));
    text::remove(&mut first, 0..head);
    out.push(first);
    for paragraph in &paragraphs[start.paragraph + 1..last] {
        out.push((*paragraph).clone());
    }
    let mut tail = paragraphs[last].clone();
    let total = text::len(&tail);
    text::remove(&mut tail, end.offset.min(total)..total);
    out.push(tail);
    out
}

/// Pastes copied paragraphs over the selection, keeping their formatting.
///
/// The first one joins onto whatever the caret was in and the last one takes
/// whatever followed it, which is what makes pasting half a sentence into the
/// middle of another sentence give one sentence rather than three paragraphs.
/// The paragraphs *between* those two arrive whole, with their own properties —
/// paste a bulleted list and it is still a bulleted list.
pub fn paste_paragraphs(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    selection: Selection,
    clip: &[Paragraph],
) -> Caret {
    paste_where(document, scope, history, selection, clip, false)
}

/// The same, after a tracked deletion standing at the caret rather than
/// before it: a paste over a selection deleted with Track Changes on.
pub fn paste_after_deletion(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    caret: Caret,
    clip: &[Paragraph],
) -> Caret {
    paste_where(document, scope, history, Selection::at(caret), clip, true)
}

fn paste_where(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    selection: Selection,
    clip: &[Paragraph],
    deletion_first: bool,
) -> Caret {
    let caret = delete_selection(document, scope, history, selection);
    let Some(target) = paragraph_at(document, scope, caret.paragraph) else {
        return caret;
    };
    let clip = without_anchors_held(document, clip);
    let Some((clip_first, rest)) = clip.split_first() else {
        return caret;
    };
    let (mut head, mut tail) = text::split_where(&target, caret.offset, deletion_first);
    crate::revise::distinct_ids(
        &mut [&mut head, &mut tail],
        crate::revise::next_revision_id(document),
    );

    if rest.is_empty() {
        // All of it lands inside the one paragraph.
        history.push(
            scope,
            Change::Paragraph {
                index: caret.paragraph,
                before: Box::new(target),
            },
        );
        let joined = text::merge(&text::merge(&head, clip_first), &tail);
        replace_range(
            document,
            scope,
            caret.paragraph..caret.paragraph + 1,
            vec![joined],
        );
        return Caret {
            paragraph: caret.paragraph,
            offset: caret.offset + text::len(clip_first),
        };
    }

    let mut built = Vec::with_capacity(clip.len());
    built.push(text::merge(&head, clip_first));
    let (clip_last, middle) = rest.split_last().expect("rest is not empty");
    built.extend(middle.iter().cloned());
    built.push(text::merge(clip_last, &tail));

    history.push(
        scope,
        Change::Range {
            first: caret.paragraph,
            before: vec![target],
            now: built.len(),
        },
    );
    let landed = Caret {
        paragraph: caret.paragraph + built.len() - 1,
        offset: text::len(clip_last),
    };
    replace_range(document, scope, caret.paragraph..caret.paragraph + 1, built);
    landed
}

/// `clip` without the anchors the document already holds — a comment's, a
/// bookmark's: a copy pasted beside its original would give one comment two
/// places. A cut leaves its anchors where the text was, so a comment on cut
/// text stays there, with nothing under it, rather than going with the
/// paste as Word's does.
fn without_anchors_held(document: &Document, clip: &[Paragraph]) -> Vec<Paragraph> {
    use wp_model::doc::{Inline, Piece};
    fn gather(content: &[Inline], anchors: &mut Vec<wp_model::Anchor>, refs: &mut Vec<u32>) {
        for inline in content {
            match inline {
                Inline::Anchor(anchor) => anchors.push(anchor.clone()),
                Inline::Run(run) => {
                    refs.extend(run.content.iter().filter_map(|piece| match piece {
                        Piece::CommentRef(id) => Some(*id),
                        _ => None,
                    }))
                }
                Inline::Revised { content, .. }
                | Inline::Wrapper { content, .. }
                | Inline::SimpleField { content, .. } => gather(content, anchors, refs),
                Inline::Hyperlink(link) => gather(&link.content, anchors, refs),
                Inline::Structured(sdt) => gather(&sdt.content, anchors, refs),
                Inline::Math(_) => {}
            }
        }
    }
    fn strip(content: &mut Vec<Inline>, anchors: &[wp_model::Anchor], refs: &[u32]) {
        content
            .retain(|inline| !matches!(inline, Inline::Anchor(anchor) if anchors.contains(anchor)));
        for inline in content.iter_mut() {
            match inline {
                Inline::Run(run) => run
                    .content
                    .retain(|piece| !matches!(piece, Piece::CommentRef(id) if refs.contains(id))),
                Inline::Revised { content, .. }
                | Inline::Wrapper { content, .. }
                | Inline::SimpleField { content, .. } => strip(content, anchors, refs),
                Inline::Hyperlink(link) => strip(&mut link.content, anchors, refs),
                Inline::Structured(sdt) => strip(&mut sdt.content, anchors, refs),
                Inline::Anchor(_) | Inline::Math(_) => {}
            }
        }
    }
    let (mut anchors, mut refs) = (Vec::new(), Vec::new());
    for scope in document.flows() {
        for paragraph in document.paragraphs_in(scope) {
            gather(&paragraph.content, &mut anchors, &mut refs);
        }
    }
    clip.iter()
        .map(|paragraph| {
            let mut paragraph = paragraph.clone();
            strip(&mut paragraph.content, &anchors, &refs);
            text::prune(&mut paragraph);
            paragraph
        })
        .collect()
}

/// The alignment of the paragraph the caret is in.
pub fn justify_at(document: &Document, scope: Scope, caret: Caret) -> Option<Justify> {
    let paragraph = document.paragraph_in(scope, caret.paragraph)?;
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

        let caret = history
            .undo(&mut document)
            .map(|(_, caret)| caret)
            .expect("there is an undo");
        assert_eq!(
            document.section.margins.top, was,
            "the old margins are back"
        );
        assert_eq!(caret, here, "undo returns to where the user was");
        history.redo(&mut document).map(|(_, caret)| caret);
        assert_eq!(document.section.margins.top, Twips(2880));
    }

    #[test]
    fn a_page_break_lands_at_the_caret_and_undoes_away() {
        use wp_model::doc::{Break, Piece};
        let mut document = document(&["hello world"]);
        let mut history = History::new();
        let caret = insert_break(
            &mut document,
            Scope::Body,
            &mut history,
            at(0, 5),
            Break::Page,
        );
        // The break ends its paragraph, and the caret is at the head of the
        // next one: the far side of the break, which an offset alone cannot say.
        assert_eq!(
            caret,
            Caret {
                paragraph: 1,
                offset: 0
            }
        );
        let pieces = |document: &Document, index: usize| -> Vec<Piece> {
            document.paragraphs()[index]
                .runs()
                .iter()
                .flat_map(|run| run.content.iter().cloned())
                .collect()
        };
        assert_eq!(document.paragraphs().len(), 2);
        assert!(
            matches!(pieces(&document, 0).last(), Some(Piece::Break(Break::Page))),
            "the break is the last thing in the paragraph it ends"
        );
        assert_eq!(document.paragraphs()[0].text(), "hello");
        assert_eq!(document.paragraphs()[1].text(), " world");

        history.undo(&mut document).map(|(_, caret)| caret);
        assert_eq!(document.paragraphs().len(), 1, "one undo takes it all back");
        assert!(
            !pieces(&document, 0)
                .iter()
                .any(|piece| matches!(piece, Piece::Break(_))),
            "break and paragraph mark both"
        );
        assert_eq!(document.paragraphs()[0].text(), "hello world");
    }

    #[test]
    fn typing_inserts_and_undo_takes_it_back() {
        let mut document = document(&["hello"]);
        let mut history = History::new();
        let caret = type_text(&mut document, Scope::Body, &mut history, at(0, 5), " world");
        assert_eq!(document.text(), "hello world");
        assert_eq!(caret.offset, 11);

        history.undo(&mut document).map(|(_, caret)| caret);
        assert_eq!(document.text(), "hello");
        history.redo(&mut document).map(|(_, caret)| caret);
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
                Scope::Body,
                &mut history,
                Selection::at(caret),
                &letter.to_string(),
            );
        }
        assert_eq!(document.text(), "hello");
        history.undo(&mut document).map(|(_, caret)| caret);
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
                Scope::Body,
                &mut history,
                Selection::at(caret),
                &letter.to_string(),
            );
        }
        history.undo(&mut document).map(|(_, caret)| caret);
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
        let caret = split_paragraph(&mut document, Scope::Body, &mut history, at(0, 5));
        assert_eq!(document.paragraphs().len(), 2);
        assert_eq!(document.text(), "hello\n world");
        assert_eq!(
            caret,
            Caret {
                paragraph: 1,
                offset: 0
            }
        );

        history.undo(&mut document).map(|(_, caret)| caret);
        assert_eq!(document.paragraphs().len(), 1);
        assert_eq!(document.text(), "hello world");
        history.redo(&mut document).map(|(_, caret)| caret);
        assert_eq!(document.paragraphs().len(), 2);
    }

    #[test]
    fn backspace_at_the_start_of_a_paragraph_joins_it_to_the_one_before() {
        let mut document = document(&["first", "second"]);
        let mut history = History::new();
        let caret = backspace(&mut document, Scope::Body, &mut history, at(1, 0));
        assert_eq!(document.paragraphs().len(), 1);
        assert_eq!(document.text(), "firstsecond");
        assert_eq!(
            caret,
            Caret {
                paragraph: 0,
                offset: 5
            }
        );

        history.undo(&mut document).map(|(_, caret)| caret);
        assert_eq!(document.paragraphs().len(), 2);
        assert_eq!(document.text(), "first\nsecond");
    }

    #[test]
    fn backspace_at_the_very_start_of_the_document_does_nothing() {
        let mut document = document(&["only"]);
        let mut history = History::new();
        backspace(&mut document, Scope::Body, &mut history, at(0, 0));
        assert_eq!(document.text(), "only");
        assert!(!history.can_undo(), "nothing happened, so nothing to undo");
    }

    #[test]
    fn delete_at_the_end_joins_the_next_paragraph_up() {
        let mut document = document(&["first", "second"]);
        let mut history = History::new();
        let caret = delete_forward(&mut document, Scope::Body, &mut history, at(0, 5));
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
        let caret = delete_selection(
            &mut document,
            Scope::Body,
            &mut history,
            span((0, 2), (2, 2)),
        );
        assert_eq!(document.paragraphs().len(), 1);
        assert_eq!(document.text(), "fist");
        assert_eq!(
            caret,
            Caret {
                paragraph: 0,
                offset: 2
            }
        );

        history.undo(&mut document).map(|(_, caret)| caret);
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
        delete_selection(
            &mut document,
            Scope::Body,
            &mut history,
            span((1, 1), (2, 1)),
        );
        assert_eq!(document.text(), "aa\nbc\ndd");
        history.undo(&mut document).map(|(_, caret)| caret);
        assert_eq!(document.text(), "aa\nbb\ncc\ndd");
        history.redo(&mut document).map(|(_, caret)| caret);
        assert_eq!(document.text(), "aa\nbc\ndd");
    }

    #[test]
    fn typing_over_a_selection_replaces_it() {
        let mut document = document(&["hello world"]);
        let mut history = History::new();
        let caret = type_text(
            &mut document,
            Scope::Body,
            &mut history,
            span((0, 0), (0, 5)),
            "goodbye",
        );
        assert_eq!(document.text(), "goodbye world");
        assert_eq!(caret.offset, 7);
    }

    #[test]
    fn bolding_half_a_word_bolds_half_a_word() {
        // The run has to be cut where the selection stops, or the whole run
        // takes the formatting and it looks like the selection was ignored.
        let mut document = document(&["boldplain"]);
        let mut history = History::new();
        format_runs(
            &mut document,
            Scope::Body,
            &mut history,
            span((0, 0), (0, 4)),
            |props| props.toggles.set(Toggle::Bold, true),
        );
        let paragraphs = document.paragraphs();
        let runs = paragraphs[0].runs();
        assert_eq!(runs.len(), 2, "the run was cut in two");
        assert!(runs[0].props.bold());
        assert!(!runs[1].props.bold());
        assert_eq!(document.text(), "boldplain", "and nothing moved");
    }

    #[test]
    fn a_selection_that_reaches_the_end_of_a_paragraph_sets_its_mark_too() {
        // The blank line between two paragraphs has no run to carry a face:
        // its height, and what the caret inherits when it types there, come
        // from the mark alone. Setting the whole document in one face has to
        // reach it.
        let mut document = document(&["one", "", "two"]);
        let mut history = History::new();
        let end = text::len(document.paragraphs()[2]);
        format_runs(
            &mut document,
            Scope::Body,
            &mut history,
            span((0, 0), (2, end)),
            |props| props.toggles.set(Toggle::Bold, true),
        );
        let paragraphs = document.paragraphs();
        for (index, paragraph) in paragraphs.iter().enumerate() {
            let mark = paragraph.props.mark.as_deref().expect("a mark was set");
            assert!(mark.bold(), "paragraph {index}'s mark");
        }
    }

    #[test]
    fn a_selection_stopping_short_of_the_end_leaves_the_mark_alone() {
        let mut document = document(&["boldplain"]);
        let mut history = History::new();
        format_runs(
            &mut document,
            Scope::Body,
            &mut history,
            span((0, 0), (0, 4)),
            |props| props.toggles.set(Toggle::Bold, true),
        );
        assert!(
            document.paragraphs()[0].props.mark.is_none(),
            "the mark is past the selection"
        );
    }

    #[test]
    fn formatting_is_undoable_like_everything_else() {
        let mut document = document(&["text"]);
        let mut history = History::new();
        format_runs(
            &mut document,
            Scope::Body,
            &mut history,
            span((0, 0), (0, 4)),
            |props| props.toggles.set(Toggle::Italic, true),
        );
        assert!(document.paragraphs()[0].runs()[0].props.italic());
        history.undo(&mut document).map(|(_, caret)| caret);
        assert!(!document.paragraphs()[0].runs()[0].props.italic());
    }

    #[test]
    fn centring_a_paragraph_needs_no_selection() {
        let mut document = document(&["one", "two"]);
        let mut history = History::new();
        format_paragraphs(
            &mut document,
            Scope::Body,
            &mut history,
            at(1, 0),
            |props| props.justify = Some(Justify::Center),
        );
        assert_eq!(document.paragraphs()[0].props.justify, None);
        assert_eq!(
            document.paragraphs()[1].props.justify,
            Some(Justify::Center)
        );
        history.undo(&mut document).map(|(_, caret)| caret);
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
        type_text(&mut document, Scope::Body, &mut history, at(0, 1), "X");
        split_paragraph(&mut document, Scope::Body, &mut history, at(0, 1));
        let after = document.text();

        history.undo(&mut document).map(|(_, caret)| caret);
        history.undo(&mut document).map(|(_, caret)| caret);
        assert_eq!(document.text(), "a\nb");
        history.redo(&mut document).map(|(_, caret)| caret);
        history.redo(&mut document).map(|(_, caret)| caret);
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
        let caret = split_paragraph(&mut document, Scope::Body, &mut history, at(0, 5));
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
        history.undo(&mut document).map(|(_, caret)| caret);
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
        let caret = backspace(&mut document, Scope::Body, &mut history, at(1, 0));
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
        history.undo(&mut document).map(|(_, caret)| caret);
        assert_eq!(
            texts(&document),
            ["first item", "second item", "after the table"]
        );
    }

    #[test]
    fn deleting_a_selection_across_cell_paragraphs_joins_them() {
        let mut document = cell_document(&["first item", "second item"]);
        let mut history = History::new();
        delete_selection(
            &mut document,
            Scope::Body,
            &mut history,
            span((0, 5), (1, 6)),
        );
        assert_eq!(texts(&document), ["first item", "after the table"]);
        history.undo(&mut document).map(|(_, caret)| caret);
        assert_eq!(
            texts(&document),
            ["first item", "second item", "after the table"]
        );
    }

    /// Measured on Word 16 (`bugs/page-break-in-table-cell.md` in the story):
    /// a break *in* a cell is nothing to the layout, and Ctrl+Enter in a cell
    /// does not put one there. It splits the table before the caret's row and
    /// writes the break in a paragraph of its own between the halves.
    #[test]
    fn a_page_break_in_a_cell_splits_the_table_before_the_carets_row() {
        use wp_model::doc::{Break, Piece};
        let row = |text: &str| wp_model::table::Row {
            cells: vec![wp_model::table::Cell {
                props: wp_model::table::CellProps::new(),
                content: vec![Block::Paragraph(Paragraph::of(text))],
            }],
            ..wp_model::table::Row::new()
        };
        let table = wp_model::table::Table {
            grid: vec![wp_model::units::Twips(4000)],
            rows: vec![row("one"), row("two"), row("three")],
            ..wp_model::table::Table::new()
        };
        let mut document = Document {
            body: vec![
                Block::Paragraph(Paragraph::of("before")),
                Block::Table(table),
                Block::Paragraph(Paragraph::of("after")),
            ],
            ..Document::new()
        };
        let shape = |document: &Document| -> Vec<String> {
            document
                .body
                .iter()
                .map(|block| match block {
                    Block::Paragraph(p) => {
                        let broken = p
                            .runs()
                            .iter()
                            .flat_map(|run| run.content.iter())
                            .any(|piece| matches!(piece, Piece::Break(Break::Page)));
                        if broken {
                            "break".to_owned()
                        } else {
                            p.text()
                        }
                    }
                    Block::Table(t) => format!("table:{}", t.rows.len()),
                    _ => "?".to_owned(),
                })
                .collect()
        };
        let mut history = History::new();
        // The caret is in "two": paragraph 2 of the walk, one character in.
        let caret = insert_break(
            &mut document,
            Scope::Body,
            &mut history,
            at(2, 1),
            Break::Page,
        );
        assert_eq!(
            shape(&document),
            ["before", "table:1", "break", "table:2", "after"],
            "the table is split before the caret's row, the break between the halves"
        );
        assert_eq!(
            caret,
            Caret {
                paragraph: 3,
                offset: 1
            },
            "the caret is where it was in its cell, which is now in the second table"
        );
        assert_eq!(texts(&document)[3], "two");
        assert!(
            !document.paragraphs()[3].text().contains('\u{c}'),
            "nothing was put in the cell"
        );

        history.undo(&mut document).map(|(_, caret)| caret);
        assert_eq!(
            shape(&document),
            ["before", "table:3", "after"],
            "one undo puts the table back together"
        );
        history.redo(&mut document).map(|(_, caret)| caret);
        assert_eq!(
            shape(&document),
            ["before", "table:1", "break", "table:2", "after"]
        );

        // In the first row there is no row to keep above the break: the
        // break goes before the table, as Word's does.
        let caret = insert_break(
            &mut document,
            Scope::Body,
            &mut history,
            at(1, 0),
            Break::Page,
        );
        assert_eq!(
            shape(&document),
            ["before", "break", "table:1", "break", "table:2", "after"],
            "no empty table is left above the break"
        );
        assert_eq!(
            caret,
            Caret {
                paragraph: 2,
                offset: 0
            }
        );
    }

    /// "before", a table, "after": the three blocks of a body with a table
    /// between two paragraphs.
    fn table_between() -> Document {
        let mut document = cell_document(&["cell"]);
        document
            .body
            .insert(0, Block::Paragraph(Paragraph::of("before")));
        document
    }

    /// A selection from before a table to after it took the table with it,
    /// and undo could not bring it back: the paragraphs around it were
    /// spliced over it as if nothing stood between.
    #[test]
    fn a_selection_across_a_table_takes_it_whole_and_undo_brings_it_back() {
        let mut document = table_between();
        let before = document.body.clone();
        let mut history = History::new();
        let caret = delete_selection(
            &mut document,
            Scope::Body,
            &mut history,
            span((0, 3), (2, 9)),
        );
        assert_eq!(caret, at(0, 3).head);
        assert_eq!(texts(&document), ["bef table"]);
        assert_eq!(document.body.len(), 1, "the table went whole");
        history.undo(&mut document);
        assert_eq!(document.body, before, "and came back whole");
        history.redo(&mut document);
        assert_eq!(texts(&document), ["bef table"]);

        // Inside a cell, a nested table between two of its paragraphs.
        let mut outer = cell_document(&["one", "two"]);
        let nested = table_between().body.remove(1);
        if let Block::Table(table) = &mut outer.body[0] {
            table.rows[0].cells[0].content.insert(1, nested);
        }
        let before = outer.body.clone();
        delete_selection(&mut outer, Scope::Body, &mut history, span((0, 1), (2, 1)));
        assert_eq!(texts(&outer), ["owo", "after the table"]);
        history.undo(&mut outer);
        assert_eq!(outer.body, before);
    }

    /// A count of paragraphs is never changed across a table: a splice that
    /// would have to reach over one is refused.
    #[test]
    fn paragraphs_are_side_by_side_only_with_nothing_but_paragraphs_between() {
        let document = table_between();
        assert!(side_by_side(&document, Scope::Body, 0..1));
        assert!(!side_by_side(&document, Scope::Body, 0..2), "into the cell");
        assert!(
            !side_by_side(&document, Scope::Body, 0..3),
            "over the table"
        );
        assert!(
            !side_by_side(&document, Scope::Body, 1..3),
            "out of the cell"
        );
        let cells = cell_document(&["one", "two"]);
        assert!(side_by_side(&cells, Scope::Body, 0..2), "in one cell");

        // Replaced one for one across the table, the paragraphs are
        // overwritten where they stand and the table stays: a splice would
        // have taken it.
        let mut document = table_between();
        let with: Vec<Paragraph> = ["a", "b", "c"]
            .iter()
            .map(|text| Paragraph::of(text))
            .collect();
        replace_range(&mut document, Scope::Body, 0..3, with);
        assert_eq!(texts(&document), ["a", "b", "c"]);
        assert!(matches!(document.body[1], Block::Table(_)));
    }

    /// Delete at the end of a cell's last paragraph joined the next cell's
    /// text onto it and left that text where it was too.
    #[test]
    fn backspace_and_delete_never_join_across_a_cells_edge() {
        let mut document = cell_document(&["one"]);
        let mut history = History::new();
        let before = document.body.clone();
        let caret = delete_forward(&mut document, Scope::Body, &mut history, at(0, 3));
        assert_eq!(caret, at(0, 3).head);
        assert_eq!(document.body, before);
        let caret = backspace(&mut document, Scope::Body, &mut history, at(1, 0));
        assert_eq!(caret, at(1, 0).head);
        assert_eq!(document.body, before);
        assert!(!history.can_undo(), "nothing happened");
    }

    /// A copy of commented words pasted beside the original took the
    /// comment's anchors along, and the comment had two places. Cut and
    /// pasted, the words still bring them.
    #[test]
    fn a_copy_pasted_beside_its_original_leaves_the_comment_where_it_was() {
        use wp_model::doc::Inline;
        let commented = Paragraph {
            content: vec![
                Inline::Anchor(wp_model::Anchor::CommentStart { id: 7 }),
                Inline::Run(wp_model::doc::Run::of("noted")),
                Inline::Anchor(wp_model::Anchor::CommentEnd { id: 7 }),
                Inline::Run(wp_model::doc::Run {
                    content: vec![wp_model::doc::Piece::CommentRef(7)],
                    ..wp_model::doc::Run::new()
                }),
            ],
            ..Paragraph::new()
        };
        let mut document = Document {
            body: vec![
                Block::Paragraph(commented.clone()),
                Block::Paragraph(Paragraph::of("here")),
            ],
            ..Document::new()
        };
        let mut history = History::new();
        let clip = copy_range(&document, Scope::Body, span((0, 0), (0, 5)));
        paste_paragraphs(&mut document, Scope::Body, &mut history, at(1, 4), &clip);
        assert_eq!(texts(&document), ["noted", "herenoted"]);
        let anchors = |paragraph: &Paragraph| {
            paragraph
                .content
                .iter()
                .filter(|inline| matches!(inline, Inline::Anchor(_)))
                .count()
        };
        assert_eq!(anchors(document.paragraphs()[1]), 0, "the copy has none");
        assert!(!document.paragraphs()[1]
            .runs()
            .iter()
            .any(|run| run.content.contains(&wp_model::doc::Piece::CommentRef(7))));

        let mut document = Document {
            body: vec![
                Block::Paragraph(commented),
                Block::Paragraph(Paragraph::of("here")),
            ],
            ..Document::new()
        };
        let clip = copy_range(&document, Scope::Body, span((0, 0), (0, 5)));
        document.body.remove(0);
        paste_paragraphs(&mut document, Scope::Body, &mut history, at(0, 4), &clip);
        assert_eq!(anchors(document.paragraphs()[0]), 2, "cut, it brings them");
    }

    /// A contents list whose field fits in one paragraph gets its entries
    /// after it: an empty range is an insertion, where it overwrote the
    /// paragraphs that followed.
    #[test]
    fn an_empty_range_is_an_insertion_after_the_paragraph_before_it() {
        let mut document = table_between();
        replace_range(&mut document, Scope::Body, 1..1, vec![Paragraph::of("new")]);
        assert_eq!(
            texts(&document),
            ["before", "new", "cell", "after the table"]
        );
        assert_eq!(document.body.len(), 4);
        // Inside a cell, it stays in the cell.
        let mut document = cell_document(&["one", "two"]);
        replace_range(&mut document, Scope::Body, 1..1, vec![Paragraph::of("new")]);
        assert_eq!(texts(&document), ["one", "new", "two", "after the table"]);
        assert_eq!(document.body.len(), 2);
    }

    /// A selection from one nested cell to another inside one outer cell was
    /// never deleted: the outer cell answered for both ends.
    #[test]
    fn a_selection_across_nested_cells_clears_each() {
        let mut outer = cell_document(&["one"]);
        let nested = Block::Table(wp_model::table::Table {
            rows: vec![wp_model::table::Row {
                cells: vec![
                    wp_model::table::Cell {
                        props: wp_model::table::CellProps::new(),
                        content: vec![Block::Paragraph(Paragraph::of("alpha"))],
                    },
                    wp_model::table::Cell {
                        props: wp_model::table::CellProps::new(),
                        content: vec![Block::Paragraph(Paragraph::of("mid"))],
                    },
                    wp_model::table::Cell {
                        props: wp_model::table::CellProps::new(),
                        content: vec![Block::Paragraph(Paragraph::of("beta"))],
                    },
                ],
                ..wp_model::table::Row::new()
            }],
            ..wp_model::table::Table::new()
        });
        if let Block::Table(table) = &mut outer.body[0] {
            table.rows[0].cells[0].content.push(nested);
        }
        let before = outer.body.clone();
        let mut history = History::new();
        let caret = delete_selection(&mut outer, Scope::Body, &mut history, span((1, 2), (3, 2)));
        assert_eq!(caret, at(1, 2).head);
        assert_eq!(texts(&outer), ["one", "al", "", "ta", "after the table"]);
        history.undo(&mut outer);
        assert_eq!(outer.body, before);
    }

    /// A change that Enter or a paste cut in two, untracked, is two changes:
    /// one change may not stand in two places.
    #[test]
    fn a_change_cut_untracked_is_two_changes() {
        let proposal = || Document {
            body: vec![Block::Paragraph(Paragraph {
                content: vec![wp_model::doc::inserted_by(
                    "Assistant",
                    1,
                    vec![wp_model::doc::Inline::Run(wp_model::doc::Run::of("abcd"))],
                )],
                ..Paragraph::new()
            })],
            ..Document::new()
        };
        let mut entered = proposal();
        split_paragraph(&mut entered, Scope::Body, &mut History::new(), at(0, 2));
        let mut pasted = proposal();
        paste_paragraphs(
            &mut pasted,
            Scope::Body,
            &mut History::new(),
            at(0, 2),
            &[Paragraph::of("X"), Paragraph::of("Y")],
        );
        for (how, document) in [("Enter", entered), ("a paste", pasted)] {
            let ids: Vec<u32> = crate::revise::tracked(&document)
                .iter()
                .map(|change| change.mark.id)
                .collect();
            assert_eq!(ids.len(), 2, "{how}: {ids:?}");
            assert_ne!(ids[0], ids[1], "{how}");
        }
    }

    /// Joining the paragraphs either side of a bookmark's end that stands
    /// between them would lose it: Backspace does nothing there.
    #[test]
    fn nothing_joins_over_what_stands_between_two_paragraphs() {
        let mut document = Document {
            body: vec![
                Block::Paragraph(Paragraph::of("one")),
                Block::Anchor(wp_model::Anchor::BookmarkEnd { id: 3 }),
                Block::Paragraph(Paragraph::of("two")),
            ],
            ..Document::new()
        };
        let before = document.body.clone();
        let mut history = History::new();
        backspace(&mut document, Scope::Body, &mut history, at(1, 0));
        assert_eq!(document.body, before);
        assert!(!side_by_side(&document, Scope::Body, 0..2));
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
        type_text(&mut document, Scope::Body, &mut history, at(0, 2), "X");
        assert_eq!(document.paragraphs()[0].text(), "inX a cell");
        assert_eq!(document.body.len(), 1, "still one table and nothing beside");
        history.undo(&mut document).map(|(_, caret)| caret);
        assert_eq!(document.paragraphs()[0].text(), "in a cell");
    }
}
