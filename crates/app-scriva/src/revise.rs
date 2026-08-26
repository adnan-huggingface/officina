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
    pub paragraph: usize,
    pub mark: Mark,
    /// What the change is, in words a person can read.
    pub what: &'static str,
    /// The text it is about, trimmed for a list.
    pub text: String,
}

/// Lists the tracked changes, so they can be walked through one at a time.
pub fn tracked(document: &Document) -> Vec<Tracked> {
    let mut out = Vec::new();
    for (index, paragraph) in document.paragraphs().iter().enumerate() {
        walk(&paragraph.content, index, &mut out);
        // A paragraph *mark* can be inserted or deleted too — that is what a
        // tracked paragraph split or merge is, and it has no text of its own.
        if let Some(revision) = &paragraph.mark_revision {
            out.push(Tracked {
                paragraph: index,
                mark: revision.mark().clone(),
                what: match revision {
                    Revision::Inserted(_) => "paragraph break inserted",
                    _ => "paragraph break deleted",
                },
                text: String::new(),
            });
        }
    }
    out
}

fn walk(content: &[Inline], paragraph: usize, out: &mut Vec<Tracked>) {
    for inline in content {
        match inline {
            Inline::Revised { revision, content } => {
                let mut text = String::new();
                for inner in content {
                    collect_text(inner, &mut text);
                }
                out.push(Tracked {
                    paragraph,
                    mark: revision.mark().clone(),
                    what: match revision {
                        Revision::Inserted(_) => "inserted",
                        Revision::Deleted(_) => "deleted",
                        Revision::MovedFrom { .. } => "moved from",
                        Revision::MovedTo { .. } => "moved to",
                    },
                    text: text.trim().chars().take(60).collect(),
                });
                walk(content, paragraph, out);
            }
            Inline::Hyperlink(link) => walk(&link.content, paragraph, out),
            Inline::Structured(sdt) => walk(&sdt.content, paragraph, out),
            Inline::Wrapper { content, .. } | Inline::SimpleField { content, .. } => {
                walk(content, paragraph, out)
            }
            Inline::Run(run) => {
                if let Some(change) = &run.prop_change {
                    out.push(Tracked {
                        paragraph,
                        mark: change.mark.clone(),
                        what: "formatting changed",
                        text: run.text().trim().chars().take(60).collect(),
                    });
                }
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

/// Settles every tracked change in the document.
pub fn resolve_all(document: &mut Document, history: &mut History, how: Resolve) -> usize {
    let before: Vec<Paragraph> = document
        .paragraphs()
        .iter()
        .map(|paragraph| (*paragraph).clone())
        .collect();
    if before.is_empty() {
        return 0;
    }
    let count = tracked(document).len();
    if count == 0 {
        return 0;
    }
    let mut resolved: Vec<Paragraph> = before
        .iter()
        .map(|paragraph| settle_paragraph(paragraph, how, None))
        .collect();
    // A paragraph mark that was inserted and is being rejected — or deleted and
    // being accepted — joins its paragraph to the next one. Done back to front
    // so an earlier join does not move a later one.
    let mut index = resolved.len();
    while index > 1 {
        index -= 1;
        let joins = before[index - 1]
            .mark_revision
            .as_ref()
            .is_some_and(|revision| !how.keeps(revision));
        if joins {
            let tail = resolved.remove(index);
            let head = resolved[index - 1].clone();
            resolved[index - 1] = crate::text::merge(&head, &tail);
            resolved[index - 1].mark_revision = None;
        }
    }
    history.push(
        wp_model::Scope::Body,
        Change::Range {
            first: 0,
            before: before.clone(),
            // Rejected paragraph marks joined their paragraphs, so fewer may
            // stand here than were recorded.
            now: resolved.len(),
        },
    );
    crate::edit::replace_range(document, wp_model::Scope::Body, 0..before.len(), resolved);
    count
}

/// Settles one tracked change, named by its mark.
pub fn resolve_one(
    document: &mut Document,
    history: &mut History,
    mark: &Mark,
    how: Resolve,
) -> bool {
    let Some(found) = tracked(document).into_iter().find(|t| &t.mark == mark) else {
        return false;
    };
    let index = found.paragraph;
    let Some(before) = document.paragraphs().get(index).map(|p| (*p).clone()) else {
        return false;
    };
    // A paragraph mark's revision joins two paragraphs, which is a change to the
    // body rather than to one paragraph.
    if before
        .mark_revision
        .as_ref()
        .is_some_and(|revision| revision.mark() == mark)
    {
        if !how.keeps(before.mark_revision.as_ref().expect("just checked")) {
            let Some(next) = document.paragraphs().get(index + 1).map(|p| (*p).clone()) else {
                return false;
            };
            history.push(
                wp_model::Scope::Body,
                Change::Merge {
                    index,
                    first: Box::new(before.clone()),
                    second: Box::new(next.clone()),
                },
            );
            let mut joined = crate::text::merge(&before, &next);
            joined.mark_revision = None;
            crate::edit::replace_range(
                document,
                wp_model::Scope::Body,
                index..index + 2,
                vec![joined],
            );
            return true;
        }
        history.push(
            wp_model::Scope::Body,
            Change::Paragraph {
                index,
                before: Box::new(before.clone()),
            },
        );
        let mut kept = before;
        kept.mark_revision = None;
        crate::edit::replace_range(
            document,
            wp_model::Scope::Body,
            index..index + 1,
            vec![kept],
        );
        return true;
    }

    history.push(
        wp_model::Scope::Body,
        Change::Paragraph {
            index,
            before: Box::new(before.clone()),
        },
    );
    let settled = settle_paragraph(&before, how, Some(mark));
    crate::edit::replace_range(
        document,
        wp_model::Scope::Body,
        index..index + 1,
        vec![settled],
    );
    true
}

/// Applies `how` to a paragraph's revisions — all of them, or just one.
fn settle_paragraph(paragraph: &Paragraph, how: Resolve, only: Option<&Mark>) -> Paragraph {
    let mut settled = paragraph.clone();
    settled.content = settle(&paragraph.content, how, only);
    crate::text::prune(&mut settled);
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

/// Types `input` at the caret as a *tracked insertion*.
///
/// Returns the caret after it, or `None` when the position is inside something
/// this cannot wrap — a hyperlink, a content control, a field's result. The
/// caller falls back to an ordinary edit and says so, because a half-recorded
/// change is worse than an unrecorded one: the user would believe the rest was
/// recorded too.
pub fn record_insertion(
    paragraph: &mut Paragraph,
    offset: usize,
    input: &str,
    author: &Author,
    id: u32,
) -> Option<usize> {
    let at = top_level_split(paragraph, offset)?;
    let mut run = Run::of(input);
    run.props = crate::text::props_at(paragraph, offset);
    paragraph
        .content
        .insert(at, as_insertion_with(author, id, run));
    Some(offset + input.len())
}

fn as_insertion_with(author: &Author, id: u32, run: Run) -> Inline {
    Inline::Revised {
        revision: Revision::Inserted(author.mark(id)),
        content: vec![Inline::Run(run)],
    }
}

/// Marks a range as a *tracked deletion* rather than removing it.
///
/// The text stays in the document — struck through, and skipped by everything
/// that reads the text — which is the difference between an editor that respects
/// tracked changes and one that quietly rewrites them.
pub fn record_deletion(
    paragraph: &mut Paragraph,
    range: std::ops::Range<usize>,
    author: &Author,
    id: u32,
) -> Option<()> {
    if range.is_empty() {
        return Some(());
    }
    // Both ends cut at the top level, so the deletion covers whole inlines.
    //
    // The *start* first: cutting at the end and then at the start inserts the
    // new inline before the one the end named, so both indices come out the
    // same and the deletion covers nothing at all.
    let start = top_level_split(paragraph, range.start)?;
    let end = top_level_split(paragraph, range.end)?;
    if start > end {
        return None;
    }
    let covered: Vec<Inline> = paragraph.content.drain(start..end).collect();
    // Text that is already an insertion by the *same* edit is simply removed:
    // Word does not record deleting something that was never in the document.
    let mut kept = Vec::new();
    for inline in covered {
        match inline {
            Inline::Revised {
                revision: Revision::Inserted(_),
                ..
            } => {}
            other => kept.push(strike(other)),
        }
    }
    if !kept.is_empty() {
        paragraph.content.insert(
            start,
            Inline::Revised {
                revision: Revision::Deleted(author.mark(id)),
                content: kept,
            },
        );
    }
    Some(())
}

/// Turns a run's text into deleted text.
fn strike(inline: Inline) -> Inline {
    match inline {
        Inline::Run(run) => Inline::Run(Run {
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
        }),
        other => other,
    }
}

/// Cuts the paragraph's *top-level* inlines so that `offset` falls between two
/// of them, and returns which index that is.
///
/// `None` when the offset lands inside something that is not a plain run at the
/// top level. Recording a change inside a hyperlink or a content control means
/// wrapping part of that container, which is a different and much larger job.
fn top_level_split(paragraph: &mut Paragraph, offset: usize) -> Option<usize> {
    let mut seen = 0usize;
    for index in 0..paragraph.content.len() {
        let width = {
            let mut text = String::new();
            collect_text(&paragraph.content[index], &mut text);
            text.len()
        };
        if offset == seen {
            return Some(index);
        }
        if offset < seen + width {
            // Inside this one: it has to be a plain run to be cut.
            let Inline::Run(run) = &mut paragraph.content[index] else {
                return None;
            };
            let within = offset - seen;
            let tail = cut_run(run, within)?;
            paragraph.content.insert(index + 1, Inline::Run(tail));
            return Some(index + 1);
        }
        seen += width;
    }
    Some(paragraph.content.len())
}

/// Splits a run at a byte offset into its text, returning the tail.
fn cut_run(run: &mut Run, offset: usize) -> Option<Run> {
    let mut seen = 0usize;
    for index in 0..run.content.len() {
        let width = match &run.content[index] {
            Piece::Text(text) | Piece::Deleted(text) => text.len(),
            Piece::Tab | Piece::Symbol { .. } => 1,
            _ => 0,
        };
        if offset < seen + width {
            let within = offset - seen;
            if let Piece::Text(text) = &run.content[index] {
                let (head, tail) = (text[..within].to_string(), text[within..].to_string());
                run.content[index] = Piece::Text(head.into());
                let mut rest: Vec<Piece> = vec![Piece::Text(tail.into())];
                rest.extend(run.content.drain(index + 1..));
                run.content
                    .retain(|piece| !matches!(piece, Piece::Text(t) if t.is_empty()));
                return Some(Run {
                    props: run.props.clone(),
                    content: rest
                        .into_iter()
                        .filter(|piece| !matches!(piece, Piece::Text(t) if t.is_empty()))
                        .collect(),
                    prop_change: None,
                });
            }
            return None;
        }
        seen += width;
    }
    Some(Run {
        props: run.props.clone(),
        content: Vec::new(),
        prop_change: None,
    })
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
    let (start, end) = selection.ordered();

    let before: Vec<Paragraph> = document
        .paragraphs()
        .iter()
        .skip(start.paragraph)
        .take(end.paragraph - start.paragraph + 1)
        .map(|p| (*p).clone())
        .collect();
    history.push(
        wp_model::Scope::Body,
        Change::Range {
            first: start.paragraph,
            before: before.clone(),
            now: before.len(),
        },
    );

    let mut comment = wp_model::Comment::new(id, author);
    comment.initials = Some(initials.into());
    comment.content = vec![Block::Paragraph(Paragraph::of(text))];
    document.comments.push(comment);

    let mut after = before.clone();
    if let Some(first) = after.first_mut() {
        first
            .content
            .insert(0, Inline::Anchor(wp_model::Anchor::CommentStart { id }));
    }
    if let Some(last) = after.last_mut() {
        last.content
            .push(Inline::Anchor(wp_model::Anchor::CommentEnd { id }));
        last.content.push(Inline::Run(Run {
            content: vec![Piece::CommentRef(id)],
            ..Run::new()
        }));
    }
    crate::edit::replace_range(
        document,
        wp_model::Scope::Body,
        start.paragraph..start.paragraph + before.len(),
        after,
    );
    id
}

/// Removes a comment and the three marks that anchor it.
pub fn delete_comment(document: &mut Document, history: &mut History, id: u32) -> bool {
    if !document.comments.iter().any(|comment| comment.id == id) {
        return false;
    }
    let before: Vec<Paragraph> = document.paragraphs().iter().map(|p| (*p).clone()).collect();
    history.push(
        wp_model::Scope::Body,
        Change::Range {
            first: 0,
            before: before.clone(),
            now: before.len(),
        },
    );
    document.comments.retain(|comment| comment.id != id);

    let after: Vec<Paragraph> = before
        .iter()
        .map(|paragraph| {
            let mut paragraph = paragraph.clone();
            paragraph.content.retain(|inline| {
                !matches!(
                    inline,
                    Inline::Anchor(wp_model::Anchor::CommentStart { id: at })
                        | Inline::Anchor(wp_model::Anchor::CommentEnd { id: at })
                    if *at == id
                )
            });
            for inline in &mut paragraph.content {
                if let Inline::Run(run) = inline {
                    run.content
                        .retain(|piece| !matches!(piece, Piece::CommentRef(at) if *at == id));
                }
            }
            crate::text::prune(&mut paragraph);
            paragraph
        })
        .collect();
    crate::edit::replace_range(document, wp_model::Scope::Body, 0..before.len(), after);
    true
}

/// Where a comment is anchored, for drawing it beside its text.
pub fn comment_at(document: &Document, id: u32) -> Option<Caret> {
    for (index, paragraph) in document.paragraphs().iter().enumerate() {
        let mut offset = 0usize;
        for inline in &paragraph.content {
            match inline {
                Inline::Anchor(wp_model::Anchor::CommentStart { id: at }) if *at == id => {
                    return Some(Caret {
                        paragraph: index,
                        offset,
                    })
                }
                other => {
                    let mut text = String::new();
                    collect_text(other, &mut text);
                    offset += text.len();
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
            Some(Caret {
                paragraph: 1,
                offset: 0
            })
        );
        assert_eq!(comment_at(&document, 99), None);
    }
}
