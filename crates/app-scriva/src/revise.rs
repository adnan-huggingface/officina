//! Accepting and rejecting tracked changes, and recording new ones.
//!
//! The model has carried revisions since C16 precisely so that this chunk could
//! exist: a reader that flattened a tracked deletion would have destroyed the
//! author, the date and the text, and no amount of work here could bring them
//! back.
//!
//! **Accepting and rejecting are the same walk with one bit flipped.** An
//! insertion survives accepting and a deletion survives rejecting; that is the
//! whole rule, and writing it twice is how the two drift apart. [`Resolve`] is
//! the bit.

use wp_model::doc::{Block, Document, Inline, Paragraph, Piece, Run};
use wp_model::revision::{Mark, Revision};
use wp_model::Scope;

use crate::edit::{Caret, Change, History, Selection};

/// Which way a tracked change is being settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolve {
    Accept,
    Reject,
}

impl Resolve {
    /// Whether the content inside `revision` survives.
    fn keeps(self, revision: &Revision) -> bool {
        match self {
            Resolve::Accept => revision.survives_accept(),
            Resolve::Reject => revision.survives_reject(),
        }
    }
}

/// Every tracked change in the document, in order, with where it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Tracked {
    /// Which of the document's flows the change is in.
    ///
    /// **A header is reviewed like anything else.** Word tracks an edit to a
    /// running head and comments on one, and a paragraph number alone cannot
    /// say which body it counts through — every flow starts again at zero — so
    /// the scope travels with the change all the way to the pane's *Go to*.
    pub scope: Scope,
    pub paragraph: usize,
    /// Where in the paragraph's text the change begins — a deletion holds no
    /// bytes of the text, so it is a point.
    pub offset: usize,
    pub mark: Mark,
    /// What the change is, in words a person can read.
    pub what: &'static str,
    /// The text it is about, trimmed for a list.
    pub text: String,
}

/// Lists the tracked changes, so they can be walked through one at a time.
///
/// Every flow, the text first: a change in a header is still a change, and one
/// the pane does not list is one nobody can settle.
pub fn tracked(document: &Document) -> Vec<Tracked> {
    let mut out = Vec::new();
    for scope in document.flows() {
        tracked_in(document, scope, &mut out);
    }
    out
}

/// The tracked changes of one flow, appended.
fn tracked_in(document: &Document, scope: Scope, out: &mut Vec<Tracked>) {
    for (index, paragraph) in document.paragraphs_in(scope).iter().enumerate() {
        // A paragraph's own formatting change (`<w:pPrChange>`) is about the
        // whole paragraph, so it comes first.
        if let Some(change) = &paragraph.prop_change {
            out.push(Tracked {
                scope,
                paragraph: index,
                offset: 0,
                mark: change.mark.clone(),
                what: "paragraph formatting changed",
                text: paragraph.text().trim().chars().take(60).collect(),
            });
        }
        let mut offset = 0usize;
        walk(&paragraph.content, scope, index, &mut offset, out);
        // The mark's own formatting can change as a run's does, and has no
        // text either.
        if let Some(change) = &paragraph.mark_change {
            out.push(Tracked {
                scope,
                paragraph: index,
                offset,
                mark: change.mark.clone(),
                what: "formatting changed",
                text: String::new(),
            });
        }
        // A paragraph *mark* can be inserted or deleted too — that is what a
        // tracked paragraph split or merge is, and it has no text of its own.
        if let Some(revision) = &paragraph.mark_revision {
            out.push(Tracked {
                scope,
                paragraph: index,
                offset,
                mark: revision.mark().clone(),
                what: match revision {
                    Revision::Inserted(_) => "paragraph break inserted",
                    _ => "paragraph break deleted",
                },
                text: String::new(),
            });
        }
        // An inserted mark another author then deleted is two changes.
        if let Some(mark) = &paragraph.mark_deleted {
            out.push(Tracked {
                scope,
                paragraph: index,
                offset,
                mark: mark.clone(),
                what: "paragraph break deleted",
                text: String::new(),
            });
        }
    }
}

fn walk(
    content: &[Inline],
    scope: Scope,
    paragraph: usize,
    offset: &mut usize,
    out: &mut Vec<Tracked>,
) {
    for inline in content {
        match inline {
            Inline::Revised { revision, content } => {
                let mut text = String::new();
                for inner in content {
                    collect_text(inner, &mut text);
                }
                out.push(Tracked {
                    scope,
                    paragraph,
                    offset: *offset,
                    mark: revision.mark().clone(),
                    what: match revision {
                        Revision::Inserted(_) => "inserted",
                        Revision::Deleted(_) => "deleted",
                        Revision::MovedFrom { .. } => "moved from",
                        Revision::MovedTo { .. } => "moved to",
                    },
                    text: text.trim().chars().take(60).collect(),
                });
                walk(content, scope, paragraph, offset, out);
            }
            Inline::Hyperlink(link) => walk(&link.content, scope, paragraph, offset, out),
            Inline::Structured(sdt) => walk(&sdt.content, scope, paragraph, offset, out),
            Inline::Wrapper { content, .. } | Inline::SimpleField { content, .. } => {
                walk(content, scope, paragraph, offset, out)
            }
            Inline::Run(run) => {
                if let Some(change) = &run.prop_change {
                    out.push(Tracked {
                        scope,
                        paragraph,
                        offset: *offset,
                        mark: change.mark.clone(),
                        what: "formatting changed",
                        text: run.text().trim().chars().take(60).collect(),
                    });
                }
                // Counted the way a caret's offset is: a deletion's text is
                // drawn and holds no bytes.
                *offset += run.content.iter().map(Piece::text_len).sum::<usize>();
            }
            Inline::Anchor(_) | Inline::Math(_) => {}
        }
    }
}

fn collect_text(inline: &Inline, out: &mut String) {
    match inline {
        Inline::Run(run) => {
            for piece in &run.content {
                match piece {
                    Piece::Text(text) | Piece::Deleted(text) => out.push_str(text),
                    _ => {}
                }
            }
        }
        Inline::Revised { content, .. }
        | Inline::Wrapper { content, .. }
        | Inline::SimpleField { content, .. } => {
            for inner in content {
                collect_text(inner, out);
            }
        }
        Inline::Hyperlink(link) => {
            for inner in &link.content {
                collect_text(inner, out);
            }
        }
        Inline::Structured(sdt) => {
            for inner in &sdt.content {
                collect_text(inner, out);
            }
        }
        Inline::Anchor(_) | Inline::Math(_) => {}
    }
}

/// Settles every tracked change in the document — in every flow it has.
///
/// Flow by flow rather than all at once, because each one is its own run of
/// paragraphs: a change recorded over the body's numbering would restore the
/// header's paragraphs into the text.
pub fn resolve_all(document: &mut Document, history: &mut History, how: Resolve) -> usize {
    let mut count = 0;
    for scope in document.flows() {
        count += resolve_all_in(document, history, scope, how);
    }
    count
}

/// Settles every tracked change of one flow.
///
/// Container by container — the flow, each cell, each content control — so
/// that a join never reaches over a table, and the undo gives back the
/// flow's blocks whole, tables and all.
fn resolve_all_in(
    document: &mut Document,
    history: &mut History,
    scope: Scope,
    how: Resolve,
) -> usize {
    let mut here = Vec::new();
    tracked_in(document, scope, &mut here);
    let count = here.len();
    if count == 0 {
        return 0;
    }
    let Some(blocks) = document.blocks_mut(scope) else {
        return 0;
    };
    let before = blocks.clone();
    settle_blocks(blocks, how);
    let now = blocks.len();
    history.push(
        scope,
        Change::Blocks {
            index: 0,
            before,
            now,
        },
    );
    count
}

/// Settles every change in `blocks`, and in the tables and content controls
/// among them. A paragraph mark that goes — a deletion accepted, an insertion
/// rejected — joins its paragraph to the next when that is a paragraph of the
/// same container; before a table, or at a cell's end, there is nothing to
/// join to, and the change is settled with the mark left standing.
fn settle_blocks(blocks: &mut Vec<Block>, how: Resolve) {
    let goes: Vec<bool> = blocks
        .iter()
        .map(|block| matches!(block, Block::Paragraph(paragraph) if mark_goes(paragraph, how)))
        .collect();
    for block in blocks.iter_mut() {
        match block {
            Block::Paragraph(paragraph) => *paragraph = settle_paragraph(paragraph, how, None),
            Block::Table(table) => {
                for cell in table.rows.iter_mut().flat_map(|row| &mut row.cells) {
                    settle_blocks(&mut cell.content, how);
                }
            }
            Block::Structured(sdt) => settle_blocks(&mut sdt.content, how),
            _ => {}
        }
    }
    // Back to front, so that a join does not move an earlier one.
    for index in (1..blocks.len()).rev() {
        if !goes[index - 1] || !matches!(blocks[index], Block::Paragraph(_)) {
            continue;
        }
        let Block::Paragraph(tail) = blocks.remove(index) else {
            unreachable!("just matched");
        };
        if let Block::Paragraph(head) = &mut blocks[index - 1] {
            *head = joined(head, &tail);
        }
    }
}

/// Whether settling every change `how` takes the paragraph's mark away. An
/// inserted mark that was then deleted goes either way: accepted, the
/// deletion stands; rejected, the insertion goes.
fn mark_goes(paragraph: &Paragraph, how: Resolve) -> bool {
    match (&paragraph.mark_revision, &paragraph.mark_deleted) {
        (_, Some(_)) => true,
        (Some(revision), None) => !how.keeps(revision),
        (None, None) => false,
    }
}

/// Settles one tracked change, named by its mark.
///
/// The mark says which flow as well as which paragraph — it is looked up, not
/// passed in, because the pane that offers the change already knows only the
/// mark and the caller should not have to carry the answer twice.
pub fn resolve_one(
    document: &mut Document,
    history: &mut History,
    mark: &Mark,
    how: Resolve,
) -> bool {
    let Some(found) = tracked(document).into_iter().find(|t| &t.mark == mark) else {
        return false;
    };
    let scope = found.scope;
    let index = found.paragraph;
    let Some(before) = document
        .paragraphs_in(scope)
        .get(index)
        .map(|p| (*p).clone())
    else {
        return false;
    };
    // A paragraph mark's revision joins two paragraphs, which is a change to the
    // body rather than to one paragraph.
    let insertion = before
        .mark_revision
        .as_ref()
        .is_some_and(|revision| revision.mark() == mark);
    let deletion = before.mark_deleted.as_ref() == Some(mark);
    if insertion || deletion {
        // An inserted mark later deleted: accepting the deletion, or rejecting
        // the insertion, takes it away, and the other leaves the other change.
        let goes = match (deletion, &before.mark_deleted) {
            (true, _) => how == Resolve::Accept,
            (false, Some(_)) => how == Resolve::Reject,
            (false, None) => !how.keeps(before.mark_revision.as_ref().expect("just checked")),
        };
        // A mark with no paragraph beside it to join — before a table, at a
        // cell's end — has its change settled and stays.
        if goes && crate::edit::side_by_side(document, scope, index..index + 2) {
            let Some(next) = document
                .paragraphs_in(scope)
                .get(index + 1)
                .map(|p| (*p).clone())
            else {
                return false;
            };
            history.push(
                scope,
                Change::Merge {
                    index,
                    first: Box::new(before.clone()),
                    second: Box::new(next.clone()),
                },
            );
            let joined = joined(&before, &next);
            crate::edit::replace_range(document, scope, index..index + 2, vec![joined]);
            return true;
        }
        history.push(
            scope,
            Change::Paragraph {
                index,
                before: Box::new(before.clone()),
            },
        );
        let mut kept = before;
        match (deletion, kept.mark_deleted.take()) {
            _ if goes => kept.mark_revision = None,
            // The deletion rejected: the insertion stands.
            (true, _) => {}
            // The insertion accepted: the deletion stands.
            (false, Some(deleted)) => kept.mark_revision = Some(Revision::Deleted(deleted)),
            (false, None) => kept.mark_revision = None,
        }
        crate::edit::replace_range(document, scope, index..index + 1, vec![kept]);
        return true;
    }

    history.push(
        scope,
        Change::Paragraph {
            index,
            before: Box::new(before.clone()),
        },
    );
    let settled = settle_paragraph(&before, how, Some(mark));
    crate::edit::replace_range(document, scope, index..index + 1, vec![settled]);
    true
}

/// Whether anything in the paragraph is tracked.
fn has_changes(paragraph: &Paragraph) -> bool {
    fn within(content: &[Inline]) -> bool {
        content.iter().any(|inline| match inline {
            Inline::Revised { .. } => true,
            Inline::Run(run) => run.prop_change.is_some(),
            Inline::Hyperlink(link) => within(&link.content),
            Inline::Structured(sdt) => within(&sdt.content),
            Inline::Wrapper { content, .. } | Inline::SimpleField { content, .. } => {
                within(content)
            }
            Inline::Anchor(_) | Inline::Math(_) => false,
        })
    }
    paragraph.mark_revision.is_some()
        || paragraph.mark_deleted.is_some()
        || paragraph.prop_change.is_some()
        || paragraph.mark_change.is_some()
        || within(&paragraph.content)
}

/// Two paragraphs made one because the mark between them went: a deletion
/// accepted, or an insertion rejected.
///
/// **The mark that stays is the second paragraph's, and so are the
/// properties** — Word's rule, measured: a heading deleted whole, accepted,
/// leaves the body paragraph after it a body paragraph. Where Word means the
/// text before the mark to keep its look, it says so in the file ahead of
/// time, as a formatting change on the following paragraph. The joined
/// paragraph keeps the first one's identity, as any join does, and whatever
/// is still tracked on the second one's mark.
fn joined(head: &Paragraph, tail: &Paragraph) -> Paragraph {
    let mut joined = crate::text::merge(head, tail);
    joined.props = tail.props.clone();
    joined.prop_change = tail.prop_change.clone();
    joined.mark_change = tail.mark_change.clone();
    joined.mark_revision = tail.mark_revision.clone();
    joined.mark_deleted = tail.mark_deleted.clone();
    coalesce(&mut joined.content);
    joined
}

/// Applies `how` to a paragraph's revisions — all of them, or just one.
///
/// With all of them, a mark that stays is an ordinary mark afterwards; a mark
/// that goes is the caller's to join, since it takes the next paragraph.
fn settle_paragraph(paragraph: &Paragraph, how: Resolve, only: Option<&Mark>) -> Paragraph {
    // Nothing to settle is nothing to rewrite: its runs, split as Word split
    // them, are written back as they were read.
    if !has_changes(paragraph) {
        return paragraph.clone();
    }
    let mut settled = paragraph.clone();
    settled.content = settle(&paragraph.content, how, only);
    if let Some(change) = &paragraph.prop_change {
        if only.is_none_or(|mark| &change.mark == mark) {
            if how == Resolve::Reject {
                // What `<w:pPrChange>` remembers is the paragraph's properties
                // alone: the mark's own formatting is not among them, and stays.
                if let wp_model::revision::PreviousProps::Paragraph(previous) = &change.previous {
                    let mark = settled.props.mark.take();
                    settled.props = (**previous).clone();
                    settled.props.mark = mark;
                }
            }
            settled.prop_change = None;
        }
    }
    if let Some(change) = &paragraph.mark_change {
        if only.is_none_or(|mark| &change.mark == mark) {
            if how == Resolve::Reject {
                if let wp_model::revision::PreviousProps::Run(previous) = &change.previous {
                    settled.props.mark = Some(previous.clone());
                }
            }
            settled.mark_change = None;
        }
    }
    if only.is_none() {
        settled.mark_revision = None;
        settled.mark_deleted = None;
    }
    crate::text::prune(&mut settled);
    // What a settled change leaves beside the text around it is one run
    // again where nothing but the change set them apart.
    coalesce(&mut settled.content);
    settled
}

fn settle(content: &[Inline], how: Resolve, only: Option<&Mark>) -> Vec<Inline> {
    let mut out = Vec::new();
    for inline in content {
        match inline {
            Inline::Revised { revision, content } => {
                let mine = only.is_none_or(|mark| revision.mark() == mark);
                if !mine {
                    out.push(Inline::Revised {
                        revision: revision.clone(),
                        content: settle(content, how, only),
                    });
                    continue;
                }
                if how.keeps(revision) {
                    // The content stays, and the wrapper goes: an accepted
                    // insertion is ordinary text, not an insertion that has been
                    // ticked off.
                    out.extend(unwrap(settle(content, how, only)));
                }
                // Otherwise the content goes with the wrapper.
            }
            Inline::Hyperlink(link) => {
                let mut link = link.clone();
                link.content = settle(&link.content, how, only);
                out.push(Inline::Hyperlink(link));
            }
            Inline::Structured(sdt) => {
                let mut sdt = sdt.clone();
                sdt.content = settle(&sdt.content, how, only);
                out.push(Inline::Structured(sdt));
            }
            Inline::Wrapper { name, content } => out.push(Inline::Wrapper {
                name: name.clone(),
                content: settle(content, how, only),
            }),
            Inline::SimpleField {
                instruction,
                content,
            } => out.push(Inline::SimpleField {
                instruction: instruction.clone(),
                content: settle(content, how, only),
            }),
            Inline::Run(run) => {
                let mut run = run.clone();
                if run
                    .prop_change
                    .as_ref()
                    .is_some_and(|change| only.is_none_or(|mark| &change.mark == mark))
                {
                    if how == Resolve::Reject {
                        // Rejecting a formatting change puts back what the
                        // `<w:rPrChange>` remembered — which is the *previous*
                        // properties, and is the whole design of that element.
                        if let Some(wp_model::revision::PreviousProps::Run(previous)) =
                            run.prop_change.as_ref().map(|change| &change.previous)
                        {
                            run.props = (**previous).clone();
                        }
                    }
                    run.prop_change = None;
                }
                out.push(Inline::Run(run));
            }
            other => out.push(other.clone()),
        }
    }
    out
}

/// Turns the text of an accepted deletion back into ordinary text.
///
/// `<w:delText>` is not `<w:t>`: a deletion that has been *rejected* keeps its
/// content, and that content has to stop being marked as deleted or it will
/// still be skipped by everything that reads the document's text.
fn unwrap(content: Vec<Inline>) -> Vec<Inline> {
    content
        .into_iter()
        .map(|inline| match inline {
            Inline::Run(run) => Inline::Run(Run {
                content: run
                    .content
                    .into_iter()
                    .map(|piece| match piece {
                        Piece::Deleted(text) => Piece::Text(text),
                        Piece::DeletedInstruction(text) => Piece::Instruction(text),
                        other => other,
                    })
                    .collect(),
                ..run
            }),
            other => other,
        })
        .collect()
}

/// Wraps a run of text as an insertion, for typing with track changes on.
pub fn as_insertion(author: &str, id: u32, run: Run) -> Inline {
    Inline::Revised {
        revision: Revision::Inserted(Mark::new(id, author)),
        content: vec![Inline::Run(run)],
    }
}

// ------------------------------------------------------- recording changes

/// Who is making the change, for the marks a recorded edit carries.
#[derive(Debug, Clone)]
pub struct Author {
    pub name: std::sync::Arc<str>,
    pub initials: std::sync::Arc<str>,
    /// ISO 8601, supplied rather than read from the clock so an edit can be
    /// tested and so two runs of the same edit agree.
    pub date: Option<std::sync::Arc<str>>,
}

impl Author {
    pub fn new(name: &str) -> Author {
        let initials: String = name
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .collect();
        Author {
            name: name.into(),
            initials: initials.into(),
            date: None,
        }
    }

    fn mark(&self, id: u32) -> Mark {
        Mark {
            id,
            author: self.name.clone(),
            date: self.date.clone(),
        }
    }
}

/// The next unused revision id in the document.
///
/// Ids only have to be unique among revisions, and Word restarts them per
/// document rather than per author — so the highest in use plus one is the
/// answer, and reusing one would make two changes look like one.
pub fn next_revision_id(document: &Document) -> u32 {
    tracked(document)
        .iter()
        .map(|change| change.mark.id + 1)
        .max()
        .unwrap_or(1)
}

/// What is said when a tracked edit cannot be recorded; nothing is changed.
pub const CANNOT_RECORD: &str =
    "Track Changes cannot record an edit inside a hyperlink, a content control or a field";

/// What a tracked deletion that would reach across a table, out of a cell or
/// over what stands between paragraphs says; nothing is changed.
pub const ACROSS_BLOCKS: &str = "Track Changes cannot record a deletion that reaches across a table, out of a cell, or over what stands between two paragraphs";

/// What a page break in a table, which splits the table, says with Track
/// Changes on.
pub const TABLE_UNTRACKED: &str =
    "Track Changes cannot record splitting a table: turn it off to put a page break here";

/// How much of the text, as offsets count it, an inline holds.
fn width(inline: &Inline) -> usize {
    match inline {
        Inline::Run(run) => run.content.iter().map(Piece::text_len).sum(),
        Inline::Revised { content, .. }
        | Inline::Wrapper { content, .. }
        | Inline::SimpleField { content, .. } => content.iter().map(width).sum(),
        Inline::Hyperlink(link) => link.content.iter().map(width).sum(),
        Inline::Structured(sdt) => sdt.content.iter().map(width).sum(),
        Inline::Anchor(_) | Inline::Math(_) => 0,
    }
}

/// The ids an edit gives the changes it makes, from the first unused one.
struct Ids(u32);

impl Ids {
    fn take(&mut self) -> u32 {
        self.0 += 1;
        self.0 - 1
    }
}

/// Cuts `content` so that `offset` falls between two of its inlines, and
/// returns which index that is: the first inline at the offset, so that what
/// is put there goes before a deletion standing at it.
///
/// A plain run is cut in two. With `ids`, so is a tracked insertion, whose
/// second half becomes a change of its own, as Word makes it. `None` inside
/// anything else — a hyperlink, a field, a content control, a moved passage —
/// where nothing may be recorded.
fn cut(
    content: &mut Vec<Inline>,
    offset: usize,
    mut ids: Option<&mut Ids>,
    side: Side,
) -> Option<usize> {
    let mut seen = 0usize;
    let mut index = 0;
    while index < content.len() {
        let wide = width(&content[index]);
        if offset == seen {
            match side {
                Side::Before => return Some(index),
                // What stands at the offset and holds no text — an anchor, a
                // deletion — stays before the cut.
                Side::After if wide == 0 => {
                    index += 1;
                    continue;
                }
                Side::After => {
                    // And so do a run's leading pieces of no width: a note's
                    // reference, a field's characters.
                    if let Inline::Run(run) = &mut content[index] {
                        let lead = run
                            .content
                            .iter()
                            .take_while(|piece| piece.text_len() == 0)
                            .count();
                        if lead > 0 {
                            let rest = run.content.drain(lead..).collect();
                            let second = Run {
                                props: run.props.clone(),
                                content: rest,
                                prop_change: run.prop_change.clone(),
                            };
                            content.insert(index + 1, Inline::Run(second));
                            return Some(index + 1);
                        }
                    }
                    return Some(index);
                }
            }
        }
        if offset < seen + wide {
            let within = offset - seen;
            let second = match &mut content[index] {
                Inline::Run(run) => Inline::Run(cut_run(run, within, side)?),
                Inline::Revised {
                    revision: Revision::Inserted(mark),
                    content: inner,
                } => {
                    let ids = ids.as_deref_mut()?;
                    let at = cut(inner, within, None, side)?;
                    let rest = inner.drain(at..).collect();
                    let mark = Mark {
                        id: ids.take(),
                        ..mark.clone()
                    };
                    Inline::Revised {
                        revision: Revision::Inserted(mark),
                        content: rest,
                    }
                }
                _ => return None,
            };
            content.insert(index + 1, second);
            return Some(index + 1);
        }
        seen += wide;
        index += 1;
    }
    Some(content.len())
}

/// Which side of what stands at an offset and holds no text a cut goes: a
/// deletion's start leaves such things before it, and its end after it, so
/// that a reference or an anchor at its edge is not deleted with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Before,
    After,
}

/// Cuts a run at a text offset inside it, and returns the second half, with
/// the run's properties and any change to them; `None` where the offset
/// falls inside something that is not text.
fn cut_run(run: &mut Run, offset: usize, side: Side) -> Option<Run> {
    let half = |content: Vec<Piece>, run: &Run| Run {
        props: run.props.clone(),
        content,
        prop_change: run.prop_change.clone(),
    };
    let mut seen = 0usize;
    for index in 0..run.content.len() {
        let wide = run.content[index].text_len();
        if offset == seen && index > 0 && !(side == Side::After && wide == 0) {
            let rest = run.content.drain(index..).collect();
            return Some(half(rest, run));
        }
        if offset < seen + wide {
            let within = offset - seen;
            let Piece::Text(text) = &run.content[index] else {
                return None;
            };
            if !text.is_char_boundary(within) {
                return None;
            }
            let (head, tail) = (text[..within].to_string(), text[within..].to_string());
            let mut rest = vec![Piece::Text(tail.into())];
            rest.extend(run.content.drain(index + 1..));
            run.content[index] = Piece::Text(head.into());
            return Some(half(rest, run));
        }
        seen += wide;
    }
    Some(half(Vec::new(), run))
}

/// Where what replaces a selection deleted at `offset` goes: after every
/// deletion standing there, and the anchors among them, with the text the
/// deletions still count (a deleted tab is a character) — as an offset, and
/// as an index into `content`. `None` when no deletion stands there.
fn after_deletions(content: &[Inline], offset: usize) -> Option<(usize, usize)> {
    let mut seen = 0usize;
    let mut first = None;
    for (index, inline) in content.iter().enumerate() {
        if seen == offset {
            first = Some(index);
            break;
        }
        seen += width(inline);
        if seen > offset {
            return None;
        }
    }
    let first = first?;
    let mut last = None;
    let mut at = offset;
    let mut reach = offset;
    for (index, inline) in content.iter().enumerate().skip(first) {
        match inline {
            Inline::Revised {
                revision: Revision::Deleted(_),
                ..
            } => {
                at += width(inline);
                reach = at;
                last = Some(index);
            }
            Inline::Anchor(_) => {}
            _ => break,
        }
    }
    last.map(|last| (reach, last + 1))
}

/// Types `input` at `offset` as a tracked insertion by `author`, the change
/// `id`, and returns the offset after it.
///
/// Typing that touches an insertion of the same author's joins it, so a word
/// typed a letter at a time is one change; typing inside another author's
/// insertion splits it around the new one, as Word does. `None` where the
/// position is inside something this cannot record — see
/// [`CANNOT_RECORD`] — and then nothing is changed: a half-recorded change is
/// worse than an unrecorded one, since the person would believe the rest was
/// recorded too.
pub fn record_insertion(
    paragraph: &mut Paragraph,
    offset: usize,
    input: &str,
    author: &Author,
    id: u32,
) -> Option<usize> {
    record_insertion_with(paragraph, offset, input, author, id, None)
}

/// The same, in `with` rather than the formatting a caret there would
/// have — the formatting chosen at the caret with nothing selected, which
/// is for the typing that follows.
pub fn record_insertion_with(
    paragraph: &mut Paragraph,
    offset: usize,
    input: &str,
    author: &Author,
    id: u32,
    with: Option<wp_model::RunProps>,
) -> Option<usize> {
    insert_in(paragraph, offset, input, author, id, with, false)
}

/// The same, after the deletions standing at `offset` rather than before
/// them: what is typed over a selection follows what it deleted, as Word
/// writes it — the old text struck, then the new.
pub fn record_replacement(
    paragraph: &mut Paragraph,
    offset: usize,
    input: &str,
    author: &Author,
    id: u32,
    with: Option<wp_model::RunProps>,
) -> Option<usize> {
    insert_in(paragraph, offset, input, author, id, with, true)
}

fn insert_in(
    paragraph: &mut Paragraph,
    offset: usize,
    input: &str,
    author: &Author,
    id: u32,
    with: Option<wp_model::RunProps>,
    deletion_first: bool,
) -> Option<usize> {
    let props = with.unwrap_or_else(|| crate::text::props_at(paragraph, offset));
    let mut content = paragraph.content.clone();
    let mut ids = Ids(id + 1);
    let mut at = cut(&mut content, offset, Some(&mut ids), Side::Before)?;
    // Over a selection, after all that was deleted there — a deleted tab
    // still counts as a character — and the anchors among it.
    let mut offset = offset;
    if deletion_first {
        if let Some((reach, index)) = after_deletions(&content, offset) {
            offset = reach;
            at = index;
        }
    }
    content.insert(
        at,
        Inline::Revised {
            revision: Revision::Inserted(author.mark(id)),
            content: vec![Inline::Run(Run {
                props,
                content: vec![Piece::Text(input.into())],
                prop_change: None,
            })],
        },
    );
    join_changes(&mut content, &author.name, id);
    paragraph.content = content;
    crate::text::prune(paragraph);
    Some(offset + input.len())
}

/// Marks `range` of a paragraph's text deleted by `author`, the change `id`,
/// rather than removing it: the text stays, struck through and skipped by
/// everything that reads the text.
///
/// Text the same author inserted was never in the document, and is taken
/// back instead. Another author's insertion keeps the deletion inside it, as
/// Word writes it, so that rejecting the deletion gives the insertion back.
/// What is deleted already stays as it is, and so do the anchors of comments
/// and bookmarks. A deletion touching one of the same author's joins it, so
/// that a run of Backspaces is one change. `None` where the range takes in
/// something this cannot record — see [`CANNOT_RECORD`] — and then nothing
/// is changed.
pub fn record_deletion(
    paragraph: &mut Paragraph,
    range: std::ops::Range<usize>,
    author: &Author,
    id: u32,
) -> Option<()> {
    delete_in(paragraph, range, author, &mut Ids(id))
}

fn delete_in(
    paragraph: &mut Paragraph,
    range: std::ops::Range<usize>,
    author: &Author,
    ids: &mut Ids,
) -> Option<()> {
    if range.is_empty() {
        return Some(());
    }
    let first = ids.0;
    let mut content = paragraph.content.clone();
    // The start first: a cut at the start moves what comes after it, and the
    // end is found afresh.
    let start = cut(&mut content, range.start, Some(ids), Side::After)?;
    let end = cut(&mut content, range.end, Some(ids), Side::Before)?;
    if start > end {
        return None;
    }
    let covered: Vec<Inline> = content.drain(start..end).collect();
    let deleted = strike_all(covered, author, ids)?;
    content.splice(start..start, deleted);
    join_changes(&mut content, &author.name, first);
    paragraph.content = content;
    crate::text::prune(paragraph);
    Some(())
}

/// `covered`, deleted by `author`: see [`record_deletion`].
fn strike_all(covered: Vec<Inline>, author: &Author, ids: &mut Ids) -> Option<Vec<Inline>> {
    fn flush(runs: &mut Vec<Inline>, out: &mut Vec<Inline>, author: &Author, ids: &mut Ids) {
        if !runs.is_empty() {
            out.push(Inline::Revised {
                revision: Revision::Deleted(author.mark(ids.take())),
                content: std::mem::take(runs),
            });
        }
    }
    let mut out = Vec::new();
    let mut runs = Vec::new();
    for inline in covered {
        match inline {
            Inline::Run(run) => runs.push(Inline::Run(struck(run))),
            Inline::Revised {
                revision: Revision::Inserted(mark),
                content,
            } => {
                flush(&mut runs, &mut out, author, ids);
                // The same author's own insertion is simply taken back — all
                // but the anchors in it, which a comment or a bookmark needs.
                if mark.author == author.name {
                    salvage(content, &mut out);
                } else {
                    let inner = strike_all(content, author, ids)?;
                    out.push(Inline::Revised {
                        revision: Revision::Inserted(mark),
                        content: inner,
                    });
                }
            }
            inline @ (Inline::Revised {
                revision: Revision::Deleted(_),
                ..
            }
            | Inline::Anchor(_)) => {
                flush(&mut runs, &mut out, author, ids);
                out.push(inline);
            }
            _ => return None,
        }
    }
    flush(&mut runs, &mut out, author, ids);
    Some(out)
}

/// What must outlive text taken back: the anchors of comments and bookmarks,
/// and a comment's reference, out of the runs that held them.
fn salvage(content: Vec<Inline>, out: &mut Vec<Inline>) {
    for inline in content {
        match inline {
            Inline::Anchor(_) => out.push(inline),
            Inline::Run(Run {
                props,
                content,
                prop_change,
            }) => {
                let kept: Vec<Piece> = content
                    .into_iter()
                    .filter(|piece| matches!(piece, Piece::CommentRef(_)))
                    .collect();
                if !kept.is_empty() {
                    out.push(Inline::Run(Run {
                        props,
                        content: kept,
                        prop_change,
                    }));
                }
            }
            Inline::Revised { content, .. } => salvage(content, out),
            _ => {}
        }
    }
}

/// A run's text as deleted text.
fn struck(run: Run) -> Run {
    Run {
        content: run
            .content
            .into_iter()
            .map(|piece| match piece {
                Piece::Text(text) => Piece::Deleted(text),
                Piece::Instruction(text) => Piece::DeletedInstruction(text),
                other => other,
            })
            .collect(),
        ..run
    }
}

/// Joins each change this edit made — the ones from the id `from` on — to a
/// change of the same kind by the same author beside it, inside another
/// author's insertion as well as out of it: `author`'s own, so that a word
/// typed a letter at a time is one change, and the halves of another
/// author's insertion that the edit cut and then put nothing between. The
/// older mark stays, and the runs are joined where they differ only in their
/// text, as Word writes them.
fn join_changes(content: &mut Vec<Inline>, author: &str, from: u32) {
    fn kin(first: &Revision, second: &Revision, from: u32) -> bool {
        let same_kind = matches!(
            (first, second),
            (Revision::Inserted(_), Revision::Inserted(_))
                | (Revision::Deleted(_), Revision::Deleted(_))
        );
        let (a, b) = (first.mark(), second.mark());
        same_kind && a.author == b.author && (a.id >= from || b.id >= from)
    }
    let mut index = 0;
    while index < content.len() {
        if let Inline::Revised {
            revision: Revision::Inserted(mark),
            content: inner,
        } = &mut content[index]
        {
            if &*mark.author != author {
                join_changes(inner, author, from);
            }
        }
        let joins = match (&content[index], content.get(index + 1)) {
            (Inline::Revised { revision: a, .. }, Some(Inline::Revised { revision: b, .. })) => {
                kin(a, b, from)
            }
            _ => false,
        };
        if !joins {
            index += 1;
            continue;
        }
        let Inline::Revised {
            revision: later,
            content: moved,
        } = content.remove(index + 1)
        else {
            unreachable!("just matched");
        };
        if let Inline::Revised { revision, content } = &mut content[index] {
            if later.mark().id < revision.mark().id {
                *revision = later;
            }
            content.extend(moved);
            coalesce(content);
        }
    }
}

/// Joins neighbouring runs that differ in nothing but their text, and
/// neighbouring pieces of text in each.
fn coalesce(content: &mut Vec<Inline>) {
    let mut index = 0;
    while index + 1 < content.len() {
        let same = matches!(
            (&content[index], &content[index + 1]),
            (Inline::Run(a), Inline::Run(b)) if a.props == b.props && a.prop_change == b.prop_change
        );
        if !same {
            index += 1;
            continue;
        }
        let Inline::Run(next) = content.remove(index + 1) else {
            unreachable!("just matched");
        };
        if let Inline::Run(run) = &mut content[index] {
            for piece in next.content {
                match (run.content.last_mut(), piece) {
                    (Some(Piece::Text(text)), Piece::Text(more)) => {
                        *text = format!("{text}{more}").into();
                    }
                    (Some(Piece::Deleted(text)), Piece::Deleted(more)) => {
                        *text = format!("{text}{more}").into();
                    }
                    (_, piece) => run.content.push(piece),
                }
            }
        }
    }
}

/// Joins the run ending before `at` to the one starting there when they
/// differ in nothing but their text: the two halves of a run that a break
/// parted and Backspace took back.
fn join_runs_at(content: &mut Vec<Inline>, at: usize) {
    if at == 0 || at >= content.len() {
        return;
    }
    let mut pair: Vec<Inline> = content.drain(at - 1..=at).collect();
    coalesce(&mut pair);
    content.splice(at - 1..at - 1, pair);
}

/// A paragraph's properties without its mark's formatting, which is not
/// among what `<w:pPrChange>` records.
fn look(props: &wp_model::prop::ParaProps) -> wp_model::prop::ParaProps {
    wp_model::prop::ParaProps {
        mark: None,
        ..props.clone()
    }
}

/// Deletes what `selection` covers in `scope` as a tracked change by
/// `author`, the way Word records it, and says where the caret goes — or why
/// nothing was done.
///
/// Within a paragraph, [`record_deletion`]. Across paragraphs, each
/// paragraph's share of the text, and each paragraph mark the selection
/// crosses: an ordinary mark is marked deleted; one the same author inserted
/// is taken back, and its two paragraphs are one again, as before the break
/// was typed; one another author inserted is marked deleted too, and keeps
/// its insertion. When text is left before the first mark, the last
/// paragraph — whose mark ends the joined paragraph once the deletion is
/// accepted — is given the first one's properties as a tracked formatting
/// change, so that accepting leaves that text looking as it did; Word does
/// the same.
///
/// `forward` is Delete's caret: after a deleted mark rather than before it.
pub fn delete_range(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    selection: Selection,
    author: &Author,
    forward: bool,
) -> Result<Caret, &'static str> {
    let (start, end) = selection.ordered();
    if start == end {
        return Ok(start);
    }
    // A table or a content control in the way, or the ends in different
    // cells: a tracked deletion of that shape is not one Scriva writes.
    if !crate::edit::side_by_side(document, scope, start.paragraph..end.paragraph + 1) {
        return Err(ACROSS_BLOCKS);
    }
    let before: Vec<Paragraph> = {
        let paragraphs = document.paragraphs_in(scope);
        match paragraphs.get(start.paragraph..=end.paragraph) {
            Some(range) => range.iter().map(|paragraph| (*paragraph).clone()).collect(),
            None => return Ok(start),
        }
    };
    let mut ids = Ids(next_revision_id(document));
    let mut after = before.clone();
    let last = after.len() - 1;
    for (index, paragraph) in after.iter_mut().enumerate() {
        let from = if index == 0 { start.offset } else { 0 };
        let to = match index == last {
            true => end.offset,
            false => crate::text::len(paragraph),
        };
        delete_in(paragraph, from..to, author, &mut ids).ok_or(CANNOT_RECORD)?;
    }
    // The marks crossed, the last first, so that a join moves none of the
    // others.
    let mut deleted_a_mark = false;
    for index in (0..last).rev() {
        let paragraph = &mut after[index];
        if paragraph.mark_deleted.is_some() {
            continue;
        }
        match &paragraph.mark_revision {
            None => {
                paragraph.mark_revision = Some(Revision::Deleted(author.mark(ids.take())));
                deleted_a_mark = true;
            }
            Some(Revision::Inserted(mark)) if mark.author == author.name => {
                let next = after.remove(index + 1);
                let at = after[index].content.len();
                let mut merged = crate::text::merge(&after[index], &next);
                join_runs_at(&mut merged.content, at);
                after[index] = merged;
            }
            Some(Revision::Inserted(_) | Revision::MovedTo { .. }) => {
                paragraph.mark_deleted = Some(author.mark(ids.take()));
                deleted_a_mark = true;
            }
            // Deleted already, or moved away.
            Some(Revision::Deleted(_) | Revision::MovedFrom { .. }) => {}
        }
    }
    // Nothing new to record: only what was deleted already was crossed.
    if after == before {
        return Ok(match forward {
            true => end,
            false => start,
        });
    }
    if deleted_a_mark && start.offset > 0 && after.len() > 1 {
        let head = look(&after[0].props);
        let tail = after.last_mut().expect("more than one");
        let was = look(&tail.props);
        if was != head && tail.prop_change.is_none() {
            let mark = tail.props.mark.take();
            tail.props = head;
            tail.props.mark = mark;
            tail.prop_change = Some(Box::new(wp_model::PropChange {
                mark: author.mark(ids.take()),
                previous: wp_model::revision::PreviousProps::Paragraph(Box::new(was)),
            }));
        }
    }
    let now = after.len();
    history.push(
        scope,
        Change::Range {
            first: start.paragraph,
            before,
            now,
        },
    );
    crate::edit::replace_range(document, scope, start.paragraph..end.paragraph + 1, after);
    Ok(
        match forward && start.paragraph != end.paragraph && now > 1 {
            true => Caret {
                paragraph: start.paragraph + now - 1,
                offset: 0,
            },
            false => start,
        },
    )
}

/// Splits the paragraph at `caret` in `scope` — Enter — as a tracked change by
/// `author`, and returns the caret at the new paragraph's start.
///
/// The new mark is the first paragraph's, and is marked inserted; the old one
/// still ends the second. At the end of a paragraph whose style names the
/// next, the new paragraph takes that style, as it does untracked, and the
/// change is recorded as a formatting change from the style it had.
pub fn split_paragraph(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    caret: Caret,
    author: &Author,
) -> Caret {
    let Some(paragraph) = crate::edit::paragraph_at(document, scope, caret.paragraph) else {
        return caret;
    };
    let mut ids = Ids(next_revision_id(document));
    let (mut head, mut tail) = crate::text::split(&paragraph, caret.offset);
    ids.0 = distinct_ids(&mut [&mut head, &mut tail], ids.0);
    head.mark_revision = Some(Revision::Inserted(author.mark(ids.take())));
    let next = paragraph
        .props
        .style
        .and_then(|style| document.styles.get(style))
        .and_then(|style| style.next)
        .filter(|next| Some(*next) != paragraph.props.style);
    if let Some(next) = next {
        if crate::text::len(&paragraph) == caret.offset {
            let was = look(&tail.props);
            tail.props.style = Some(next);
            if tail.prop_change.is_none() {
                tail.prop_change = Some(Box::new(wp_model::PropChange {
                    mark: author.mark(ids.take()),
                    previous: wp_model::revision::PreviousProps::Paragraph(Box::new(was)),
                }));
            }
        }
    }
    history.push(
        scope,
        Change::Range {
            first: caret.paragraph,
            before: vec![paragraph],
            now: 2,
        },
    );
    crate::edit::replace_range(
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

/// Pastes `clip` over `selection` in `scope` as a tracked change by
/// `author`, and returns the caret after it — or why nothing was done.
///
/// The selection is deleted as [`delete_range`] deletes it; the clip arrives
/// as it reads with its own changes accepted, all of it inserted, and every
/// paragraph break the paste makes marked inserted. It lands as an untracked
/// paste lands: the first paragraph joining what the caret was in, the last
/// taking what followed.
pub fn paste_paragraphs(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    selection: Selection,
    clip: &[Paragraph],
    author: &Author,
) -> Result<Caret, &'static str> {
    can_record(document, scope, selection)?;
    let mut caret = delete_range(document, scope, history, selection, author, false)?;
    // Over a selection, the paste follows what it deleted, as typing does —
    // after a deleted tab, too, which still takes a place.
    let deletion_first = !selection.is_empty();
    if deletion_first {
        if let Some((reach, _)) = crate::edit::paragraph_at(document, scope, caret.paragraph)
            .and_then(|paragraph| after_deletions(&paragraph.content, caret.offset))
        {
            caret.offset = reach;
        }
    }
    let first = next_revision_id(document);
    let mut ids = Ids(first);
    let clip: Vec<Paragraph> = clip
        .iter()
        .map(|paragraph| {
            let mut pasted = settle_paragraph(paragraph, Resolve::Accept, None);
            pasted.mark_revision = None;
            pasted.mark_deleted = None;
            pasted.content = inserted(std::mem::take(&mut pasted.content), author, &mut ids);
            pasted
        })
        .collect();
    let landed = match deletion_first {
        true => crate::edit::paste_after_deletion(document, scope, history, caret, &clip),
        false => {
            crate::edit::paste_paragraphs(document, scope, history, Selection::at(caret), &clip)
        }
    };
    // A change the paste cut in two is two changes.
    {
        let mut paragraphs = document.paragraphs_in_mut(scope);
        let mut landed_range: Vec<&mut Paragraph> = paragraphs
            .iter_mut()
            .skip(caret.paragraph)
            .take(landed.paragraph + 1 - caret.paragraph)
            .map(|paragraph| &mut **paragraph)
            .collect();
        ids.0 = distinct_ids(&mut landed_range, ids.0);
    }
    let mut paragraphs = document.paragraphs_in_mut(scope);
    for index in caret.paragraph..=landed.paragraph {
        let Some(paragraph) = paragraphs.get_mut(index) else {
            continue;
        };
        if index < landed.paragraph {
            paragraph.mark_revision = Some(Revision::Inserted(author.mark(ids.take())));
        }
        join_changes(&mut paragraph.content, &author.name, first);
        crate::text::prune(paragraph);
    }
    Ok(landed)
}

/// `content`, all of it inserted by `author`: runs in insertions of their
/// own, and what may not stand inside an insertion — a hyperlink, a field —
/// with its runs inserted inside it, as Word writes them.
fn inserted(content: Vec<Inline>, author: &Author, ids: &mut Ids) -> Vec<Inline> {
    fn flush(runs: &mut Vec<Inline>, out: &mut Vec<Inline>, author: &Author, ids: &mut Ids) {
        if !runs.is_empty() {
            out.push(Inline::Revised {
                revision: Revision::Inserted(author.mark(ids.take())),
                content: std::mem::take(runs),
            });
        }
    }
    let mut out = Vec::new();
    let mut runs = Vec::new();
    for inline in content {
        match inline {
            Inline::Hyperlink(mut link) => {
                flush(&mut runs, &mut out, author, ids);
                link.content = inserted(std::mem::take(&mut link.content), author, ids);
                out.push(Inline::Hyperlink(link));
            }
            Inline::SimpleField {
                instruction,
                content,
            } => {
                flush(&mut runs, &mut out, author, ids);
                out.push(Inline::SimpleField {
                    instruction,
                    content: inserted(content, author, ids),
                });
            }
            inline @ Inline::Anchor(_) => {
                flush(&mut runs, &mut out, author, ids);
                out.push(inline);
            }
            other => runs.push(other),
        }
    }
    flush(&mut runs, &mut out, author, ids);
    out
}

/// Gives a change that stands in more than one place among `paragraphs` —
/// one a split or a paste cut in two — an id of its own at each place after
/// the first, as Word does: one change may not stand in two places. Fresh
/// ids are taken from `next` on; the next unused one is returned.
pub(crate) fn distinct_ids(paragraphs: &mut [&mut Paragraph], next: u32) -> u32 {
    fn fresh(mark: &mut Mark, seen: &mut Vec<u32>, ids: &mut Ids) {
        if seen.contains(&mark.id) {
            mark.id = ids.take();
        }
        seen.push(mark.id);
    }
    fn walk(content: &mut [Inline], seen: &mut Vec<u32>, ids: &mut Ids) {
        for inline in content {
            match inline {
                Inline::Revised { revision, content } => {
                    match revision {
                        Revision::Inserted(mark) | Revision::Deleted(mark) => {
                            fresh(mark, seen, ids)
                        }
                        Revision::MovedFrom { mark, .. } | Revision::MovedTo { mark, .. } => {
                            fresh(mark, seen, ids)
                        }
                    }
                    walk(content, seen, ids);
                }
                Inline::Run(run) => {
                    if let Some(change) = &mut run.prop_change {
                        fresh(&mut change.mark, seen, ids);
                    }
                }
                Inline::Hyperlink(link) => walk(&mut link.content, seen, ids),
                Inline::Structured(sdt) => walk(&mut sdt.content, seen, ids),
                Inline::Wrapper { content, .. } | Inline::SimpleField { content, .. } => {
                    walk(content, seen, ids)
                }
                Inline::Anchor(_) | Inline::Math(_) => {}
            }
        }
    }
    let mut ids = Ids(next);
    let mut seen = Vec::new();
    for paragraph in paragraphs.iter_mut() {
        if let Some(change) = &mut paragraph.prop_change {
            fresh(&mut change.mark, &mut seen, &mut ids);
        }
        walk(&mut paragraph.content, &mut seen, &mut ids);
        if let Some(change) = &mut paragraph.mark_change {
            fresh(&mut change.mark, &mut seen, &mut ids);
        }
    }
    ids.0
}

/// Whether `selection` can be replaced by something put in, as a tracked
/// change — its deletion recorded, and the insertion after it — or what
/// stands in the way: asked before anything is done, so that a refusal
/// changes nothing, a package included.
pub fn can_record(
    document: &Document,
    scope: Scope,
    selection: Selection,
) -> Result<(), &'static str> {
    let (start, end) = selection.ordered();
    if start != end {
        if !crate::edit::side_by_side(document, scope, start.paragraph..end.paragraph + 1) {
            return Err(ACROSS_BLOCKS);
        }
        let paragraphs = document.paragraphs_in(scope);
        let Some(range) = paragraphs.get(start.paragraph..=end.paragraph) else {
            return Err(CANNOT_RECORD);
        };
        let last = range.len() - 1;
        for (index, paragraph) in range.iter().enumerate() {
            let mut paragraph = (*paragraph).clone();
            let from = if index == 0 { start.offset } else { 0 };
            let to = match index == last {
                true => end.offset,
                false => crate::text::len(&paragraph),
            };
            if delete_in(&mut paragraph, from..to, &Author::new(""), &mut Ids(0)).is_none() {
                return Err(CANNOT_RECORD);
            }
        }
    }
    match can_insert_at(document, scope, start) {
        true => Ok(()),
        false => Err(CANNOT_RECORD),
    }
}

/// Puts a page, column or line break at `caret` — Ctrl+Enter — as a tracked
/// change by `author`: the break inserted at the end of the first paragraph
/// and the mark after it inserted, as Word records it. Where nothing can be
/// recorded, nothing is done.
pub fn insert_break(
    document: &mut Document,
    scope: Scope,
    history: &mut History,
    caret: Caret,
    kind: wp_model::doc::Break,
    author: &Author,
) -> Result<Caret, &'static str> {
    if !can_insert_at(document, scope, caret) {
        return Err(CANNOT_RECORD);
    }
    let Some(paragraph) = crate::edit::paragraph_at(document, scope, caret.paragraph) else {
        return Ok(caret);
    };
    let mut ids = Ids(next_revision_id(document));
    let (mut head, mut tail) = crate::text::split(&paragraph, caret.offset);
    ids.0 = distinct_ids(&mut [&mut head, &mut tail], ids.0);
    let first = ids.0;
    let props = crate::text::props_at(&paragraph, caret.offset);
    head.content.push(Inline::Revised {
        revision: Revision::Inserted(author.mark(ids.take())),
        content: vec![Inline::Run(Run {
            props,
            content: vec![Piece::Break(kind)],
            prop_change: None,
        })],
    });
    join_changes(&mut head.content, &author.name, first);
    head.mark_revision = Some(Revision::Inserted(author.mark(ids.take())));
    history.push(
        scope,
        Change::Range {
            first: caret.paragraph,
            before: vec![paragraph],
            now: 2,
        },
    );
    crate::edit::replace_range(
        document,
        scope,
        caret.paragraph..caret.paragraph + 1,
        vec![head, tail],
    );
    Ok(Caret {
        paragraph: caret.paragraph + 1,
        offset: 0,
    })
}

/// Marks the paragraph's `nth` drawing — as `Paragraph::drawings` counts
/// them — deleted by `author`, the change `id`: the run that holds it cut
/// around it and struck, as Word writes a picture deleted with Track Changes
/// on. One the same author inserted is taken back, and one in another
/// author's insertion keeps its deletion inside it. `None` where it cannot be
/// recorded — in a hyperlink, a field, a content control — and then nothing
/// is changed.
pub fn record_drawing_deletion(
    paragraph: &mut Paragraph,
    nth: usize,
    author: &Author,
    id: u32,
) -> Option<()> {
    fn drawings(inline: &Inline) -> usize {
        match inline {
            Inline::Run(run) => run
                .content
                .iter()
                .filter(|piece| matches!(piece, Piece::Drawing(_)))
                .count(),
            Inline::Revised { content, .. }
            | Inline::Wrapper { content, .. }
            | Inline::SimpleField { content, .. } => content.iter().map(drawings).sum(),
            Inline::Hyperlink(link) => link.content.iter().map(drawings).sum(),
            Inline::Structured(sdt) => sdt.content.iter().map(drawings).sum(),
            Inline::Anchor(_) | Inline::Math(_) => 0,
        }
    }
    /// Strikes the `nth` drawing among `content`'s own runs.
    fn strike_in(
        content: &mut Vec<Inline>,
        mut nth: usize,
        author: &Author,
        id: u32,
    ) -> Option<()> {
        for index in 0..content.len() {
            let held = drawings(&content[index]);
            if nth >= held {
                nth -= held;
                continue;
            }
            let Inline::Run(run) = &mut content[index] else {
                return None;
            };
            let at = run
                .content
                .iter()
                .enumerate()
                .filter(|(_, piece)| matches!(piece, Piece::Drawing(_)))
                .nth(nth)
                .map(|(at, _)| at)?;
            let after: Vec<Piece> = run.content.drain(at + 1..).collect();
            let drawing: Vec<Piece> = run.content.drain(at..).collect();
            let half = |content: Vec<Piece>, run: &Run| Run {
                props: run.props.clone(),
                content,
                prop_change: run.prop_change.clone(),
            };
            let struck = Inline::Revised {
                revision: Revision::Deleted(author.mark(id)),
                content: vec![Inline::Run(half(drawing, run))],
            };
            let tail = half(after, run);
            content.insert(index + 1, struck);
            content.insert(index + 2, Inline::Run(tail));
            return Some(());
        }
        None
    }
    let mut content = paragraph.content.clone();
    let mut left = nth;
    for index in 0..content.len() {
        let held = drawings(&content[index]);
        if left >= held {
            left -= held;
            continue;
        }
        match &mut content[index] {
            Inline::Run(_) => strike_in(&mut content, nth, author, id)?,
            // Deleted already.
            Inline::Revised {
                revision: Revision::Deleted(_),
                ..
            } => return Some(()),
            Inline::Revised {
                revision: Revision::Inserted(mark),
                content: inner,
            } => {
                if mark.author == author.name {
                    if !paragraph.remove_drawing(nth) {
                        return None;
                    }
                    crate::text::prune(paragraph);
                    return Some(());
                }
                strike_in(inner, left, author, id)?;
            }
            _ => return None,
        }
        join_changes(&mut content, &author.name, id);
        paragraph.content = content;
        crate::text::prune(paragraph);
        return Some(());
    }
    None
}

/// Whether a tracked insertion can be recorded at `caret`: not inside a
/// hyperlink, a field or a content control.
pub fn can_insert_at(document: &Document, scope: Scope, caret: Caret) -> bool {
    let Some(paragraph) = document.paragraph_in(scope, caret.paragraph) else {
        return false;
    };
    let mut content = paragraph.content.clone();
    cut(&mut content, caret.offset, Some(&mut Ids(0)), Side::Before).is_some()
}

/// Cuts the paragraph's top-level inlines so that `offset` falls between two
/// of them, and returns which index that is; `None` where it falls inside
/// anything but a plain run.
pub(crate) fn top_level_split(paragraph: &mut Paragraph, offset: usize) -> Option<usize> {
    cut(&mut paragraph.content, offset, None, Side::Before)
}

// ------------------------------------------------------------- comments

/// Adds a comment over the selection.
///
/// The comment's prose goes in `comments.xml` and the document gets the anchors:
/// a start, an end, and the mark the balloon's line points at. All three, or
/// Word reports the file as damaged.
pub fn add_comment(
    document: &mut Document,
    history: &mut History,
    scope: Scope,
    selection: Selection,
    author: &str,
    initials: &str,
    text: &str,
) -> u32 {
    let id = document
        .comments
        .iter()
        .map(|comment| comment.id + 1)
        .max()
        .unwrap_or(1);
    comment_with(
        document, history, scope, selection, author, initials, text, None, id,
    );
    id
}

/// A reply: a comment of its own, anchored to the same words as the one it
/// answers, that names its parent. Word draws it under the parent, and so
/// does the pane. Nothing when there is no such comment.
pub fn reply_to_comment(
    document: &mut Document,
    history: &mut History,
    parent: u32,
    author: &str,
    initials: &str,
    text: &str,
) -> Option<u32> {
    let range = comment_ranges(document)
        .into_iter()
        .find(|range| range.id == parent)?;
    let id = document
        .comments
        .iter()
        .map(|comment| comment.id + 1)
        .max()
        .unwrap_or(1);
    comment_with(
        document,
        history,
        range.scope,
        range.range,
        author,
        initials,
        text,
        Some(parent),
        id,
    );
    Some(id)
}

/// Marks a comment resolved, or open again. Undoable; nothing when there is
/// no such comment or it already says so.
pub fn resolve_comment(
    document: &mut Document,
    history: &mut History,
    id: u32,
    done: bool,
) -> bool {
    let Some(at) = document
        .comments
        .iter()
        .position(|comment| comment.id == id)
    else {
        return false;
    };
    if document.comments[at].done == done {
        return false;
    }
    history.push(
        Scope::Body,
        Change::Comments {
            before: document.comments.clone(),
        },
    );
    document.comments[at].done = done;
    true
}

#[allow(clippy::too_many_arguments)]
fn comment_with(
    document: &mut Document,
    history: &mut History,
    scope: Scope,
    selection: Selection,
    author: &str,
    initials: &str,
    text: &str,
    parent: Option<u32>,
    id: u32,
) {
    let (start, end) = selection.ordered();

    let before: Vec<Paragraph> = document
        .paragraphs_in(scope)
        .iter()
        .skip(start.paragraph)
        .take(end.paragraph - start.paragraph + 1)
        .map(|p| (*p).clone())
        .collect();
    // The anchors and the comment are one thing to undo.
    history.push(
        scope,
        Change::Many(vec![
            Change::Range {
                first: start.paragraph,
                before: before.clone(),
                now: before.len(),
            },
            Change::Comments {
                before: document.comments.clone(),
            },
        ]),
    );

    let mut comment = wp_model::Comment::new(id, author);
    comment.initials = Some(initials.into());
    comment.parent = parent;
    comment.content = vec![Block::Paragraph(Paragraph::of(text))];
    document.comments.push(comment);

    // The anchors go at the selection's own offsets, splitting a run where
    // the selection starts or ends inside one: a comment is about the words
    // that were selected, and Word anchors it at them. The end goes in
    // first, so that the start's anchor — which has no width — cannot move
    // it when both are in one paragraph. This once anchored the whole
    // paragraph, and the wash the page draws for a comment showed it.
    let mut after = before.clone();
    if let Some(last) = after.last_mut() {
        insert_inlines(
            last,
            end.offset,
            vec![
                Inline::Anchor(wp_model::Anchor::CommentEnd { id }),
                Inline::Run(Run {
                    content: vec![Piece::CommentRef(id)],
                    ..Run::new()
                }),
            ],
        );
    }
    if let Some(first) = after.first_mut() {
        insert_inlines(
            first,
            start.offset,
            vec![Inline::Anchor(wp_model::Anchor::CommentStart { id })],
        );
    }
    crate::edit::replace_range(
        document,
        scope,
        start.paragraph..start.paragraph + before.len(),
        after,
    );
}

/// Puts `inlines` into a paragraph at a byte offset of its text, splitting
/// the run the offset falls inside.
///
/// Only the paragraph's own runs are split: an offset inside a hyperlink or
/// a tracked change lands before that inline, which is the nearest place an
/// anchor can stand without opening it. Past the end of the text, they go at
/// the end.
fn insert_inlines(paragraph: &mut Paragraph, offset: usize, inlines: Vec<Inline>) {
    let mut at = 0usize;
    for index in 0..paragraph.content.len() {
        if at == offset {
            paragraph.content.splice(index..index, inlines);
            return;
        }
        // Measured as a caret measures, as the selection it anchors was.
        let length = width(&paragraph.content[index]);
        if at + length <= offset {
            at += length;
            continue;
        }
        // The offset is inside this inline.
        let within = offset - at;
        let Inline::Run(run) = &paragraph.content[index] else {
            paragraph.content.splice(index..index, inlines);
            return;
        };
        let (head, tail) = split_run(run, within);
        let mut replacement = vec![Inline::Run(head)];
        replacement.extend(inlines);
        replacement.push(Inline::Run(tail));
        paragraph.content.splice(index..index + 1, replacement);
        return;
    }
    paragraph.content.extend(inlines);
}

/// A run cut at a byte offset of its text, both halves keeping its
/// properties. The pieces that carry no text stay with the half they
/// were in.
fn split_run(run: &Run, offset: usize) -> (Run, Run) {
    let mut head = Run {
        props: run.props.clone(),
        content: Vec::new(),
        prop_change: run.prop_change.clone(),
    };
    let mut tail = head.clone();
    let mut at = 0usize;
    for piece in &run.content {
        let length = piece.text_len();
        if at + length <= offset {
            head.content.push(piece.clone());
        } else if at >= offset {
            tail.content.push(piece.clone());
        } else {
            let mut cut = offset - at;
            match piece {
                Piece::Text(text) => {
                    while !text.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    head.content.push(Piece::Text(text[..cut].into()));
                    tail.content.push(Piece::Text(text[cut..].into()));
                }
                other => head.content.push(other.clone()),
            }
        }
        at += length;
    }
    (head, tail)
}

/// Removes a comment and the three marks that anchor it — and its replies,
/// which answer nothing once it is gone.
///
/// The anchors of one comment are all in one flow, so the flow holding its
/// start is the flow the marks are pulled out of.
pub fn delete_comment(document: &mut Document, history: &mut History, id: u32) -> bool {
    if !document.comments.iter().any(|comment| comment.id == id) {
        return false;
    }
    let replies: Vec<u32> = document
        .comments
        .iter()
        .filter(|comment| comment.parent == Some(id))
        .map(|comment| comment.id)
        .collect();
    let scope = comment_at(document, id).map_or(Scope::Body, |(scope, _)| scope);
    let before: Vec<Paragraph> = document
        .paragraphs_in(scope)
        .iter()
        .map(|p| (*p).clone())
        .collect();
    history.push(
        scope,
        Change::Many(vec![
            Change::Range {
                first: 0,
                before: before.clone(),
                now: before.len(),
            },
            Change::Comments {
                before: document.comments.clone(),
            },
        ]),
    );
    let gone = |at: u32| at == id || replies.contains(&at);
    document.comments.retain(|comment| !gone(comment.id));

    let after: Vec<Paragraph> = before
        .iter()
        .map(|paragraph| {
            let mut paragraph = paragraph.clone();
            paragraph.content.retain(|inline| {
                !matches!(
                    inline,
                    Inline::Anchor(wp_model::Anchor::CommentStart { id: at })
                        | Inline::Anchor(wp_model::Anchor::CommentEnd { id: at })
                    if gone(*at)
                )
            });
            for inline in &mut paragraph.content {
                if let Inline::Run(run) = inline {
                    run.content
                        .retain(|piece| !matches!(piece, Piece::CommentRef(at) if gone(*at)));
                }
            }
            crate::text::prune(&mut paragraph);
            paragraph
        })
        .collect();
    crate::edit::replace_range(document, scope, 0..before.len(), after);
    true
}

/// One comment's range: which comment, which flow, and the stretch of text
/// between its start and end anchors, in the flow's own carets.
#[derive(Debug, Clone, PartialEq)]
pub struct CommentRange {
    pub id: u32,
    pub scope: Scope,
    pub range: Selection,
}

/// Every comment's range, for washing the text a comment is about.
///
/// A range may cross paragraphs — the start anchor in one, the end in the
/// next — so the walk carries the open comments along the flow rather than
/// looking inside one paragraph at a time. A start with no end runs to the
/// end of its paragraph, which is where Word puts the range of a comment
/// whose end anchor an edit has lost.
pub fn comment_ranges(document: &Document) -> Vec<CommentRange> {
    let mut out = Vec::new();
    for scope in document.flows() {
        let mut open: Vec<(u32, Caret)> = Vec::new();
        let paragraphs = document.paragraphs_in(scope);
        for (index, paragraph) in paragraphs.iter().enumerate() {
            let mut offset = 0usize;
            for inline in &paragraph.content {
                match inline {
                    Inline::Anchor(wp_model::Anchor::CommentStart { id }) => {
                        open.push((
                            *id,
                            Caret {
                                paragraph: index,
                                offset,
                            },
                        ));
                    }
                    Inline::Anchor(wp_model::Anchor::CommentEnd { id }) => {
                        if let Some(at) = open.iter().position(|(open, _)| open == id) {
                            let (id, start) = open.remove(at);
                            out.push(CommentRange {
                                id,
                                scope,
                                range: Selection {
                                    anchor: start,
                                    head: Caret {
                                        paragraph: index,
                                        offset,
                                    },
                                },
                            });
                        }
                    }
                    other => offset += width(other),
                }
            }
        }
        for (id, start) in open {
            let end = paragraphs
                .get(start.paragraph)
                .map(|p| crate::text::len(p))
                .unwrap_or(start.offset);
            out.push(CommentRange {
                id,
                scope,
                range: Selection {
                    anchor: start,
                    head: Caret {
                        paragraph: start.paragraph,
                        offset: end,
                    },
                },
            });
        }
    }
    out
}

/// Where a comment is anchored, for drawing it beside its text — and in which
/// of the document's flows.
pub fn comment_at(document: &Document, id: u32) -> Option<(Scope, Caret)> {
    for scope in document.flows() {
        for (index, paragraph) in document.paragraphs_in(scope).iter().enumerate() {
            let mut offset = 0usize;
            for inline in &paragraph.content {
                match inline {
                    Inline::Anchor(wp_model::Anchor::CommentStart { id: at }) if *at == id => {
                        return Some((
                            scope,
                            Caret {
                                paragraph: index,
                                offset,
                            },
                        ))
                    }
                    other => offset += width(other),
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::doc::inserted_by;
    use wp_model::Toggle;

    #[test]
    fn a_reply_shares_its_parents_words_and_undoes_with_it_in_one_step() {
        let mut document = document(vec![Block::Paragraph(Paragraph::of("the quick fox"))]);
        let mut history = History::new();
        let quick = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 4,
            },
            head: Caret {
                paragraph: 0,
                offset: 9,
            },
        };
        let parent = add_comment(
            &mut document,
            &mut history,
            Scope::Body,
            quick,
            "A",
            "A",
            "why?",
        );
        let reply = reply_to_comment(&mut document, &mut history, parent, "B", "B", "because")
            .expect("the parent is there");
        assert_eq!(document.comments[1].parent, Some(parent));
        let ranges = comment_ranges(&document);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].range, quick);
        assert_eq!(ranges[1].range, quick, "the reply is about the same words");
        assert_eq!(
            document.text(),
            "the quick fox",
            "and the text is untouched"
        );

        assert!(resolve_comment(&mut document, &mut history, parent, true));
        assert!(document.comments[0].done);
        history.undo(&mut document);
        assert!(!document.comments[0].done, "resolving undoes");
        history.undo(&mut document);
        assert_eq!(document.comments.len(), 1, "the reply undoes as one step");
        assert_eq!(comment_ranges(&document).len(), 1, "anchors and all");
        assert!(!document.comments.iter().any(|c| c.id == reply));
        history.undo(&mut document);
        assert!(document.comments.is_empty());
        assert!(comment_ranges(&document).is_empty());
    }

    #[test]
    fn a_comments_range_runs_from_its_start_anchor_to_its_end_across_paragraphs() {
        let mut document = document(vec![
            Block::Paragraph(Paragraph::of("the quick fox")),
            Block::Paragraph(Paragraph::of("and the dog")),
        ]);
        let mut history = History::new();
        let across = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 4,
            },
            head: Caret {
                paragraph: 1,
                offset: 7,
            },
        };
        let id = add_comment(
            &mut document,
            &mut history,
            Scope::Body,
            across,
            "Reviewer",
            "R",
            "spans two",
        );
        let ranges = comment_ranges(&document);
        assert_eq!(
            ranges,
            vec![CommentRange {
                id,
                scope: Scope::Body,
                range: across,
            }]
        );
    }

    fn document(blocks: Vec<Block>) -> Document {
        Document {
            body: blocks,
            ..Document::new()
        }
    }

    /// "kept " + an insertion + a deletion.
    fn reviewed() -> Document {
        document(vec![Block::Paragraph(Paragraph {
            content: vec![
                Inline::Run(Run::of("kept ")),
                inserted_by("Adnan Khan", 1, vec![Inline::Run(Run::of("added "))]),
                Inline::Revised {
                    revision: Revision::Deleted(Mark::new(2, "Adnan Khan")),
                    content: vec![Inline::Run(Run {
                        content: vec![Piece::Deleted("removed".into())],
                        ..Run::new()
                    })],
                },
            ],
            ..Paragraph::new()
        })])
    }

    #[test]
    fn the_changes_are_listed_with_who_made_them() {
        let found = tracked(&reviewed());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].what, "inserted");
        assert_eq!(found[0].text, "added");
        assert_eq!(found[0].mark.author.as_ref(), "Adnan Khan");
        assert_eq!(found[1].what, "deleted");
    }

    #[test]
    fn accepting_keeps_the_insertion_and_drops_the_deletion() {
        let mut document = reviewed();
        let mut history = History::new();
        assert_eq!(resolve_all(&mut document, &mut history, Resolve::Accept), 2);
        assert_eq!(document.text(), "kept added ");
        assert_eq!(
            document.paragraphs()[0].shown_text(),
            "kept added ",
            "and the deleted text is gone rather than merely hidden"
        );
        assert!(tracked(&document).is_empty());
    }

    #[test]
    fn rejecting_drops_the_insertion_and_keeps_the_deletion() {
        let mut document = reviewed();
        let mut history = History::new();
        resolve_all(&mut document, &mut history, Resolve::Reject);
        // The deleted text comes back as *ordinary* text: `<w:delText>` is not
        // `<w:t>`, and text left marked as deleted is skipped by everything that
        // reads the document.
        assert_eq!(document.text(), "kept removed");
        assert!(tracked(&document).is_empty());
    }

    #[test]
    fn settling_a_change_is_undoable_like_everything_else() {
        let mut document = reviewed();
        let mut history = History::new();
        resolve_all(&mut document, &mut history, Resolve::Accept);
        assert_eq!(document.text(), "kept added ");
        history.undo(&mut document);
        assert_eq!(tracked(&document).len(), 2, "both changes are back");
    }

    #[test]
    fn one_change_can_be_settled_without_touching_the_others() {
        let mut document = reviewed();
        let mut history = History::new();
        let mark = Mark::new(1, "Adnan Khan");
        assert!(resolve_one(
            &mut document,
            &mut history,
            &mark,
            Resolve::Reject
        ));
        assert_eq!(document.text(), "kept ", "the insertion went");
        let left = tracked(&document);
        assert_eq!(left.len(), 1, "the deletion is still there");
        assert_eq!(left[0].what, "deleted");
    }

    #[test]
    fn rejecting_a_formatting_change_puts_back_what_it_remembered() {
        // A `<w:rPrChange>` holds the *previous* properties, which is the whole
        // design of the element and the opposite of what it looks like.
        let mut run = Run::of("text");
        run.props.toggles.set(Toggle::Bold, true);
        let mut was = wp_model::RunProps::default();
        was.toggles.set(Toggle::Italic, true);
        run.prop_change = Some(Box::new(wp_model::PropChange {
            mark: Mark::new(5, "A"),
            previous: wp_model::revision::PreviousProps::Run(Box::new(was)),
        }));
        let mut document = document(vec![Block::Paragraph(Paragraph {
            content: vec![Inline::Run(run)],
            ..Paragraph::new()
        })]);
        let mut history = History::new();

        assert_eq!(tracked(&document).len(), 1);
        resolve_all(&mut document, &mut history, Resolve::Reject);
        let runs = document.paragraphs()[0].runs().len();
        assert_eq!(runs, 1);
        assert!(!document.paragraphs()[0].runs()[0].props.bold());
        assert!(document.paragraphs()[0].runs()[0].props.italic());
        assert!(tracked(&document).is_empty());
    }

    #[test]
    fn accepting_a_formatting_change_keeps_the_formatting_and_drops_the_record() {
        let mut run = Run::of("text");
        run.props.toggles.set(Toggle::Bold, true);
        run.prop_change = Some(Box::new(wp_model::PropChange {
            mark: Mark::new(5, "A"),
            previous: wp_model::revision::PreviousProps::Run(Box::default()),
        }));
        let mut document = document(vec![Block::Paragraph(Paragraph {
            content: vec![Inline::Run(run)],
            ..Paragraph::new()
        })]);
        let mut history = History::new();
        resolve_all(&mut document, &mut history, Resolve::Accept);
        assert!(document.paragraphs()[0].runs()[0].props.bold());
        assert!(tracked(&document).is_empty());
    }

    #[test]
    fn a_deleted_paragraph_mark_joins_two_paragraphs_when_it_is_accepted() {
        // That is what a tracked paragraph merge *is*.
        let mut first = Paragraph::of("first");
        first.mark_revision = Some(Revision::Deleted(Mark::new(9, "A")));
        let mut document = document(vec![
            Block::Paragraph(first),
            Block::Paragraph(Paragraph::of("second")),
        ]);
        let mut history = History::new();
        assert_eq!(tracked(&document).len(), 1);

        resolve_all(&mut document, &mut history, Resolve::Accept);
        assert_eq!(document.paragraphs().len(), 1);
        assert_eq!(document.text(), "firstsecond");
    }

    #[test]
    fn an_inserted_paragraph_mark_is_undone_by_rejecting_it() {
        let mut first = Paragraph::of("first");
        first.mark_revision = Some(Revision::Inserted(Mark::new(9, "A")));
        let mut document = document(vec![
            Block::Paragraph(first),
            Block::Paragraph(Paragraph::of("second")),
        ]);
        let mut history = History::new();
        resolve_all(&mut document, &mut history, Resolve::Reject);
        assert_eq!(document.paragraphs().len(), 1);
        assert_eq!(document.text(), "firstsecond");
    }

    // ---- tracked paragraph marks, in the shapes Word writes them ----------
    //
    // Measured on Word 16 through COM: the story workspace's
    // `bugs/evidence/word/paragraph-marks.ps1`, with each case's tracked XML
    // and what Accept All and Reject All made of it.

    /// Normal, named, as the style after a heading names it.
    const NORMAL: wp_model::StyleId = wp_model::StyleId(0);
    const HEADING: wp_model::StyleId = wp_model::StyleId(1);
    const QUOTE: wp_model::StyleId = wp_model::StyleId(2);

    fn by(id: u32) -> Mark {
        Mark::new(id, "Adnan Khan")
    }

    /// A paragraph of `text` in `style`, Normal when `None`.
    fn para(style: Option<wp_model::StyleId>, text: &str) -> Paragraph {
        let mut paragraph = Paragraph::of(text);
        paragraph.props.style = style;
        paragraph
    }

    /// `text`, deleted as change `id`.
    fn struck(id: u32, text: &str) -> Inline {
        Inline::Revised {
            revision: Revision::Deleted(by(id)),
            content: vec![Inline::Run(Run {
                content: vec![Piece::Deleted(text.into())],
                ..Run::default()
            })],
        }
    }

    /// A paragraph deleted whole: its mark as change `id`, its text as the
    /// next.
    fn deleted_whole(style: Option<wp_model::StyleId>, text: &str, id: u32) -> Paragraph {
        let mut paragraph = Paragraph {
            content: vec![struck(id + 1, text)],
            ..Paragraph::new()
        };
        paragraph.props.style = style;
        paragraph.mark_revision = Some(Revision::Deleted(by(id)));
        paragraph
    }

    /// A formatting change to a paragraph, which had `previous` before it.
    fn restyled(id: u32, previous: Option<wp_model::StyleId>) -> Option<Box<wp_model::PropChange>> {
        Some(Box::new(wp_model::PropChange {
            mark: by(id),
            previous: wp_model::revision::PreviousProps::Paragraph(Box::new(
                wp_model::prop::ParaProps {
                    style: previous,
                    ..Default::default()
                },
            )),
        }))
    }

    type Shape = Vec<(Option<wp_model::StyleId>, String)>;

    /// Each paragraph's style and text, as the probe reported Word's.
    fn shape(document: &Document) -> Shape {
        document
            .paragraphs()
            .iter()
            .map(|paragraph| (paragraph.props.style, paragraph.text()))
            .collect()
    }

    fn row(style: Option<wp_model::StyleId>, text: &str) -> (Option<wp_model::StyleId>, String) {
        (style, text.to_owned())
    }

    /// `word` with every change settled `how` at once, with nothing left
    /// tracked, and one undo giving `word` back.
    fn all_at_once(word: &Document, how: Resolve) -> Shape {
        let mut document = word.clone();
        let mut history = History::new();
        resolve_all(&mut document, &mut history, how);
        let left = tracked(&document);
        assert!(left.is_empty(), "{how:?} left {left:?}");
        let settled = shape(&document);
        history.undo(&mut document);
        assert_eq!(document.body, word.body, "undo after {how:?}");
        settled
    }

    /// The same, one change at a time in the list's order, each of which
    /// goes from the list when it is settled.
    fn one_by_one(word: &Document, how: Resolve) -> Shape {
        let mut document = word.clone();
        let mut history = History::new();
        let mut steps = 0;
        while let Some(change) = tracked(&document).first().cloned() {
            let before = tracked(&document).len();
            assert!(
                resolve_one(&mut document, &mut history, &change.mark, how),
                "{change:?}"
            );
            assert!(
                !tracked(&document)
                    .iter()
                    .any(|left| left.mark == change.mark),
                "{how:?} of {change:?} left it listed"
            );
            assert!(tracked(&document).len() < before);
            steps += 1;
        }
        let settled = shape(&document);
        for _ in 0..steps {
            history.undo(&mut document);
        }
        assert_eq!(
            document.body, word.body,
            "undo, step by step, after {how:?}"
        );
        settled
    }

    /// Settled both ways, all at once and one at a time, the same each way.
    fn settles(word: &Document, accepted: Shape, rejected: Shape) {
        for (how, expected) in [(Resolve::Accept, accepted), (Resolve::Reject, rejected)] {
            assert_eq!(all_at_once(word, how), expected, "{how:?} All");
            assert_eq!(one_by_one(word, how), expected, "{how:?}, one by one");
        }
    }

    /// Word's rule for a paragraph mark that goes, whether a deletion is
    /// accepted or an insertion rejected: the paragraph left has the
    /// properties of the mark that stays, which is the following paragraph's.
    /// A deleted heading accepted leaves the body text as body text.
    #[test]
    fn a_paragraph_mark_that_goes_leaves_the_following_paragraphs_properties() {
        // delete-first: a heading deleted whole, before a Normal paragraph.
        let word = document(vec![
            Block::Paragraph(deleted_whole(Some(HEADING), "Title words", 0)),
            Block::Paragraph(para(None, "Body words")),
        ]);
        settles(
            &word,
            vec![row(None, "Body words")],
            vec![row(Some(HEADING), "Title words"), row(None, "Body words")],
        );

        // delete-middle: a quotation deleted whole.
        let word = document(vec![
            Block::Paragraph(para(Some(HEADING), "Title words")),
            Block::Paragraph(deleted_whole(Some(QUOTE), "Quoted words", 0)),
            Block::Paragraph(para(None, "Body words")),
        ]);
        settles(
            &word,
            vec![row(Some(HEADING), "Title words"), row(None, "Body words")],
            vec![
                row(Some(HEADING), "Title words"),
                row(Some(QUOTE), "Quoted words"),
                row(None, "Body words"),
            ],
        );

        // delete-first-two: the heading and the quotation, from the start.
        let word = document(vec![
            Block::Paragraph(deleted_whole(Some(HEADING), "Title words", 0)),
            Block::Paragraph(deleted_whole(Some(QUOTE), "Quoted words", 2)),
            Block::Paragraph(para(None, "Body words")),
        ]);
        settles(
            &word,
            vec![row(None, "Body words")],
            vec![
                row(Some(HEADING), "Title words"),
                row(Some(QUOTE), "Quoted words"),
                row(None, "Body words"),
            ],
        );

        // insert-after-heading: Enter at the end of a heading, then words.
        let mut title = para(Some(HEADING), "Title words");
        title.mark_revision = Some(Revision::Inserted(by(0)));
        let added = Paragraph {
            props: title.props.clone(),
            content: vec![inserted_by(
                "Adnan Khan",
                1,
                vec![Inline::Run(Run::of("New words"))],
            )],
            ..Paragraph::new()
        };
        let word = document(vec![
            Block::Paragraph(title),
            Block::Paragraph(added),
            Block::Paragraph(para(None, "Body words")),
        ]);
        settles(
            &word,
            vec![
                row(Some(HEADING), "Title words"),
                row(Some(HEADING), "New words"),
                row(None, "Body words"),
            ],
            vec![row(Some(HEADING), "Title words"), row(None, "Body words")],
        );

        // insert-at-start: a new paragraph before the heading.
        let mut added = Paragraph {
            content: vec![inserted_by(
                "Adnan Khan",
                1,
                vec![Inline::Run(Run::of("New words"))],
            )],
            ..Paragraph::new()
        };
        added.props.style = Some(HEADING);
        added.mark_revision = Some(Revision::Inserted(by(0)));
        let word = document(vec![
            Block::Paragraph(added),
            Block::Paragraph(para(Some(HEADING), "Title words")),
            Block::Paragraph(para(None, "Body words")),
        ]);
        settles(
            &word,
            vec![
                row(Some(HEADING), "New words"),
                row(Some(HEADING), "Title words"),
                row(None, "Body words"),
            ],
            vec![row(Some(HEADING), "Title words"), row(None, "Body words")],
        );

        // replace-one-with-two: a quotation's text replaced by two
        // paragraphs of it.
        let mut first = Paragraph {
            content: vec![inserted_by(
                "Adnan Khan",
                1,
                vec![Inline::Run(Run::of("First new"))],
            )],
            ..Paragraph::new()
        };
        first.props.style = Some(QUOTE);
        first.mark_revision = Some(Revision::Inserted(by(0)));
        let mut second = Paragraph {
            content: vec![
                inserted_by("Adnan Khan", 2, vec![Inline::Run(Run::of("Second new"))]),
                struck(3, "Body words"),
            ],
            ..Paragraph::new()
        };
        second.props.style = Some(QUOTE);
        let word = document(vec![
            Block::Paragraph(para(Some(HEADING), "Title words")),
            Block::Paragraph(first),
            Block::Paragraph(second),
            Block::Paragraph(para(None, "End words")),
        ]);
        settles(
            &word,
            vec![
                row(Some(HEADING), "Title words"),
                row(Some(QUOTE), "First new"),
                row(Some(QUOTE), "Second new"),
                row(None, "End words"),
            ],
            vec![
                row(Some(HEADING), "Title words"),
                row(Some(QUOTE), "Body words"),
                row(None, "End words"),
            ],
        );
    }

    /// Word keeps the text before a deleted mark looking as it did by giving
    /// the following paragraph that look ahead of time, as a tracked
    /// formatting change (`delete-last`: from the end of a heading's text to
    /// the end of the document). A paragraph's formatting change is listed,
    /// and accepting it keeps the look while rejecting puts the old one
    /// back — all but the mark's own formatting, which it does not record.
    #[test]
    fn a_paragraphs_formatting_change_is_listed_accepted_and_rejected() {
        let mut title = para(Some(HEADING), "Title words");
        title.mark_revision = Some(Revision::Deleted(by(0)));
        title.prop_change = restyled(1, Some(HEADING));
        let mut body = Paragraph {
            content: vec![struck(3, "Body words")],
            ..Paragraph::new()
        };
        body.props.style = Some(HEADING);
        let mut bold = wp_model::RunProps::default();
        bold.toggles.set(Toggle::Bold, true);
        body.props.mark = Some(Box::new(bold.clone()));
        body.prop_change = restyled(2, None);
        let word = document(vec![Block::Paragraph(title), Block::Paragraph(body)]);
        // The same, its mark made bold as a tracked change of its own.
        let mut marked = word.clone();
        if let Block::Paragraph(body) = &mut marked.body[1] {
            body.mark_change = Some(Box::new(wp_model::PropChange {
                mark: by(4),
                previous: wp_model::revision::PreviousProps::Run(Box::default()),
            }));
        }

        let listed: Vec<(usize, &str)> = tracked(&word)
            .iter()
            .map(|change| (change.paragraph, change.what))
            .collect();
        assert_eq!(
            listed,
            [
                (0, "paragraph formatting changed"),
                (0, "paragraph break deleted"),
                (1, "paragraph formatting changed"),
                (1, "deleted"),
            ]
        );
        assert_eq!(tracked(&word)[0].text, "Title words");
        assert_eq!(
            next_revision_id(&word),
            4,
            "a formatting change's id is taken"
        );

        settles(
            &word,
            vec![row(Some(HEADING), "Title words")],
            vec![row(Some(HEADING), "Title words"), row(None, "Body words")],
        );
        // The mark accepted on its own takes the heading's change with it,
        // and leaves the following paragraph's changes to be settled on
        // their own: rejected, the joined paragraph is body text again, and
        // its mark plain.
        let mut document = marked.clone();
        let mut history = History::new();
        assert!(resolve_one(
            &mut document,
            &mut history,
            &by(0),
            Resolve::Accept
        ));
        let left: Vec<u32> = tracked(&document)
            .iter()
            .map(|change| change.mark.id)
            .collect();
        assert_eq!(left, [2, 3, 4]);
        for id in [2, 4] {
            assert!(resolve_one(
                &mut document,
                &mut history,
                &by(id),
                Resolve::Reject
            ));
        }
        assert_eq!(shape(&document), [row(None, "Title words")]);
        assert_eq!(document.paragraphs()[0].props.mark, Some(Box::default()));

        for how in [Resolve::Accept, Resolve::Reject] {
            let mut document = word.clone();
            resolve_all(&mut document, &mut History::new(), how);
            let last = document.paragraphs().last().map(|p| p.props.mark.clone());
            assert_eq!(
                last,
                Some(Some(Box::new(bold.clone()))),
                "{how:?}: the last mark keeps its own formatting"
            );
        }

        // A mark's own formatting change is listed where the mark is, and
        // settled as a run's is: rejected, the mark is plain again.
        let listed: Vec<(usize, &str, usize)> = tracked(&marked)
            .iter()
            .map(|change| (change.paragraph, change.what, change.mark.id as usize))
            .collect();
        assert_eq!(
            listed,
            [
                (0, "paragraph formatting changed", 1),
                (0, "paragraph break deleted", 0),
                (1, "paragraph formatting changed", 2),
                (1, "deleted", 3),
                (1, "formatting changed", 4),
            ]
        );
        settles(
            &marked,
            vec![row(Some(HEADING), "Title words")],
            vec![row(Some(HEADING), "Title words"), row(None, "Body words")],
        );
        for (how, mark) in [
            (Resolve::Accept, Some(Box::new(bold.clone()))),
            (Resolve::Reject, Some(Box::default())),
        ] {
            let mut document = marked.clone();
            resolve_all(&mut document, &mut History::new(), how);
            let last = document.paragraphs().last().map(|p| p.props.mark.clone());
            assert_eq!(last, Some(mark), "{how:?}: the mark's own change");
        }
    }

    // ---- recording, in the shapes Word writes ------------------------------
    //
    // Measured on Word 16 through COM: the story workspace's
    // `bugs/evidence/word/track-keys.ps1` and `track-others.ps1`.

    fn me() -> Author {
        Author::new("Adnan Khan")
    }

    fn inserted_by_assistant(id: u32, text: &str) -> Inline {
        inserted_by("Assistant", id, vec![Inline::Run(Run::of(text))])
    }

    fn deleted_by(author: &str, id: u32, text: &str) -> Inline {
        Inline::Revised {
            revision: Revision::Deleted(Mark::new(id, author)),
            content: vec![Inline::Run(Run {
                content: vec![Piece::Deleted(text.into())],
                ..Run::default()
            })],
        }
    }

    fn typed(id: u32, text: &str) -> Inline {
        inserted_by("Adnan Khan", id, vec![Inline::Run(Run::of(text))])
    }

    /// Backspace twice, and Delete twice: each pair is one deletion, as Word
    /// writes "ds" and "Bo" (`backspace-twice`).
    #[test]
    fn letters_deleted_one_after_another_are_one_deletion() {
        let mut paragraph = Paragraph::of("Body words");
        record_deletion(&mut paragraph, 9..10, &me(), 1).expect("recorded");
        record_deletion(&mut paragraph, 8..9, &me(), 2).expect("recorded");
        assert_eq!(
            paragraph.content,
            [
                Inline::Run(Run::of("Body wor")),
                deleted_by("Adnan Khan", 1, "ds")
            ]
        );
        let mut paragraph = Paragraph::of("Body words");
        record_deletion(&mut paragraph, 0..1, &me(), 1).expect("recorded");
        record_deletion(&mut paragraph, 0..1, &me(), 2).expect("recorded");
        assert_eq!(
            paragraph.content,
            [
                deleted_by("Adnan Khan", 1, "Bo"),
                Inline::Run(Run::of("dy words"))
            ]
        );
    }

    /// A word typed a letter at a time is one insertion, and so is a letter
    /// typed inside it (Word's `type-then-backspace` has one `<w:ins>`).
    #[test]
    fn a_word_typed_a_letter_at_a_time_is_one_insertion() {
        let mut paragraph = Paragraph::of("Body words");
        for (offset, letter) in [(10, "n"), (11, "e"), (12, "w")] {
            let id = offset as u32;
            record_insertion(&mut paragraph, offset, letter, &me(), id).expect("recorded");
        }
        record_insertion(&mut paragraph, 11, "X", &me(), 20).expect("recorded");
        assert_eq!(
            paragraph.content,
            [Inline::Run(Run::of("Body words")), typed(10, "nXew")]
        );
        // And a letter taken back from it goes, untracked.
        record_deletion(&mut paragraph, 11..12, &me(), 21).expect("recorded");
        assert_eq!(paragraph.shown_text(), "Body wordsnew");
        assert_eq!(
            tracked(&document(vec![Block::Paragraph(paragraph)])).len(),
            1
        );
    }

    /// Typing inside another author's insertion splits it around the new one
    /// (`type-in-others`).
    #[test]
    fn typing_inside_another_authors_insertion_splits_it() {
        let mut paragraph = Paragraph {
            content: vec![
                Inline::Run(Run::of("Body words")),
                inserted_by_assistant(1, " and more"),
            ],
            ..Paragraph::new()
        };
        record_insertion(&mut paragraph, 15, "x", &me(), 2).expect("recorded");
        assert_eq!(
            paragraph.content,
            [
                Inline::Run(Run::of("Body words")),
                inserted_by_assistant(1, " and "),
                typed(2, "x"),
                inserted_by_assistant(3, "more"),
            ]
        );
    }

    /// Deleting what another author inserted puts the deletion inside the
    /// insertion (`backspace-in-others`, `delete-word-in-others`,
    /// `delete-across-others`), so that rejecting it gives the insertion back,
    /// and rejecting the insertion takes both away.
    #[test]
    fn deleting_another_authors_insertion_keeps_the_deletion_inside_it() {
        let proposal = || Paragraph {
            content: vec![
                Inline::Run(Run::of("Body words")),
                inserted_by_assistant(1, " and more"),
            ],
            ..Paragraph::new()
        };
        // The edit cut the insertion first, and that half's id is spent.
        let nested = |kept: &str, gone: &str| {
            inserted_by(
                "Assistant",
                1,
                vec![
                    Inline::Run(Run::of(kept)),
                    deleted_by("Adnan Khan", 3, gone),
                ],
            )
        };
        let mut paragraph = proposal();
        record_deletion(&mut paragraph, 18..19, &me(), 2).expect("recorded");
        assert_eq!(
            paragraph.content,
            [Inline::Run(Run::of("Body words")), nested(" and mor", "e")]
        );
        let mut paragraph = proposal();
        record_deletion(&mut paragraph, 15..19, &me(), 2).expect("recorded");
        assert_eq!(
            paragraph.content,
            [Inline::Run(Run::of("Body words")), nested(" and ", "more")]
        );

        let mut paragraph = proposal();
        record_deletion(&mut paragraph, 5..15, &me(), 2).expect("recorded");
        assert_eq!(
            paragraph.content,
            [
                Inline::Run(Run::of("Body ")),
                deleted_by("Adnan Khan", 3, "words"),
                inserted_by(
                    "Assistant",
                    1,
                    vec![
                        deleted_by("Adnan Khan", 4, " and "),
                        Inline::Run(Run::of("more")),
                    ]
                ),
            ]
        );
        let word = document(vec![Block::Paragraph(paragraph)]);
        settles(
            &word,
            vec![row(None, "Body more")],
            vec![row(None, "Body words")],
        );
    }

    /// What is deleted already stays as it is, and a deletion crossing it is
    /// two; the anchors of a comment stay where they are.
    #[test]
    fn a_deletion_leaves_what_is_deleted_already_and_the_anchors_where_they_are() {
        let mut paragraph = Paragraph {
            content: vec![
                Inline::Run(Run::of("ab")),
                deleted_by("Someone", 1, "XY"),
                Inline::Anchor(wp_model::Anchor::CommentStart { id: 7 }),
                Inline::Run(Run::of("cd")),
            ],
            ..Paragraph::new()
        };
        record_deletion(&mut paragraph, 1..3, &me(), 2).expect("recorded");
        assert_eq!(
            paragraph.content,
            [
                Inline::Run(Run::of("a")),
                deleted_by("Adnan Khan", 2, "b"),
                deleted_by("Someone", 1, "XY"),
                Inline::Anchor(wp_model::Anchor::CommentStart { id: 7 }),
                deleted_by("Adnan Khan", 3, "c"),
                Inline::Run(Run::of("d")),
            ]
        );
        assert_eq!(paragraph.text(), "ad");
    }

    #[test]
    fn a_deletion_across_a_hyperlink_refuses_and_changes_nothing() {
        let mut paragraph = Paragraph {
            content: vec![
                Inline::Run(Run::of("see ")),
                Inline::Hyperlink(Box::new(wp_model::Hyperlink {
                    rel: None,
                    anchor: Some("x".into()),
                    tooltip: None,
                    history: true,
                    content: vec![Inline::Run(Run::of("linked"))],
                })),
            ],
            ..Paragraph::new()
        };
        let before = paragraph.clone();
        assert_eq!(record_deletion(&mut paragraph, 2..8, &me(), 1), None);
        assert_eq!(paragraph, before);
    }

    /// A heading, then body text, each with a style of its own, in a document
    /// whose heading names Normal as the style after it.
    fn title_and_body(title: &str, body: &str) -> Document {
        let mut document = document(vec![
            Block::Paragraph(para(Some(HEADING), title)),
            Block::Paragraph(para(None, body)),
        ]);
        let normal = document.styles.insert(wp_model::Style::new(
            "Normal",
            wp_model::StyleKind::Paragraph,
        ));
        assert_eq!(normal, NORMAL);
        let mut heading = wp_model::Style::new("Heading1", wp_model::StyleKind::Paragraph);
        heading.next = Some(normal);
        assert_eq!(document.styles.insert(heading), HEADING);
        document
    }

    fn at(paragraph: usize, offset: usize) -> Caret {
        Caret { paragraph, offset }
    }

    fn span(anchor: Caret, head: Caret) -> Selection {
        Selection { anchor, head }
    }

    /// Backspace at the start of body text after a heading (`backspace-at-start`),
    /// or Delete at the heading's end (`delete-at-end`): the heading's mark is
    /// deleted, and the body paragraph is given the heading's style as a
    /// formatting change, so that accepting leaves one heading.
    #[test]
    fn a_paragraph_mark_deleted_after_text_is_recorded_as_word_records_it() {
        for (forward, caret) in [(false, at(0, 11)), (true, at(1, 0))] {
            let mut word = title_and_body("Title words", "Body words");
            let before = word.clone();
            let mut history = History::new();
            let landed = delete_range(
                &mut word,
                Scope::Body,
                &mut history,
                span(at(0, 11), at(1, 0)),
                &me(),
                forward,
            );
            assert_eq!(landed, Ok(caret));
            let paragraphs = word.paragraphs();
            assert_eq!(paragraphs[0].mark_revision, Some(Revision::Deleted(by(1))));
            assert_eq!(paragraphs[1].props.style, Some(HEADING));
            assert_eq!(paragraphs[1].prop_change, restyled(2, None));
            settles(
                &word,
                vec![row(Some(HEADING), "Title wordsBody words")],
                vec![row(Some(HEADING), "Title words"), row(None, "Body words")],
            );
            history.undo(&mut word);
            assert_eq!(word.body, before.body);
        }

        // After an empty paragraph there is no text to keep a look for
        // (`backspace-after-empty`).
        let mut word = document(vec![
            Block::Paragraph(para(None, "")),
            Block::Paragraph(para(Some(HEADING), "Body words")),
        ]);
        delete_range(
            &mut word,
            Scope::Body,
            &mut History::new(),
            span(at(0, 0), at(1, 0)),
            &me(),
            false,
        )
        .expect("recorded");
        assert_eq!(word.paragraphs()[1].prop_change, None);
        settles(
            &word,
            vec![row(Some(HEADING), "Body words")],
            vec![row(None, ""), row(Some(HEADING), "Body words")],
        );
    }

    /// A selection from inside a heading to inside the body (`delete-across`).
    #[test]
    fn a_selection_across_paragraphs_is_deleted_as_word_deletes_it() {
        let mut word = title_and_body("Title words", "Body words");
        delete_range(
            &mut word,
            Scope::Body,
            &mut History::new(),
            span(at(1, 5), at(0, 6)),
            &me(),
            false,
        )
        .expect("recorded");
        let paragraphs = word.paragraphs();
        assert_eq!(
            paragraphs[0].content,
            [
                Inline::Run(Run::of("Title ")),
                deleted_by("Adnan Khan", 1, "words")
            ]
        );
        assert_eq!(paragraphs[0].mark_revision, Some(Revision::Deleted(by(3))));
        assert_eq!(
            paragraphs[1].content,
            [
                deleted_by("Adnan Khan", 2, "Body "),
                Inline::Run(Run::of("words"))
            ]
        );
        assert_eq!(paragraphs[1].props.style, Some(HEADING));
        assert_eq!(paragraphs[1].prop_change, restyled(4, None));
        settles(
            &word,
            vec![row(Some(HEADING), "Title words")],
            vec![row(Some(HEADING), "Title words"), row(None, "Body words")],
        );
    }

    /// Enter inside a heading (`enter-mid`) marks the new mark inserted; at a
    /// heading's end (`enter-at-end`) the new paragraph takes the style after
    /// the heading, as a formatting change. Backspace straight after takes the
    /// break back with nothing to show for it (`enter-then-backspace`).
    #[test]
    fn enter_is_recorded_as_word_records_it_and_backspace_takes_it_back() {
        let mut word = title_and_body("Title words", "Body words");
        let before = word.clone();
        let mut history = History::new();
        let caret = split_paragraph(&mut word, Scope::Body, &mut history, at(0, 5), &me());
        assert_eq!(caret, at(1, 0));
        let paragraphs = word.paragraphs();
        assert_eq!(paragraphs[0].mark_revision, Some(Revision::Inserted(by(1))));
        assert_eq!(
            (paragraphs[1].props.style, &paragraphs[1].mark_revision),
            (Some(HEADING), &None)
        );
        assert_eq!(paragraphs[1].prop_change, None);
        settles(
            &word,
            vec![
                row(Some(HEADING), "Title"),
                row(Some(HEADING), " words"),
                row(None, "Body words"),
            ],
            vec![row(Some(HEADING), "Title words"), row(None, "Body words")],
        );
        history.undo(&mut word);
        assert_eq!(word.body, before.body, "undone exactly");

        let mut word = before.clone();
        split_paragraph(&mut word, Scope::Body, &mut history, at(0, 11), &me());
        if let Block::Paragraph(new) = &mut word.body[1] {
            record_insertion(new, 0, "New", &me(), 3).expect("recorded");
        }
        let paragraphs = word.paragraphs();
        assert_eq!(
            paragraphs[1].props.style,
            Some(NORMAL),
            "the style after a heading"
        );
        assert_eq!(paragraphs[1].prop_change, restyled(2, Some(HEADING)));
        settles(
            &word,
            vec![
                row(Some(HEADING), "Title words"),
                row(Some(NORMAL), "New"),
                row(None, "Body words"),
            ],
            vec![row(Some(HEADING), "Title words"), row(None, "Body words")],
        );

        let mut word = before.clone();
        split_paragraph(&mut word, Scope::Body, &mut history, at(0, 11), &me());
        delete_range(
            &mut word,
            Scope::Body,
            &mut history,
            span(at(0, 11), at(1, 0)),
            &me(),
            false,
        )
        .expect("recorded");
        assert_eq!(word.body, before.body, "nothing to show for it");
        assert!(tracked(&word).is_empty());
    }

    /// Backspace over a break another author inserted marks it deleted as
    /// well (`backspace-after-others-break`): Accept All and Reject All both
    /// join the two paragraphs, and each change settles on its own.
    #[test]
    fn a_break_another_author_inserted_is_marked_deleted_too() {
        let mut first = para(None, "First");
        first.mark_revision = Some(Revision::Inserted(Mark::new(1, "Assistant")));
        let mut word = document(vec![
            Block::Paragraph(first),
            Block::Paragraph(para(None, "Second")),
        ]);
        delete_range(
            &mut word,
            Scope::Body,
            &mut History::new(),
            span(at(0, 5), at(1, 0)),
            &me(),
            false,
        )
        .expect("recorded");
        let paragraphs = word.paragraphs();
        assert_eq!(
            paragraphs[0].mark_revision,
            Some(Revision::Inserted(Mark::new(1, "Assistant")))
        );
        assert_eq!(paragraphs[0].mark_deleted, Some(by(2)));
        let listed: Vec<&str> = tracked(&word).iter().map(|change| change.what).collect();
        assert_eq!(
            listed,
            ["paragraph break inserted", "paragraph break deleted"]
        );
        settles(
            &word,
            vec![row(None, "FirstSecond")],
            vec![row(None, "FirstSecond")],
        );
        // The insertion accepted leaves the deletion; the deletion rejected
        // leaves the insertion.
        let mut one = word.clone();
        let mut history = History::new();
        assert!(resolve_one(
            &mut one,
            &mut history,
            &Mark::new(1, "Assistant"),
            Resolve::Accept
        ));
        assert_eq!(
            one.paragraphs()[0].mark_revision,
            Some(Revision::Deleted(by(2)))
        );
        assert_eq!(one.paragraphs()[0].mark_deleted, None);
        let mut one = word.clone();
        assert!(resolve_one(&mut one, &mut history, &by(2), Resolve::Reject));
        assert_eq!(
            one.paragraphs()[0].mark_revision,
            Some(Revision::Inserted(Mark::new(1, "Assistant")))
        );
        assert_eq!(one.paragraphs()[0].mark_deleted, None);
    }

    /// Paragraphs pasted with Track Changes on are inserted, text and breaks,
    /// as they read with their own changes accepted.
    #[test]
    fn pasted_paragraphs_are_inserted_breaks_and_all() {
        let mut word = title_and_body("Title words", "Body words");
        let mut copied = para(Some(QUOTE), "one");
        copied.content.push(deleted_by("Someone", 9, "gone"));
        let clip = vec![Paragraph::of("A "), copied, Paragraph::of("two ")];
        let caret = paste_paragraphs(
            &mut word,
            Scope::Body,
            &mut History::new(),
            Selection::at(at(1, 0)),
            &clip,
            &me(),
        )
        .expect("recorded");
        assert_eq!(caret, at(3, 4));
        let listed: Vec<(usize, &str, String)> = tracked(&word)
            .iter()
            .map(|change| (change.paragraph, change.what, change.text.clone()))
            .collect();
        assert_eq!(
            listed,
            [
                (1, "inserted", "A".to_owned()),
                (1, "paragraph break inserted", String::new()),
                (2, "inserted", "one".to_owned()),
                (2, "paragraph break inserted", String::new()),
                (3, "inserted", "two".to_owned()),
            ]
        );
        settles(
            &word,
            vec![
                row(Some(HEADING), "Title words"),
                row(None, "A "),
                row(Some(QUOTE), "one"),
                row(None, "two Body words"),
            ],
            vec![row(Some(HEADING), "Title words"), row(None, "Body words")],
        );
    }

    // ---- what the review of the recording found -----------------------------

    fn picture() -> Piece {
        Piece::Drawing(Box::new(wp_model::doc::Drawing {
            source: Vec::new().into(),
            source_format: wp_model::SourceFormat::Authored,
            anchored: false,
            extent: (wp_model::Emu(914_400), wp_model::Emu(914_400)),
            rel: Some("rId9".into()),
            chart: None,
            name: Some("Picture".into()),
            description: None,
            wrap: wp_model::doc::Wrap::None,
            distance: (
                wp_model::Emu(0),
                wp_model::Emu(0),
                wp_model::Emu(0),
                wp_model::Emu(0),
            ),
            position: None,
            behind_text: false,
            text: None,
            tone: None,
            outline: None,
        }))
    }

    /// A table of one cell holding `text`.
    fn one_cell(content: Vec<Block>) -> Block {
        Block::Table(wp_model::table::Table {
            rows: vec![wp_model::table::Row {
                cells: vec![wp_model::table::Cell {
                    props: wp_model::table::CellProps::new(),
                    content,
                }],
                ..wp_model::table::Row::new()
            }],
            ..wp_model::table::Table::new()
        })
    }

    /// Accept All and Reject All replaced the flow's paragraphs as one list,
    /// and a table among them went: its paragraphs became the body's.
    #[test]
    fn settling_everything_keeps_tables_whole_and_joins_nothing_across_one() {
        let mut first = Paragraph {
            content: vec![Inline::Run(Run::of("keep")), typed(1, "new")],
            ..Paragraph::new()
        };
        first.mark_revision = Some(Revision::Deleted(by(2)));
        let mut in_cell = Paragraph::of("cell");
        in_cell.content.push(deleted_by("Adnan Khan", 3, "x"));
        // A break the Assistant put in and someone took out again, at the
        // cell's end, where there is nothing to join: settled where it stands.
        in_cell.mark_revision = Some(Revision::Inserted(Mark::new(4, "Assistant")));
        in_cell.mark_deleted = Some(by(5));
        let word = document(vec![
            Block::Paragraph(first),
            one_cell(vec![Block::Paragraph(in_cell)]),
            Block::Paragraph(para(None, "after")),
        ]);
        for how in [Resolve::Accept, Resolve::Reject] {
            let mut document = word.clone();
            let mut history = History::new();
            resolve_all(&mut document, &mut history, how);
            assert!(
                matches!(document.body[1], Block::Table(_)),
                "{how:?}: the table stands"
            );
            assert_eq!(document.body.len(), 3, "{how:?}: nothing joined across it");
            assert!(tracked(&document).is_empty(), "{how:?}");
            history.undo(&mut document);
            assert_eq!(document.body, word.body, "{how:?}: undone whole");
        }
        let mut accepted = word.clone();
        resolve_all(&mut accepted, &mut History::new(), Resolve::Accept);
        let texts: Vec<String> = accepted.paragraphs().iter().map(|p| p.text()).collect();
        assert_eq!(texts, ["keepnew", "cell", "after"]);

        // One change at a time: a deleted mark before a table is settled where
        // it stands.
        let mut document = word.clone();
        assert!(resolve_one(
            &mut document,
            &mut History::new(),
            &by(2),
            Resolve::Accept
        ));
        assert_eq!(document.body.len(), 3);
        assert_eq!(document.paragraphs()[0].mark_revision, None);
    }

    /// Settling rewrote every paragraph of the flow, joining runs Word had
    /// split, so a save wrote them all afresh: one with nothing tracked in it
    /// is left exactly as it was.
    #[test]
    fn settling_leaves_a_paragraph_with_nothing_tracked_as_it_was() {
        let untouched = Paragraph {
            content: vec![
                Inline::Run(Run::of("Hello ")),
                Inline::Run(Run::of("wrold")),
            ],
            ..Paragraph::new()
        };
        let changed = Paragraph {
            content: vec![Inline::Run(Run::of("one ")), typed(1, "two")],
            ..Paragraph::new()
        };
        let mut word = document(vec![
            Block::Paragraph(untouched.clone()),
            Block::Paragraph(changed),
        ]);
        resolve_all(&mut word, &mut History::new(), Resolve::Accept);
        assert_eq!(word.paragraphs()[0], &untouched);
        assert_eq!(
            word.paragraphs()[1].content,
            [Inline::Run(Run::of("one two"))],
            "the settled one is joined up"
        );
    }

    /// A tracked deletion reaching over a table is refused, and so is one
    /// from one cell to another: nothing is changed, and nothing to undo.
    #[test]
    fn a_tracked_deletion_across_a_table_is_refused() {
        let mut word = document(vec![
            Block::Paragraph(para(None, "before")),
            one_cell(vec![Block::Paragraph(para(None, "cell"))]),
            Block::Paragraph(para(None, "after")),
        ]);
        let before = word.clone();
        let mut history = History::new();
        for (from, to) in [
            (at(0, 2), at(2, 1)),
            (at(0, 2), at(1, 1)),
            (at(1, 2), at(2, 1)),
        ] {
            assert_eq!(
                delete_range(
                    &mut word,
                    Scope::Body,
                    &mut history,
                    span(from, to),
                    &me(),
                    false
                ),
                Err(ACROSS_BLOCKS)
            );
        }
        assert_eq!(word.body, before.body);
        assert!(!history.can_undo());
    }

    /// Delete over a mark that is deleted already has nothing new to record:
    /// no formatting change, no undo step.
    #[test]
    fn crossing_a_mark_deleted_already_records_nothing() {
        let mut title = para(Some(HEADING), "Title");
        title.mark_revision = Some(Revision::Deleted(Mark::new(1, "Someone")));
        let mut word = document(vec![
            Block::Paragraph(title),
            Block::Paragraph(para(None, "Body")),
        ]);
        let before = word.clone();
        let mut history = History::new();
        let caret = delete_range(
            &mut word,
            Scope::Body,
            &mut history,
            span(at(0, 5), at(1, 0)),
            &me(),
            true,
        );
        assert_eq!(caret, Ok(at(1, 0)), "Delete's caret goes past it");
        assert_eq!(word.body, before.body);
        assert!(!history.can_undo());

        // With text on either side deleted, only the text is recorded: no mark
        // went, so no look is kept for one.
        delete_range(
            &mut word,
            Scope::Body,
            &mut history,
            span(at(0, 2), at(1, 2)),
            &me(),
            false,
        )
        .expect("recorded");
        let listed: Vec<&str> = tracked(&word).iter().map(|change| change.what).collect();
        assert_eq!(
            listed,
            ["deleted", "paragraph break deleted", "deleted"],
            "{listed:?}"
        );
    }

    /// Text one's own insertion held is taken back, but a comment's anchors
    /// and its reference in it stay: the comment was left with no place.
    #[test]
    fn text_taken_back_keeps_the_anchors_in_it() {
        let reference = Inline::Run(Run {
            content: vec![Piece::CommentRef(7)],
            ..Run::default()
        });
        let mut paragraph = Paragraph {
            content: vec![
                Inline::Run(Run::of("kept ")),
                inserted_by(
                    "Adnan Khan",
                    1,
                    vec![
                        Inline::Anchor(wp_model::Anchor::CommentStart { id: 7 }),
                        Inline::Run(Run::of("new")),
                        Inline::Anchor(wp_model::Anchor::CommentEnd { id: 7 }),
                        reference.clone(),
                    ],
                ),
            ],
            ..Paragraph::new()
        };
        record_deletion(&mut paragraph, 5..8, &me(), 2).expect("recorded");
        assert_eq!(
            paragraph.content,
            [
                Inline::Run(Run::of("kept ")),
                Inline::Anchor(wp_model::Anchor::CommentStart { id: 7 }),
                Inline::Anchor(wp_model::Anchor::CommentEnd { id: 7 }),
                reference,
            ]
        );
    }

    /// Enter inside another author's insertion leaves two changes with ids of
    /// their own, as Word's `type-in-others` does; and Enter then Backspace in
    /// the middle of a run leaves the run as it was.
    #[test]
    fn a_change_cut_by_enter_is_two_changes_and_backspace_puts_the_run_back() {
        let mut word = document(vec![Block::Paragraph(Paragraph {
            content: vec![inserted_by_assistant(1, "abcd")],
            ..Paragraph::new()
        })]);
        split_paragraph(&mut word, Scope::Body, &mut History::new(), at(0, 2), &me());
        let ids: Vec<u32> = tracked(&word)
            .iter()
            .filter(|change| change.what == "inserted")
            .map(|change| change.mark.id)
            .collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1], "{ids:?}");

        let mut word = title_and_body("Title words", "Body words");
        let before = word.clone();
        let mut history = History::new();
        split_paragraph(&mut word, Scope::Body, &mut history, at(0, 5), &me());
        delete_range(
            &mut word,
            Scope::Body,
            &mut history,
            span(at(0, 5), at(1, 0)),
            &me(),
            false,
        )
        .expect("recorded");
        assert_eq!(word.body, before.body, "one run again");
    }

    /// Typed over a selection, the old words are struck and the new follow
    /// them, as Word types them (`type-over`); a paste over one follows them
    /// too, and a break made over one comes before them (`enter-over`,
    /// `page-break-over`).
    #[test]
    fn what_replaces_a_selection_follows_what_it_deleted() {
        let mut paragraph = Paragraph::of("Body words");
        record_deletion(&mut paragraph, 0..5, &me(), 1).expect("recorded");
        record_replacement(&mut paragraph, 0, "Some ", &me(), 2, None).expect("recorded");
        assert_eq!(
            paragraph.content,
            [
                deleted_by("Adnan Khan", 1, "Body "),
                typed(2, "Some "),
                Inline::Run(Run::of("words")),
            ]
        );

        let mut word = title_and_body("Title words", "Body words");
        paste_paragraphs(
            &mut word,
            Scope::Body,
            &mut History::new(),
            span(at(1, 0), at(1, 5)),
            &[Paragraph::of("A "), Paragraph::of("B ")],
            &me(),
        )
        .expect("recorded");
        let listed: Vec<(usize, &str, String)> = tracked(&word)
            .iter()
            .map(|change| (change.paragraph, change.what, change.text.clone()))
            .collect();
        assert_eq!(
            listed,
            [
                (1, "deleted", "Body".to_owned()),
                (1, "inserted", "A".to_owned()),
                (1, "paragraph break inserted", String::new()),
                (2, "inserted", "B".to_owned()),
            ]
        );
        settles(
            &word,
            vec![
                row(Some(HEADING), "Title words"),
                row(None, "A "),
                row(None, "B words"),
            ],
            vec![row(Some(HEADING), "Title words"), row(None, "Body words")],
        );

        let mut word = title_and_body("Title words", "Body words");
        delete_range(
            &mut word,
            Scope::Body,
            &mut History::new(),
            span(at(1, 0), at(1, 5)),
            &me(),
            false,
        )
        .expect("recorded");
        split_paragraph(&mut word, Scope::Body, &mut History::new(), at(1, 0), &me());
        let listed: Vec<(usize, &str)> = tracked(&word)
            .iter()
            .map(|change| (change.paragraph, change.what))
            .collect();
        assert_eq!(listed, [(1, "paragraph break inserted"), (2, "deleted")]);
    }

    /// A paste where nothing can be recorded — inside a hyperlink — is refused
    /// before anything is done, the selection it was to replace included.
    #[test]
    fn a_paste_where_nothing_can_be_recorded_is_refused_whole() {
        let mut word = document(vec![Block::Paragraph(Paragraph {
            content: vec![
                Inline::Run(Run::of("see ")),
                Inline::Hyperlink(Box::new(wp_model::Hyperlink {
                    rel: None,
                    anchor: Some("x".into()),
                    tooltip: None,
                    history: true,
                    content: vec![Inline::Run(Run::of("linked"))],
                })),
            ],
            ..Paragraph::new()
        })]);
        let before = word.clone();
        let mut history = History::new();
        let clip = vec![Paragraph::of("one"), Paragraph::of("two")];
        for selection in [Selection::at(at(0, 7)), span(at(0, 1), at(0, 7))] {
            assert_eq!(
                paste_paragraphs(
                    &mut word,
                    Scope::Body,
                    &mut history,
                    selection,
                    &clip,
                    &me()
                ),
                Err(CANNOT_RECORD)
            );
        }
        assert_eq!(word.body, before.body);
        assert!(!history.can_undo());
    }

    /// A picture deleted with Track Changes on is struck, in a run of its own;
    /// one the same author put in is taken back.
    #[test]
    fn a_picture_deleted_is_struck_and_one_just_put_in_is_taken_back() {
        let mut paragraph = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![Piece::Text("a".into()), picture(), Piece::Text("b".into())],
                ..Run::default()
            })],
            ..Paragraph::new()
        };
        record_drawing_deletion(&mut paragraph, 0, &me(), 1).expect("recorded");
        assert_eq!(
            paragraph.content,
            [
                Inline::Run(Run::of("a")),
                Inline::Revised {
                    revision: Revision::Deleted(by(1)),
                    content: vec![Inline::Run(Run {
                        content: vec![picture()],
                        ..Run::default()
                    })],
                },
                Inline::Run(Run::of("b")),
            ]
        );
        assert_eq!(paragraph.drawings().len(), 1, "still drawn, struck");

        let mut paragraph = Paragraph {
            content: vec![inserted_by(
                "Adnan Khan",
                1,
                vec![Inline::Run(Run {
                    content: vec![picture()],
                    ..Run::default()
                })],
            )],
            ..Paragraph::new()
        };
        record_drawing_deletion(&mut paragraph, 0, &me(), 2).expect("recorded");
        assert!(paragraph.content.is_empty(), "{:?}", paragraph.content);
    }

    /// Ctrl+Enter with Track Changes on: the break inserted at the end of the
    /// first paragraph, and the mark after it inserted too.
    #[test]
    fn a_page_break_is_inserted_with_its_mark() {
        let mut word = title_and_body("Title words", "Body words");
        let before = word.clone();
        let mut history = History::new();
        let caret = insert_break(
            &mut word,
            Scope::Body,
            &mut history,
            at(1, 4),
            wp_model::doc::Break::Page,
            &me(),
        );
        assert_eq!(caret, Ok(at(2, 0)));
        let listed: Vec<(usize, &str)> = tracked(&word)
            .iter()
            .map(|change| (change.paragraph, change.what))
            .collect();
        assert_eq!(listed, [(1, "inserted"), (1, "paragraph break inserted")]);
        settles(
            &word,
            vec![
                row(Some(HEADING), "Title words"),
                row(None, "Body"),
                row(None, " words"),
            ],
            vec![row(Some(HEADING), "Title words"), row(None, "Body words")],
        );
        history.undo(&mut word);
        assert_eq!(word.body, before.body);
    }

    // ---- what the second review found --------------------------------------

    /// Comments were anchored, and found, by a count that took in deleted
    /// text and left out tabs: a comment on words after a tracked deletion
    /// landed on the wrong ones, or inside a character.
    #[test]
    fn a_comment_after_a_tracked_deletion_is_anchored_on_its_words() {
        let mut document = document(vec![Block::Paragraph(Paragraph::of("X\u{e9}ab"))]);
        let mut history = History::new();
        if let Block::Paragraph(paragraph) = &mut document.body[0] {
            record_deletion(paragraph, 0..1, &me(), 1).expect("recorded");
        }
        // "éab" as a caret counts: é is 0..2, a 2..3, b 3..4.
        let id = add_comment(
            &mut document,
            &mut history,
            Scope::Body,
            span(at(0, 2), at(0, 3)),
            "A",
            "A",
            "on a",
        );
        let ranges = comment_ranges(&document);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].id, id);
        assert_eq!(ranges[0].range.ordered(), (at(0, 2), at(0, 3)));

        // After a tab, which a caret counts and the old count did not.
        let mut document = document_with_tab();
        let id = add_comment(
            &mut document,
            &mut History::new(),
            Scope::Body,
            span(at(0, 2), at(0, 4)),
            "A",
            "A",
            "on bc",
        );
        let range = comment_ranges(&document)
            .into_iter()
            .find(|range| range.id == id)
            .expect("its range")
            .range;
        assert_eq!(range.ordered(), (at(0, 2), at(0, 4)));
        assert_eq!(comment_at(&document, id), Some((Scope::Body, at(0, 2))));
    }

    /// A comment whose end an edit lost runs to the end of its paragraph as
    /// a caret counts it, which an equation takes no part of.
    #[test]
    fn a_comment_whose_end_is_lost_runs_to_the_end_a_caret_reaches() {
        let document = document(vec![Block::Paragraph(Paragraph {
            content: vec![
                Inline::Run(Run::of("ab")),
                Inline::Math(Box::new(wp_model::doc::MathBlob {
                    source: std::sync::Arc::from(&b"<m:oMath/>"[..]),
                    text: "x+y".into(),
                })),
                Inline::Anchor(wp_model::Anchor::CommentStart { id: 4 }),
                Inline::Run(Run::of("cd")),
            ],
            ..Paragraph::new()
        })]);
        let ranges = comment_ranges(&document);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].range.ordered(), (at(0, 2), at(0, 4)));
    }

    /// An anchor put at an offset inside a character — a selection left
    /// over from before an edit — goes before the character rather than
    /// cutting it.
    #[test]
    fn an_anchor_put_inside_a_character_goes_before_it() {
        let mut paragraph = Paragraph::of("\u{e9}t\u{e9}");
        insert_inlines(
            &mut paragraph,
            1,
            vec![Inline::Anchor(wp_model::Anchor::CommentStart { id: 1 })],
        );
        assert_eq!(paragraph.text(), "\u{e9}t\u{e9}");
        let document = document(vec![Block::Paragraph(paragraph)]);
        assert_eq!(comment_at(&document, 1), Some((Scope::Body, at(0, 0))));
    }

    fn document_with_tab() -> Document {
        document(vec![Block::Paragraph(Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::Text("a".into()),
                    Piece::Tab,
                    Piece::Text("bc".into()),
                ],
                ..Run::default()
            })],
            ..Paragraph::new()
        })])
    }

    /// Backspace just after a footnote's reference, or a comment's, or at a
    /// field's first letter, took the reference or the field's characters
    /// into the deletion, and Accept All took them away.
    #[test]
    fn a_tracked_deletion_leaves_what_stands_at_its_edges() {
        let note = Piece::FootnoteRef {
            id: 1,
            custom_mark: false,
        };
        let mut word = document(vec![Block::Paragraph(Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::Text("word".into()),
                    note.clone(),
                    Piece::Text(" next".into()),
                ],
                ..Run::default()
            })],
            ..Paragraph::new()
        })]);
        delete_range(
            &mut word,
            Scope::Body,
            &mut History::new(),
            span(at(0, 4), at(0, 5)),
            &me(),
            false,
        )
        .expect("recorded");
        resolve_all(&mut word, &mut History::new(), Resolve::Accept);
        let pieces: Vec<Piece> = word.paragraphs()[0]
            .runs()
            .iter()
            .flat_map(|run| run.content.clone())
            .collect();
        assert!(pieces.contains(&note), "{pieces:?}");
        assert_eq!(word.paragraphs()[0].text(), "wordnext");

        // A field's result deleted from its first letter keeps the field.
        let mut field = document(vec![Block::Paragraph(Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::FieldStart {
                        dirty: false,
                        lock: false,
                    },
                    Piece::Instruction(" PAGE ".into()),
                    Piece::FieldSeparate,
                    Piece::Text("12".into()),
                    Piece::FieldEnd,
                ],
                ..Run::default()
            })],
            ..Paragraph::new()
        })]);
        delete_range(
            &mut field,
            Scope::Body,
            &mut History::new(),
            span(at(0, 0), at(0, 1)),
            &me(),
            false,
        )
        .expect("recorded");
        resolve_all(&mut field, &mut History::new(), Resolve::Accept);
        let count = |wanted: fn(&Piece) -> bool| {
            field.paragraphs()[0]
                .runs()
                .iter()
                .flat_map(|run| run.content.iter())
                .filter(|piece| wanted(piece))
                .count()
        };
        assert_eq!(count(|piece| matches!(piece, Piece::FieldStart { .. })), 1);
        assert_eq!(count(|piece| matches!(piece, Piece::FieldEnd)), 1);
        assert_eq!(field.paragraphs()[0].text(), "2");

        // Words deleted from just after an equation leave the equation,
        // which a deletion cannot hold, and are recorded.
        let mut math = document(vec![Block::Paragraph(Paragraph {
            content: vec![
                Inline::Math(Box::new(wp_model::doc::MathBlob {
                    source: std::sync::Arc::from(&b"<m:oMath/>"[..]),
                    text: "x+y".into(),
                })),
                Inline::Run(Run::of("gone kept")),
            ],
            ..Paragraph::new()
        })]);
        delete_range(
            &mut math,
            Scope::Body,
            &mut History::new(),
            span(at(0, 0), at(0, 5)),
            &me(),
            false,
        )
        .expect("recorded beside the equation");
        resolve_all(&mut math, &mut History::new(), Resolve::Accept);
        assert!(matches!(math.paragraphs()[0].content[0], Inline::Math(_)));
        assert_eq!(math.paragraphs()[0].text(), "x+ykept");
    }

    /// A deletion across a table's cells or around a table says so in its own
    /// words, not a hyperlink's.
    #[test]
    fn a_deletion_across_blocks_says_what_is_in_the_way() {
        assert!(ACROSS_BLOCKS.contains("table"));
        assert_ne!(ACROSS_BLOCKS, CANNOT_RECORD);
    }

    /// A change that a split or a paste cut in two is two changes, a
    /// paragraph's and a run's formatting change included.
    #[test]
    fn no_change_stands_in_two_places() {
        let mut restyled = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![Piece::Text("abcd".into())],
                prop_change: Some(Box::new(wp_model::PropChange {
                    mark: by(1),
                    previous: wp_model::revision::PreviousProps::Run(Box::default()),
                })),
                ..Run::default()
            })],
            ..Paragraph::new()
        };
        restyled.prop_change = restyled_mark(2);
        let mut word = document(vec![Block::Paragraph(restyled)]);
        split_paragraph(&mut word, Scope::Body, &mut History::new(), at(0, 2), &me());
        let mut ids: Vec<u32> = tracked(&word)
            .iter()
            .filter(|change| change.what.contains("formatting"))
            .map(|change| change.mark.id)
            .collect();
        assert_eq!(ids.len(), 4, "{ids:?}");
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 4, "each its own");

        // One paragraph pasted inside another author's insertion.
        let mut word = document(vec![Block::Paragraph(Paragraph {
            content: vec![inserted_by_assistant(1, "abcd")],
            ..Paragraph::new()
        })]);
        paste_paragraphs(
            &mut word,
            Scope::Body,
            &mut History::new(),
            Selection::at(at(0, 2)),
            &[Paragraph::of("X")],
            &me(),
        )
        .expect("recorded");
        let mut ids: Vec<u32> = tracked(&word).iter().map(|change| change.mark.id).collect();
        let listed = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), listed, "{:?}", tracked(&word));
    }

    fn restyled_mark(id: u32) -> Option<Box<wp_model::PropChange>> {
        restyled(id, Some(HEADING))
    }

    /// Typed or pasted over a selection that held a tab, the new words follow
    /// the whole deletion, which is one change; and they follow a comment's
    /// start standing at the selection's start.
    #[test]
    fn what_replaces_a_selection_with_a_tab_follows_all_of_it() {
        let mut typed_over = document_with_tab();
        delete_range(
            &mut typed_over,
            Scope::Body,
            &mut History::new(),
            span(at(0, 0), at(0, 3)),
            &me(),
            false,
        )
        .expect("recorded");
        if let Block::Paragraph(paragraph) = &mut typed_over.body[0] {
            let after = record_replacement(paragraph, 0, "Z", &me(), 5, None).expect("recorded");
            assert_eq!(after, 2, "after the deleted tab, which takes a place");
        }
        let listed: Vec<&str> = tracked(&typed_over)
            .iter()
            .map(|change| change.what)
            .collect();
        assert_eq!(listed, ["deleted", "inserted"]);

        let mut pasted_over = document_with_tab();
        paste_paragraphs(
            &mut pasted_over,
            Scope::Body,
            &mut History::new(),
            span(at(0, 0), at(0, 3)),
            &[Paragraph::of("Z")],
            &me(),
        )
        .expect("recorded");
        let listed: Vec<(&str, u32)> = tracked(&pasted_over)
            .iter()
            .map(|change| (change.what, change.mark.id))
            .collect();
        assert_eq!(listed.len(), 2, "one deletion, one insertion: {listed:?}");
        assert_eq!(listed[0].0, "deleted");
        assert_eq!(listed[1].0, "inserted");

        // A comment starting where the selection starts: the new words are
        // inside it still.
        let mut commented = document(vec![Block::Paragraph(Paragraph {
            content: vec![
                Inline::Run(Run::of("x ")),
                Inline::Anchor(wp_model::Anchor::CommentStart { id: 7 }),
                Inline::Run(Run::of("old")),
                Inline::Anchor(wp_model::Anchor::CommentEnd { id: 7 }),
            ],
            ..Paragraph::new()
        })]);
        delete_range(
            &mut commented,
            Scope::Body,
            &mut History::new(),
            span(at(0, 2), at(0, 5)),
            &me(),
            false,
        )
        .expect("recorded");
        if let Block::Paragraph(paragraph) = &mut commented.body[0] {
            record_replacement(paragraph, 2, "new", &me(), 5, None).expect("recorded");
            let start = paragraph
                .content
                .iter()
                .position(|inline| {
                    matches!(
                        inline,
                        Inline::Anchor(wp_model::Anchor::CommentStart { .. })
                    )
                })
                .expect("the start");
            let new = paragraph
                .content
                .iter()
                .position(|inline| {
                    matches!(
                        inline,
                        Inline::Revised {
                            revision: Revision::Inserted(_),
                            ..
                        }
                    )
                })
                .expect("the new words");
            assert!(start < new, "{:?}", paragraph.content);
        }
    }

    /// With a bookmark's end standing between two paragraphs, a mark between
    /// them that goes settles in place, all at once or on its own, and the
    /// bookmark's end is not lost.
    #[test]
    fn a_mark_before_a_bookmarks_end_settles_in_place() {
        let mut first = para(None, "one");
        first.mark_revision = Some(Revision::Deleted(by(1)));
        let word = document(vec![
            Block::Paragraph(first),
            Block::Anchor(wp_model::Anchor::BookmarkEnd { id: 3 }),
            Block::Paragraph(para(None, "two")),
        ]);
        for how in [Resolve::Accept, Resolve::Reject] {
            let mut all = word.clone();
            resolve_all(&mut all, &mut History::new(), how);
            let mut one = word.clone();
            assert!(resolve_one(&mut one, &mut History::new(), &by(1), how));
            for document in [&all, &one] {
                assert_eq!(document.body.len(), 3, "{how:?}");
                assert!(matches!(document.body[1], Block::Anchor(_)));
                assert!(tracked(document).is_empty());
            }
        }
    }

    #[test]
    fn a_document_with_nothing_tracked_is_left_alone() {
        let mut document = document(vec![Block::Paragraph(Paragraph::of("plain"))]);
        let mut history = History::new();
        assert_eq!(resolve_all(&mut document, &mut history, Resolve::Accept), 0);
        assert!(!history.can_undo(), "nothing happened, so nothing to undo");
    }

    #[test]
    fn typing_with_track_changes_on_records_an_insertion() {
        let mut paragraph = Paragraph::of("hello world");
        let author = Author::new("Adnan Khan");
        let after = record_insertion(&mut paragraph, 5, " there", &author, 1).expect("recorded");
        assert_eq!(after, 11);
        assert_eq!(paragraph.text(), "hello there world");
        // And it is an insertion rather than ordinary text.
        let document = document(vec![Block::Paragraph(paragraph)]);
        let found = tracked(&document);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].what, "inserted");
        assert_eq!(found[0].mark.author.as_ref(), "Adnan Khan");
    }

    #[test]
    fn deleting_with_track_changes_on_keeps_the_text_and_marks_it() {
        // The difference between an editor that respects tracked changes and one
        // that quietly rewrites them.
        let mut paragraph = Paragraph::of("keep this away");
        let author = Author::new("Adnan Khan");
        // Bytes 5..10 are "this " — the word and the space after it, which is
        // what a word-wise delete covers.
        record_deletion(&mut paragraph, 5..10, &author, 2).expect("recorded");
        assert_eq!(paragraph.text(), "keep away", "gone from the text");
        assert_eq!(
            paragraph.shown_text(),
            "keep this away",
            "and still drawn, struck through"
        );
    }

    #[test]
    fn deleting_something_that_was_just_inserted_removes_it_rather_than_recording_it() {
        // Word does not record deleting text that was never in the document.
        let mut paragraph = Paragraph::of("kept ");
        let author = Author::new("A");
        record_insertion(&mut paragraph, 5, "new", &author, 1).expect("recorded");
        assert_eq!(paragraph.text(), "kept new");
        record_deletion(&mut paragraph, 5..8, &author, 2).expect("recorded");
        assert_eq!(paragraph.text(), "kept ");
        assert_eq!(paragraph.shown_text(), "kept ", "nothing left behind");
    }

    #[test]
    fn a_position_inside_a_hyperlink_refuses_rather_than_half_recording() {
        // A half-recorded change is worse than an unrecorded one: the user would
        // believe the rest was recorded too.
        let mut paragraph = Paragraph {
            content: vec![Inline::Hyperlink(Box::new(wp_model::Hyperlink {
                rel: None,
                anchor: Some("x".into()),
                tooltip: None,
                history: true,
                content: vec![Inline::Run(Run::of("linked"))],
            }))],
            ..Paragraph::new()
        };
        let author = Author::new("A");
        assert_eq!(record_insertion(&mut paragraph, 3, "X", &author, 1), None);
        assert_eq!(paragraph.text(), "linked", "and nothing was changed");
    }

    #[test]
    fn revision_ids_do_not_repeat() {
        let document = reviewed();
        assert_eq!(next_revision_id(&document), 3);
        assert_eq!(next_revision_id(&Document::new()), 1);
    }

    #[test]
    fn initials_come_from_the_name_when_nobody_supplies_them() {
        assert_eq!(Author::new("Adnan Khan").initials.as_ref(), "AK");
        assert_eq!(Author::new("Prince").initials.as_ref(), "P");
    }

    #[test]
    fn a_comment_gets_a_range_a_mark_and_a_body() {
        // All three, or Word reports the document as damaged.
        let mut document = document(vec![Block::Paragraph(Paragraph::of("some text"))]);
        let mut history = History::new();
        let selection = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 0,
            },
            head: Caret {
                paragraph: 0,
                offset: 4,
            },
        };
        let id = add_comment(
            &mut document,
            &mut history,
            Scope::Body,
            selection,
            "Adnan Khan",
            "AK",
            "Needs a citation.",
        );
        assert_eq!(document.comments.len(), 1);
        assert_eq!(document.comment(id).unwrap().text(), "Needs a citation.");

        let paragraph = &document.paragraphs()[0];
        assert!(paragraph.content.iter().any(|inline| matches!(
            inline,
            Inline::Anchor(wp_model::Anchor::CommentStart { .. })
        )));
        assert!(paragraph
            .content
            .iter()
            .any(|inline| matches!(inline, Inline::Anchor(wp_model::Anchor::CommentEnd { .. }))));
        assert!(paragraph
            .runs()
            .iter()
            .flat_map(|run| &run.content)
            .any(|piece| matches!(piece, Piece::CommentRef(_))));
        assert_eq!(document.text(), "some text", "and the text is untouched");
    }

    #[test]
    fn deleting_a_comment_takes_its_anchors_with_it() {
        // A comment part with anchors left behind is a document Word offers to
        // repair, which is worse than no comment at all.
        let mut document = document(vec![Block::Paragraph(Paragraph::of("some text"))]);
        let mut history = History::new();
        let id = add_comment(
            &mut document,
            &mut history,
            Scope::Body,
            Selection::at(Caret {
                paragraph: 0,
                offset: 0,
            }),
            "A",
            "A",
            "note",
        );
        assert!(delete_comment(&mut document, &mut history, id));
        assert!(document.comments.is_empty());
        let paragraph = &document.paragraphs()[0];
        assert!(!paragraph.content.iter().any(|inline| matches!(
            inline,
            Inline::Anchor(wp_model::Anchor::CommentStart { .. })
                | Inline::Anchor(wp_model::Anchor::CommentEnd { .. })
        )));
        assert!(!paragraph
            .runs()
            .iter()
            .flat_map(|run| &run.content)
            .any(|piece| matches!(piece, Piece::CommentRef(_))));
    }

    #[test]
    fn a_comment_knows_where_it_is_anchored() {
        let mut document = document(vec![
            Block::Paragraph(Paragraph::of("first")),
            Block::Paragraph(Paragraph::of("second")),
        ]);
        let mut history = History::new();
        let id = add_comment(
            &mut document,
            &mut history,
            Scope::Body,
            Selection::at(Caret {
                paragraph: 1,
                offset: 0,
            }),
            "A",
            "A",
            "note",
        );
        assert_eq!(
            comment_at(&document, id),
            Some((
                Scope::Body,
                Caret {
                    paragraph: 1,
                    offset: 0
                }
            ))
        );
        assert_eq!(comment_at(&document, 99), None);
    }
}
